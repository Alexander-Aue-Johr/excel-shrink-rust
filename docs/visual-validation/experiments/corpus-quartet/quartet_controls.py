"""Make reproducible, visible numeric controls in prepared XLSX test copies.

Only a cell's <v> text is edited. The source corpus and shrink outputs remain
intact, and no XML document is serialized again by ElementTree.
"""

from copy import deepcopy
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
import hashlib
from io import BytesIO
import os
from pathlib import Path
import posixpath
import re
import tempfile
import xml.etree.ElementTree as ET
from zipfile import ZipFile


PROFILES = ('default', 'maximum', 'too-aggressive')
CONTROL_VALUES = ('101', '202', '303')
CELL_OPEN = re.compile(rb'<(?:[A-Za-z_][\w.-]*:)?c\b(?P<attrs>[^>]*)>')
CELL_CLOSE = re.compile(rb'</(?:[A-Za-z_][\w.-]*:)?c\s*>')
VALUE_TAG = re.compile(
    rb'<(?:[A-Za-z_][\w.-]*:)?v\s*>(?P<value>[^<]*)'
    rb'</(?:[A-Za-z_][\w.-]*:)?v\s*>')
CELL_ADDRESS = re.compile(r'^([A-Z]+)([1-9][0-9]*)$')
SCOPE = re.compile(r'^A1:([A-Z]+)([1-9][0-9]*)$')


@dataclass(frozen=True)
class NumericCell:
    address: str
    row: int
    column: int
    value: Decimal
    raw_value: str


def _column_number(letters):
    result = 0
    for letter in letters:
        result = result * 26 + ord(letter) - ord('A') + 1
    return result


def _coordinates(address):
    match = CELL_ADDRESS.fullmatch(address.replace('$', ''))
    if not match:
        raise ValueError(f'Invalid cell address: {address!r}')
    return int(match[2]), _column_number(match[1])


def _is_hidden(value):
    return value is not None and value.lower() in ('1', 'true')


def _sheet_xml(path, sheet_name):
    with ZipFile(path) as archive:
        book = ET.fromstring(archive.read('xl/workbook.xml'))
        sheets = book.find('{*}sheets')
        selected = next((item for item in sheets
                         if item.get('name') == sheet_name), None)
        if selected is None or selected.get('state', 'visible') != 'visible':
            raise ValueError(f'Visible worksheet {sheet_name!r} missing in {path}')
        relation = next(value for key, value in selected.attrib.items()
                        if key.endswith('}id'))
        rels = ET.fromstring(archive.read('xl/_rels/workbook.xml.rels'))
        target = next(item.get('Target') for item in rels
                      if item.get('Id') == relation)
        if target.startswith('/'):
            member = target.lstrip('/')
        else:
            member = posixpath.normpath(posixpath.join('xl', target))
        return member, archive.read(member)


def _numeric_cells(xml, bottom, right):
    """Return candidate cells that Excel should visibly render in the scope."""
    hidden_columns = []
    merged = []
    candidates = {}
    for _, element in ET.iterparse(BytesIO(xml), events=('end',)):
        tag = element.tag.rsplit('}', 1)[-1]
        if tag == 'col' and _is_hidden(element.get('hidden')):
            hidden_columns.append((int(element.get('min')),
                                   int(element.get('max'))))
        elif tag == 'row':
            row_number = int(element.get('r', '0'))
            if row_number <= bottom and not _is_hidden(element.get('hidden')):
                for cell in element:
                    if cell.tag.rsplit('}', 1)[-1] != 'c':
                        continue
                    address = cell.get('r')
                    if not address or cell.get('t', 'n') not in ('n', ''):
                        continue
                    row, column = _coordinates(address)
                    if row != row_number or column > right or any(
                            start <= column <= end for start, end in hidden_columns):
                        continue
                    if cell.find('{*}f') is not None:
                        continue  # Editing a formula cache is not a stable control.
                    value = cell.find('{*}v')
                    if value is None or value.text is None:
                        continue
                    try:
                        number = Decimal(value.text)
                    except InvalidOperation:
                        continue
                    if number.is_finite():
                        candidates[address] = NumericCell(
                            address, row, column, number, value.text)
            element.clear()
        elif tag == 'mergeCell':
            merged.append(element.get('ref'))
    for ref in merged:
        if not ref:
            continue
        start, _, end = ref.partition(':')
        top, left = _coordinates(start)
        bottom_row, right_column = _coordinates(end or start)
        for address, cell in tuple(candidates.items()):
            if top <= cell.row <= bottom_row and left <= cell.column <= right_column:
                del candidates[address]
    return candidates


