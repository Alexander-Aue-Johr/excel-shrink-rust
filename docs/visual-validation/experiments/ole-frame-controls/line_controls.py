"""Prepare a removal control for the two retained Abwasser horizontal lines."""

import argparse
import hashlib
import json
import re
import xml.etree.ElementTree as ET
from pathlib import Path
from zipfile import ZipFile

from prepare import identity, write_variant
from zero_area import sheet_drawing_parts


def remove_lines(data):
    namespace = "{http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing}"
    drawing = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
    root = ET.fromstring(data)
    anchors = list(root)
    if len(anchors) != 2:
        raise ValueError("Expected exactly two retained line anchors")
    for index, anchor in enumerate(anchors, start=1):
        if anchor.tag != namespace + "twoCellAnchor":
            raise ValueError("Unexpected anchor kind")
        shape = anchor.find(namespace + "sp")
        if shape is None:
            raise ValueError("Expected line shape")
        name = shape.find(namespace + "nvSpPr/" + namespace + "cNvPr")
        geometry = shape.find(namespace + "spPr/" + drawing + "prstGeom")
        stroke = shape.find(namespace + "spPr/" + drawing + "ln")
        if name is None or name.get("name") != f"Line {index}":
            raise ValueError("Unexpected retained line name")
        if geometry is None or geometry.get("prst") != "line":
            raise ValueError("Unexpected retained line geometry")
        if stroke is None or stroke.get("w") != "10795":
            raise ValueError("Unexpected retained line width")
    # Preserve package XML bytes outside the two complete anchors.
    modified, count = re.subn(
        rb"<xdr:twoCellAnchor\b[^>]*>.*?</xdr:twoCellAnchor>",
        b"",
        data,
        flags=re.DOTALL,
    )
    if count != 2 or list(ET.fromstring(modified)):
        raise ValueError("Could not remove exactly the validated anchors")
    return modified


def prepare(source_path, output):
    source_before = identity(source_path)
    plan = [{"sheet": "Inhalt", "range": "A1:H50"}]
    output.mkdir(parents=True, exist_ok=False)
    with ZipFile(source_path) as source:
        parts = sheet_drawing_parts(source, plan)
        part = parts["Inhalt"]
        replacement = remove_lines(source.read(part))
        write_variant(source, output / "removed.xlsx", {part: replacement})
    with ZipFile(source_path) as source, ZipFile(output / "removed.xlsx") as changed:
        if source.namelist() != changed.namelist():
            raise ValueError("Mutation changed the package member list")
        differences = [
            name for name in source.namelist()
            if source.read(name) != changed.read(name)
        ]
        if differences != [part] or changed.testzip() is not None:
            raise ValueError("Mutation changed unexpected package contents")
    if source_before != identity(source_path):
        raise ValueError("Source identity changed during preparation")
    (output / "plan.json").write_text(json.dumps(plan), encoding="utf-8")
    receipt = {
        "purpose": "Native line removal control; no bounded-coverage qualification",
        "source": source_before,
        "sheet": "Inhalt",
        "changed_parts": differences,
        "replacement_sha256": hashlib.sha256(replacement).hexdigest(),
        "outputs": {name: identity(output / name) for name in ["removed.xlsx", "plan.json"]},
    }
    (output / "preparation.json").write_text(
        json.dumps(receipt, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    prepare(args.source, args.output)
