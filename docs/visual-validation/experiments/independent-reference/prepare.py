"""Bind a frozen sample to verified preparations and independent dimension plans.

This experiment uses ElementTree independently of the candidate planner. A plan
covers A1 through the union of both stored worksheet dimensions. Basic drawings
are permitted only in the explicit native-object qualification mode. Dimensions
alone never establish graphical coverage.
"""

import argparse
import hashlib
import io
import json
from pathlib import Path
import posixpath
import re
import xml.etree.ElementTree as ET
import zipfile


MAIN = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PACKAGE = "http://schemas.openxmlformats.org/package/2006/relationships"
CELL = re.compile(r"([A-Z]{1,3})([1-9][0-9]*)\Z")


class UnsupportedEnvelope(ValueError):
    """The stored cell dimensions cannot establish this reference envelope."""


def endpoint(reference):
    """Validate the stored range and return its bottom-right grid coordinate."""
    parts = reference.split(":")
    if len(parts) not in (1, 2):
        raise ValueError("Invalid worksheet dimension")
    coordinates = []
    for part in parts:
        match = CELL.fullmatch(part)
        if not match:
            raise ValueError("Invalid worksheet dimension coordinate")
        column = 0
        for char in match[1]:
            column = column * 26 + ord(char) - ord("A") + 1
        row = int(match[2])
        if column > 16384 or row > 1048576:
            raise ValueError("Worksheet dimension outside Excel grid")
        coordinates.append((row, column))
    if coordinates[0] > coordinates[-1] or coordinates[0][1] > coordinates[-1][1]:
        raise ValueError("Reversed worksheet dimension")
    return coordinates[-1]


def column_name(column):
    result = ""
    while column:
        column, remainder = divmod(column - 1, 26)
        result = chr(ord("A") + remainder) + result
    return result


def dimensions(payload, require_native_objects=False):
    """Read stored dimensions without using selector decisions or cell parsing."""
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("Duplicate package members")
        if require_native_objects:
            if any(name.startswith(("xl/chartsheets/", "xl/activeX/", "xl/embeddings/"))
                   or name.endswith(".vml") for name in names):
                raise UnsupportedEnvelope("Legacy drawings, chart sheets and embedded objects require a separate reference envelope")
            for name in names:
                if name.startswith(("xl/drawings/", "xl/charts/")) and name.endswith(".xml"):
                    drawing = ET.fromstring(archive.read(name))
                    if any(node.tag.rsplit("}", 1)[-1] in ("effectLst", "effectDag") and len(node)
                           for node in drawing.iter()):
                        raise UnsupportedEnvelope("Drawing effects require a separate ink envelope")
        elif any(name.startswith(("xl/drawings/", "xl/charts/", "xl/chartsheets/",
                                  "xl/activeX/", "xl/embeddings/", "xl/media/")) for name in names):
            raise UnsupportedEnvelope("Graphical or embedded parts require a separate reference envelope")
        workbook = ET.fromstring(archive.read("xl/workbook.xml"))
        relationships = ET.fromstring(archive.read("xl/_rels/workbook.xml.rels"))
        targets = {item.attrib["Id"]: item for item in relationships.findall(f"{{{PACKAGE}}}Relationship")}
        result = {}
        for sheet in workbook.findall(f"{{{MAIN}}}sheets/{{{MAIN}}}sheet"):
            relationship = targets[sheet.attrib[f"{{{REL}}}id"]]
            if relationship.get("TargetMode") == "External" or not relationship.get("Type", "").endswith("/worksheet"):
                raise UnsupportedEnvelope("Unsupported sheet relationship")
            target = relationship.attrib["Target"]
            member = posixpath.normpath(target.lstrip("/") if target.startswith("/") else "xl/" + target)
            if not member.startswith("xl/") or "\\" in member:
                raise ValueError("Worksheet relationship leaves the workbook")
            root = ET.fromstring(archive.read(member))
            dimension = root.find(f"{{{MAIN}}}dimension")
            if dimension is None:
                raise ValueError("Worksheet has no stored dimension")
            name = sheet.attrib["name"]
            if name in result:
                raise ValueError("Duplicate worksheet names")
            result[name] = endpoint(dimension.attrib["ref"])
        if not result:
            raise ValueError("No supported worksheets")
        return result


