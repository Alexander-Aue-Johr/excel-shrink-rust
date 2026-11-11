"""Freeze a finite row-border change over a worksheet-wide flat background."""

import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import zipfile

from fixtures import workbook
from representation_controls import write_package


def create(output, mutation="row-border"):
    if mutation not in ("row-border", "default-height", "implicit-default-height"):
        raise ValueError("Unknown background mutation")
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(dir=output) as temporary:
        base = Path(temporary) / "base.xlsx"
        workbook(base, "identical", False)
        with zipfile.ZipFile(base) as archive:
            parts = {name: archive.read(name) for name in archive.namelist()}
    inputs = {}
    for lane in ("original", "candidate"):
        style = 0 if mutation == "row-border" and lane == "candidate" else 2
        package = dict(parts)
        sheet = parts["xl/worksheets/sheet1.xml"].replace(
            b'<cols><col min="1" max="1" width="12" customWidth="1"/></cols>',
            b'<cols><col min="1" max="16384" width="12" style="1" customWidth="1"/></cols>',
        )
        sheet = sheet.replace(
            b'<row r="1">',
            f'<row r="1" s="{style}" customFormat="1">'.encode(),
        )
        if mutation in ("default-height", "implicit-default-height"):
            if mutation == "default-height":
                sheet = sheet.replace(
                    b'defaultRowHeight="15"', b'defaultRowHeight="15" customHeight="1"',
                )
            if lane == "candidate":
                sheet = sheet.replace(b'defaultRowHeight="15"', b'defaultRowHeight="16"')
        assert b'max="16384"' in sheet and b'customFormat="1"' in sheet
        package["xl/worksheets/sheet1.xml"] = sheet
        path = output / f"{lane}.xlsx"
        write_package(path, package)
        inputs[lane] = {
            "file": str(path),
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        }
    (output / "plan.json").write_text(json.dumps([
        {"sheet": "Data", "range": "A1:D5"},
        {"sheet": "Data", "range": "XFA1:XFD5"},
    ]) + "\n", encoding="utf-8")
    (output / "protocol.json").write_text(json.dumps({
        "schema": 1,
        "synthetic": True,
        "mutation": mutation,
        "inputs": inputs,
        "changed_input": (
            "Only the first row's style changes from a bottom border to style zero."
            if mutation == "row-border"
            else "Only defaultRowHeight changes from 15 to 16 points; the row border remains."
        ),
        "expected": "A visible change remains observable at the far-right worksheet edge.",
    }, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--mutation",
        choices=("row-border", "default-height", "implicit-default-height"),
        default="row-border",
    )
    args = parser.parse_args()
    create(args.output, args.mutation)
