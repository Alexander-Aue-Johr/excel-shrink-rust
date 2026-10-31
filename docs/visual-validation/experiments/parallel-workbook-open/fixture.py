"""Stream deterministic numeric workbooks without holding a million rows in RAM."""
from pathlib import Path
import zipfile

ROWS = 1_000_000
SHEET_NS = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
REL_NS = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
PACKAGE_NS = 'http://schemas.openxmlformats.org/package/2006/relationships'


def write_numeric_sheet(path: Path):
    """Write exactly one million populated rows and three numeric columns."""
    with path.open('wb') as output:
        output.write((f'<worksheet xmlns="{SHEET_NS}"><dimension ref="A1:C{ROWS}"/>'
                      '<sheetFormatPr defaultRowHeight="15"/><sheetData>').encode('ascii'))
        batch = []
        for row in range(1, ROWS + 1):
            batch.append(
                f'<row r="{row}"><c r="A{row}"><v>{row}</v></c>'
                f'<c r="B{row}"><v>{(row * 48271) % 2147483647}</v></c>'
                f'<c r="C{row}"><v>{(row * 69621) % 2147483647}</v></c></row>')
            if len(batch) == 5000:
                output.write(''.join(batch).encode('ascii'))
                batch.clear()
        if batch:
            output.write(''.join(batch).encode('ascii'))
        output.write(b'</sheetData></worksheet>')


def package_workbook(path: Path, sheet_xml: Path, sheet_count: int):
    """Package repeated million-row sheets; no formulas or external links are used."""
    sheets = ''.join(f'<sheet name="Data {i}" sheetId="{i}" r:id="rId{i}"/>'
                     for i in range(1, sheet_count + 1))
    worksheet_types = ''.join(
        f'<Override PartName="/xl/worksheets/sheet{i}.xml" '
        'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        for i in range(1, sheet_count + 1))
    relationships = ''.join(
        f'<Relationship Id="rId{i}" Type="{REL_NS}/worksheet" Target="worksheets/sheet{i}.xml"/>'
        for i in range(1, sheet_count + 1))
    parts = {
        '[Content_Types].xml': (
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/xl/workbook.xml" '
            'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
            + worksheet_types + '</Types>'),
        '_rels/.rels': (
            f'<Relationships xmlns="{PACKAGE_NS}"><Relationship Id="rId1" '
            f'Type="{REL_NS}/officeDocument" Target="xl/workbook.xml"/></Relationships>'),
        'xl/workbook.xml': (
            f'<workbook xmlns="{SHEET_NS}" xmlns:r="{REL_NS}"><sheets>{sheets}</sheets>'
            '<calcPr calcId="191029" fullCalcOnLoad="0"/></workbook>'),
        'xl/_rels/workbook.xml.rels': f'<Relationships xmlns="{PACKAGE_NS}">{relationships}</Relationships>'
    }
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name, xml in parts.items():
            archive.writestr(name, xml)
        for index in range(1, sheet_count + 1):
            archive.write(sheet_xml, f'xl/worksheets/sheet{index}.xml')
    return path
