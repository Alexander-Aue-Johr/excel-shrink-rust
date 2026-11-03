"""Verify a late formula candidate after rebuilding the existing SD instance."""
import argparse
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile

from openpyxl.utils.cell import range_boundaries
from report_audit import load_report


def covers_formula(finding):
    left, top, right, bottom = range_boundaries(finding['cell'])
    return left <= 26 <= right and top <= 20 <= bottom


def verify(directory):
    report = load_report(directory)
    assert report['execution_strategy'] == 'default-inplace-rebuild-v1'
    assert report['excel_pids']['SD'] == report['excel_pids']['SDR']
    assert report['state_decisions']['OR']['status'] == 'disabled'
    transitions = {item['label']: item for item in report['transitions']}
    for label in ('default input versus original input', 'original opening/saving control',
                  'default opening/saving control'):
        assert not any(change['location'] == 'cell:Z20' for change in transitions[label]['changes'])
    rebuild = transitions['Default in-place explicit rebuild and save']
    assert any(change['location'] == 'cell:Z20' for change in rebuild['changes'])
    rebuilt = next(pair for pair in report['comparisons'] if pair['right'] == 'SDR')
    finding = next(item for item in rebuilt['findings'] if covers_formula(item))
    assert finding['pixels_differ']
    assert Path(finding['left_png']).is_file() and Path(finding['right_png']).is_file()
    for name, expected in [('original.xlsx', '4'), ('default.xlsx', '4'), ('SDR-rebuilt.xlsx', '0')]:
        with zipfile.ZipFile(directory / name) as archive:
            sheet = ET.fromstring(archive.read('xl/worksheets/sheet1.xml'))
        cell = next(item for item in sheet.iter() if item.tag.endswith('}c') and item.attrib.get('r') == 'Z20')
        assert cell.find('{http://schemas.openxmlformats.org/spreadsheetml/2006/main}v').text == expected
    print('Indirect formula difference captured after SD rebuild without reopening SD or rebuilding Original.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    verify(parser.parse_args().directory)