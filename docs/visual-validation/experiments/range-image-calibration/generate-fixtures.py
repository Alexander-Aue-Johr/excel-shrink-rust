#!/usr/bin/env python3
"""Deterministic diagnostic XLSX fixtures, assembled directly from OOXML.
No spreadsheet library, no external data, no formula or font downloads.
Run from the repository root with --dry-run or --apply to (re)create fixtures.
"""
from __future__ import annotations
import argparse
import hashlib
import io
import json
from pathlib import Path
import posixpath
import zipfile
from xml.etree import ElementTree as ET
from xml.sax.saxutils import escape, quoteattr

NS = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
PKG = 'http://schemas.openxmlformats.org/package/2006/relationships'
OUT = 'docs/visual-validation/experiments/range-image-calibration/fixtures'
WIDTHS = [0.5, 1, 2, 5, 8.43, 8.7109375, 10, 11.42578125, 12, 12.5703125, 20, 30, 35.7109375]
HEIGHTS = [8.25, 11.25, 12, 12.6, 12.75, 15, 15.95, 18, 24, 30, 72]
COLORS = ['E6194B', '3CB44B', '4363D8', 'F58231', '911EB4', '42D4F4',
          'F032E6', 'BFEF45', 'FABED4', '469990', '9A6324', '800000']

def col(n: int) -> str:
    s = ''
    while n:
        n, r = divmod(n - 1, 26)
        s = chr(65 + r) + s
    return s

def xml(text: str) -> str:
    return '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n' + text + '\n'

def styles(font: str, size: int) -> str:
    fills = '<fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill>'
    fills += ''.join(f'<fill><patternFill patternType="solid"><fgColor rgb="FF{c}"/><bgColor indexed="64"/></patternFill></fill>' for c in COLORS)
    def xf(fill: int = 0, border: int = 0) -> str:
        return f'<xf numFmtId="0" fontId="0" fillId="{fill}" borderId="{border}" xfId="0" applyFill="1" applyBorder="1"/>'
    # 0 default, 1 thin borders, 2..13 marker fills.
    xfs = xf() + xf(border=1) + ''.join(xf(fill=i + 2) for i in range(len(COLORS)))
    return xml(f'<styleSheet xmlns="{NS}"><fonts count="1"><font><sz val="{size}"/><color rgb="FF000000"/><name val={quoteattr(font)}/><family val="2"/></font></fonts>'
        f'<fills count="{len(COLORS) + 2}">{fills}</fills><borders count="2">'
        '<border><left/><right/><top/><bottom/><diagonal/></border>'
        '<border><left style="thin"><color rgb="FF000000"/></left><right style="thin"><color rgb="FF000000"/></right>'
        '<top style="thin"><color rgb="FF000000"/></top><bottom style="thin"><color rgb="FF000000"/></bottom><diagonal/></border></borders>'
        '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
        f'<cellXfs count="{len(COLORS) + 2}">{xfs}</cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>'
        '<dxfs count="0"/><tableStyles count="0" defaultTableStyle="TableStyleMedium2" defaultPivotStyle="PivotStyleLight16"/></styleSheet>')

