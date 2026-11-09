"""Prepare two explicit mutations of the retained Elterngeld PDF-icon fixture.

This is a fixture-specific experiment, without a general OLE rewrite contract.
It preserves each input and refuses unexpected anchors or existing output paths.
"""

import argparse
import copy
import hashlib
import json
import re
from pathlib import Path
from zipfile import ZipFile


def identity(path):
    with path.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    return {"sha256": digest, "bytes": path.stat().st_size}


def replace_once(data, old, new):
    if data.count(old) != 1:
        raise ValueError(f"Expected exactly one fixture field {old!r}")
    return data.replace(old, new, 1)


def move_anchor(data, prefix):
    for old_row, new_row in [(7, 10), (14, 17)]:
        old = f"<{prefix}row>{old_row}</{prefix}row>".encode()
        new = f"<{prefix}row>{new_row}</{prefix}row>".encode()
        data = replace_once(data, old, new)
    return data


def moved_parts(source):
    sheet = source.read("xl/worksheets/sheet2.xml")
    drawing = source.read("xl/drawings/drawing2.xml")
    vml = source.read("xl/drawings/vmlDrawing1.vml")
    sheet = move_anchor(sheet, "xdr:")
    drawing = move_anchor(drawing, "xdr:")
    vml = replace_once(vml, b"margin-top:114.75pt", b"margin-top:159.75pt")
    anchor = re.compile(rb"(<x:Anchor>\s*)1, 7, 7, 14, 2, 79, 14, 9(</x:Anchor>)")
    vml, count = anchor.subn(rb"\g<1>1, 7, 10, 14, 2, 79, 17, 9\g<2>", vml)
    if count != 1:
        raise ValueError("Expected the retained PDF-icon VML anchor")
    return {
        "xl/worksheets/sheet2.xml": sheet,
        "xl/drawings/drawing2.xml": drawing,
        "xl/drawings/vmlDrawing1.vml": vml,
    }


def write_variant(source, destination, replacements):
    names = source.namelist()
    if len(names) != len(set(names)):
        raise ValueError("Duplicate ZIP entries in fixture")
    if not replacements.keys() <= set(names):
        raise ValueError("Replacement part absent from fixture")
    with ZipFile(destination, "x") as output:
        for entry in source.infolist():
            payload = replacements.get(entry.filename)
            if payload is None:
                payload = source.read(entry)
            output.writestr(copy.copy(entry), payload)


def prepare(pdf_source, preview_source, output):
    output.mkdir(parents=True, exist_ok=False)
    with ZipFile(pdf_source) as source, ZipFile(preview_source) as donor:
        preview = donor.read("xl/media/image3.emf")
        if preview == source.read("xl/media/image3.emf"):
            raise ValueError("The preview mutation must change bytes")
        write_variant(source, output / "moved.xlsx", moved_parts(source))
        write_variant(source, output / "preview.xlsx", {"xl/media/image3.emf": preview})
    plan = [{"sheet": "Erläuterung", "range": "A1:K24"}]
    (output / "plan.json").write_text(json.dumps(plan, ensure_ascii=False), encoding="utf-8")
    receipt = {
        "purpose": "Independent native mutation controls; no OLE coverage qualification",
        "source": identity(pdf_source),
        "preview_donor": identity(preview_source),
        "mutations": {
            "moved.xlsx": "Move both XML anchors and the VML frame down three rows",
            "preview.xlsx": "Replace only the cached EMF with a different retained valid EMF",
        },
        "outputs": {name: identity(output / name) for name in ["moved.xlsx", "preview.xlsx", "plan.json"]},
    }
    (output / "preparation.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pdf-source", type=Path, required=True)
    parser.add_argument("--preview-source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    prepare(args.pdf_source, args.preview_source, args.output)
