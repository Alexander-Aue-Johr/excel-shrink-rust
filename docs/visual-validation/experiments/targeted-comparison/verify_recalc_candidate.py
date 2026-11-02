"""Verify that an indirect formula change is captured before losing Original's pre-state."""
import argparse
import json
from report_audit import load_report
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile
from openpyxl.utils.cell import range_boundaries


def covers_formula(finding):
    """Candidate labels now describe the actual clipped capture rectangle."""
    left, top, right, bottom = range_boundaries(finding['cell'])
    return left <= 26 <= right and top <= 20 <= bottom


def verify(directory):
    report = load_report(directory)
    transitions = {item['label']: item for item in report['transitions']}
    for label in ('default input versus original input', 'original opening/saving control',
                  'default opening/saving control', 'SD rebuild-copy opening/saving control'):
        assert not any(change['location'] == 'cell:Z20' for change in transitions[label]['changes']), label
    assert any(change['location'] == 'cell:Z20'
               for change in transitions['SD explicit rebuild and save']['changes'])
    comparisons = {item['label']: item for item in report['comparisons']}
    for label, expected in (
        ('direct Default', False),
        ('Default rebuilt versus Original before rebuild', True),
        ('both rebuilt Default', True),
    ):
        finding = next(item for item in comparisons[label]['findings'] if covers_formula(item))
        assert finding['pixels_differ'] is expected, (label, finding)
        assert Path(finding['left_png']).is_file() and Path(finding['right_png']).is_file()
    original_control = next(item for item in comparisons['Original own rebuild']['findings'] if covers_formula(item))
    assert original_control['geometry_differs'] is False
    # Exact pixels may drift between Excel instances even with unchanged values.
    # Keep that control visible; verify the value independently in the saved XML.
    for name in ('original.xlsx', 'OR-rebuilt.before-rebuild.xlsx', 'OR-rebuilt.xlsx'):
        with zipfile.ZipFile(directory / name) as archive:
            sheet = ET.fromstring(archive.read('xl/worksheets/sheet1.xml'))
        cell = next(item for item in sheet.iter() if item.tag.endswith('}c') and item.attrib.get('r') == 'Z20')
        assert cell.find('{http://schemas.openxmlformats.org/spreadsheetml/2006/main}v').text == '4'
    assert not any(change['location'] == 'cell:Z20' for change in transitions['O explicit rebuild and save']['changes'])
    phases = report['phases']
    assert phases.index('variants rebuilt; Original still unchanged') < phases.index('Original rebuilt last')
    print('Late formula candidate, retained pre-rebuild evidence and Original-last ordering verified.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    verify(parser.parse_args().directory)
