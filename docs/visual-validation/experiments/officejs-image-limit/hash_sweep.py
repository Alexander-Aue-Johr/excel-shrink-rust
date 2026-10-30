"""Hash every Office.js PNG in the tall conditional-formatting tail of Lysimeter.

Run with Windows Python. A resident opens the real workbook once, then captures
bounded batches. The script persists only SHA-256 hashes and timing metadata.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import run as resident


FIRST_ROW = 70_953
LAST_ROW = 1_048_576
ROWS_PER_TILE = 1_700
BATCH_TILES = 20
SHEET_NAME = '1990 E Maize Lys 5-Min'


def file_sha256(path):
    """Identify the exact input workbook without loading it into memory."""
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def tile_starts(first_row, last_row, rows_per_tile, max_tiles):
    """List the contiguous capture starts, optionally stopping after a pilot."""
    starts = range(first_row, last_row + 1, rows_per_tile)
    if max_tiles is not None:
        starts = list(starts)[:max_tiles]
    return starts


def aggregate_sha256(tiles):
    """Hash ordered range/hash pairs so omitted or reordered tiles change the root."""
    digest = hashlib.sha256()
    for tile in tiles:
        digest.update(f'{tile["range"]}\0{tile["sha256"]}\n'.encode('ascii'))
    return digest.hexdigest()


def start_client(directory, workbook, binary, timeout):
    """Stage and start one private resident client for the real workbook."""
    session = directory / 'session'
    control = directory / 'control'
    resident.publish(control / 'control.json', {'paused': False, 'hide_excel': True})
    resident.publish(session / 'capacity.json', {'reserved_clients': 1})
    resident.publish(session / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(workbook), 'workbook': 'one.xlsx',
        'sheets': [{'index': 3, 'name': SHEET_NAME, 'bottom_row': 105120,
                    'base_right_column': 41, 'capture_right_column': 105,
                    'horizontal_overflow_required': True,
                    'drawing_fallback_required': False}],
    })
    environment = {**os.environ, 'EXCEL_SHRINK_CONTROL_ROOM_DIR': str(control),
                   'EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID': 'lysimeter-hash-sweep',
                   'EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID': 'lysimeter-hash-sweep'}
    stdout = (directory / 'stdout.log').open('wb')
    stderr = (directory / 'stderr.log').open('wb')
    try:
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
            '-File', str(resident.COPY / 'resident.ps1'), '-SessionPath', str(session),
            '-HostExecutable', str(binary), '-StartupTimeoutSeconds', '300',
            '-CaptureTimeoutSeconds', str(timeout), '-HideExcel',
        ], stdout=stdout, stderr=stderr, env=environment)
    finally:
        stdout.close()
        stderr.close()
    try:
        resident.wait_until(lambda: True if (resident.read_json(session / 'metrics.json') or {}).get(
            'primary_clients_ready') else None, process, 300)
    except Exception:
        close_client(session, process)
        raise
    return process, session


def capture_batch(session, process, batch_index, first_row, last_row, rows_per_tile):
    """Run at most 20 captures and return their hashes without their PNG bytes."""
    command_id = f'sweep{batch_index:03d}'
    resident.publish(session / f'commands/one/{command_id}.json', {
        'id': command_id, 'kind': 'diagnosis', 'operation': 'hash_sweep',
        'sheet_name': SHEET_NAME, 'first_row': first_row,
        'last_row': last_row, 'rows_per_tile': rows_per_tile,
    })
    response = resident.wait_until(lambda: resident.read_json(
        session / f'responses/one/{command_id}.json'), process, 900)
    if response.get('ok') is not True or response.get('diagnostic_error'):
        raise RuntimeError(f'Batch {batch_index} failed: {response.get("diagnostic_error") or response.get("error")}')
    return response['tiles']


def close_client(session, process):
    """Request a clean close, then terminate a resident that does not exit."""
    resident.publish(session / 'finish.json', {'complete': True})
    try:
        process.wait(timeout=45)
    except subprocess.TimeoutExpired:
        subprocess.run(['taskkill', '/F', '/T', '/PID', str(process.pid)], timeout=20,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def sweep(directory, workbook, binary, first_row, last_row, rows_per_tile, max_tiles, timeout):
    """Persist each completed batch so an interrupted run keeps measured evidence."""
    starts = tile_starts(first_row, last_row, rows_per_tile, max_tiles)
    manifest = {
        'workbook': str(workbook), 'workbook_sha256': file_sha256(workbook),
        'sheet_name': SHEET_NAME, 'requested_range': f'J{first_row}:J{last_row}',
        'rows_per_tile': rows_per_tile, 'tiles': [], 'complete': False,
    }
    process, session = start_client(directory, workbook, binary, timeout)
    started = time.monotonic()
    try:
        for offset in range(0, len(starts), BATCH_TILES):
            batch = starts[offset:offset + BATCH_TILES]
            end = min(last_row, batch[-1] + rows_per_tile - 1)
            tiles = capture_batch(session, process, offset // BATCH_TILES,
                                  batch[0], end, rows_per_tile)
            if len(tiles) != len(batch):
                raise RuntimeError(f'Batch returned {len(tiles)} of {len(batch)} hashes')
            manifest['tiles'].extend(tiles)
            manifest['elapsed_seconds'] = round(time.monotonic() - started, 3)
            manifest['aggregate_sha256'] = aggregate_sha256(manifest['tiles'])
            metrics = resident.read_json(session / 'metrics.json') or {}
            client = metrics.get('clients', {}).get('one', {})
            manifest['peak_private_bytes'] = client.get('peak_private_bytes')
            manifest['peak_working_set_bytes'] = client.get('peak_working_set_bytes')
            resident.publish(directory / 'manifest.json', manifest)
            print(json.dumps({'completed_tiles': len(manifest['tiles']),
                              'last_range': tiles[-1]['range'],
                              'elapsed_seconds': manifest['elapsed_seconds']}), flush=True)
        manifest['complete'] = bool(starts) and manifest['tiles'][-1]['range'].endswith(f':J{last_row}')
        resident.publish(directory / 'manifest.json', manifest)
    finally:
        close_client(session, process)
    return manifest


def main():
    """Validate the native inputs and run the requested pilot or full sweep."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workbook', required=True, type=Path)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--first-row', type=int, default=FIRST_ROW)
    parser.add_argument('--last-row', type=int, default=LAST_ROW)
    parser.add_argument('--rows-per-tile', type=int, default=ROWS_PER_TILE)
    parser.add_argument('--max-tiles', type=int, help='Stop after this many tiles for a pilot')
    parser.add_argument('--timeout', type=int, default=1800)
    args = parser.parse_args()
    if os.name != 'nt':
        parser.error('Run with Windows Python')
    if not args.workbook.is_file() or not args.binary.is_file():
        parser.error('Workbook and Windows host executable must exist')
    if not 1 <= args.first_row <= args.last_row <= LAST_ROW or not 1 <= args.rows_per_tile <= 1700:
        parser.error('Invalid row range or tile size')
    if args.max_tiles is not None and args.max_tiles < 1:
        parser.error('--max-tiles must be positive')
    if not 300 <= args.timeout <= 7200:
        parser.error('--timeout must be 300..7200 seconds')

    directory = Path(tempfile.gettempdir()) / f'lysimeter-hash-sweep-{time.time_ns()}'
    directory.mkdir(parents=True)
    resident.prepare_worker_copy()
    try:
        result = sweep(directory, args.workbook, args.binary, args.first_row,
                       args.last_row, args.rows_per_tile, args.max_tiles, args.timeout)
        print(json.dumps({'manifest': str(directory / 'manifest.json'),
                          'complete': result['complete'],
                          'tiles': len(result['tiles']),
                          'elapsed_seconds': result['elapsed_seconds']}), flush=True)
    finally:
        shutil.rmtree(resident.COPY)


if __name__ == '__main__':
    main()
