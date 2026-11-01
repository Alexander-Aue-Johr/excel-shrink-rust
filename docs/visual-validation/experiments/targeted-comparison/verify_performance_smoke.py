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
    """Require XML removal, CP-only differences, all captures and actual process exits."""
    report = json.loads((directory / 'report.json').read_text())
    telemetry = json.loads((directory / 'telemetry.json').read_text())
    evidence = report['check_performance_evidence']
    assert report['check_performance'] == 'optimization_confirmed'
    assert evidence['native_result']['method'] == 'window_messages'
    assert 'Z10000' in worksheet_cells(evidence['before_optimization'])
    assert 'Z10000' not in worksheet_cells(evidence['after_optimization'])
    assert evidence['optimization_changes']
    assert any(f['optimization_xml_changed'] and f['before_recalc_differs']
               for f in evidence['findings'])
    assert not any(f['original_recalc_drift'] for f in evidence['findings'])
    # Both supplied inputs are identical; only CP is allowed to introduce differences.
    assert not any(f['before_recalc_differs'] or f['after_shrink_recalc_differs']
                   or f['before_recalc_geometry_differs'] or f['after_shrink_recalc_geometry_differs']
                   for f in report['findings'])
    for lane in telemetry['lanes']:
        assert lane['pid'] and lane['lifecycle'] == 'closed'
        assert lane['process_alive'] is False
        assert lane['captures_done'] == lane['captures_total'] == report['candidate_count'] * 2
    for finding in evidence['findings']:
        assert len(finding['images']) == 4
        assert all(Path(path).is_file() for path in finding['images'].values())
    print('Real Check Performance XML removal, visual controls, evidence and cleanup verified.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    verify(parser.parse_args().directory)
