#!/usr/bin/env python3
"""Deterministic OOXML cases for automatic versus fixed row heights.
Run from the repository root with --dry-run or --apply. No Excel or third-party library.
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
OUT = 'docs/visual-validation/experiments/range-image-calibration/row-height-fixtures'
SCHEMA = 'row-height-fixtures-v1'

def xml(s):
    return '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n' + s + '\n'

def styles(normal_size):
    # IDs: 0 inherited Normal; 1=8 pt; 2=11 pt; 3=24 pt; 4=48 pt;
    # 5=8 pt wrapped; 6=24 pt wrapped. All are Calibri; no font-family confound.
    fonts = ''.join(f'<font><sz val="{s}"/><color rgb="FF000000"/><name val="Calibri"/><family val="2"/></font>'
                    for s in [normal_size, 8, 11, 24, 48])
    xfs = []
    for f, wrap in [(0, False), (1, False), (2, False), (3, False), (4, False), (1, True), (3, True)]:
        xfs.append(f'<xf numFmtId="0" fontId="{f}" fillId="0" borderId="0" xfId="0" applyFont="1" applyAlignment="1">'
                   f'<alignment vertical="bottom" wrapText="{int(wrap)}"/></xf>')
    return xml(f'<styleSheet xmlns="{NS}"><fonts count="5">{fonts}</fonts>'
       '<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>'
       '<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>'
       '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
       f'<cellXfs count="7">{"".join(xfs)}</cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>'
       '<dxfs count="0"/></styleSheet>')

def cell(address, text=None, style=None, number=None):
    attr = f'r="{address}"' + (f' s="{style}"' if style is not None else '')
    if number is not None:
        return f'<c {attr}><v>{number}</v></c>'
    if text is None:
        return f'<c {attr}/>'
    return f'<c {attr} t="inlineStr"><is><t xml:space="preserve">{escape(text)}</t></is></c>'

def cases():
    # Labels stay OUTSIDE measured cells, in manifest.json. No annotation font affects a case.
    return [
        {'id': 'no-row-no-cell', 'omit_row': True},
        {'id': 'row-only'},
        {'id': 'blank-cell-no-style', 'cells': [('A', None, None, None)]},
        {'id': 'text-inherits-normal', 'cells': [('A', 'Hgj', None, None)]},
        {'id': 'number-inherits-normal', 'cells': [('A', None, None, 123)]},
        {'id': 'explicit-normal-text', 'cells': [('A', 'Hgj', 0, None)]},
        {'id': 'blank-font8', 'cells': [('A', None, 1, None)]},
        {'id': 'text-font8', 'cells': [('A', 'Hgj', 1, None)]},
        {'id': 'text-font11', 'cells': [('A', 'Hgj', 2, None)]},
        {'id': 'blank-font24', 'cells': [('A', None, 3, None)]},
        {'id': 'text-font24', 'cells': [('A', 'Hgj', 3, None)]},
        {'id': 'blank-font48', 'cells': [('A', None, 4, None)]},
        {'id': 'text-font48', 'cells': [('A', 'Hgj', 4, None)]},
        {'id': 'mixed-font8-and48', 'cells': [('A', 'Hgj', 1, None), ('C', 'Hgj', 4, None)]},
        {'id': 'row-font48-no-cell', 'row_style': 4},
        {'id': 'row-font48-inherited-text', 'row_style': 4, 'cells': [('A', 'Hgj', None, None)]},
        {'id': 'row-font48-cell-font8', 'row_style': 4, 'cells': [('A', 'Hgj', 1, None)]},
        {'id': 'wrap-font8-three-lines', 'cells': [('A', 'Hgj\nHgj\nHgj', 5, None)]},
        {'id': 'wrap-font24-two-lines', 'cells': [('A', 'Hgj\nHgj', 6, None)]},
    ]

def sheet_xml(rows, total_rows, default_height=12.6, sheet_custom=None, width=40):
    flag = '' if sheet_custom is None else f' customHeight="{sheet_custom}"'
    return xml(f'<worksheet xmlns="{NS}" xmlns:r="{REL}"><dimension ref="A1:D{total_rows}"/>'
       '<sheetViews><sheetView workbookViewId="0" showGridLines="0" zoomScale="100" zoomScaleNormal="100">'
       '<selection activeCell="A1" sqref="A1"/></sheetView></sheetViews>'
       f'<sheetFormatPr baseColWidth="8" defaultColWidth="{width}" defaultRowHeight="{default_height}"{flag}/>'
       f'<cols><col min="1" max="4" width="{width}" customWidth="1"/></cols>'
       '<sheetData>' + ''.join(rows) + '</sheetData></worksheet>')

def matrix(name, default_height, sheet_custom=None, row_height=None, row_custom=None):
    rows, definitions = [], []
    for i, case in enumerate(cases(), 1):
        attrs = f'r="{i}"'
        if row_height is not None: attrs += f' ht="{row_height}"'
        if row_custom is not None: attrs += f' customHeight="{row_custom}"'
        if 'row_style' in case: attrs += f' s="{case["row_style"]}" customFormat="1"'
        row_present = not case.get('omit_row') or row_height is not None or row_custom is not None
        if row_present:
            contents = ''.join(cell(f'{c}{i}', text, sty, number) for c, text, sty, number in case.get('cells', []))
            rows.append(f'<row {attrs}>{contents}</row>')
        definitions.append({**case, 'row': i, 'xml_row_present': bool(row_present),
                            'xml_ht': row_height, 'xml_custom_height': row_custom})
    return (name, sheet_xml(rows, len(definitions), default_height, sheet_custom)), {
        'sheet': name, 'default_row_height_pt': default_height, 'sheet_custom_height': sheet_custom,
        'row_ht': row_height, 'row_custom_height': row_custom, 'cases': definitions,
        'address': f'A1:D{len(definitions)}', 'row_count': len(definitions), 'last_column': 'D'
    }

def uniform(name, height, *, row_height=None, sheet_custom=None, row_custom=None, count=1000):
    rows = []
    for i in range(1, count + 1):
        attrs = f'r="{i}"'
        if row_height is not None: attrs += f' ht="{row_height}"'
        if row_custom is not None: attrs += f' customHeight="{row_custom}"'
        rows.append(f'<row {attrs}>{cell(f"A{i}", number=i)}</row>')
    return name, sheet_xml(rows, count, height, sheet_custom, width=8.43)

def pack(sheets, normal_size):
    parts = {
        '[Content_Types].xml': xml('<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
            '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>'
            + ''.join(f'<Override PartName="/xl/worksheets/sheet{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
                      for i in range(1, len(sheets)+1)) + '</Types>'),
        '_rels/.rels': xml(f'<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>'),
        'xl/workbook.xml': xml(f'<workbook xmlns="{NS}" xmlns:r="{REL}"><bookViews><workbookView activeTab="0"/></bookViews><sheets>'
            + ''.join(f'<sheet name={quoteattr(name)} sheetId="{i}" r:id="rId{i}"/>' for i,(name,_) in enumerate(sheets,1)) + '</sheets></workbook>'),
        'xl/_rels/workbook.xml.rels': xml(f'<Relationships xmlns="{PKG}">'
            + ''.join(f'<Relationship Id="rId{i}" Type="{REL}/worksheet" Target="worksheets/sheet{i}.xml"/>' for i in range(1,len(sheets)+1))
            + f'<Relationship Id="rId{len(sheets)+1}" Type="{REL}/styles" Target="styles.xml"/></Relationships>'),
        'xl/styles.xml': styles(normal_size),
    }
    parts.update({f'xl/worksheets/sheet{i}.xml': content for i,(_,content) in enumerate(sheets,1)})
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name, content in sorted(parts.items()):
            ET.fromstring(content)
            entry = zipfile.ZipInfo(name, (2026,1,1,0,0,0)); entry.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(entry, content.encode('utf-8'), compresslevel=9)
    data = stream.getvalue(); validate_package(data); return data

def validate_package(data):
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        if z.testzip() is not None: raise ValueError('ZIP CRC failure')
        names = set(z.namelist())
        if len(names) != len(z.namelist()): raise ValueError('Duplicate ZIP member')
        for name in names:
            root = ET.fromstring(z.read(name))
            if name.endswith('.rels'):
                base = '' if name == '_rels/.rels' else posixpath.dirname(posixpath.dirname(name))
                for relationship in root:
                    if relationship.get('TargetMode') == 'External': raise ValueError('External link not allowed')
                    if posixpath.normpath(posixpath.join(base, relationship.get('Target'))) not in names:
                        raise ValueError('Broken package relationship')
        wb = ET.fromstring(z.read('xl/workbook.xml'))
        sheets = wb.find(f'{{{NS}}}sheets')
        if len({s.get('name') for s in sheets}) != len(sheets): raise ValueError('Duplicate worksheet name')

def build_all():
    files, fixtures = {}, []
    cover = ('Cover', sheet_xml(['<row r="1" ht="30" customHeight="1">' + cell('A1', 'Row-height diagnostics: see manifest.json for case labels.') + '</row>'], 1, 15))
    def add(name, normal_size, sheets, **meta):
        data = pack([cover] + sheets, normal_size)
        files[f'{OUT}/{name}'] = data
        fixtures.append({'workbook': name, 'kind': 'row-heights', 'normal_font': 'Calibri',
            'normal_font_size_pt': normal_size, 'sha256': hashlib.sha256(data).hexdigest(), **meta})
    specs = [('Auto126',12.6,None,None,None), ('Auto300',30,None,None,None),
             ('SheetCustom126',12.6,1,None,None), ('SheetCustom0',12.6,0,None,None),
             ('HtNoFlag126',12.6,None,12.6,None), ('RowCustom0',12.6,None,12.6,0),
             ('RowFixed126',12.6,None,12.6,1), ('RowFixed300',30,None,30,1)]
    for size in [11,16]:
        selection = specs if size == 11 else [specs[0], specs[2], specs[6]]
        pairs = [matrix(*spec) for spec in selection]
        sheets, definitions = [p[0] for p in pairs], [p[1] for p in pairs]
        runtime = []
        if size == 11:
            for name, custom in [('RuntimeAuto',False),('RuntimeFixed',True)]:
                attrs = ' ht="12.6" customHeight="1"' if custom else ''
                sheets.append((name, sheet_xml([f'<row r="1"{attrs}>{cell("A1", "Hgj", 2)}</row>'], 1, 12.6)))
                runtime.append({'sheet':name, 'address':'A1:D1', 'row_count':1, 'last_column':'D',
                    'default_row_height_pt':12.6, 'row_ht':12.6 if custom else None,
                    'row_custom_height':1 if custom else None})
        add(f'vv-row-height-calibri{size}.xlsx', size, sheets, matrices=definitions, runtime_controls=runtime)
    sheets, geometry = [], []
    for label, height, shcustom, rh, rcustom in [('Defaults',12.6,1,None,None), ('Auto',12.6,None,None,None),
                                             ('Fixed126',12.6,None,12.6,1), ('Fixed1275',12.75,None,12.75,1)]:
        for order, prefix in [('geometry-first','Geo'),('image-first','Img')]:
            name = prefix + label
            sheets.append(uniform(name,height,row_height=rh,sheet_custom=shcustom,row_custom=rcustom))
            geometry.append({'sheet':name,'pair':label,'measurement_order':order,'address':'A1:A1000',
                'row_count':1000,'last_column':'A','default_row_height_pt':height,
                'sheet_custom_height':shcustom,'row_ht':rh,'row_custom_height':rcustom})
    heights = [round(11.95 + i*0.05,2) for i in range(23)] + [15.7,15.75,15.8,15.95,16.0]
    rows = [f'<row r="{i}" ht="{h}" customHeight="1">{cell(f"A{i}",number=1)}</row>' for i,h in enumerate(heights,1)]
    sheets.append(('Rounding',sheet_xml(rows,len(rows),15,width=8.43)))
    add('vv-row-height-geometry.xlsx',16,sheets,geometry=geometry,
        rounding={'sheet':'Rounding','address':f'A1:A{len(rows)}','row_count':len(rows),'last_column':'A',
                  'default_row_height_pt':15,'heights':heights,'row_custom_height':1})
    manifest = {'schema':SCHEMA,'purpose':'Separate Normal font, cell font, sheet customHeight, row customHeight, read order and raster rounding.',
        'no_expected_heights_hardcoded':True,'fixtures':fixtures,
        'sources':['https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.row',
                   'https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.spreadsheet.sheetformatproperties',
                   'https://learn.microsoft.com/en-us/javascript/api/excel/excel.rangeformat']}
    files[f'{OUT}/manifest.json'] = (json.dumps(manifest,indent=2)+'\n').encode('utf-8')
    return files

def main():
    p=argparse.ArgumentParser(description=__doc__)
    mode=p.add_mutually_exclusive_group(required=True)
    mode.add_argument('--dry-run',action='store_true',help='Validate packages in memory; write nothing.')
    mode.add_argument('--apply',action='store_true',help='Create missing deterministic fixtures; refuse changed files.')
    args=p.parse_args(); root=Path.cwd().resolve(); payload=build_all()
    if not (root/'docs/visual-validation/experiments').is_dir(): p.error('Run from the repository root.')
    for relative,data in payload.items():
        path=root/relative
        if any(q.is_symlink() for q in [path,*path.parents] if q!=root.parent): p.error('Symlink in fixture path: '+relative)
        if path.exists() and path.read_bytes()!=data: p.error('Refusing changed fixture: '+relative)
    for relative,data in payload.items():
        path=root/relative; print('UNCHANGED' if path.exists() else 'CREATE', relative)
        if args.apply and not path.exists(): path.parent.mkdir(parents=True,exist_ok=True); path.write_bytes(data)
    return 0

if __name__=='__main__':
    raise SystemExit(main())
