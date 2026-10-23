"""Checks for the XLSX numeric controls; runs with Python's unittest."""

import hashlib
from pathlib import Path
import sys
import tempfile
import unittest
from zipfile import ZipFile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from quartet_controls import create_controls


BOOK = (b'<?xml version="1.0"?><workbook '
        b'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        b'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        b'<sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>')
RELS = (b'<?xml version="1.0"?><Relationships '
        b'xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        b'<Relationship Id="rId1" Target="worksheets/sheet1.xml" '
        b'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet"/>'
        b'</Relationships>')
SHEET = (b'<?xml version="1.0"?><worksheet '
         b'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
         b'<cols><col min="3" max="3" hidden="1"/></cols><sheetData>'
         b'<row r="1"><c r="A1"><v>1</v></c></row>'
         b'<row r="4" hidden="1"><c r="B4"><v>4</v></c></row>'
         b'<row r="5"><c r="A5"/><c r="B5"><v>5</v></c>'
         b'<c r="C5"><v>999</v></c><c r="D5"><f>2+3</f><v>5</v></c></row>'
         b'<row r="6"><c r="B6"><v>6</v></c></row>'
         b'</sheetData><mergeCells count="1"><mergeCell ref="B6:C6"/>'
         b'</mergeCells></worksheet>')


def write_book(path, sheet=SHEET):
    with ZipFile(path, 'w') as book:
        book.writestr('xl/workbook.xml', BOOK)
        book.writestr('xl/_rels/workbook.xml.rels', RELS)
        book.writestr('xl/worksheets/sheet1.xml', sheet)
        book.writestr('docProps/app.xml', b'<Properties/>')


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class ControlsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.entry = {'name': 'sample.xlsx'}
        for profile in ('original', 'default', 'maximum', 'too-aggressive'):
            path = self.root / profile / 'sample.xlsx'
            path.parent.mkdir()
            write_book(path)
            self.entry[profile] = {'path': str(path), 'sha256': sha256(path)}

    def test_mutates_only_central_visible_numeric_cell_in_copies(self):
        initial = {profile: sha256(Path(value['path']))
                   for profile, value in self.entry.items() if profile != 'name'}
        changed, provenance = create_controls(
            self.entry, 'Data!A1:D8', self.root / 'controls')
        self.assertEqual('B5', provenance['cell'])
        self.assertEqual('5', provenance['original_value'])
        self.assertEqual('Data!A1:D8', provenance['scope'])
        self.assertEqual(self.entry['original'], changed['original'])
        self.assertEqual(len({item['value'] for item in
                              provenance['variants'].values()}), 3)
        for profile in ('default', 'maximum', 'too-aggressive'):
            source = Path(self.entry[profile]['path'])
            target = Path(changed[profile]['path'])
            value = provenance['variants'][profile]['value'].encode()
            self.assertEqual(initial[profile], sha256(source))
            self.assertNotEqual(source, target)
            self.assertEqual(sha256(target), changed[profile]['sha256'])
            with ZipFile(target) as archive:
                self.assertIsNone(archive.testzip())
                self.assertEqual(
                    SHEET.replace(b'<c r="B5"><v>5</v></c>',
                                  b'<c r="B5"><v>' + value + b'</v></c>'),
                    archive.read('xl/worksheets/sheet1.xml'))
                self.assertEqual(b'<Properties/>', archive.read('docProps/app.xml'))
        self.assertEqual(initial['original'],
                         sha256(Path(self.entry['original']['path'])))

    def test_rejects_when_no_numeric_cell_matches_all_four_books(self):
        for profile in ('default', 'maximum', 'too-aggressive'):
            path = Path(self.entry[profile]['path'])
            sheet = SHEET.replace(b'<v>1</v>', b'<v>11</v>')
            sheet = sheet.replace(b'<v>5</v></c><c r="C5">',
                                  b'<v>55</v></c><c r="C5">')
            write_book(path, sheet)
        with self.assertRaisesRegex(ValueError, 'No common visible numeric'):
            create_controls(self.entry, 'Data!A1:D8', self.root / 'controls')
        self.assertFalse((self.root / 'controls').exists())

    def test_skips_cell_already_equal_to_a_control_value(self):
        for profile in ('original', 'default', 'maximum', 'too-aggressive'):
            path = Path(self.entry[profile]['path'])
            write_book(path, SHEET.replace(b'<c r="B5"><v>5</v></c>',
                                           b'<c r="B5"><v>101</v></c>'))
        _, provenance = create_controls(self.entry, 'Data!A1:D8',
                                        self.root / 'controls')
        self.assertEqual('A1', provenance['cell'])
        self.assertEqual('1', provenance['original_value'])

    def test_reuses_prepared_copies_with_identical_provenance(self):
        directory = self.root / 'controls'
        first_entry, first_provenance = create_controls(
            self.entry, 'Data!A1:D8', directory)
        second_entry, second_provenance = create_controls(
            self.entry, 'Data!A1:D8', directory, reuse_existing=True)
        self.assertEqual(first_entry, second_entry)
        self.assertEqual(first_provenance, second_provenance)

    def test_rejects_uncaptured_or_hidden_sheet_scope(self):
        with self.assertRaisesRegex(ValueError, 'Unsupported control scope'):
            create_controls(self.entry, 'Data!B2:D8', self.root / 'controls')
        with self.assertRaisesRegex(ValueError, 'Visible worksheet'):
            create_controls(self.entry, 'Other!A1:D8', self.root / 'controls')


if __name__ == '__main__':
    unittest.main()
