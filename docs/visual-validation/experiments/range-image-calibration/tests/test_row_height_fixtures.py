#!/usr/bin/env python3
"""Package/experimental-design checks, not proof of Excel rendering behaviour."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import unittest
from xml.etree import ElementTree as ET
import zipfile

spec=importlib.util.spec_from_file_location('row_height_fixtures',Path(__file__).resolve().parents[1]/'generate-row-height-fixtures.py')
f=importlib.util.module_from_spec(spec);spec.loader.exec_module(f)
files=f.build_all();manifest=json.loads(files[f.OUT+'/manifest.json'])
ns={'s':f.NS}

def sheet_map(fixture):
    z=zipfile.ZipFile(io.BytesIO(files[f.OUT+'/'+fixture['workbook']]))
    wb=ET.fromstring(z.read('xl/workbook.xml'))
    return z,{s.get('name'):ET.fromstring(z.read(f'xl/worksheets/sheet{i}.xml')) for i,s in enumerate(wb.find('s:sheets',ns),1)}

class RowHeightFixtures(unittest.TestCase):
    def test_determinism_and_packages(self):
        self.assertEqual(files,f.build_all())
        self.assertEqual(len(manifest['fixtures']),3)
        for meta in manifest['fixtures']:
            data=files[f.OUT+'/'+meta['workbook']]
            f.validate_package(data)
            self.assertEqual(hashlib.sha256(data).hexdigest(),meta['sha256'])
    def test_normal_and_cell_fonts_are_separate(self):
        for meta in manifest['fixtures']:
            z,_=sheet_map(meta)
            styles=ET.fromstring(z.read('xl/styles.xml'))
            fonts=styles.find('s:fonts',ns)
            self.assertEqual(float(fonts[0].find('s:sz',ns).get('val')),meta['normal_font_size_pt'])
            self.assertEqual([float(x.find('s:sz',ns).get('val')) for x in list(fonts)[1:]],[8,11,24,48])
    def test_sheet_and_row_flags_and_absent_row(self):
        for meta in manifest['fixtures']:
            _,sheets=sheet_map(meta)
            for profile in meta.get('matrices',[]):
                sh=sheets[profile['sheet']]
                sf=sh.find('s:sheetFormatPr',ns)
                self.assertEqual(sf.get('customHeight'),None if profile['sheet_custom_height'] is None else str(profile['sheet_custom_height']))
                rows={int(r.get('r')):r for r in sh.find('s:sheetData',ns)}
                for c in profile['cases']:
                    self.assertEqual(c['row'] in rows,c['xml_row_present'])
                    if c['row'] not in rows: continue
                    row=rows[c['row']]
                    self.assertEqual(row.get('ht'),None if c['xml_ht'] is None else str(c['xml_ht']))
                    self.assertEqual(row.get('customHeight'),None if c['xml_custom_height'] is None else str(c['xml_custom_height']))
    def test_blank_cells_have_no_hidden_labels(self):
        _,sheets=sheet_map(manifest['fixtures'][0])
        for profile in manifest['fixtures'][0]['matrices']:
            sh=sheets[profile['sheet']]
            for c in profile['cases']:
                if c['id'].startswith('blank-'):
                    cell=sh.find(f'.//s:c[@r="A{c["row"]}"]',ns)
                    self.assertIsNotNone(cell)
                    self.assertEqual(len(cell),0)
                for col,text,style,number in c.get('cells',[]):
                    cell=sh.find(f'.//s:c[@r="{col}{c["row"]}"]',ns)
                    self.assertEqual(cell.get('s'),None if style is None else str(style))
    def test_order_twins_are_identical_xml(self):
        meta=manifest['fixtures'][2];_,sheets=sheet_map(meta)
        for pair in {p['pair'] for p in meta['geometry']}:
            self.assertEqual(ET.tostring(sheets['Geo'+pair]),ET.tostring(sheets['Img'+pair]))
        self.assertEqual(len(meta['geometry']),8)
    def test_no_macros_links_merges_formulas_or_drawings(self):
        for meta in manifest['fixtures']:
            z,sheets=sheet_map(meta)
            self.assertFalse(any('vba' in x.lower() or 'external' in x.lower() for x in z.namelist()))
            for sh in sheets.values():
                for tag in ['mergeCells','drawing','f']:
                    self.assertFalse(sh.findall('.//s:'+tag,ns))
    def test_rounding_sweep_uses_fixed_rows(self):
        meta=manifest['fixtures'][2];_,sheets=sheet_map(meta)
        rows=sheets['Rounding'].find('s:sheetData',ns)
        self.assertEqual(len(rows),28)
        self.assertTrue(all(r.get('customHeight')=='1' for r in rows))
        self.assertIn(12.6,meta['rounding']['heights'])
        self.assertIn(12.75,meta['rounding']['heights'])

if __name__=='__main__': unittest.main()
