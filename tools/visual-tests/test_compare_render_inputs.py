#!/usr/bin/env python3
"""Tests for the read-only report comparison utility (no Rust/Excel needed)."""
from __future__ import annotations
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
import compare_render_inputs as tool


class RenderReportTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.a = self.root / "a.json"
        self.b = self.root / "b.json"
        self.document = {"version": 10, "cache_root": "/cache/v10-x", "shrunk_package_hash": "book-a", "shrunk": [{
            "sheet_index": 1, "sheet_file": "xl/worksheets/sheet1.xml", "sheet_xml_hash": "xml",
            "sheet_render_hash": "render", "capture_range_hash": "range", "cacheable": True,
            "fallback_reason": None, "dependencies": [{"part": "xl/styles.xml", "bytes": 10, "content_hash": "style"}]}]}
        self.write(self.a)
        self.write(self.b)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write(self, path: Path) -> None:
        path.write_text(json.dumps(self.document), encoding="utf-8")

    def result(self) -> dict:
        return tool.compare(self.a, self.b)

    def test_different_packages_can_have_same_sheet_identity(self) -> None:
        self.document["shrunk_package_hash"] = "book-b"
        self.write(self.b)
        result = self.result()
        self.assertFalse(result["package_hash_equal"])
        self.assertEqual(result["sheets"][0]["status"], "same_reusable_identity")

    def test_changed_dependency_explained(self) -> None:
        row = self.document["shrunk"][0]
        row["sheet_render_hash"] = "changed"
        row["dependencies"][0]["content_hash"] = "style-b"
        self.write(self.b)
        row = self.result()["sheets"][0]
        self.assertEqual(row["changed_dependencies"][0]["part"], "xl/styles.xml")
        self.assertTrue(row["sheet_xml_equal"])
        self.assertFalse(row["render_hash_equal"])

    def test_ranges_and_namespace_are_separate_causes(self) -> None:
        self.document["cache_root"] = "/cache/v10-y"
        self.document["shrunk"][0]["capture_range_hash"] = "range-b"
        self.write(self.b)
        row = self.result()["sheets"][0]
        self.assertFalse(row["requested_range_equal"])
        self.assertFalse(row["cache_namespace_equal"])
        self.assertTrue(row["render_hash_equal"])
        self.assertEqual(row["changed_dependencies"], [])

    def test_runtime_dependency_blocks_reuse(self) -> None:
        self.document["shrunk"][0]["cacheable"] = False
        self.document["shrunk"][0]["fallback_reason"] = "external data"
        self.write(self.b)
        self.assertEqual(self.result()["sheets"][0]["status"], "not_same_reusable_identity")

    def test_unknown_selection_rejected(self) -> None:
        with self.assertRaises(ValueError):
            tool.compare(self.a, self.b, selected={54})

    def test_duplicate_indices_rejected(self) -> None:
        self.document["shrunk"].append(self.document["shrunk"][0])
        self.write(self.b)
        with self.assertRaises(ValueError):
            self.result()

    def test_duplicate_dependencies_rejected(self) -> None:
        items = self.document["shrunk"][0]["dependencies"]
        items.append(items[0])
        self.write(self.b)
        with self.assertRaises(ValueError):
            self.result()

    def test_old_schema_rejected(self) -> None:
        self.document["version"] = 7
        self.write(self.b)
        with self.assertRaises(ValueError):
            self.result()

    def test_cli_cannot_overwrite_input(self) -> None:
        before = self.a.read_bytes()
        with contextlib.redirect_stderr(io.StringIO()):
            code = tool.main([str(self.a), str(self.b), "--json", str(self.a)])
        self.assertEqual(code, 1)
        self.assertEqual(self.a.read_bytes(), before)

    def test_cli_writes_new_result(self) -> None:
        out = self.root / "comparison.json"
        with contextlib.redirect_stdout(io.StringIO()):
            code = tool.main([str(self.a), str(self.b), "--json", str(out)])
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out.read_text())["sheets"][0]["status"], "same_reusable_identity")


if __name__ == "__main__":
    unittest.main(verbosity=2)
