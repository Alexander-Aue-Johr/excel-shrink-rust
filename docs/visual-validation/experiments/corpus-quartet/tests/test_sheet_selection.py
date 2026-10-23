"""The startup A/B fixture varies worksheet selection, never workbook content."""
from pathlib import Path
import sys
import tempfile
import unittest
from zipfile import ZipFile
import xml.etree.ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from sheet_selection_probe import write_fixture


class SelectionFixtureTests(unittest.TestCase):
    def test_unopened_placeholders_still_contain_a_large_target_index(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'placeholder.xlsx'
            write_fixture(path, True, 84, 84)
            with ZipFile(path) as archive:
                book = ET.fromstring(archive.read('xl/workbook.xml'))
                self.assertEqual(len(book.find('{*}sheets')), 84)
                self.assertEqual(book.find('.//{*}workbookView').get('activeTab'), '83')
                target = ET.fromstring(archive.read('xl/worksheets/sheet84.xml'))
                self.assertEqual(target.find('.//{*}sheetView').get('tabSelected'), '1')

    def test_only_sheet_selection_changes_and_the_target_is_not_the_first_sheet(self):
        with tempfile.TemporaryDirectory() as temporary:
            paths = [Path(temporary) / name for name in ('stale.xlsx', 'consistent.xlsx')]
            for path, consistent in zip(paths, (False, True)):
                write_fixture(path, consistent)
            with ZipFile(paths[0]) as stale, ZipFile(paths[1]) as consistent:
                self.assertEqual(stale.namelist(), consistent.namelist())
                book = ET.fromstring(stale.read('xl/workbook.xml'))
                self.assertEqual(book.find('.//{*}workbookView').get('activeTab'), '1')
                for name in stale.namelist():
                    if not name.startswith('xl/worksheets/'):
                        self.assertEqual(stale.read(name), consistent.read(name))
                        continue
                    before = ET.fromstring(stale.read(name))
                    after = ET.fromstring(consistent.read(name))
                    view_before = before.find('.//{*}sheetView')
                    view_after = after.find('.//{*}sheetView')
                    self.assertNotEqual(view_before.get('tabSelected'), view_after.get('tabSelected'))
                    view_before.set('tabSelected', '0')
                    view_after.set('tabSelected', '0')
                    self.assertEqual(ET.tostring(before), ET.tostring(after))


if __name__ == '__main__':
    unittest.main()
