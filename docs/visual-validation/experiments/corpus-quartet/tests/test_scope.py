"""Static right-edge check for explicit Office.js capture scopes."""

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location('quartet_scope_runner', ROOT / 'run.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

BOOK = (b'<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        b'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        b'<sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>')
RELS = (b'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        b'<Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>')


def write_book(path, cells):
    sheet = (b'<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
             b'<dimension ref="A1:E30"/><sheetData>' + cells + b'</sheetData></worksheet>')
    with ZipFile(path, 'w') as archive:
        archive.writestr('xl/workbook.xml', BOOK)
        archive.writestr('xl/_rels/workbook.xml.rels', RELS)
        archive.writestr('xl/worksheets/sheet1.xml', sheet)


class ScopeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.entry = {'name': 'sample.xlsx'}
        for profile in ('original', *runner.PROFILES):
            path = root / f'{profile}.xlsx'
            write_book(path, b'<row r="3"><c r="E3" s="2"/></row>')
            self.entry[profile] = {'path': str(path)}

    def test_reports_occupied_adjacent_cell_and_suggests_wider_scope(self):
        source = Path(self.entry['original']['path'])
        write_book(source, b'<row r="5"><c r="E5"><v>42</v></c></row>')
        scope, overflow = runner.scope_and_overflow(source, 'Data!A1:D29')
        self.assertTrue(overflow)
        with self.assertRaisesRegex(ValueError, 'E5.*Data!A1:E29'):
            runner.validate_explicit_scope(self.entry, scope, overflow)

    def test_formula_in_variant_is_occupied_but_style_only_cell_is_not(self):
        runner.validate_explicit_scope(self.entry, 'Data!A1:D29', True)
        variant = Path(self.entry['maximum']['path'])
        write_book(variant, b'<row r="7"><c r="E7"><f>A1+1</f><v/></c></row>')
        with self.assertRaisesRegex(ValueError, 'E7 in maximum'):
            runner.validate_explicit_scope(self.entry, 'Data!A1:D29', True)

    def test_ignores_content_below_scope_or_when_probe_is_disabled(self):
        source = Path(self.entry['original']['path'])
        write_book(source, b'<row r="30"><c r="E30"><v>42</v></c></row>')
        runner.validate_explicit_scope(self.entry, 'Data!A1:D29', True)
        write_book(source, b'<row r="5"><c r="E5"><v>42</v></c></row>')
        runner.validate_explicit_scope(self.entry, 'Data!A1:D29', False)


if __name__ == '__main__':
    unittest.main()
