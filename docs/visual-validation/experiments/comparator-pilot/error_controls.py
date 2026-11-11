"""Freeze formula-error and broken-reference mutations before native observation."""

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
    for name, formulas in (
        ("error-value", (("1/0", "#DIV/0!"), ("NA()", "#N/A"))),
        ("broken-reference", (("#REF!", "#REF!"), ("1/0", "#DIV/0!"))),
    ):
        directory = output / name
        directory.mkdir()
        inputs = {}
        for lane, (formula, cached_error) in zip(("original", "candidate"), formulas):
            package = dict(parts)
            cell = f'<c r="A1" s="1" t="e"><f>{formula}</f><v>{cached_error}</v></c>'
            sheet = parts["xl/worksheets/sheet1.xml"].replace(
                b'<c r="A1" s="1"><v>1</v></c>', cell.encode(),
            )
            assert cell.encode() in sheet
            package["xl/worksheets/sheet1.xml"] = sheet
            path = directory / f"{lane}.xlsx"
            write_package(path, package)
            inputs[lane] = {
                "file": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "formula": formula, "cached_error": cached_error,
            }
        cases.append({"case": name, "inputs": inputs,
                      "expected": "A1 displays different formula errors after native rebuild."})
    (output / "plan.json").write_text(
        json.dumps([{"sheet": "Data", "range": "A1:N30"}]) + "\n", encoding="utf-8",
    )
    (output / "protocol.json").write_text(
        json.dumps({"schema": 1, "synthetic": True, "cases": cases,
                    "native_status": "pending"}, indent=2) + "\n", encoding="utf-8",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    create(parser.parse_args().output)
