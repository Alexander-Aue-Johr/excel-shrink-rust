"""Equal-count partitions preserve original bytes and global size order."""
import csv
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile


spec = importlib.util.spec_from_file_location(
    "copy_prepared_originals", Path(__file__).resolve().parents[1] / "copy-prepared-originals.py")
copy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(copy)


class PartitionTests(unittest.TestCase):
    def test_enron_counts_differ_by_at_most_one(self):
        self.assertEqual(copy.partition_counts(20368, 6), [3395, 3395, 3395, 3395, 3394, 3394])

    def test_sorted_copies_and_resume_preserve_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            rows = []
            payloads = {"a": b"large workbook", "b": b"x", "c": b"medium"}
            for case_id, payload in payloads.items():
                bundle = root / (case_id + ".zip")
                with zipfile.ZipFile(bundle, "w") as archive:
                    archive.writestr("original.xlsx", payload)
                rows.append({"case_id": case_id, "source_member": "same-name.xls",
                             "bundle": str(bundle), "original_xlsx_bytes": len(payload)})
            inventory = root / "inventory.csv"
            with inventory.open("w", newline="", encoding="utf8") as stream:
                writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
                writer.writeheader()
                writer.writerows(rows)
            output = root / "output"
            with patch("builtins.print"):
                first = copy.copy_originals(inventory, output, 2)
                second = copy.copy_originals(inventory, output, 2)
            self.assertEqual(first["state"], "complete")
            self.assertEqual([f["verified"] for f in first["folders"]], [2, 1])
            files = sorted(output.glob("part-*/*.xlsx"))
            self.assertEqual([p.read_bytes() for p in files], [payloads["b"], payloads["c"], payloads["a"]])
            self.assertEqual(second["reused"], 3)
            self.assertEqual(second["copied"], 0)


if __name__ == "__main__":
    unittest.main()
