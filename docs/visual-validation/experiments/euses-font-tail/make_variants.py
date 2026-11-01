"""Controlled copies of the EUSES border/font tail; never modify the source."""
from copy import deepcopy
from pathlib import Path
import re
import zipfile
import xml.etree.ElementTree as ET

NS = {'x': 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}


def append_style_record(xml, container, record):
    """Append to a small style table without reserializing namespace declarations."""
    pattern = rb'<' + container + rb'\b[^>]*count="([0-9]+)"[^>]*>'
    match = re.search(pattern, xml)
    if match is None:
        raise ValueError(f'Missing style table: {container!r}')
    index = int(match[1])
    xml = xml[:match.start(1)] + str(index + 1).encode() + xml[match.end(1):]
    closing = b'</' + container + b'>'
    return xml.replace(closing, record + closing, 1), index


def build_variant_styles(styles, mode):
    """Append controlled font/fill records without changing existing identities."""
    if mode == 'height-only':
        return styles, {4: 4, 5: 5}
    parsed = ET.fromstring(styles)
    fonts = list(parsed.find('x:fonts', NS))
    xfs = list(parsed.find('x:cellXfs', NS))
    # Read small records with XML; write only their self-contained fragments.
    ET.register_namespace('', NS['x'])
    font = deepcopy(fonts[2])
    font.find('x:sz', NS).set('val', '30')
    font_bytes = ET.tostring(font).replace(b' xmlns="' + NS['x'].encode() + b'"', b'')
    styles, font_id = append_style_record(styles, b'fonts', font_bytes)
    fill_id = None
    if mode == 'blue':
        styles, fill_id = append_style_record(styles, b'fills',
            b'<fill><patternFill patternType="solid"><fgColor rgb="FF0070C0"/></patternFill></fill>')
    replacements = {}
    for old_id in (4, 5):
        xf = deepcopy(xfs[old_id])
        xf.set('fontId', str(font_id))
        if fill_id is not None:
            xf.set('fillId', str(fill_id))
            xf.set('applyFill', '1')
        record = ET.tostring(xf).replace(b' xmlns="' + NS['x'].encode() + b'"', b'')
        styles, replacements[old_id] = append_style_record(styles, b'cellXfs', record)
    return styles, replacements


def create_variant(source, output, mode):
    """Change only selected empty-tail formatting and an optional content anchor."""
    if source.resolve() == output.resolve():
        raise ValueError('A variant must not overwrite its source workbook.')
    with zipfile.ZipFile(source) as archive:
        parts = {name: archive.read(name) for name in archive.namelist()}
    styles, replacements = build_variant_styles(parts['xl/styles.xml'], mode)
    sheet = parts['xl/worksheets/sheet1.xml']

    def replace_empty_cell(match):
        column, row, old_id = match[1], int(match[2]), int(match[3])
        if row < 53 or mode == 'height-only':
            return match[0]
        if mode == 'single' and (row != 1000 or column != b'A'):
            return match[0]
        return match[0].replace(b's="' + match[3] + b'"', f's="{replacements[old_id]}"'.encode())

    sheet = re.sub(rb'<c r="([AB])([0-9]+)" s="([45])"\s*/>', replace_empty_cell, sheet)
    if mode in ('below', 'one-sentinel', 'no-sentinel'):
        sheet = sheet.replace(b'</sheetData>',
            b'<row r="65446"><c r="C65446" t="inlineStr"><is><t>Content below the empty rows</t></is></c></row></sheetData>')
        sheet = re.sub(rb'<dimension ref="[^"]+"', b'<dimension ref="A1:D65446"', sheet, count=1)
    if mode in ('one-sentinel', 'no-sentinel'):
        def remove_sentinel(match):
            if int(match[2]) < 53:
                return match[0]
            return b'' if mode == 'no-sentinel' or match[1] == b'B' else match[0]
        sheet = re.sub(rb'<c r="([AB])([0-9]+)" s="[0-9]+"\s*/>', remove_sentinel, sheet)
    if mode in ('fixed', 'height-only'):
        def fix_height(match):
            if int(match[1]) < 53:
                return match[0]
            return match[0][:-1] + b' ht="40" customHeight="1">'
        sheet = re.sub(rb'<row r="([0-9]+)"[^>]*>', fix_height, sheet)
    if mode == 'hidden':
        sheet = re.sub(rb'<col min="([12])" max="\1"[^>]*/>',
                       lambda m: m[0][:-2] + b' hidden="1"/>', sheet)
    parts['xl/styles.xml'] = styles
    parts['xl/worksheets/sheet1.xml'] = sheet
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name, data in parts.items():
            archive.writestr(name, data)


if __name__ == '__main__':
    import sys
    source, directory = map(Path, sys.argv[1:])
    for mode in ('single', 'all', 'below', 'one-sentinel', 'no-sentinel',
                 'fixed', 'height-only', 'hidden', 'blue'):
        create_variant(source, directory / f'{mode}.xlsx', mode)
