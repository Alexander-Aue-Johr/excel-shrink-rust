"""Verify unconditional chart coverage with identical worksheet XML inputs."""
import json
from report_audit import load_report
import sys
from pathlib import Path
import zipfile
import xml.etree.ElementTree as ET


def verify(directory: Path, collapsed_chart=False) -> None:
    report = load_report(directory)
    initial = next(item for item in report['transitions'] if item['label'] == 'default input versus original input')
    assert not initial['changes'], 'Chart selection must not depend on changed worksheet XML'
    direct = next(pair for pair in report['comparisons'] if pair['label'] == 'direct Default')
    findings = direct['charts']
    assert len(findings) == 8, findings
    for kind in ('embedded', 'chart_sheet'):
        selected = [item for item in findings if item['kind'] == kind]
        assert len(selected) == 4, selected
        assert sum(item['presence_differs'] for item in selected) == 2, selected
        common = [item for item in selected if not item['presence_differs']]
        assert len(common) == 2
        assert sum(item['pixels_differ'] for item in common) == 1, common
        if collapsed_chart and kind == 'embedded':
            collapsed = next(item for item in common if item['chart'] == 'Stable')
            assert collapsed['geometry_differs'] and collapsed['rendering_differs'], collapsed
            assert not collapsed['pixels_compared'], collapsed
            assert collapsed['right_rendering'] == 'zero-area-chart', collapsed
            assert collapsed['right_image'] is None, collapsed
        else:
            assert not any(item['geometry_differs'] for item in common), common
    if report['check_performance'] != 'not_requested':
        labels = {pair['label'] for pair in report['comparisons']}
        assert 'direct Check Performance' in labels
        assert 'Check Performance own rebuild' not in labels
    for pair in report['comparisons']:
        assert pair['charts'], pair['label']
        assert any(item['kind'] == 'chart_sheet' for item in pair['charts']), pair['label']
        for item in pair['charts']:
            for key in ('left_image', 'right_image'):
                if item[key] is not None:
                    assert Path(item[key]).is_file(), item
    for path in directory.glob('*.xlsx'):
        with zipfile.ZipFile(path) as archive:
            workbook = ET.fromstring(archive.read('xl/workbook.xml'))
        stable = next(sheet for sheet in workbook.findall('.//{*}sheet') if sheet.attrib['name'] == 'Sheet Stable')
        assert stable.attrib['state'] == 'hidden', f'Chart visibility was not restored: {path}'
    print('Unchanged, changed, added and removed charts verified across retained states.')


if __name__ == '__main__':
    verify(Path(sys.argv[1]), '--collapsed-chart' in sys.argv[2:])
