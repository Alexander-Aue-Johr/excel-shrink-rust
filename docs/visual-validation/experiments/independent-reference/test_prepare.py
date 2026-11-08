"""Check independent envelope construction and explicit unsupported outcomes."""

import io
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

from prepare import MAIN, PACKAGE, REL, endpoint, make_plan, prepare


def workbook(dimension, extra=None):
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w") as archive:
        archive.writestr("xl/workbook.xml", f'<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="Data" r:id="one"/></sheets></workbook>')
        archive.writestr("xl/_rels/workbook.xml.rels", f'<Relationships xmlns="{PACKAGE}"><Relationship Id="one" Target="worksheets/sheet1.xml" Type="{REL}/worksheet"/></Relationships>')
        archive.writestr("xl/worksheets/sheet1.xml", f'<worksheet xmlns="{MAIN}"><dimension ref="{dimension}"/><sheetData/></worksheet>')
        if extra:
            archive.writestr(extra, "<unsupported/>")
    return stream.getvalue()


class PreparationTests(unittest.TestCase):
    def test_union_includes_leading_geometry_and_removed_extent(self):
        self.assertEqual(make_plan(workbook("B3:Z9"), workbook("A1:C20")),
                         [{"sheet": "Data", "range": "A1:Z20"}])

    def test_graphics_cannot_silently_use_cell_dimensions(self):
        for part in ("xl/drawings/drawing1.xml", "xl/chartsheets/sheet1.xml", "xl/media/image1.png"):
            with self.subTest(part=part), self.assertRaisesRegex(ValueError, "reference envelope"):
                make_plan(workbook("A1", part), workbook("A1"))

    def test_grid_edges_and_invalid_dimensions(self):
        self.assertEqual(endpoint("A1:XFD1048576"), (1048576, 16384))
        for reference in ("A0", "XFE1", "A1048577", "C3:B4", "B4:C3", "A1:B2:C3"):
            with self.subTest(reference=reference), self.assertRaises(ValueError):
                endpoint(reference)

    def test_corrupt_preparation_stays_in_denominator_without_working_copies(self):
        with tempfile.TemporaryDirectory(prefix="reference-prepare-") as temporary:
            root = Path(temporary)
            (root / "study/prepared").mkdir(parents=True)
            payload = workbook("A1:C5")
            digest = hashlib.sha256(payload).hexdigest()
            selection = root / "selection.json"
            selection.write_text(json.dumps({"selected": [
                {"group_id": "first", "identity": "identity-1"},
                {"group_id": "missing", "identity": "identity-2"},
            ]}))
            with zipfile.ZipFile(root / "study/prepared/first.zip", "w") as archive:
                archive.writestr("original.xlsx", payload)
                archive.writestr("preparation.json", json.dumps({
                    "identity": "identity-1", "converted_member": "original.xlsx",
                    "converted_sha256": "0" * 64, "prepared_by": "test",
                    "variants": [{"profile": "default", "member": "original.xlsx", "sha256": digest}],
                }))
            report = prepare(selection, root / "study", root / "output")
            self.assertEqual([row["status"] for row in report["rows"]], ["failed", "failed"])
            self.assertIn("checksum mismatch", report["rows"][0]["reason"])
            self.assertFalse((root / "output/first").exists())


if __name__ == "__main__":
    unittest.main()
