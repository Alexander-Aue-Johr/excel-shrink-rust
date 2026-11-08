"""Create tiny stale-cache controls and independently verify native worker receipts."""

import argparse
from copy import copy
import hashlib
import json
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile


SPREADSHEET = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
NS = {"s": SPREADSHEET}


def create_controls(seed: Path, output: Path) -> None:
    output.mkdir(parents=True, exist_ok=False)
    with zipfile.ZipFile(seed) as source:
        workbook = ET.fromstring(source.read("xl/workbook.xml"))
        sheets = workbook.findall("s:sheets/s:sheet", NS)
        if len(sheets) != 1:
            raise ValueError("The seed must contain exactly one worksheet")
        sheets[0].set("name", "Sheet1")
        for label, value, cached in [("a", 2, 999), ("b", 3, 1111)]:
            sheet = ET.fromstring(source.read("xl/worksheets/sheet1.xml"))
            rows = sheet.find("s:sheetData", NS)
            if rows is None:
                raise ValueError("The seed lacks sheetData")
            rows.clear()
            row = ET.SubElement(rows, f"{{{SPREADSHEET}}}row", {"r": "1"})
            cell = ET.SubElement(row, f"{{{SPREADSHEET}}}c", {"r": "A1"})
            ET.SubElement(cell, f"{{{SPREADSHEET}}}v").text = str(value)
            cell = ET.SubElement(row, f"{{{SPREADSHEET}}}c", {"r": "B1"})
            ET.SubElement(cell, f"{{{SPREADSHEET}}}f").text = "A1*3"
            ET.SubElement(cell, f"{{{SPREADSHEET}}}v").text = str(cached)
            replacements = {
                "xl/workbook.xml": ET.tostring(workbook, encoding="utf-8", xml_declaration=True),
                "xl/worksheets/sheet1.xml": ET.tostring(sheet, encoding="utf-8", xml_declaration=True),
            }
            with zipfile.ZipFile(output / f"control-{label}.xlsx", "x", zipfile.ZIP_DEFLATED) as target:
                for entry in source.infolist():
                    # ZipFile writes update ZipInfo offsets. Preserve the source archive's metadata.
                    target.writestr(copy(entry), replacements.get(entry.filename, source.read(entry)))


def verify(result: Path) -> None:
    report = json.loads(result.read_text(encoding="utf-8-sig"))
    if not all(report.get(key) is True for key in [
        "rebuild_snapshots", "owned_excel_exit_confirmed", "source_hashes_unchanged"
    ]):
        raise ValueError("Missing native rebuild or cleanup evidence")
    samples = report["samples"]
    if len(samples) != 4 or len({sample["pid"] for sample in samples}) != 1:
        raise ValueError("Expected four visits in one Excel process")
    for index, sample in enumerate(samples):
        for phase, expected in [("before", 999 if index % 2 == 0 else 1111),
                                ("after", 6 if index % 2 == 0 else 9)]:
            with zipfile.ZipFile(sample["snapshots"][phase]) as workbook:
                sheet = ET.fromstring(workbook.read("xl/worksheets/sheet1.xml"))
                cell = sheet.find('.//s:c[@r="B1"]', NS)
                if cell is None or cell.findtext("s:f", namespaces=NS) != "A1*3":
                    raise ValueError("Snapshot formula differs")
                if int(cell.findtext("s:v", namespaces=NS)) != expected:
                    raise ValueError(f"Unexpected {phase} formula cache for visit {index}")
        for field in ["resident", "resident_rebuilt"]:
            for image in sample[field]["images"]["images"]:
                digest = hashlib.sha256(Path(image["image_path"]).read_bytes()).hexdigest()
                if digest != image["sha256"]:
                    raise ValueError("PNG bytes differ from the receipt")
        if sample["resident"]["capture"]["tiles"] == sample["resident_rebuilt"]["capture"]["tiles"]:
            raise ValueError("The stale-cache visual difference was not observed")
    for first, second in [(0, 2), (1, 3)]:
        for field in ["resident", "resident_rebuilt"]:
            if samples[first][field]["capture"]["tiles"] != samples[second][field]["capture"]["tiles"]:
                raise ValueError("Repeated source visits produced different capture evidence")
    print("Verified four rebuild transitions and eight PNG receipts in one Excel process.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("create")
    create.add_argument("seed", type=Path)
    create.add_argument("output", type=Path)
    check = commands.add_parser("verify")
    check.add_argument("result", type=Path)
    arguments = parser.parse_args()
    if arguments.command == "create":
        create_controls(arguments.seed, arguments.output)
    else:
        verify(arguments.result)
