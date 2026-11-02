"""Make a tiny border/font-height counterexample for the comparison workflow."""

import argparse
from pathlib import Path
import zipfile
import xml.etree.ElementTree as ET


def create_chart_fixtures(directory):
    """Keep all worksheet cells identical while changing only chart objects."""
    from openpyxl import Workbook
    from openpyxl.chart import BarChart, Reference

    directory.mkdir(parents=True, exist_ok=False)
    for lane in ('original', 'default'):
        book = Workbook()
        sheet = book.active
        sheet.title = 'Charts'
        for value in (1, 3, 2):
            sheet.append([value])
        names = ['Stable', 'Changed', 'Removed' if lane == 'original' else 'Added']
        for index, name in enumerate(names):
            chart = BarChart()
            chart.title = 'Different title' if name == 'Changed' and lane == 'default' else name
            chart.add_data(Reference(sheet, min_col=1, min_row=1, max_row=3))
            sheet.add_chart(chart, f'D{1 + index * 16}')
        for name in names:
            chart = BarChart()
            chart.title = 'Different title' if name == 'Changed' and lane == 'default' else name
            chart.add_data(Reference(sheet, min_col=1, min_row=1, max_row=3))
            chart_sheet = book.create_chartsheet(f'Sheet {name}')
            chart_sheet.add_chart(chart)
            if name == 'Stable':
                chart_sheet.sheet_state = 'hidden'
        path = directory / f'{lane}.xlsx'
        book.save(path)
        with zipfile.ZipFile(path) as archive:
            parts = {name: archive.read(name) for name in archive.namelist()}
        drawing = ET.fromstring(parts['xl/drawings/drawing1.xml'])
        properties = drawing.findall('.//{http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing}cNvPr')
        assert len(properties) == len(names)
        for element, name in zip(properties, names):
            element.set('name', name)
        parts['xl/drawings/drawing1.xml'] = ET.tostring(drawing)
        with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as archive:
            for name, content in parts.items():
                archive.writestr(name, content)


def create_fixtures(directory, check_performance=False, clean_workbook=False, recalc_candidate=False, save_overlap=False):
    """Write a font-height control, CP redundant-format fixture, or clean control."""
    directory.mkdir(parents=True, exist_ok=False)
    namespace = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
    parts = {
        '[Content_Types].xml': '''<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
          <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
          <Default Extension="xml" ContentType="application/xml"/>
          <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
          <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
          <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
        </Types>''',
        '_rels/.rels': '''<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
        </Relationships>''',
        'xl/workbook.xml': f'''<workbook xmlns="{namespace}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
          <sheets><sheet name="Rows" sheetId="1" r:id="rId1"/></sheets><calcPr calcMode="manual"/>
        </workbook>''',
        'xl/_rels/workbook.xml.rels': '''<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
          <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
          <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
        </Relationships>''',
        'xl/styles.xml': f'''<styleSheet xmlns="{namespace}">
          <fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><sz val="30"/><name val="Calibri"/></font></fonts>
          <fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
          <borders count="2"><border/><border><bottom style="thin"><color rgb="FF007040"/></bottom></border></borders>
          <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
          <cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
            <xf numFmtId="0" fontId="1" fillId="0" borderId="1" xfId="0" applyFont="1" applyBorder="1"/>
            <xf numFmtId="0" fontId="0" fillId="0" borderId="1" xfId="0" applyBorder="1"/></cellXfs>
          <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
        </styleSheet>''',
    }
    redundant = '<row r="10000"><c r="Z10000" s="1"/></row>' if check_performance else ''
    for name, sentinel in [('original', '<row r="10"><c r="A10" s="1"/></row>'), ('default', '')]:
        if check_performance:
            # Identical inputs must still run CP: its optimization finds new candidates.
            sentinel = '<row r="10"><c r="A10" s="1"/></row>'
        columns = '<cols><col min="1" max="2" width="18" style="2" customWidth="1"/></cols>'
        if clean_workbook:
            sentinel = redundant = columns = ''
        sheet = f'''<worksheet xmlns="{namespace}"><dimension ref="A1:B11"/><sheetFormatPr defaultRowHeight="15"/>
          {columns}
          <sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Font-height control</t></is></c></row>{sentinel}{redundant}</sheetData></worksheet>'''
        if recalc_candidate:
            # Match the earlier opening-control experiment: an obsolete calcId
            # would force recalculation on open and defeat this late-candidate test.
            parts['xl/workbook.xml'] = parts['xl/workbook.xml'].replace(
                '<calcPr calcMode="manual"/>',
                '<calcPr calcId="191029" calcMode="manual" calcOnSave="0" fullCalcOnLoad="0" forceFullCalc="0"/>')
            source = '<row r="2"><c r="A2"><v>2</v></c></row>' if name == 'original' else ''
            sheet = f'''<worksheet xmlns="{namespace}"><dimension ref="A1:Z20"/><sheetFormatPr defaultRowHeight="15"/>
              <sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Recalc control</t></is></c></row>{source}
              <row r="20"><c r="Z20"><f>A2*2</f><v>4</v></c></row></sheetData></worksheet>'''
        if save_overlap:
            removable = '<row r="10"><c r="A10"/></row>' if name == 'original' else ''
            sheet = f'''<worksheet xmlns="{namespace}"><dimension ref="A1:A10"/><sheetFormatPr defaultRowHeight="15"/>
              <sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Save overlap</t></is></c></row>{removable}</sheetData></worksheet>'''
        with zipfile.ZipFile(directory / f'{name}.xlsx', 'w', zipfile.ZIP_DEFLATED) as archive:
            for part, xml in {**parts, 'xl/worksheets/sheet1.xml': sheet}.items():
                archive.writestr(part, xml.encode('utf-8'))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--check-performance', action='store_true')
    parser.add_argument('--clean-workbook', action='store_true')
    parser.add_argument('--recalc-candidate', action='store_true')
    parser.add_argument('--save-overlap', action='store_true')
    parser.add_argument('--charts', action='store_true')
    parser.add_argument('--conditional-formatting', choices=[
        'constant', 'constant-string', 'styles', 'dummy-missing', 'dummy-inert',
        'dummy-present', 'dummy-added', 'duplicate', 'color-scale',
        'ref-relative', 'ref-column', 'ref-row', 'ref-fixed', 'ref-disjoint',
    ])
    args = parser.parse_args()
    if args.conditional_formatting:
        from cf_fixtures import create
        create(args.directory, args.conditional_formatting)
    elif args.charts:
        create_chart_fixtures(args.directory)
    else:
        create_fixtures(args.directory, args.check_performance, args.clean_workbook, args.recalc_candidate, args.save_overlap)