def sheet(widths: list[float], heights: list[float], *, grid: bool = False,
          zoom: int = 100, border: bool = False, hidden_rows: tuple[int, ...] = (),
          hidden_cols: tuple[int, ...] = (), explicit_rows: bool = True,
          explicit_cols: bool = True, markers: list[dict] | None = None,
          marker_axis: str = 'y') -> str:
    ncols, nrows = len(widths), len(heights)
    columns = ''
    if explicit_cols:
        columns = '<cols>' + ''.join(f'<col min="{i}" max="{i}" width="{w}" customWidth="1"' + (' hidden="1"' if i in hidden_cols else '') + '/>' for i, w in enumerate(widths, 1)) + '</cols>'
    markers = markers or []
    body = []
    for r, h in enumerate(heights, 1):
        attrs = f'r="{r}"' + (f' ht="{h}" customHeight="1"' if explicit_rows else '') + (' hidden="1"' if r in hidden_rows else '')
        cells = []
        # Tall fixtures stay small: sparse rows, complete solid bands across all columns.
        row_marker = next((m for m in markers if marker_axis == 'y' and m['start_index'] <= r <= m['end_index']), None)
        use_cols = range(1, ncols + 1) if nrows < 500 or row_marker or r in (1, nrows) else (1,)
        for c in use_cols:
            marker = row_marker or next((m for m in markers if marker_axis == 'x' and m['start_index'] <= c <= m['end_index']), None)
            style = marker['style'] if marker else 1 if border else 0
            # The marker's sampled cross-section is blank; text cannot cover its fill.
            value = str(r) if c == 1 else '0'
            if marker:
                cells.append(f'<c r="{col(c)}{r}" s="{style}"/>')
            else:
                cells.append(f'<c r="{col(c)}{r}" s="{style}"><v>{value}</v></c>')
        body.append(f'<row {attrs}>' + ''.join(cells) + '</row>')
    return xml(f'<worksheet xmlns="{NS}" xmlns:r="{REL}"><dimension ref="A1:{col(ncols)}{nrows}"/>'
        f'<sheetViews><sheetView workbookViewId="0" showGridLines="{int(grid)}" zoomScale="{zoom}" zoomScaleNormal="{zoom}"><selection activeCell="A1" sqref="A1"/></sheetView></sheetViews>'
        f'<sheetFormatPr baseColWidth="8" defaultColWidth="{widths[0]}" defaultRowHeight="{heights[0]}" customHeight="1"/>'
        + columns + '<sheetData>' + ''.join(body) + '</sheetData></worksheet>')

def pack(font: str, size: int, sheets: list[tuple[str, str]]) -> bytes:
    parts = {
        '[Content_Types].xml': xml('<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
          '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>'
          '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
          '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>'
          + ''.join(f'<Override PartName="/xl/worksheets/sheet{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' for i in range(1, len(sheets) + 1)) + '</Types>'),
        '_rels/.rels': xml(f'<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>'),
        'xl/workbook.xml': xml(f'<workbook xmlns="{NS}" xmlns:r="{REL}"><bookViews><workbookView activeTab="0"/></bookViews><sheets>'
          + ''.join(f'<sheet name={quoteattr(name)} sheetId="{i}" r:id="rId{i}"/>' for i, (name, _) in enumerate(sheets, 1)) + '</sheets></workbook>'),
        'xl/_rels/workbook.xml.rels': xml(f'<Relationships xmlns="{PKG}">' + ''.join(f'<Relationship Id="rId{i}" Type="{REL}/worksheet" Target="worksheets/sheet{i}.xml"/>' for i in range(1, len(sheets) + 1))
          + f'<Relationship Id="rId{len(sheets) + 1}" Type="{REL}/styles" Target="styles.xml"/></Relationships>'),
        'xl/styles.xml': styles(font, size),
    }
    for i, (_, source) in enumerate(sheets, 1):
        parts[f'xl/worksheets/sheet{i}.xml'] = source
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name, source in sorted(parts.items()):
            ET.fromstring(source)
            info = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, source.encode('utf-8'), compresslevel=9)
    data = buffer.getvalue()
    validate_package(data)
    return data

def validate_package(data: bytes) -> None:
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        assert z.testzip() is None
        names = set(z.namelist())
        assert len(names) == len(z.namelist())
        for name in names:
            doc = ET.fromstring(z.read(name))
            if name.endswith('.rels'):
                base = '' if name == '_rels/.rels' else posixpath.dirname(posixpath.dirname(name))
                for rel in doc:
                    target = posixpath.normpath(posixpath.join(base, rel.attrib['Target']))
                    assert target in names, (name, target)
        main = ET.fromstring(z.read('xl/workbook.xml'))
        sheet_names = [v.attrib['name'] for v in main.find(f'{{{NS}}}sheets')]
        assert len(sheet_names) == len(set(sheet_names))

