"""Write deterministic OOXML test inputs with explicitly declared mutations."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import zipfile


SHEET = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PACKAGE = "http://schemas.openxmlformats.org/package/2006/relationships"
CHART = "http://schemas.openxmlformats.org/drawingml/2006/chart"


def workbook(path, case, changed):
    """Keep package metadata fixed so identical inputs are byte-identical."""
    fill = "FFFF8800" if case == "direct-presentation" and changed else "FF99CCFF"
    styles = (
        f'<styleSheet xmlns="{SHEET}"><fonts count="1"><font><sz val="11"/>'
        '<name val="Calibri"/></font></fonts><fills count="3"><fill><patternFill '
        'patternType="none"/></fill><fill><patternFill patternType="gray125"/>'
        f'</fill><fill><patternFill patternType="solid"><fgColor rgb="{fill}"/>'
        '<bgColor indexed="64"/></patternFill></fill></fills><borders count="2">'
        '<border/><border><bottom style="thin"><color rgb="FF000000"/></bottom></border>'
        '</borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" '
        'borderId="0"/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0" fontId="0" '
        'fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="0" fillId="2" '
        'borderId="0" xfId="0" applyFill="1"/><xf numFmtId="0" fontId="0" fillId="0" '
        'borderId="1" xfId="0" applyBorder="1"/></cellXfs><cellStyles count="1">'
        '<cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>'
        '<dxfs count="1"><dxf><fill><patternFill patternType="solid">'
        '<fgColor rgb="FFFF0000"/><bgColor rgb="FFFF0000"/></patternFill></fill></dxf></dxfs>'
        '</styleSheet>'
    )
    direct = case == "direct-presentation"
    value = 9 if case == "dependent-presentation" and changed else 1
    height = ' ht="30" customHeight="1"' if direct and changed else ''
    width = 24 if direct and changed else 12
    border = 2 if direct and changed else 0
    rows = [f'<row r="1"{height}><c r="A1" s="1"><v>1</v></c>'
            f'<c r="B1"><v>{value}</v></c><c r="C1" s="{border}"><v>3</v></c></row>',
            '<row r="2"><c r="A2"><v>2</v></c><c r="B2"><v>2</v></c></row>',
            '<row r="3"><c r="A3"><v>3</v></c><c r="B3"><v>3</v></c></row>']
    if case == "offscreen-hidden":
        value = 9 if changed else 7
        rows.extend([f'<row r="2000"><c r="B2000"><v>{value}</v></c></row>',
                     f'<row r="3000" hidden="1"><c r="B3000"><v>{value}</v></c></row>'])
    if case == "sparse-formatting":
        for row in range(100, 2100):
            cells = ''.join(f'<c r="{column}{row}" s="1"/>' for column in "ABCDEFGHIJ")
            rows.append(f'<row r="{row}">{cells}</row>')
        # A changed blank cell tests formatting detection separately from input values.
        if changed:
            rows[-1] = rows[-1].replace('r="J2099" s="1"', 'r="J2099" s="2"')
    cf = ('<conditionalFormatting sqref="D1:D3"><cfRule type="expression" dxfId="0" '
          'priority="1"><formula>$B$1&gt;5</formula></cfRule></conditionalFormatting>')
    dependency = case == "dependent-presentation"
    sheet = (f'<worksheet xmlns="{SHEET}" xmlns:r="{REL}"><sheetViews><sheetView '
             'workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/>'
             f'<cols><col min="1" max="1" width="{width}" customWidth="1"/></cols>'
             '<sheetData>' + ''.join(rows) + '</sheetData>' + (cf if dependency else '')
             + ('<drawing r:id="drawing"/>' if dependency else '') + '</worksheet>')
    parts = {
        "xl/worksheets/sheet1.xml": sheet,
        "xl/styles.xml": styles,
        "xl/workbook.xml": f'<workbook xmlns="{SHEET}" xmlns:r="{REL}"><bookViews>'
            '<workbookView/></bookViews><sheets><sheet name="Data" sheetId="1" '
            'r:id="sheet"/></sheets></workbook>',
        "_rels/.rels": f'<Relationships xmlns="{PACKAGE}"><Relationship Id="book" '
            f'Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>',
        "xl/_rels/workbook.xml.rels": f'<Relationships xmlns="{PACKAGE}">'
            f'<Relationship Id="sheet" Type="{REL}/worksheet" Target="worksheets/sheet1.xml"/>'
            f'<Relationship Id="styles" Type="{REL}/styles" Target="styles.xml"/></Relationships>',
    }
    overrides = {
        "xl/workbook.xml": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
        "xl/styles.xml": "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml",
        "xl/worksheets/sheet1.xml": "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
    }
    if dependency:
        add_chart(parts, overrides, value)
    parts["[Content_Types].xml"] = (
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        + ''.join(f'<Override PartName="/{name}" ContentType="{kind}"/>'
                  for name, kind in sorted(overrides.items())) + '</Types>')
    with zipfile.ZipFile(path, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name, xml in sorted(parts.items()):
            member = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            member.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(member, xml.encode("utf-8"))


def add_chart(parts, overrides, value):
    """Anchor one native pie chart whose unchanged reference has a changed input/cache."""
    parts["xl/worksheets/_rels/sheet1.xml.rels"] = (
        f'<Relationships xmlns="{PACKAGE}"><Relationship Id="drawing" '
        f'Type="{REL}/drawing" Target="../drawings/drawing1.xml"/></Relationships>')
    parts["xl/drawings/drawing1.xml"] = (
        '<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" '
        f'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{REL}">'
        '<xdr:twoCellAnchor><xdr:from><xdr:col>5</xdr:col><xdr:colOff>0</xdr:colOff>'
        '<xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>12</xdr:col>'
        '<xdr:colOff>0</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>'
        '<xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Input chart"/>'
        '<xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/>'
        '<a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData '
        f'uri="{CHART}"><c:chart xmlns:c="{CHART}" r:id="chart"/></a:graphicData></a:graphic>'
        '</xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>')
    parts["xl/drawings/_rels/drawing1.xml.rels"] = (
        f'<Relationships xmlns="{PACKAGE}"><Relationship Id="chart" '
        f'Type="{REL}/chart" Target="../charts/chart1.xml"/></Relationships>')
    points = ''.join(f'<c:pt idx="{index}"><c:v>{number}</c:v></c:pt>'
                     for index, number in enumerate((value, 2, 3)))
    parts["xl/charts/chart1.xml"] = (
        f'<c:chartSpace xmlns:c="{CHART}"><c:chart><c:plotArea><c:layout/><c:pieChart>'
        '<c:varyColors val="1"/><c:ser><c:idx val="0"/><c:order val="0"/><c:val>'
        '<c:numRef><c:f>Data!$B$1:$B$3</c:f><c:numCache><c:formatCode>General</c:formatCode>'
        f'<c:ptCount val="3"/>{points}</c:numCache></c:numRef></c:val></c:ser>'
        '</c:pieChart></c:plotArea><c:plotVisOnly val="1"/></c:chart></c:chartSpace>')
    overrides["xl/drawings/drawing1.xml"] = "application/vnd.openxmlformats-officedocument.drawing+xml"
    overrides["xl/charts/chart1.xml"] = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml"


def create(output):
    """Publish the frozen identities only after all five pairs have been written."""
    output.mkdir(parents=True, exist_ok=False)
    mutations = {
        "identical": [],
        "offscreen-hidden": ["Data!B2000 value 7 to 9", "Data!B3000 hidden value 7 to 9"],
        "direct-presentation": ["Data!A1 fill", "Data!C1 bottom border", "row 1 height", "column A width"],
        "dependent-presentation": ["Data!B1 value 1 to 9", "dependent CF D1:D3", "dependent pie chart"],
        "sparse-formatting": ["Data!J2099 blank-cell fill replaced by bottom border"],
    }
    pairs = []
    for name, changes in mutations.items():
        directory = output / name
        directory.mkdir()
        left, right = directory / "original.xlsx", directory / "candidate.xlsx"
        workbook(left, name, False)
        if name == "identical":
            shutil.copyfile(left, right)
        else:
            workbook(right, name, True)
        pairs.append({"id": name, "expected_changes": changes, "original": str(left),
                      "candidate": str(right), "sha256": {
                          path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in (left, right)}})
    manifest = {"version": 1, "synthetic": True, "pairs": pairs,
                "native_excel_acceptance": "pending", "visual_oracle": "pending"}
    (output / "fixtures.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    create(parser.parse_args().output)