def _replace_value(xml, address, old_value, new_value):
    """Change exactly one numeric <v> byte span, preserving all other XML."""
    changes = []
    address_bytes = address.encode('ascii')
    for cell in CELL_OPEN.finditer(xml):
        attributes = cell.group('attrs')
        match = re.search(rb'\br\s*=\s*(["\'])([^"\']+)\1', attributes)
        if not match or match[2] != address_bytes:
            continue
        if attributes.rstrip().endswith(b'/'):
            raise ValueError(f'Control cell {address} has no numeric value')
        close = CELL_CLOSE.search(xml, cell.end())
        if close is None or CELL_OPEN.search(xml, cell.end(), close.start()) is not None:
            raise ValueError(f'Malformed XML around control cell {address}')
        body = xml[cell.end():close.start()]
        if re.search(rb'<(?:[A-Za-z_][\w.-]*:)?f\b', body):
            raise ValueError(f'Control cell {address} unexpectedly has a formula')
        values = list(VALUE_TAG.finditer(body))
        if len(values) != 1 or values[0].group('value').decode('ascii') != old_value:
            raise ValueError(f'Control cell {address} changed before XML patch')
        start = cell.end() + values[0].start('value')
        end = cell.end() + values[0].end('value')
        changes.append((start, end))
    if len(changes) != 1:
        raise ValueError(f'Expected one XML cell {address}, found {len(changes)}')
    start, end = changes[0]
    return xml[:start] + new_value.encode('ascii') + xml[end:]


def _digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def _write_changed_zip(source, destination, member, changed_xml):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        raise FileExistsError(f'Control copy already exists: {destination}')
    with tempfile.NamedTemporaryFile(dir=destination.parent, suffix='.xlsx',
                                     delete=False) as temporary:
        temporary_path = Path(temporary.name)
    try:
        with ZipFile(source) as original, ZipFile(temporary_path, 'w') as output:
            for info in original.infolist():
                payload = changed_xml if info.filename == member else original.read(info)
                output.writestr(info, payload)
        with ZipFile(temporary_path) as check:
            if check.testzip() is not None or check.read(member) != changed_xml:
                raise RuntimeError(f'Invalid control workbook: {temporary_path}')
        os.replace(temporary_path, destination)
    finally:
        temporary_path.unlink(missing_ok=True)


def create_controls(entry, scope, directory, reuse_existing=False):
    """Return a modified entry and provenance for three variant-only controls.

    ``scope`` is the Office.js capture scope (``Sheet!A1:H400``). Source files
    are never edited. The control cell must have the same original numeric
    value in all four workbooks, so a visual difference has a single cause.
    """
    sheet_name, address = scope.rsplit('!', 1)
    match = SCOPE.fullmatch(address)
    if not match:
        raise ValueError(f'Unsupported control scope: {scope!r}')
    bottom, right = int(match[2]), _column_number(match[1])
    source = Path(entry['original']['path'])
    _, original_xml = _sheet_xml(source, sheet_name)
    original_cells = _numeric_cells(original_xml, bottom, right)
    variants = {}
    for profile in PROFILES:
        path = Path(entry[profile]['path'])
        member, xml = _sheet_xml(path, sheet_name)
        variants[profile] = (path, member, xml,
                             _numeric_cells(xml, bottom, right))
    control_numbers = {Decimal(value) for value in CONTROL_VALUES}
    common = [cell for address, cell in original_cells.items()
              if cell.value not in control_numbers and
              all(address in variants[profile][3] and
                  variants[profile][3][address].value == cell.value
                  for profile in PROFILES)]
    if not common:
        raise ValueError(f'No common visible numeric control cell in {scope}')
    middle_row, middle_column = (bottom + 1) / 2, (right + 1) / 2
    chosen = min(common, key=lambda cell: (
        abs(cell.row - middle_row) / max(bottom, 1)
        + abs(cell.column - middle_column) / max(right, 1),
        abs(cell.row - middle_row), abs(cell.column - middle_column),
        cell.address))
    values = CONTROL_VALUES
    new_entry = deepcopy(entry)
    provenance = {'sheet': sheet_name, 'cell': chosen.address,
                  'scope': scope, 'original_value': chosen.raw_value,
                  'original_sha256': _digest(source), 'variants': {}}
    directory = Path(directory)
    for profile, value in zip(PROFILES, values):
        path, member, xml, cells = variants[profile]
        target = directory / profile / path.name
        if reuse_existing:
            if not target.is_file():
                raise FileNotFoundError(f'Recorded control copy is missing: {target}')
        else:
            changed = _replace_value(xml, chosen.address,
                                     cells[chosen.address].raw_value, value)
            _write_changed_zip(path, target, member, changed)
        _, verified = _sheet_xml(target, sheet_name)
        selected = _numeric_cells(verified, bottom, right).get(chosen.address)
        if selected is None or selected.value != Decimal(value):
            target.unlink()
            raise RuntimeError(f'Control value verification failed: {target}')
        sha256 = _digest(target)
        new_entry[profile] = {**entry[profile], 'path': str(target),
                              'bytes': target.stat().st_size,
                              'sha256': sha256}
        provenance['variants'][profile] = {
            'value': value, 'source_path': str(path),
            'source_sha256': _digest(path), 'path': str(target),
            'sha256': sha256}
    return new_entry, provenance