def build_all() -> dict[str, bytes]:
    output: dict[str, bytes] = {}
    fixtures = []
    for slug, font, size in [('calibri11', 'Calibri', 11), ('arial10', 'Arial', 10), ('calibri16', 'Calibri', 16)]:
        name = f'vv-render-{slug}.xlsx'
        sheets = [(n, sheet(WIDTHS, HEIGHTS, **opts)) for n, opts in [
            ('Units', {}), ('Zoom150', {'zoom': 150}), ('GridOn', {'grid': True}), ('Borders', {'border': True})]]
        sheets += [('Fractional', sheet([8.43] * 4, [12.6] * 1000)),
                   ('Defaults', sheet([8.43] * 4, [12.6] * 1000, explicit_rows=False, explicit_cols=False)),
                   ('Hidden', sheet([10] * 8, [12.75] * 8, hidden_rows=(3, 4), hidden_cols=(3, 4)))]
        data = pack(font, size, sheets)
        output[f'{OUT}/{name}'] = data
        fixtures.append({'workbook': name, 'kind': 'units', 'normal_font': font, 'normal_font_size_pt': size,
            'widths': WIDTHS, 'heights': HEIGHTS, 'fractional_row_height_pt': 12.6,
            'fractional_column_width_xml': 8.43, 'sha256': hashlib.sha256(data).hexdigest()})
    tall_starts = [1, 128, 512, 1024, 1500, 1900, 2100, 2500, 3000, 3500, 4000, 4092]
    wide_starts = [1, 32, 128, 256, 384, 448, 480, 512, 600, 700, 750, 796]
    def bands(starts: list[int]) -> list[dict]:
        return [{'name': f'band-{i+1}-at-{start}', 'start_index': start, 'end_index': start + 3,
                 'rgb': COLORS[i], 'style': i + 2} for i, start in enumerate(starts)]
    tall, wide = bands(tall_starts), bands(wide_starts)
    name = 'vv-render-markers.xlsx'
    data = pack('Calibri', 11, [
        ('Tall', sheet([10] * 16, [12.75] * 4096, markers=tall)),
        ('Wide', sheet([10] * 800, [12.75] * 4, markers=wide, marker_axis='x'))])
    output[f'{OUT}/{name}'] = data
    fixtures.append({'workbook': name, 'kind': 'markers', 'normal_font': 'Calibri', 'normal_font_size_pt': 11,
        'row_height_pt': 12.75, 'column_width_xml': 10, 'tall_rows': 4096, 'wide_columns': 800,
        'tall_markers': tall, 'wide_markers': wide, 'sha256': hashlib.sha256(data).hexdigest()})
    manifest = {'schema': 'render-diagnostics-fixtures-v2', 'created_from': 'minimal OOXML, not a transformed corpus workbook',
        'fixtures': fixtures, 'sources': [
            'https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.column',
            'https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.row',
            'https://learn.microsoft.com/en-us/javascript/api/excel/excel.range']}
    output[f'{OUT}/manifest.json'] = (json.dumps(manifest, ensure_ascii=False, indent=2) + '\n').encode('utf-8')
    return output

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--dry-run', action='store_true', help='Validate generated OOXML; write nothing.')
    mode.add_argument('--apply', action='store_true', help='Create fixtures; refuse different existing contents.')
    args = parser.parse_args()
    root = Path.cwd()
    if not (root / 'docs/visual-validation/experiments').is_dir():
        parser.error('Run from the repository root.')
    planned = build_all()
    for relative, data in planned.items():
        path = root / relative
        if path.exists() and path.read_bytes() != data:
            parser.error('Refusing to overwrite changed fixture: ' + relative)
    for relative, data in planned.items():
        path = root / relative
        action = 'UNCHANGED' if path.exists() else 'CREATE'
        print(action, relative, len(data))
        if args.apply and not path.exists():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
