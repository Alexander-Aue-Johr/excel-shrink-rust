"""Fixture-specific experiment: deduplicate empty Arial 8 pt cells per row.

This is not a general cleanup policy. It targets only styles 29 and 49 in the
two export sheets of Electricity_2520Meters_25_A866E.xls. All other styles,
row properties, column properties, and workbook parts remain unchanged.
"""

import argparse
from pathlib import Path
import re
import xml.etree.ElementTree as ET
import zipfile

NAMESPACE = {'x': 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}
TARGET_PARTS = ('xl/worksheets/sheet9.xml', 'xl/worksheets/sheet10.xml')
REMOVABLE_STYLES = {29, 49}
ROW = re.compile(rb'<row\b[^>]*>.*?</row>', re.DOTALL)
CELL = re.compile(rb'<c\b[^>]*(?:/>|>.*?</c>)', re.DOTALL)
STYLE = re.compile(rb'\bs="([0-9]+)"')


def cell_style(cell):
    match = STYLE.search(cell.split(b'>', 1)[0])
    return int(match[1]) if match else 0


def deduplicate_row(row, font_ids):
    """Retain an existing font-7 cell before removing identical-font peers."""
    cells = list(CELL.finditer(row))
    represented = any(
        font_ids[cell_style(cell[0])] == 7
        and (not cell[0].endswith(b'/>') or cell_style(cell[0]) not in REMOVABLE_STYLES)
        for cell in cells
    )
    removed = 0

    def replace_cell(match):
        nonlocal represented, removed
        cell = match[0]
        if cell_style(cell) not in REMOVABLE_STYLES or not cell.endswith(b'/>'):
            return cell
        if not represented:
            represented = True
            return cell
        removed += 1
        return b''

    return CELL.sub(replace_cell, row), removed


def create_candidate(source, output):
    if source.resolve() == output.resolve():
        raise ValueError('The candidate must not overwrite its input workbook.')
    with zipfile.ZipFile(source) as archive:
        parts = {name: archive.read(name) for name in archive.namelist()}
    styles = ET.fromstring(parts['xl/styles.xml'])
    xfs = list(styles.find('x:cellXfs', NAMESPACE))
    font_ids = [int(xf.get('fontId', '0')) for xf in xfs]
    for index in REMOVABLE_STYLES:
        xf = xfs[index]
        if font_ids[index] != 7 or xf.get('fillId') != '0' or xf.get('borderId') != '0' or len(xf):
            raise ValueError(f'The fixture no longer has the expected plain style {index}.')
    counts = {}
    for part in TARGET_PARTS:
        removed = 0

        def replace_row(match):
            nonlocal removed
            row, count = deduplicate_row(match[0], font_ids)
            removed += count
            return row

        parts[part] = ROW.sub(replace_row, parts[part])
        counts[part] = removed
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name, data in parts.items():
            archive.writestr(name, data)
    return counts


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    arguments = parser.parse_args()
    print(create_candidate(arguments.source, arguments.output))
