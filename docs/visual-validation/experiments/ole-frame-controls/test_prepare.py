import io
import tempfile
import unittest
from pathlib import Path
from zipfile import ZipFile

from prepare import move_anchor, replace_once, write_variant
from zero_area import mutate, sheet_drawing_parts


class FixtureMutationTests(unittest.TestCase):
    def test_fixture_drawing_mapping_uses_relationships_and_rejects_external_targets(self):
        main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
        rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
        for external in [False, True]:
            buffer = io.BytesIO()
            mode = ' TargetMode="External"' if external else ""
            parts = {
                "xl/workbook.xml": f'<workbook xmlns="{main}" xmlns:r="{rel}"><sheets><sheet name="A" r:id="sheet"/></sheets></workbook>',
                "xl/_rels/workbook.xml.rels": f'<Relationships><Relationship Id="sheet" Type="{rel}/worksheet" Target="worksheets/sheet9.xml"/></Relationships>',
                "xl/worksheets/sheet9.xml": f'<worksheet xmlns="{main}" xmlns:r="{rel}"><drawing r:id="drawing"/></worksheet>',
                "xl/worksheets/_rels/sheet9.xml.rels": f'<Relationships><Relationship Id="drawing" Type="{rel}/drawing" Target="../drawings/drawing42.xml"{mode}/></Relationships>',
            }
            with ZipFile(buffer, "w") as output:
                for name, text in parts.items():
                    output.writestr(name, text)
            buffer.seek(0)
            with ZipFile(buffer) as source:
                if external:
                    with self.assertRaises(ValueError):
                        sheet_drawing_parts(source, [{"sheet": "A"}])
                else:
                    self.assertEqual(
                        sheet_drawing_parts(source, [{"sheet": "A"}]),
                        {"A": "xl/drawings/drawing42.xml"},
                    )

    def test_zero_area_controls_require_the_expected_marker_and_transform(self):
        marker = b"<xdr:col>13</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>99</xdr:row><xdr:rowOff>0</xdr:rowOff>"
        anchor = (
            b"<xdr:twoCellAnchor><xdr:from>" + marker + b"</xdr:from><xdr:to>"
            + marker + b'</xdr:to><xdr:cNvPr name="Text 1"/>'
            + b'<a:ext cx="0" cy="0"/></xdr:twoCellAnchor>'
        )
        source = b"<xdr:wsDr>" + anchor + b"</xdr:wsDr>"
        removed, expanded = mutate(source)
        self.assertEqual(removed, b"<xdr:wsDr></xdr:wsDr>")
        self.assertIn(b"<xdr:row>103</xdr:row>", expanded)
        self.assertIn(b'<a:ext cx="1524000" cy="762000"/>', expanded)
        for invalid in [
            source.replace(b'cx="0"', b'cx="1"'),
            source.replace(b"Text 1", b"Other"),
            b"<xdr:wsDr>" + anchor + anchor + b"</xdr:wsDr>",
        ]:
            with self.assertRaises(ValueError):
                mutate(invalid)

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
