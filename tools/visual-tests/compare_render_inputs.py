#!/usr/bin/env python3
"""Explain screenshot cache identities using two screenshot-render-inputs.json files.

Read-only. Python 3.10+, standard library only. Compares recorded dependency
hashes, not visual equivalence, and does not read or modify the source workbooks.
"""
from __future__ import annotations
import argparse
import json
import sys
from pathlib import Path
from typing import Any


def load(path: Path, side: str) -> tuple[dict[str, Any], dict[int, dict[str, Any]]]:
    with path.open(encoding="utf-8-sig") as stream:
        document = json.load(stream)
    version = document.get("version") if isinstance(document, dict) else None
    if type(version) is not int or version < 8:
        raise ValueError(f"{path}: unsupported render-input report version {version!r}")
    rows = document.get(side)
    if not isinstance(rows, list):
        raise ValueError(f"{path}: missing {side!r} sheet array")
    sheets: dict[int, dict[str, Any]] = {}
    for row in rows:
        if not isinstance(row, dict) or type(row.get("sheet_index")) is not int:
            raise ValueError(f"{path}: invalid sheet record")
        index = row["sheet_index"]
        if index < 1 or index in sheets:
            raise ValueError(f"{path}: duplicate/invalid sheet index {index}")
        if not isinstance(row.get("dependencies"), list):
            raise ValueError(f"{path}: missing dependencies for sheet {index}")
        sheets[index] = row
    return document, sheets


def dependencies(sheet: dict[str, Any]) -> dict[str, tuple[str, int]]:
    result: dict[str, tuple[str, int]] = {}
    for item in sheet["dependencies"]:
        name = item.get("part")
        digest = item.get("content_hash")
        size = item.get("bytes")
        if not isinstance(name, str) or not isinstance(digest, str) or type(size) is not int or size < 0 or name in result:
            raise ValueError("Invalid or duplicate dependency record")
        result[name] = (digest, size)
    return result


def compare(left_path: Path, right_path: Path, side: str = "shrunk", selected: set[int] | None = None) -> dict[str, Any]:
    ld, left = load(left_path, side)
    rd, right = load(right_path, side)
    rows: list[dict[str, Any]] = []
    indices = sorted(left.keys() | right.keys())
    if selected:
        unknown = selected - set(indices)
        if unknown:
            raise ValueError("Unknown sheet indices: " + ", ".join(map(str, sorted(unknown))))
        indices = [i for i in indices if i in selected]
    for index in indices:
        a, b = left.get(index), right.get(index)
        if a is None or b is None:
            rows.append({"sheet_index": index, "status": "added" if a is None else "removed"})
            continue
        da, db = dependencies(a), dependencies(b)
        changes = [
            {"part": part, "change": "added" if part not in da else "removed" if part not in db else "changed",
             "before": da.get(part), "after": db.get(part)}
            for part in sorted(da.keys() | db.keys()) if da.get(part) != db.get(part)
        ]
        render_equal = a.get("sheet_render_hash") == b.get("sheet_render_hash")
        range_equal = a.get("capture_range_hash") == b.get("capture_range_hash")
        namespace_equal = ld.get("cache_root") == rd.get("cache_root")
        file_equal = a.get("sheet_file") == b.get("sheet_file")
        reusable_identity = render_equal and range_equal and namespace_equal and file_equal and a.get("cacheable") is True and b.get("cacheable") is True
        rows.append({
            "sheet_index": index, "sheet_file": b.get("sheet_file"),
            "status": "same_reusable_identity" if reusable_identity else "not_same_reusable_identity",
            "sheet_xml_equal": a.get("sheet_xml_hash") == b.get("sheet_xml_hash"),
            "render_hash_equal": render_equal, "requested_range_equal": range_equal,
            "cache_namespace_equal": namespace_equal, "sheet_file_equal": file_equal,
            "before_cacheable": a.get("cacheable"), "after_cacheable": b.get("cacheable"),
            "before_fallback": a.get("fallback_reason"), "after_fallback": b.get("fallback_reason"),
            "before_render_hash": a.get("sheet_render_hash"), "after_render_hash": b.get("sheet_render_hash"),
            "changed_dependencies": changes,
        })
    return {"left": str(left_path), "right": str(right_path), "side": side,
            "package_hash_equal": ld.get(side + "_package_hash") == rd.get(side + "_package_hash"),
            "note": "Equal keys permit lookup, not a guaranteed hit: PNG integrity and captured geometry are checked separately.",
            "sheets": rows}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path)
    parser.add_argument("--side", choices=("original", "shrunk"), default="shrunk")
    parser.add_argument("--sheet", type=int, action="append", help="Sheet index; repeat to select multiple sheets.")
    parser.add_argument("--json", type=Path, dest="json_path", help="Write a new detailed JSON report (does not overwrite).")
    args = parser.parse_args(argv)
    try:
        report = compare(args.left, args.right, args.side, set(args.sheet or []))
        if args.json_path:
            # Never overwrite a source report or an unrelated output file.
            with args.json_path.open("x", encoding="utf-8") as stream:
                json.dump(report, stream, indent=2, ensure_ascii=False)
                stream.write("\n")
        print(f"Package hashes equal: {report['package_hash_equal']}")
        for sheet in report["sheets"]:
            print(f"\nSheet {sheet['sheet_index']}: {sheet['status']}")
            if "changed_dependencies" not in sheet:
                continue
            print(f"  XML equal={sheet['sheet_xml_equal']}; render inputs equal={sheet['render_hash_equal']}; requested range equal={sheet['requested_range_equal']}; namespace equal={sheet['cache_namespace_equal']}")
            if sheet["after_fallback"]:
                print(f"  Conservative context/reuse policy: {sheet['after_fallback']}")
            for change in sheet["changed_dependencies"]:
                print(f"  {change['change']}: {change['part']}")
        print("\n" + report["note"])
        return 0
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
