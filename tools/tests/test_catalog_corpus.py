import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile


spec = importlib.util.spec_from_file_location(
    "catalog_corpus", Path(__file__).resolve().parents[1] / "catalog-corpus.py"
)
catalog_corpus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(catalog_corpus)


class CorpusCatalogTests(unittest.TestCase):
    def test_records_member_bytes_hashes_and_original_archive_indices(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "corpus.zip"
            with zipfile.ZipFile(source, "w") as archive:
                archive.writestr("README.txt", b"description")
                archive.writestr("nested/book.xlsx", b"workbook fixture")
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            result = catalog_corpus.catalog(source, "fixture", "Fixture", "https://example.test/corpus.zip", digest)
            self.assertEqual(result["archive"]["sha256"], digest)
            self.assertEqual(result["workbooks"], [{
                "index": 1, "name": "nested/book.xlsx", "bytes": 16,
                "crc32": zipfile.crc32(b"workbook fixture"),
                "sha256": hashlib.sha256(b"workbook fixture").hexdigest(),
            }])
            with self.assertRaisesRegex(ValueError, "checksum"):
                catalog_corpus.catalog(source, "fixture", "Fixture", "", "0" * 64)

    def test_rejects_unsafe_workbook_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "corpus.zip"
            with zipfile.ZipFile(source, "w") as archive:
                archive.writestr("../escape.xlsx", b"fixture")
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            with self.assertRaisesRegex(ValueError, "safe relative path"):
                catalog_corpus.catalog(source, "fixture", "Fixture", "", digest)


if __name__ == "__main__":
    unittest.main()
