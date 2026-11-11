"""Freeze conditional-format and defined-name edits before native observation."""

import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import zipfile

from fixtures import workbook
from representation_controls import write_package


def create(output):
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(dir=output) as temporary:
        base = Path(temporary) / "base.xlsx"
        workbook(base, "identical", False)
        with zipfile.ZipFile(base) as archive:
            parts = {name: archive.read(name) for name in archive.namelist()}

    cases = []
    for case in ("color-scale", "data-bar", "defined-name"):
        directory = output / case
        directory.mkdir()
        inputs = {}
        for changed, lane in ((False, "original"), (True, "candidate")):
            package = dict(parts)
            sheet = parts["xl/worksheets/sheet1.xml"].decode()
            if case == "defined-name":
                sheet = sheet.replace(
                    '<c r="A1" s="1"><v>1</v></c>',
                    '<c r="A1" s="1"><f>SUM(SourceValues)</f><v>0</v></c>',
                )
                endpoint = 3 if changed else 2
                definition = (
                    '<definedNames><definedName name="SourceValues">'
                    f'Data!$B$1:$B${endpoint}</definedName></definedNames>'
                )
                package["xl/workbook.xml"] = parts["xl/workbook.xml"].replace(
                    b"</workbook>", (definition + "</workbook>").encode(),
                )
            else:
                color = "FF0000FF" if changed else "FFFF0000"
                thresholds = '<cfvo type="min"/><cfvo type="max"/>'
                if case == "color-scale":
                    rule = (
                        '<cfRule type="colorScale" priority="1"><colorScale>'
                        f'{thresholds}<color rgb="{color}"/>'
                        '<color rgb="FF00FF00"/></colorScale></cfRule>'
                    )
                else:
                    rule = (
                        '<cfRule type="dataBar" priority="1"><dataBar>'
                        f'{thresholds}<color rgb="{color}"/>'
                        '</dataBar></cfRule>'
                    )
                formatting = (
                    '<conditionalFormatting sqref="A1:C3">'
                    + rule + '</conditionalFormatting>'
                )
                sheet = sheet.replace("</worksheet>", formatting + "</worksheet>")
            package["xl/worksheets/sheet1.xml"] = sheet.encode()
            path = directory / f"{lane}.xlsx"
            write_package(path, package)
            inputs[lane] = {
                "file": str(path),
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            }
        cases.append({
            "case": case,
            "inputs": inputs,
            "expected": (
                "A1 changes from 3 to 6 after native rebuild."
                if case == "defined-name"
                else "The conditional-format colors visibly change in A1:C3."
            ),
        })
    (output / "plan.json").write_text(
        json.dumps([{"sheet": "Data", "range": "A1:N30"}]) + "\n",
        encoding="utf-8",
    )
    (output / "protocol.json").write_text(
        json.dumps({"schema": 1, "synthetic": True, "cases": cases}, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    create(parser.parse_args().output)
