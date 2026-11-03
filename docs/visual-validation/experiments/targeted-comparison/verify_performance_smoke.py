"""Verify real optimization evidence, independent controls, and confirmed cleanup."""
import argparse
import json
from pathlib import Path
import zipfile
import xml.etree.ElementTree as ET


def worksheet_cells(path):
    """Read cell addresses from the small synthetic worksheet."""
    with zipfile.ZipFile(path) as archive:
        sheet = ET.fromstring(archive.read('xl/worksheets/sheet1.xml'))
    return {
        cell.attrib['r'] for cell in sheet.iter()
        if cell.tag.endswith('}c')
    }


def verify(directory):
    """Require XML removal, auditable exterior skips, in-content controls and actual exits."""
    report = json.loads((directory / 'report.json').read_text())
    telemetry = json.loads((directory / 'telemetry.json').read_text())
    evidence = report['check_performance_evidence']
    assert report['check_performance'] == 'optimization_confirmed'
    assert evidence['native_result']['method'] == 'window_messages'
    assert 'Z10000' in worksheet_cells(evidence['before_optimization'])
    assert 'Z10000' not in worksheet_cells(evidence['after_optimization'])
    optimization = next(item for item in report['transitions']
                        if item['label'] == 'Check Performance optimization')
    assert optimization['skipped_outside_content_cell_changes']['Rows'] >= 1
    assert not evidence['optimization_changes'], 'The removed formatting-only tail must not be captured.'
    assert report['state_decisions']['OR']['status'] == 'disabled'
    assert report['state_decisions']['CPR']['status'] == 'disabled'
    for lane in telemetry['lanes']:
        if lane['pid']:
            assert lane['lifecycle'] == 'closed' and lane['process_alive'] is False
    direct = next(pair for pair in report['comparisons'] if pair['right'] == 'CP')
    for finding in direct['findings']:
        for side in ('left', 'right'):
            path = finding[f'{side}_png']
            if path is not None:
                assert Path(path).is_file()
    print('Real Check Performance XML removal, visual controls, evidence and cleanup verified.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    verify(parser.parse_args().directory)
