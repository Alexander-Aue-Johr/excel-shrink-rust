"""Make a tiny border/font-height counterexample for the comparison workflow."""

import argparse
from pathlib import Path
import zipfile


def create_fixtures(directory, check_performance=False, clean_workbook=False):
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
        with zipfile.ZipFile(directory / f'{name}.xlsx', 'w', zipfile.ZIP_DEFLATED) as archive:
            for part, xml in {**parts, 'xl/worksheets/sheet1.xml': sheet}.items():
                archive.writestr(part, xml.encode('utf-8'))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--check-performance', action='store_true')
    parser.add_argument('--clean-workbook', action='store_true')
    args = parser.parse_args()
    create_fixtures(args.directory, args.check_performance, args.clean_workbook)
