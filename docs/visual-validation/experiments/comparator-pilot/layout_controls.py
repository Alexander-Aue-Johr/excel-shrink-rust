"""Freeze isolated wrapping and merged-cell mutation pairs for native tests."""

import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import xml.etree.ElementTree as ET
import zipfile

from fixtures import SHEET, workbook
from representation_controls import write_package


def create(output):
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(dir=output) as temporary:
        base = Path(temporary) / "base.xlsx"
        workbook(base, "identical", False)
        with zipfile.ZipFile(base) as archive:
            parts = {name: archive.read(name) for name in archive.namelist()}

    observations = []
    for case in ("wrap-text", "merge-cells"):
        directory = output / case
        directory.mkdir()
        original = dict(parts)
        candidate = dict(parts)
        if case == "wrap-text":
            text = "A deliberately long sentence wraps across several visible lines."
            cell = f'<c r="A1" s="1" t="inlineStr"><is><t>{text}</t></is></c>'
            for package in (original, candidate):
                package["xl/worksheets/sheet1.xml"] = parts["xl/worksheets/sheet1.xml"].replace(
                    b'<c r="A1" s="1"><v>1</v></c>', cell.encode(),
                ).replace(b'<row r="1">', b'<row r="1" ht="90" customHeight="1">')
            styles = ET.fromstring(parts["xl/styles.xml"])
            style = styles.find(f"{{{SHEET}}}cellXfs")[1]
            style.set("applyAlignment", "1")
            ET.SubElement(style, f"{{{SHEET}}}alignment", {"wrapText": "1"})
            candidate["xl/styles.xml"] = ET.tostring(styles)
            expectation = "Long A1 text wraps within the fixed-height row."
        else:
            candidate["xl/worksheets/sheet1.xml"] = parts["xl/worksheets/sheet1.xml"].replace(
                b'</worksheet>',
                b'<mergeCells count="1"><mergeCell ref="A1:D1"/></mergeCells></worksheet>',
            )
            expectation = "Merging A1:D1 changes the visible first-row presentation."

        inputs = {}
        for lane, package in (("original", original), ("candidate", candidate)):
            path = directory / f"{lane}.xlsx"
            write_package(path, package)
            inputs[lane] = {"file": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        observations.append({"case": case, "expected": expectation, "inputs": inputs})

    (output / "plan.json").write_text(
        json.dumps([{"sheet": "Data", "range": "A1:N30"}]) + "\n", encoding="utf-8",
    )
    (output / "protocol.json").write_text(
        json.dumps({"schema": 1, "cases": observations, "native_status": "pending"}, indent=2)
        + "\n", encoding="utf-8",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    create(parser.parse_args().output)
