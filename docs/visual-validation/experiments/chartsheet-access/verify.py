"""Validate actual chart pixels, Office.js availability and source preservation."""
import json
import sys
from pathlib import Path

from PIL import Image, ImageChops


def verify(directory: Path) -> None:
    result = json.loads((directory / 'result.json').read_text(encoding='utf-8-sig'))
    assert not result.get('error'), result.get('error')
    assert result['standalone_count'] == 2
    assert result['pid'] == result['pid_after_export']
    assert 'UnsupportedSheet' in result['office_js_chart_sheet_error']
    assert result['source_unchanged']
    assert result['close']['closed']
    assert result['office_js_after_export']['charts'] == result['office_js_charts']['charts']
    images = []
    for export in result['exports']:
        assert export['bytes'] > 0
        with Image.open(export['path']) as image:
            images.append(image.convert('RGBA'))
    assert len(images) == 3
    assert images[0].size == images[1].size == images[2].size
    assert ImageChops.difference(images[0], images[1]).getbbox(alpha_only=False) is None
    difference = ImageChops.difference(images[0], images[2]).getbbox(alpha_only=False)
    assert difference is not None
    print(f'Owned PID {result["pid"]}; exports {images[0].size}; repeated pixels identical; changed-title bounds {difference}.')
    print('Office.js remained available, source unchanged, owned Excel closed.')


if __name__ == '__main__':
    verify(Path(sys.argv[1]))
