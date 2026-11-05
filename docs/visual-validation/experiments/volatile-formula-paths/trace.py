"""Explore one dependency path to a random/time function without evaluating Excel.

This diagnostic prototype reads source cached values, not the values rendered
after a rebuild. A found path is a dependency observation, not a pass verdict.
"""
import argparse
from collections import deque
from dataclasses import dataclass
import json
from pathlib import PurePosixPath
import re
import xml.etree.ElementTree as ET
import zipfile

REFERENCE = re.compile(
    r"(?<![\w.])(?:(?:'(?P<quoted>(?:[^']|'')+)'|(?P<sheet>[A-Za-z_][\w.]*))!)?"
    r"(?P<column>\$?[A-Za-z]{1,3})(?P<row>\$?[1-9][0-9]{0,6})(?![\w.(])"
)
VOLATILE = re.compile(r"(?<![\w.])(?:_xlfn\.)?(RAND|RANDBETWEEN|RANDARRAY|NOW|TODAY)\s*\(", re.I)
STRING = re.compile(r'"(?:[^"]|"")*"')
LITERAL_INDIRECT = re.compile(r'\bINDIRECT\(\s*"((?:[^"]|"")*)"\s*(?:,\s*(?:TRUE|1)\s*)?\)', re.I)


def outside_strings(formula):
    """Preserve offsets while hiding Excel string literals from token matching."""
    return STRING.sub(lambda match: ' ' * len(match.group()), formula)


def coordinate(column, row):
    number = 0
    for letter in column.replace('$', '').upper():
        number = number * 26 + ord(letter) - ord('A') + 1
    height = int(row.replace('$', ''))
    if number > 16384 or height > 1048576:
        raise ValueError('Reference outside Excel worksheet bounds')
    return number, height


def address(column, row):
    letters = ''
    while column:
        column, digit = divmod(column - 1, 26)
        letters = chr(65 + digit) + letters
    return f'{letters}{row}'


def shifted_formula(formula, source, target):
    """Translate relative A1 tokens of a shared formula, preserving $ anchors."""
    source_match = REFERENCE.fullmatch(source)
    target_match = REFERENCE.fullmatch(target)
    sx, sy = coordinate(source_match['column'], source_match['row'])
    tx, ty = coordinate(target_match['column'], target_match['row'])
    edits = []
    for match in REFERENCE.finditer(outside_strings(formula)):
        x, y = coordinate(match['column'], match['row'])
        x += 0 if match['column'].startswith('$') else tx - sx
        y += 0 if match['row'].startswith('$') else ty - sy
        if not (1 <= x <= 16384 and 1 <= y <= 1048576):
            raise ValueError('Shared-formula translation leaves worksheet bounds')
        cell = address(x, y)
        letters = cell.rstrip('0123456789')
        replacement = ('$' if match['column'].startswith('$') else '') + letters
        replacement += ('$' if match['row'].startswith('$') else '') + str(y)
        edits.append((match.start('column'), match.end('row'), replacement))
    for start, end, replacement in reversed(edits):
        formula = formula[:start] + replacement + formula[end:]
    return formula


def volatile_span(formula):
    """Return the complete volatile call span, including nested parentheses."""
    masked = outside_strings(formula)
    match = VOLATILE.search(masked)
    if not match:
        return None
    depth = 1
    for position in range(match.end(), len(masked)):
        depth += (masked[position] == '(') - (masked[position] == ')')
        if depth == 0:
            return [match.start(), position + 1]
    return None