def make_plan(original, default, require_native_objects=False, tile_rows=None, max_regions=64):
    left = dimensions(original, require_native_objects)
    right = dimensions(default, require_native_objects)
    if left.keys() != right.keys():
        raise UnsupportedEnvelope("Worksheet names differ")
    if tile_rows is not None and not 1 <= tile_rows <= 512:
        raise ValueError("Reference tile rows must be within 1..512")
    result = []
    for name in left:
        bottom = max(left[name][0], right[name][0])
        column = column_name(max(left[name][1], right[name][1]))
        step = tile_rows or bottom
        if len(result) + (bottom + step - 1) // step > max_regions:
            raise UnsupportedEnvelope("Reference rectangle count exceeds the declared per-workbook budget")
        for start in range(1, bottom + 1, step):
            result.append({"sheet": name, "range": f"A{start}:{column}{min(start + step - 1, bottom)}"})
    return result


def verified_member(archive, member, expected):
    payload = archive.read(member)
    if hashlib.sha256(payload).hexdigest() != expected:
        raise ValueError("Prepared member checksum mismatch")
    return payload


def prepare(selection, study, output, require_native_objects=False, tile_rows=None):
    """Preserve every selected identity, including unsupported and failed cases."""
    manifest = json.loads(selection.read_text(encoding="utf-8"))
    output.mkdir(parents=True, exist_ok=False)
    rows = []
    for selected in manifest["selected"]:
        group = selected["group_id"]
        row = {"group_id": group, "identity": selected["identity"], "status": "failed"}
        rows.append(row)
        try:
            bundle = study / "prepared" / f"{group}.zip"
            with bundle.open("rb") as stream:
                row["bundle_sha256"] = hashlib.file_digest(stream, "sha256").hexdigest()
            with zipfile.ZipFile(bundle) as archive:
                preparation = json.loads(archive.read("preparation.json"))
                if preparation["identity"] != selected["identity"]:
                    raise ValueError("Preparation identity mismatch")
                variants = [variant for variant in preparation["variants"] if variant["profile"] == "default"]
                if len(variants) != 1:
                    raise ValueError("Expected exactly one Default variant")
                original = verified_member(archive, preparation["converted_member"], preparation["converted_sha256"])
                default = verified_member(archive, variants[0]["member"], variants[0]["sha256"])
            plan = make_plan(original, default, require_native_objects, tile_rows)
            directory = output / group
            directory.mkdir()
            (directory / "original.xlsx").write_bytes(original)
            (directory / "default.xlsx").write_bytes(default)
            (directory / "plan.json").write_text(json.dumps(plan, indent=2), encoding="utf-8")
            row.update(status="plan_frozen", plan=plan,
                       original_sha256=hashlib.sha256(original).hexdigest(),
                       default_sha256=hashlib.sha256(default).hexdigest(),
                       prepared_by=preparation["prepared_by"],
                       requires_native_objects=require_native_objects)
        except UnsupportedEnvelope as error:
            row.update(status="unsupported", reason=str(error))
        except (OSError, ValueError, KeyError, zipfile.BadZipFile, ET.ParseError) as error:
            row["reason"] = str(error)
    result = {"selection_sha256": hashlib.sha256(selection.read_bytes()).hexdigest(),
              "preparation_script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "envelope": "A1 through union of stored dimensions; native object qualification required"
                          if require_native_objects else "A1 through union of stored dimensions, no graphical/embedded parts",
              "requires_native_objects": require_native_objects, "tile_rows": tile_rows,
              "max_regions_per_workbook": 64,
              "limits": ["Stored dimensions are a declared reference envelope, not a proof of all rendered ink.",
                         "Text overflow beyond the envelope and other effects require separate coverage.",
                         "Native geometry limits and failures remain in the selected denominator."],
              "rows": rows}
    (output / "preparation.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--selection", required=True, type=Path)
    parser.add_argument("--study", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--require-native-objects", action="store_true",
                        help="Permit basic drawings only with mandatory native reference --verify-objects qualification")
    parser.add_argument("--tile-rows", type=int)
    args = parser.parse_args()
    result = prepare(args.selection, args.study, args.output, args.require_native_objects, args.tile_rows)
    print(json.dumps({"selected": len(result["rows"]),
                      "plans": sum(row["status"] == "plan_frozen" for row in result["rows"])}))
