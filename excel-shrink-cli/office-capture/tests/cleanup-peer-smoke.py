"""Opt-in native Excel check: clean up session A, then capture again in B.

Requires Windows, Excel and openpyxl. Existing Office processes may mask shared
runtime dependencies; success does not prove isolation on every desktop.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from openpyxl import Workbook

ROOT = Path(__file__).resolve().parents[3]


def publish(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix('.pending')
    pending.write_text(json.dumps(value), encoding='utf-8')
    os.replace(pending, path)


def read(path):
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except (OSError, ValueError):
        return None


def response(path, process):
    deadline = time.monotonic() + 100
    while time.monotonic() < deadline:
        value = read(path)
        if value is not None:
            if value.get('ok') is not True:
                raise RuntimeError(value)
            return value
        if process.poll() is not None:
            raise RuntimeError(f'Launcher exited before {path}')
        time.sleep(.2)
    raise TimeoutError(str(path))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', action='store_true')
    args = parser.parse_args()
    if not args.run:
        parser.error('Pass --run to open two isolated Excel test sessions')

    root = ROOT / '.tmp/cleanup-peer-check' / str(time.time_ns())
    root.mkdir(parents=True)
    source = root / 'source.xlsx'
    workbook = Workbook()
    workbook.active.title = 'Probe'
    workbook.active['A1'] = 'Peer survival'
    workbook.save(source)
    sheet = dict(index=1, name='Probe', bottom_row=9, base_right_column=3,
                 right_column=3, horizontal_overflow_required=False,
                 drawing_fallback_required=False)
    sessions = []
    logs = []
    results = {}
    try:
        for name in ('a', 'b'):
            session = root / name
            publish(session / 'clients/one.json', dict(
                client_id='one', source_path=str(source),
                workbook='one.xlsx', sheets=[sheet]))
            publish(session / 'commands/one/0001.json', dict(
                id='0001', kind='capture', sheets=[sheet], tile_megapixels=32))
            stdout = (session / 'stdout.log').open('w')
            stderr = (session / 'stderr.log').open('w')
            logs.extend((stdout, stderr))
            process = subprocess.Popen([
                'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
                '-File', str(ROOT / 'excel-shrink-cli/office-capture/resident.ps1'),
                '-SessionPath', str(session), '-StartupTimeoutSeconds', '90',
                '-CaptureTimeoutSeconds', '240', '-HideExcel'],
                stdout=stdout, stderr=stderr, creationflags=0x08000000)
            sessions.append((session, process))
            results[name] = response(session / 'responses/one/0001.json', process)
            print(name, 'captured', flush=True)

        session, process = sessions[0]
        publish(session / 'finish.json', {'complete': True})
        results['a_exit'] = process.wait(timeout=40)
        metrics = read(session / 'metrics.json')
        assert results['a_exit'] == 0 and metrics['status'] == 'complete', metrics
        print('a cleaned up', flush=True)

        session, process = sessions[1]
        publish(session / 'commands/one/0002.json', dict(
            id='0002', kind='capture', sheets=[sheet], tile_megapixels=32))
        results['after_cleanup'] = response(session / 'responses/one/0002.json', process)
        assert results['after_cleanup']['tiles'] == results['b']['tiles'], results
        results['passed'] = True
        print('b captured identical pixels after a cleanup', flush=True)
    except Exception as error:
        results['error'] = repr(error)
        print(repr(error), flush=True)
    finally:
        for session, process in sessions:
            if process.poll() is None:
                publish(session / 'finish.json', {'complete': True})
        for session, process in sessions:
            try:
                process.wait(timeout=40)
            except subprocess.TimeoutExpired:
                # Only this test's owned launcher; its Job Object owns its Excel.
                process.terminate()
                process.wait(timeout=10)
        for log in logs:
            log.close()
        (root / 'receipt.json').write_text(json.dumps(results, indent=2))
        print('Receipt:', root / 'receipt.json', flush=True)
    return 0 if results.get('passed') else 1


if __name__ == '__main__':
    raise SystemExit(main())
