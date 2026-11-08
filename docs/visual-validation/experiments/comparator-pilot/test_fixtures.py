"""Check the declared mutations independently through parsed package members."""

from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile

from fixtures import CHART, SHEET, create


def read_xml(path, member):
    with zipfile.ZipFile(path) as archive:
        return ET.fromstring(archive.read(member))


def cell_value(sheet, address):
    return sheet.find(f'.//{{{SHEET}}}c[@r="{address}"]/{{{SHEET}}}v').text


class FixtureTests(unittest.TestCase):
    def test_repeatable_packages_and_exact_mutations(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first = create(root / "first")
            second = create(root / "second")
            self.assertEqual(len(first["pairs"]), 5)
            for left, right in zip(first["pairs"], second["pairs"]):
                self.assertEqual(left["sha256"], right["sha256"])
                same = left["sha256"]["original.xlsx"] == left["sha256"]["candidate.xlsx"]
                self.assertEqual(same, left["id"] == "identical")
                for lane in ("original", "candidate"):
                    with zipfile.ZipFile(left[lane]) as archive:
                        for member in archive.namelist():
                            ET.fromstring(archive.read(member))
            cases = {pair["id"]: pair for pair in first["pairs"]}
            offscreen = cases["offscreen-hidden"]
            before = read_xml(offscreen["original"], "xl/worksheets/sheet1.xml")
            after = read_xml(offscreen["candidate"], "xl/worksheets/sheet1.xml")
            for address in ("B2000", "B3000"):
                self.assertEqual(cell_value(before, address), "7")
                self.assertEqual(cell_value(after, address), "9")
            self.assertEqual(after.find(f'.//{{{SHEET}}}row[@r="3000"]').get("hidden"), "1")
            dependent = cases["dependent-presentation"]
            before = read_xml(dependent["original"], "xl/worksheets/sheet1.xml")
            after = read_xml(dependent["candidate"], "xl/worksheets/sheet1.xml")
            self.assertEqual(cell_value(before, "B1"), "1")
            self.assertEqual(cell_value(after, "B1"), "9")
            self.assertEqual(ET.tostring(before.find(f'{{{SHEET}}}conditionalFormatting')),
                             ET.tostring(after.find(f'{{{SHEET}}}conditionalFormatting')))
            for lane, expected in (("original", "1"), ("candidate", "9")):
                chart = read_xml(dependent[lane], "xl/charts/chart1.xml")
                self.assertEqual(chart.find(f'.//{{{CHART}}}f').text, 'Data!$B$1:$B$3')
                self.assertEqual(chart.find(f'.//{{{CHART}}}pt[@idx="0"]/{{{CHART}}}v').text, expected)
            direct = read_xml(cases["direct-presentation"]["candidate"], "xl/worksheets/sheet1.xml")
            self.assertEqual(direct.find(f'.//{{{SHEET}}}row[@r="1"]').get("ht"), "30")
            self.assertEqual(direct.find(f'.//{{{SHEET}}}col').get("width"), "24")
            self.assertEqual(direct.find(f'.//{{{SHEET}}}c[@r="C1"]').get("s"), "2")
            sparse = read_xml(cases["sparse-formatting"]["candidate"], "xl/worksheets/sheet1.xml")
            self.assertEqual(sparse.find(f'.//{{{SHEET}}}c[@r="J2099"]').get("s"), "2")
            self.assertEqual(len(sparse.findall(f'.//{{{SHEET}}}c')), 20_007)
            with self.assertRaises(FileExistsError):
                create(root / "first")


if __name__ == "__main__":
    unittest.main()
