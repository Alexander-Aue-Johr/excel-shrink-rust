import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

from sample import freeze, select


class ReferenceSampleTests(unittest.TestCase):
    def test_selection_uses_only_the_fixed_frame_and_seed(self):
        entries = [{"archive_index": index, "member_name": f"book-{index}.xls",
                    "bytes": index + 1, "crc32": index} for index in range(80)]
        frame, chosen, strata = select(entries, "a" * 64, "fixed", 3)
        repeated = select(list(reversed(entries)), "a" * 64, "fixed", 3)
        self.assertEqual((frame, chosen, strata), repeated)
        self.assertEqual(len(chosen), 12)
        self.assertEqual([row["population"] for row in strata], [20] * 4)
        self.assertEqual([row["selected"] for row in strata], [3] * 4)
        self.assertNotEqual(chosen, select(entries, "a" * 64, "another", 3)[1])

    def test_frame_and_manifest_are_reproducible_and_preserve_occurrences(self):
        with tempfile.TemporaryDirectory(prefix="reference-sample-") as directory:
            root = Path(directory)
            archive = root / "source.zip"
            with zipfile.ZipFile(archive, "w") as source:
                source.writestr("readme.txt", "excluded")
                source.writestr("a.xls", b"same payload")
                source.writestr("b.XLSX", b"same payload")
            first = freeze(archive, "a" * 64, root / "one")
            second = freeze(archive, "a" * 64, root / "two")
            self.assertEqual(first, second)
            self.assertEqual(first["population"], 2)
            compressed = (root / "one/frame.jsonl.gz").read_bytes()
            self.assertEqual(compressed, (root / "two/frame.jsonl.gz").read_bytes())
            frame = gzip.decompress(compressed)
            self.assertEqual(hashlib.sha256(frame).hexdigest(), first["frame_uncompressed_sha256"])
            self.assertEqual([json.loads(line)["archive_index"] for line in frame.splitlines()], [1, 2])
            with self.assertRaises(FileExistsError):
                freeze(archive, "a" * 64, root / "one")


if __name__ == "__main__":
    unittest.main()
