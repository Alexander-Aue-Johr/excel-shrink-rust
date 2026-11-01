"""Check the retained measurements against independently known synthetic geometry."""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

ROWS = 1_048_576


def worksheet_digest(path, remove_column_style=False):
    """Hash streamed worksheet content, optionally normalizing the one column-style attribute."""
    digest = hashlib.sha256()
    rows = 0
    with zipfile.ZipFile(path) as archive, archive.open('xl/worksheets/sheet1.xml') as sheet:
        # The generated header is short; stop before the first row so body alignment is identical.
        prefix = bytearray()
        while not prefix.endswith(b'<sheetData>'):
            byte = sheet.read(1)
            if not byte or len(prefix) > 4096:
                raise AssertionError('Unexpected worksheet header')
            prefix.extend(byte)
        if remove_column_style:
            assert prefix.count(b' style="1"') == 1
            prefix = prefix.replace(b' style="1"', b'')
        digest.update(prefix)
        tail = b''
        while chunk := sheet.read(1024 * 1024):
            digest.update(chunk)
            combined = tail + chunk
            rows += combined.count(b'<row ')
            tail = combined[-4:]
    assert rows == ROWS, f'Expected {ROWS} populated rows, got {rows}'
    return digest.hexdigest()


def verify(directory):
    """Reject incorrect uniform/outlier results and any false completeness in the bounded stress case."""
    raw = json.loads((directory / 'results.json').read_text())
    results = {result['case']: result for result in raw['results']}
    assert len(results) == 5
    expected_outliers = [
        {'start_row': 1, 'end_row': 8191, 'height': 15},
        {'start_row': 8192, 'end_row': 8192, 'height': 0},
        {'start_row': 8193, 'end_row': 524287, 'height': 15},
        {'start_row': 524288, 'end_row': 524288, 'height': 30},
        {'start_row': 524289, 'end_row': 1048575, 'height': 15},
        {'start_row': 1048576, 'end_row': 1048576, 'height': 30},
    ]
    for case, result in results.items():
        assert not result.get('error'), result.get('error')
        metrics = result['final_metrics']
        assert all(client.get('process_alive') is False and client.get('closed') is True
                   for client in metrics['clients'].values())
        worksheet_digest(directory / case / 'input.xlsx')
        if case == 'alternating':
            first = result['measurements'][0]
            assert first['complete'] is False
            assert 'deadline' in first.get('error', '') or 'request budget' in first.get('error', '')
            continue
        assert len(result['measurements']) == 2
        for measurement in result['measurements']:
            assert measurement['complete'] is True
            if case == 'outliers':
                assert measurement['sample_runs'] == expected_outliers
                assert measurement['total_height'] == 15 * ROWS + 15
            else:
                expected_height = 39 if case == 'column-font' else 15
                assert measurement['sample_runs'] == [
                    {'start_row': 1, 'end_row': ROWS, 'height': expected_height}]
                assert measurement['total_height'] == ROWS * expected_height
    before = worksheet_digest(directory / 'column-font/input.xlsx', remove_column_style=True)
    after = worksheet_digest(directory / 'column-font-removed/input.xlsx')
    assert before == after, 'Comparison pair differs beyond its column-format attribute'
    print('All full-row fixtures, known heights, column-only mutation and cleanup verified.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    verify(parser.parse_args().directory)