def references(formula, sheet, cells=None):
    """Read A1 dependencies and literal INDIRECT; never evaluate dynamic text.

    Ranges visit only indexed formula cells, so a million-row range does not
    allocate a million cell references. Non-formula cells cannot lead to RAND.
    """
    result = []
    literals = list(LITERAL_INDIRECT.finditer(formula))
    strings = list(STRING.finditer(formula))
    for literal in literals:
        if any(string.start() <= literal.start() < string.end() for string in strings):
            continue
        target = literal[1].replace('""', '"')
        if REFERENCE.fullmatch(target) is None:
            raise ValueError('INDIRECT literal is not a single A1 reference')
        result.extend(references(target, sheet))
        formula = formula[:literal.start()] + ' ' * len(literal.group()) + formula[literal.end():]
    masked = outside_strings(formula)
    if '[' in masked or ']' in masked:
        raise ValueError('External or structured reference is unsupported')
    if re.search(r'\b(?:INDIRECT|OFFSET)\s*\(', masked, re.I):
        raise ValueError('Dynamic reference is unsupported')
    consumed = 0
    for match in REFERENCE.finditer(masked):
        if match.start() < consumed:
            continue
        x, y = coordinate(match['column'], match['row'])
        target_sheet = match['quoted'] or match['sheet'] or sheet
        target_sheet = target_sheet.replace("''", "'")
        tail = match.end()
        if tail < len(masked) and masked[tail] == ':':
            end = REFERENCE.match(masked, tail + 1)
            if end is None or end['quoted'] or end['sheet'] or cells is None:
                raise ValueError('Range reference cannot be resolved')
            right, bottom = coordinate(end['column'], end['row'])
            consumed = end.end()
            for key in cells:
                if key[0] != target_sheet:
                    continue
                cell = REFERENCE.fullmatch(key[1])
                cx, cy = coordinate(cell['column'], cell['row'])
                if min(x, right) <= cx <= max(x, right) and min(y, bottom) <= cy <= max(y, bottom):
                    result.append(key)
        else:
            result.append((target_sheet, address(x, y)))
    return result


@dataclass
class Cell:
    formula: str
    value: str | None
    shared_anchor: str | None = None


def find_path(cells, start, max_nodes=10000):
    """Find a shortest supported dependency path; cycles never recurse forever."""
    queue = deque([(start, [])])
    seen = set()
    unresolved = []
    while queue and len(seen) < max_nodes:
        key, path = queue.popleft()
        if key in seen:
            continue
        seen.add(key)
        cell = cells.get(key)
        if cell is None:
            continue
        node = {'sheet': key[0], 'cell': key[1], 'formula': cell.formula,
                'source_cached_value': cell.value, 'shared_anchor': cell.shared_anchor}
        path = path + [node]
        span = volatile_span(cell.formula)
        if span:
            node['volatile_span'] = span
            return {'status': 'path_found', 'path': path, 'unresolved_branches': unresolved}
        try:
            queue.extend((reference, path) for reference in references(cell.formula, key[0], cells))
        except ValueError as error:
            unresolved.append({'cell': key, 'reason': str(error)})
    return {'status': 'not_resolved' if queue or unresolved else 'no_supported_path',
            'path': [], 'unresolved_branches': unresolved}


def workbook_formulas(path):
    """Read shared formulas in document order; keep only formula-bearing cells."""
    cells = {}
    with zipfile.ZipFile(path) as package:
        relationships = ET.fromstring(package.read('xl/_rels/workbook.xml.rels'))
        targets = {r.get('Id'): r.get('Target') for r in relationships}
        sheets = ET.fromstring(package.read('xl/workbook.xml')).find('{*}sheets')
        for sheet in sheets:
            rid = sheet.get('{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id')
            target = targets[rid]
            member = target.lstrip('/') if target.startswith('/') else str(PurePosixPath('xl') / target)
            anchors = {}
            with package.open(member) as stream:
                for _, element in ET.iterparse(stream, events=('end',)):
                    if element.tag.rsplit('}', 1)[-1] != 'c':
                        if element.tag.rsplit('}', 1)[-1] == 'row':
                            element.clear()
                        continue
                    formula = element.find('{*}f')
                    if formula is None:
                        element.clear()
                        continue
                    cell = element.get('r')
                    text = formula.text or ''
                    anchor = None
                    if formula.get('t') == 'shared':
                        index = formula.get('si')
                        if text:
                            anchors[index] = (cell, text)
                        elif index in anchors:
                            anchor, definition = anchors[index]
                            text = shifted_formula(definition, anchor, cell)
                        else:
                            raise ValueError(f'Shared formula {index} has no preceding anchor')
                    value = element.find('{*}v')
                    cells[(sheet.get('name'), cell)] = Cell(text, value.text if value is not None else None, anchor)
                    element.clear()
    return cells


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('workbook')
    parser.add_argument('sheet')
    parser.add_argument('cell')
    args = parser.parse_args()
    print(json.dumps(find_path(workbook_formulas(args.workbook), (args.sheet, args.cell)), indent=2))
