import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

from prepare import move_anchor, replace_once, write_variant


class FixtureMutationTests(unittest.TestCase):
    def test_writing_two_variants_preserves_source_entry_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "input.zip"
            with ZipFile(path, "x") as output:
                output.writestr("preview", b"original preview")
                output.writestr("other", b"unchanged")
            original = path.read_bytes()
            with ZipFile(path) as source:
                write_variant(source, root / "first.zip", {"preview": b"first"})
                write_variant(source, root / "second.zip", {"preview": b"second"})
                self.assertEqual(source.read("preview"), b"original preview")
            self.assertEqual(path.read_bytes(), original)
            for name, expected in [("first.zip", b"first"), ("second.zip", b"second")]:
                with ZipFile(root / name) as result:
                    self.assertIsNone(result.testzip())
                    self.assertEqual(result.read("preview"), expected)
                    self.assertEqual(result.read("other"), b"unchanged")

    def test_fixture_mutations_reject_absent_or_ambiguous_fields(self):
        for data in [b"absent", b"old old"]:
            with self.assertRaises(ValueError):
                replace_once(data, b"old", b"new")
        with self.assertRaises(ValueError):
            move_anchor(b"<xdr:row>8</xdr:row>", "xdr:")
        self.assertEqual(
            move_anchor(b"<xdr:row>7</xdr:row><xdr:row>14</xdr:row>", "xdr:"),
            b"<xdr:row>10</xdr:row><xdr:row>17</xdr:row>",
        )


if __name__ == "__main__":
    unittest.main()
