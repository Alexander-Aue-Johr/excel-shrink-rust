#!/usr/bin/env python3
"""Structural checks only, not an Excel-rendering test."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import unittest
from xml.etree import ElementTree as ET
import zipfile

spec = importlib.util.spec_from_file_location('fixtures', Path(__file__).resolve().parents[1] / 'generate-fixtures.py')
f = importlib.util.module_from_spec(spec)
spec.loader.exec_module(f)

class Fixtures(unittest.TestCase):
    def test_determinism(self):
        self.assertEqual(f.build_all(), f.build_all())
    def test_packages_and_metadata(self):
        files = f.build_all()
        manifest = json.loads(files[f.OUT + '/manifest.json'])
        self.assertEqual(len(manifest['fixtures']), 4)
        for fixture in manifest['fixtures']:
            data = files[f.OUT + '/' + fixture['workbook']]
            self.assertEqual(hashlib.sha256(data).hexdigest(), fixture['sha256'])
            f.validate_package(data)
            with zipfile.ZipFile(io.BytesIO(data)) as z:
                styles = ET.fromstring(z.read('xl/styles.xml'))
                ns = {'s': f.NS}
                self.assertEqual(styles.find('s:fonts/s:font/s:name', ns).get('val'), fixture['normal_font'])
                for name in z.namelist():
                    if not name.startswith('xl/worksheets/'): continue
                    sheet = ET.fromstring(z.read(name))
                    self.assertIsNone(sheet.find('s:drawing', ns))
                    self.assertIsNone(sheet.find('s:mergeCells', ns))
                    self.assertFalse(sheet.findall('.//s:f', ns))
    def test_marker_cells_are_blank_and_filled(self):
        files = f.build_all()
        with zipfile.ZipFile(io.BytesIO(files[f.OUT + '/vv-render-markers.xlsx'])) as z:
            root = ET.fromstring(z.read('xl/worksheets/sheet1.xml'))
            rows = root.find(f'{{{f.NS}}}sheetData')
            for row in list(rows)[:4]:
                self.assertEqual(len(row), 16)
                for cell in row:
                    self.assertEqual(cell.get('s'), '2')
                    self.assertEqual(len(cell), 0)

if __name__ == '__main__':
    unittest.main()
