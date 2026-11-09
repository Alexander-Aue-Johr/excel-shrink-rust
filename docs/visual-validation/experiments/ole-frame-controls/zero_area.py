"""Prepare removal and expansion controls for the retained Text 1 fixture.

Each tested sheet's linked drawing must contain one zero-size two-cell text box.
This experiment does not define a production rule for arbitrary zero-size shapes.
"""

import argparse
import hashlib
import json
import posixpath
import re
import xml.etree.ElementTree as ET
from pathlib import Path
from zipfile import ZipFile

from prepare import identity, replace_once, write_variant


def mutate(data):
    anchors = re.findall(
        rb"<xdr:twoCellAnchor>.*?</xdr:twoCellAnchor>", data, re.DOTALL
    )
    if len(anchors) != 1:
        raise ValueError("Expected exactly one two-cell anchor")
    anchor = anchors[0]
    if b'name="Text 1"' not in anchor:
        raise ValueError("Expected the retained Text 1 shape")
    markers = []
    for name in [b"from", b"to"]:
        marker = re.findall(
            rb"<xdr:" + name + rb">(.*?)</xdr:" + name + rb">", anchor, re.DOTALL
        )
        if len(marker) != 1:
            raise ValueError("Missing or ambiguous anchor marker")
        markers.append(marker[0])
    if markers[0] != markers[1]:
        raise ValueError("The original anchor must have zero extent")
    row_match = re.fullmatch(
        rb"<xdr:col>13</xdr:col><xdr:colOff>0</xdr:colOff>"
        rb"<xdr:row>(99|100|101)</xdr:row><xdr:rowOff>0</xdr:rowOff>",
        markers[0],
    )
    if row_match is None:
        raise ValueError("Unexpected retained anchor coordinates")
    row = int(row_match[1])
    expanded_marker = markers[1].replace(
        b"<xdr:col>13</xdr:col>", b"<xdr:col>16</xdr:col>"
    )
    expanded_marker = expanded_marker.replace(
        f"<xdr:row>{row}</xdr:row>".encode(), f"<xdr:row>{row + 4}</xdr:row>".encode()
    )
    expanded = replace_once(
        anchor,
        b"<xdr:to>" + markers[1] + b"</xdr:to>",
        b"<xdr:to>" + expanded_marker + b"</xdr:to>",
    )
    expanded = replace_once(
        expanded, b'<a:ext cx="0" cy="0"/>', b'<a:ext cx="1524000" cy="762000"/>'
    )
    return replace_once(data, anchor, b""), replace_once(data, anchor, expanded)


def related_part(source, part, relationship_id, kind):
    folder, name = posixpath.split(part)
    relationships = ET.fromstring(source.read(f"{folder}/_rels/{name}.rels"))
    matches = [item for item in relationships if item.get("Id") == relationship_id]
    if len(matches) != 1:
        raise ValueError("Missing or ambiguous fixture relationship")
    item = matches[0]
    if item.get("TargetMode", "Internal") != "Internal":
        raise ValueError("External fixture relationship")
    if not item.get("Type", "").endswith("/" + kind):
        raise ValueError("Unsupported fixture relationship")
    target = item.attrib["Target"]
    if target.startswith("/"):
        target = target.lstrip("/")
    else:
        target = posixpath.join(folder, target)
    resolved = posixpath.normpath(target)
    if not resolved.startswith("xl/"):
        raise ValueError("Fixture relationship outside xl package parts")
    return resolved


def sheet_drawing_parts(source, plan):
    main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    relationship = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"
    sheets = ET.fromstring(source.read("xl/workbook.xml")).findall(
        f"{{{main}}}sheets/{{{main}}}sheet"
    )
    result = {}
    for area in plan:
        matches = [sheet for sheet in sheets if sheet.get("name") == area["sheet"]]
        if len(matches) != 1:
            raise ValueError("Missing or ambiguous fixture sheet")
        worksheet = related_part(
            source, "xl/workbook.xml", matches[0].attrib[relationship], "worksheet"
        )
        drawings = ET.fromstring(source.read(worksheet)).findall(f"{{{main}}}drawing")
        if len(drawings) != 1:
            raise ValueError("Expected one linked drawing per fixture sheet")
        result[area["sheet"]] = related_part(
            source, worksheet, drawings[0].attrib[relationship], "drawing"
        )
    if len(set(result.values())) != len(result):
        raise ValueError("Unexpected shared drawing part in fixture")
    return result


def prepare(source_path, output):
    plan = [
        {"sheet": "Deutschland VZ", "range": "A1:W116"},
        {"sheet": "West VZ ", "range": "A1:W116"},
        {"sheet": "Ost VZ ", "range": "A1:AA117"},
    ]
    output.mkdir(parents=True, exist_ok=False)
    removed, expanded = {}, {}
    with ZipFile(source_path) as source:
        drawing_parts = sheet_drawing_parts(source, plan)
        for name in drawing_parts.values():
            removed[name], expanded[name] = mutate(source.read(name))
        write_variant(source, output / "removed.xlsx", removed)
        write_variant(source, output / "expanded.xlsx", expanded)
    (output / "plan.json").write_text(
        json.dumps(plan, ensure_ascii=False), encoding="utf-8"
    )
    receipt = {
        "purpose": "Independent zero-size shape controls; no coverage qualification",
        "source": identity(source_path),
        "sheet_drawings": drawing_parts,
        "removed_parts": {
            name: hashlib.sha256(data).hexdigest() for name, data in removed.items()
        },
        "expanded_parts": {
            name: hashlib.sha256(data).hexdigest() for name, data in expanded.items()
        },
        "outputs": {
            name: identity(output / name)
            for name in ["removed.xlsx", "expanded.xlsx", "plan.json"]
        },
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
