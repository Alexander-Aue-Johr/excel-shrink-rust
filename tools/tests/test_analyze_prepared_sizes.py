"""ZIP directory selection must not depend on XML payload decoding."""
import csv
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile


spec = importlib.util.spec_from_file_location(
    "analyze_prepared_sizes", Path(__file__).resolve().parents[1] / "analyze-prepared-sizes.py")
sizes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sizes)


def workbook(payload):
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("xl/worksheets/sheet1.xml", payload)
        archive.writestr("xl/styles.xml", b"irrelevant styles")
    return output.getvalue()


class MetadataOnlyTests(unittest.TestCase):
    def test_nested_directories_need_no_member_open_or_valid_xml(self):
        original = bytearray(workbook(b"not XML" * 100))
        # Damage the first member's compressed payload. Its directory remains
        # readable, while payload decompression or CRC verification would fail.
        with zipfile.ZipFile(io.BytesIO(original)) as archive:
            info = archive.getinfo("xl/worksheets/sheet1.xml")
            offset = info.header_offset + 30 + len(info.filename.encode())
        original[offset] ^= 0xff
        outer = io.BytesIO()
        with zipfile.ZipFile(outer, "w") as archive:
            archive.writestr("original.xlsx", original)
            archive.writestr("default.xlsx", workbook(b"small"))
        with zipfile.ZipFile(outer) as archive, patch.object(
                zipfile.ZipFile, "open", side_effect=AssertionError("Payload access")):
            before_size, before = sizes.inspect_xlsx(archive, "original.xlsx")
            after_size, after = sizes.inspect_xlsx(archive, "default.xlsx")
        self.assertEqual(before_size, len(original))
        self.assertGreater(before_size, after_size)
        self.assertEqual(before["xl/worksheets/sheet1.xml"][0], 700)
        self.assertEqual(after["xl/worksheets/sheet1.xml"][0], 5)

    def test_compressed_outer_member_is_rejected_without_extraction(self):
        outer = io.BytesIO()
        with zipfile.ZipFile(outer, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("original.xlsx", workbook(b"data"))
        with zipfile.ZipFile(outer) as archive:
            with self.assertRaisesRegex(ValueError, "stored XLSX"):
                sizes.inspect_xlsx(archive, "original.xlsx")

    def test_thresholds_are_strict_and_buckets_include_the_upper_bound(self):
        self.assertFalse(sizes.exceeds(100, 50, 50))
        self.assertTrue(sizes.exceeds(100, 49, 50))
        self.assertEqual(sizes.bucket(100, 50), "(0, 50] %")
        self.assertEqual(sizes.bucket(10000, 10), "(99, 99.9] %")
        self.assertEqual(sizes.bucket(0, 10), "No original reference")

    def test_control_comparison_separates_roundtrip_and_optimization(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            attempt = root / "attempt"
            payloads = {
                "inputs/original.xlsx": b"x" * 1000,
                "inputs/default/original.xlsx": b"abcdefghij" * 10,
                "artifacts/original.saved.xlsx": b"x" * 800,
                "artifacts/check-performance.before-optimization.xlsx": b"x" * 700,
                "artifacts/check-performance.optimized.saved.xlsx": b"x" * 50}
            for name, payload in payloads.items():
                path = attempt / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(workbook(payload))
            (root / "results").mkdir()
            (root / "results/fixture.json").write_text(json.dumps({
                "id": "fixture", "status": "prepared", "source_member": "fixture.xls",
                "attempt": str(attempt)}), encoding="utf8")
            with patch.object(zipfile.ZipFile, "open", side_effect=AssertionError("Payload access")), patch("builtins.print"):
                sizes.compare_controls(root, root / "comparison")
            summary = json.loads((root / "comparison/summary.json").read_bytes())
            self.assertEqual(summary["workbook_results"], {"check_performance": 1})
            self.assertEqual(summary["sheet_xml_results"], {"check_performance": 1})
            with (root / "comparison/sheets.csv").open(encoding="utf-8-sig") as stream:
                sheet = next(csv.DictReader(stream))
            self.assertEqual(float(sheet["default_xml_reduction_percent"]), 90)
            self.assertEqual(float(sheet["cp_roundtrip_xml_reduction_percent"]), 95)
            self.assertAlmostEqual(float(sheet["cp_optimization_xml_reduction_percent"]), 100 * 650 / 700)


if __name__ == "__main__":
    unittest.main()
