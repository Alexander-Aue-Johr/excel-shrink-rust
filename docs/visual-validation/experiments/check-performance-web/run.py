"""Create a disposable workbook and run the Excel for the web probe in Edge."""

import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import shutil
import sys
from textwrap import dedent
import zipfile
import xml.etree.ElementTree as ET


HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
SHEET = "xl/worksheets/sheet1.xml"
NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"


def create_fixture(path: Path, already_clean: bool = False) -> None:
    """Make a workbook with A1=42 and optional distant empty formatting."""
    dimension = "A1" if already_clean else "A1:Z10000"
    empty_row = "" if already_clean else '<row r="10000"><c r="Z10000" s="1"/></row>'
    parts = {
        "[Content_Types].xml": """
            <Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
              <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
              <Default Extension="xml" ContentType="application/xml"/>
              <Override PartName="/xl/workbook.xml"
                ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
              <Override PartName="/xl/worksheets/sheet1.xml"
                ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
              <Override PartName="/xl/styles.xml"
                ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
            </Types>""",
        "_rels/.rels": """
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
              <Relationship Id="rId1"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
                Target="xl/workbook.xml"/>
            </Relationships>""",
        "xl/workbook.xml": f"""
            <workbook xmlns="{NS}"
              xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
              <sheets><sheet name="Probe" sheetId="1" r:id="rId1"/></sheets>
            </workbook>""",
        "xl/_rels/workbook.xml.rels": """
            <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
              <Relationship Id="rId1"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"
                Target="worksheets/sheet1.xml"/>
              <Relationship Id="rId2"
                Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles"
                Target="styles.xml"/>
            </Relationships>""",
        "xl/styles.xml": f"""
            <styleSheet xmlns="{NS}">
              <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
              <fills count="3">
                <fill><patternFill patternType="none"/></fill>
                <fill><patternFill patternType="gray125"/></fill>
                <fill><patternFill patternType="solid">
                  <fgColor rgb="FFFF0000"/><bgColor indexed="64"/>
                </patternFill></fill>
              </fills>
              <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
              <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
              <cellXfs count="2">
                <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
                <xf numFmtId="0" fontId="0" fillId="2" borderId="0" xfId="0" applyFill="1"/>
              </cellXfs>
              <cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
            </styleSheet>""",
        SHEET: f"""
            <worksheet xmlns="{NS}">
              <dimension ref="{dimension}"/>
              <sheetData>
                <row r="1"><c r="A1"><v>42</v></c></row>
                {empty_row}
              </sheetData>
            </worksheet>""",
    }
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as workbook:
        for name, content in parts.items():
            workbook.writestr(name, '<?xml version="1.0" encoding="UTF-8"?>\n' + dedent(content))


def inspect_workbook(path: Path) -> dict:
    """Read the two cells that distinguish actual cleanup from a UI success signal."""
    with zipfile.ZipFile(path) as workbook:
        sheet = ET.fromstring(workbook.read(SHEET))
    cells = {cell.attrib["r"]: cell for cell in sheet.iter(f"{{{NS}}}c")}
    value = cells.get("A1")
    return {
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "bytes": path.stat().st_size,
        "a1_value": value.findtext(f"{{{NS}}}v") if value is not None else None,
        "z10000_present": "Z10000" in cells,
    }


def run_browser(directory: Path, fixture: Path, fresh_profile: bool, headless: bool) -> None:
    """Let the Windows Edge probe own its profile and write browser observations."""
    if sys.platform != "win32":
        raise RuntimeError("Browser automation must run with Windows Python")
    profile = directory / "edge-profile"
    consent = directory / "account-consent.json"
    if fresh_profile:
        if profile.exists():
            shutil.rmtree(profile)
        consent.unlink(missing_ok=True)
    elif profile.exists() and not consent.exists():
        answer = ctypes.windll.user32.MessageBoxW(
            None,
            "Reuse this experiment's Edge profile and saved account choice?\n\n"
            "Choose No to remove both and start with a fresh profile.",
            "Excel for the web experiment",
            0x04 | 0x20 | 0x40000,
        )
        if answer == 7:
            shutil.rmtree(profile)
            consent.unlink(missing_ok=True)
        elif answer != 6:
            raise RuntimeError("Browser profile choice was cancelled")
    sys.path.insert(0, str(directory / "python-packages"))
    from browser import run_experiment
    run_experiment(directory, fixture, headless=headless)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=REPO / ".tmp/check-performance-web")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--fresh-profile", action="store_true")
    parser.add_argument("--headless", action="store_true")
    parser.add_argument("--already-clean", action="store_true")
    args = parser.parse_args()
    directory = args.output.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    fixture_name = "clean-probe.xlsx" if args.already_clean else "excel-shrink-check-performance-probe.xlsx"
    fixture = directory / fixture_name
    if not fixture.exists():
        create_fixture(fixture, already_clean=args.already_clean)
    before = inspect_workbook(fixture)
    if before["a1_value"] != "42" or before["z10000_present"] == args.already_clean:
        raise RuntimeError("The experiment fixture is not intact")
    (directory / "fixture.json").write_text(json.dumps(before, indent=2) + "\n", encoding="utf-8")
    if args.prepare_only:
        print(f"Prepared {fixture}")
        return
    run_browser(directory, fixture, args.fresh_profile, args.headless)
    browser_result = json.loads((directory / "browser-result.json").read_text(encoding="utf-8"))
    if browser_result["status"] != "downloaded":
        raise RuntimeError("The browser run did not download an XLSX")
    after = inspect_workbook(Path(browser_result["download"]))
    report = {
        "fixture": before,
        "download": after,
        "optimization": browser_result["optimization"],
        "passed": (after["a1_value"] == "42"
                   and not after["z10000_present"]
                   and browser_result["optimization"] == (
                       "already_clean" if args.already_clean else "optimized")),
    }
    (directory / "result.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    if not report["passed"]:
        raise RuntimeError("The downloaded workbook did not pass the fixture check")


if __name__ == "__main__":
    main()
