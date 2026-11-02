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
    assert not any(f['original_recalc_drift'] for f in evidence['findings'])
    # Both supplied inputs are identical; only CP is allowed to introduce differences.
    assert not any(f['before_recalc_differs'] or f['after_shrink_recalc_differs']
                   or f['before_recalc_geometry_differs'] or f['after_shrink_recalc_geometry_differs']
                   for f in report['findings'])
    for lane in telemetry['lanes']:
        assert lane['pid'] and lane['lifecycle'] == 'closed'
        assert lane['process_alive'] is False
        assert lane['captures_done'] == lane['captures_total'] == report['candidate_count']
    for finding in evidence['findings']:
        assert len(finding['images']) == 4
        assert all(Path(path).is_file() for path in finding['images'].values())
    print('Real Check Performance XML removal, visual controls, evidence and cleanup verified.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    verify(parser.parse_args().directory)
