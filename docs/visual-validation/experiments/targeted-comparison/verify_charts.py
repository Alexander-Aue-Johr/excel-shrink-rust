"""Verify unconditional chart coverage with identical worksheet XML inputs."""
import json
import sys
from pathlib import Path


def verify(directory: Path) -> None:
    report = json.loads((directory / 'report.json').read_text(encoding='utf-8'))
    initial = next(item for item in report['transitions'] if item['label'] == 'default input versus original input')
    assert not initial['changes'], 'Chart selection must not depend on changed worksheet XML'
    direct = next(pair for pair in report['comparisons'] if pair['label'] == 'direct Default')
    findings = direct['charts']
    assert len(findings) == 4, findings
    assert sum(item['presence_differs'] for item in findings) == 2, findings
    common = [item for item in findings if not item['presence_differs']]
    assert len(common) == 2
    assert sum(item['pixels_differ'] for item in common) == 1, common
    if report['check_performance'] != 'not_requested':
        labels = {pair['label'] for pair in report['comparisons']}
        assert 'direct Check Performance' in labels
        assert 'Check Performance own rebuild' in labels
    for pair in report['comparisons']:
        assert pair['charts'], pair['label']
        for item in pair['charts']:
            for key in ('left_image', 'right_image'):
                if item[key] is not None:
                    assert Path(item[key]).is_file(), item
    print('Unchanged, changed, added and removed charts verified across retained states.')


if __name__ == '__main__':
    verify(Path(sys.argv[1]))
