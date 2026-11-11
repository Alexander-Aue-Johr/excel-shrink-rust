"""Create pairs with equivalent style or shared-string table permutations."""

import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import xml.etree.ElementTree as ET
import zipfile

from fixtures import SHEET, REL, workbook


def write_package(path, parts):
    with zipfile.ZipFile(path, "x") as archive:
        for name, data in sorted(parts.items()):
            member = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            member.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(member, data)


def create(output):
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(dir=output) as temporary:
        base = Path(temporary) / "base.xlsx"
        workbook(base, "identical", False)
        with zipfile.ZipFile(base) as archive:
            parts = {name: archive.read(name) for name in archive.namelist()}
    pairs = []
    for case in ("style-permutation", "shared-string-permutation"):
        directory = output / case
        directory.mkdir()
        original = dict(parts)
        candidate = dict(parts)
        if case == "style-permutation":
            tree = ET.fromstring(parts["xl/styles.xml"])
            styles = tree.find(f"{{{SHEET}}}cellXfs")
            styles[:] = [styles[0], styles[2], styles[1]]
            candidate["xl/styles.xml"] = ET.tostring(tree)
            candidate["xl/worksheets/sheet1.xml"] = parts["xl/worksheets/sheet1.xml"].replace(
                b's="1"', b's="2"',
            )
        else:
            content_type = (
                '<Override PartName="/xl/sharedStrings.xml" ContentType="'
                'application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>'
            )
            relationship = (
                f'<Relationship Id="strings" Type="{REL}/sharedStrings" Target="sharedStrings.xml"/>'
            )
            for package, strings, indices in (
                (original, ("Alpha", "Beta"), (0, 1)),
                (candidate, ("Beta", "Alpha"), (1, 0)),
            ):
                package["[Content_Types].xml"] = parts["[Content_Types].xml"].replace(
                    b"</Types>", (content_type + "</Types>").encode(),
                )
                package["xl/_rels/workbook.xml.rels"] = parts["xl/_rels/workbook.xml.rels"].replace(
                    b"</Relationships>", (relationship + "</Relationships>").encode(),
                )
                package["xl/sharedStrings.xml"] = (
                    f'<sst xmlns="{SHEET}" count="2" uniqueCount="2">'
                    + "".join(f"<si><t>{value}</t></si>" for value in strings) + "</sst>"
                ).encode()
                row = (
                    f'<row r="4"><c r="A4" t="s"><v>{indices[0]}</v></c>'
                    f'<c r="B4" t="s"><v>{indices[1]}</v></c></row>'
                )
                package["xl/worksheets/sheet1.xml"] = parts["xl/worksheets/sheet1.xml"].replace(
                    b"</sheetData>", (row + "</sheetData>").encode(),
                )
        files = {"original": directory / "original.xlsx", "candidate": directory / "candidate.xlsx"}
        write_package(files["original"], original)
        write_package(files["candidate"], candidate)
        pairs.append({
            "case": case, "expected": "Equal native presentation over Data!A1:N30",
            "inputs": {
                lane: {"file": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                for lane, path in files.items()
            },
        })
    (output / "plan.json").write_text(
        json.dumps([{"sheet": "Data", "range": "A1:N30"}]) + "\n", encoding="utf-8",
    )
    (output / "protocol.json").write_text(
        json.dumps({"schema": 1, "cases": pairs, "native_status": "pending"}, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    create(parser.parse_args().output)
