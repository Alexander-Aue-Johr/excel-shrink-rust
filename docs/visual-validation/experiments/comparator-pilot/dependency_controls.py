"""Freeze finite conditional-format mutations with literal and dynamic INDIRECT."""

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

    observations = []
    for case, formula in (
        ("literal-indirect", 'INDIRECT("X1")&gt;5'),
        ("dynamic-indirect", "INDIRECT($Z$1)&gt;5"),
    ):
        directory = output / case
        directory.mkdir()
        inputs = {}
        for lane, value in (("original", 1), ("candidate", 9)):
            package = dict(parts)
            cells = (
                f'<c r="X1"><v>{value}</v></c>'
                '<c r="Z1" t="inlineStr"><is><t>X1</t></is></c>'
            )
            conditional = (
                '<conditionalFormatting sqref="A1:A3"><cfRule type="expression" '
                f'dxfId="0" priority="1"><formula>{formula}</formula>'
                '</cfRule></conditionalFormatting>'
            )
            sheet = parts["xl/worksheets/sheet1.xml"].replace(
                b'<c r="C1" s="0"><v>3</v></c></row>',
                ('<c r="C1" s="0"><v>3</v></c>' + cells + '</row>').encode(),
            )
            assert cells.encode() in sheet
            package["xl/worksheets/sheet1.xml"] = sheet.replace(
                b'</worksheet>', (conditional + '</worksheet>').encode(),
            )
            path = directory / f"{lane}.xlsx"
            write_package(path, package)
            inputs[lane] = {"file": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        observations.append({
            "case": case, "inputs": inputs,
            "changed_input": "Data!X1 changes from 1 to 9; Z1 remains the text X1",
            "expected": "The red conditional fill appears across A1:A3, alongside the X1 value change.",
            "formula_xml": formula,
        })

    (output / "plan.json").write_text(
        json.dumps([{"sheet": "Data", "range": "A1:AA30"}]) + "\n", encoding="utf-8",
    )
    (output / "protocol.json").write_text(
        json.dumps({"schema": 1, "cases": observations, "native_status": "pending"}, indent=2)
        + "\n", encoding="utf-8",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    create(parser.parse_args().output)
