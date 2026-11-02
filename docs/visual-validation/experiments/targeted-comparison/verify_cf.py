"""Require correct CF selection and actual Excel pixels for positive/negative controls."""
import json
import sys
from collections import Counter
from pathlib import Path


def verify(root: Path, mode: str) -> None:
    report = json.loads((root / 'report.json').read_text(encoding='utf-8'))
    check = next(item for item in report['conditional_formatting'] if item['left'] == 'O' and item['right'] == 'SD')
    decisions = check['decisions']
    assert decisions, 'No CF decisions were recorded'
    direct = next(item for item in report['comparisons'] if item['label'] == 'direct Default')
    different = any(item['pixels_differ'] or item['geometry_differs'] for item in direct['findings'])
    if direct['findings']:
        from PIL import Image
        sample = next((item for item in direct['findings'] if ':' in item['cell']), direct['findings'][0])
        colors = []
        for key in ('left_png', 'right_png'):
            with Image.open(sample[key]) as image:
                colors.append(Counter(image.convert('RGB').getdata()))
        if mode in ('constant', 'constant-string', 'styles', 'dummy-present'):
            assert colors[0][(255, 0, 0)] > 100, 'Positive CF control did not render its red fill'
        if mode == 'styles':
            assert colors[1][(0, 0, 255)] > 100, 'Changed differential format did not render blue'
        if mode == 'dummy-added':
            assert colors[1][(255, 0, 0)] > 100, 'New target did not enable its CF fill'
        if mode in ('duplicate', 'color-scale', 'data-bar'):
            from PIL import ImageChops
            with Image.open(sample['left_png']) as left, Image.open(sample['right_png']) as right:
                assert left.size == right.size
                # The changed value is in row 3; rows 1-2 must show an indirect CF change.
                region = (0, 0, left.width, left.height * 2 // 5)
                diff = ImageChops.difference(left.convert('RGB').crop(region), right.convert('RGB').crop(region))
                assert diff.getbbox(), 'Dependent unchanged cells were not visibly affected'
    if mode.startswith('ref-'):
        if mode == 'ref-mixed-group':
            assert any(item['outcome'] == 'capture' for item in decisions), decisions
        else:
            assert any(item['outcome'] == 'partial' for item in decisions), decisions
            assert not any(item['outcome'] == 'capture' for item in decisions), decisions
        assert different, 'Changed reference did not produce visual evidence'
        expected = {
            'ref-relative': 'B2:B2', 'ref-column': 'B2:D2', 'ref-row': 'B2:B5',
            'ref-fixed': 'B2:D5', 'ref-disjoint': 'B2:B2', 'ref-search': 'B2:B2',
            'ref-multiple': 'B2:D5', 'ref-cell-is': 'B2:D5',
            'ref-range': 'B2:B2', 'ref-range-fixed': 'B2:D5', 'ref-range-growing': 'B2:D5',
            'ref-mixed': 'B2:D5', 'ref-mixed-group': 'B2:D5',
            'ref-cross-relative': 'B2:B2', 'ref-cross-quoted': 'B2:D5', 'ref-cross-range': 'B2:D5',
        }[mode]
        finding = next(item for item in direct['findings'] if item['cell'] == expected)
        assert finding['pixels_differ'], 'Dependent target did not change in native Excel'
    elif mode == 'data-bar-unchanged':
        assert all(item['outcome'] == 'excluded' for item in decisions), decisions
        assert not different
    elif mode == 'dummy-missing':
        assert all(item['outcome'] == 'excluded' for item in decisions), decisions
        assert all(item['reason'] == 'fixed-dummy-targets-absent-in-both-states' for item in decisions), decisions
        assert not different
    elif mode in ('constant', 'constant-string'):
        assert any(item['outcome'] == 'partial' for item in decisions), decisions
        assert not any(item['outcome'] == 'capture' for item in decisions), decisions
        assert different, 'A changed local input must remain visibly compared'
    else:
        assert any(item['outcome'] == 'capture' for item in decisions), decisions
        assert direct['findings'], 'Fallback produced no captures'
        if mode == 'dummy-inert':
            assert not different, 'A missing-target CF rule unexpectedly changed Excel rendering'
        else:
            assert different, 'The positive control lost its visual difference'
    if report['check_performance'] != 'not_requested':
        assert any(item['left'] == 'O' and item['right'] == 'CP' for item in report['conditional_formatting'])
    print(f'{mode}: CF decisions and native Excel pixels verified.')


if __name__ == '__main__':
    verify(Path(sys.argv[1]), sys.argv[2])
