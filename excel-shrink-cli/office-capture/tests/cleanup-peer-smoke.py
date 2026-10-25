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
import urllib.request

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
    parser.add_argument('--groups', type=int, choices=(2, 4), default=2)
    parser.add_argument('--clients', type=int, choices=(1, 4), default=1)
    parser.add_argument('--reset-first', action='store_true', help='Reset one ready pane before any capture jobs.')
    parser.add_argument('--image-batches', type=int, default=0, help='Repeat 0 to 12 image-first batches in the surviving panes and record memory snapshots.')
    args = parser.parse_args()
    if not args.run:
        parser.error('Pass --run to open two isolated Excel test sessions')
    if not 0 <= args.image_batches <= 12:
        parser.error('--image-batches must be between 0 and 12')

    root = ROOT / '.tmp/cleanup-peer-check' / str(time.time_ns())
    root.mkdir(parents=True)
    source = root / 'source.xlsx'
    workbook = Workbook()
    workbook.active.title = 'Probe'
    workbook.active['A1'] = 'Peer survival'
    if args.image_batches:
        for row in range(2,1001):
            for column in range(1,9):
                workbook.active.cell(row,column,row*column)
    workbook.save(source)
    sheet = dict(index=1, name='Probe', bottom_row=999 if args.image_batches else 9, base_right_column=7 if args.image_batches else 3,
                 right_column=7 if args.image_batches else 3, horizontal_overflow_required=False,
                 drawing_fallback_required=False)
    sessions = []
    logs = []
    results = {}
    client_ids = ('one', 'two', 'three', 'four')[:args.clients]
    control = root / 'control'
    publish(control / 'control.json', {'paused':False, 'hide_excel':True})

    def queue_images(session, originals, number):
        command_id = f'images-{number:04d}'
        for client in client_ids:
            publish(session / f'commands/{client}/{command_id}.json', dict(id=command_id,
                kind='capture_images', capture_id='0001', tiles=originals[client]['tiles']))
        return command_id

    def check_images(session, process, originals, command_id):
        responses = {client:response(session / f'responses/{client}/{command_id}.json',process) for client in client_ids}
        for client, value in responses.items():
            expected = [(tile['range'],tile['width'],tile['height'],tile['sha256']) for tile in originals[client]['tiles']]
            actual = [(tile['range'],tile['width'],tile['height'],tile['sha256']) for tile in value['images']]
            assert actual == expected, f'Image-first pixels changed for {client}'
            assert value['pane_resources']['pending_png_characters'] == 0, value
        metrics = read(session / 'metrics.json')
        return dict(command=command_id, clients={client:value['pane_resources'] for client,value in responses.items()},
                    sampled_unix_ms=metrics.get('sampled_unix_ms'), processes=metrics.get('owned_process_resources'))
    try:
        for name in ('a', 'b', 'c', 'd')[:args.groups]:
            session = root / name
            for client in client_ids:
                publish(session / f'clients/{client}.json', dict(
                    client_id=client, source_path=str(source),
                    workbook=f'{client}.xlsx', sheets=[sheet]))
            stdout = (session / 'stdout.log').open('w')
            stderr = (session / 'stderr.log').open('w')
            logs.extend((stdout, stderr))
            process = subprocess.Popen([
                'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
                '-File', str(ROOT / 'excel-shrink-cli/office-capture/resident.ps1'),
                '-SessionPath', str(session), '-StartupTimeoutSeconds', '90',
                '-CaptureTimeoutSeconds', '240', '-HideExcel'],
                stdout=stdout, stderr=stderr, creationflags=0x08000000,
                env={**os.environ, 'EXCEL_SHRINK_CONTROL_ROOM_DIR':str(control),
                     'EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID':name,
                     'EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID':f'smoke-{name}'})
            sessions.append((session, process))
            deadline = time.monotonic() + 180
            while time.monotonic() < deadline:
                metrics = read(session / 'metrics.json') or {}
                if metrics.get('primary_clients_ready'):
                    break
                if process.poll() is not None: raise RuntimeError(f'Launcher {name} exited during startup: {metrics}')
                time.sleep(.2)
            else: raise TimeoutError(f'Add-in readiness {name}')
            if args.reset_first:
                target = metrics['clients']['one']
                reset_path = control / f'groups/{name}/client-resets/test.json'
                publish(reset_path, dict(id='test', status='pending', client_id='one',
                    group_id=name, run_id='smoke', attempt_id=f'smoke-{name}',
                    expected_pid=target['pid'], expected_started_unix_ms=target['process_started_unix_ms'],
                    hide_excel=True, reason='Native pane reset test'))
                deadline = time.monotonic() + 120
                while time.monotonic() < deadline:
                    reset = read(reset_path)
                    if reset.get('status') == 'complete': break
                    if reset.get('status') in ('failed','rejected','group_restart'): raise RuntimeError(reset)
                    if process.poll() is not None: raise RuntimeError(f'Launcher {name} exited during reset')
                    time.sleep(.2)
                else: raise TimeoutError(f'Reset {name}')
                print(name, 'reset one pane', flush=True)
            for client in client_ids:
                publish(session / f'commands/{client}/0001.json', dict(
                    id='0001', kind='capture', sheets=[sheet], tile_megapixels=32))
            results[name] = {client:response(session / f'responses/{client}/0001.json', process) for client in client_ids}
            metrics = read(session / 'metrics.json')
            for client in client_ids:
                sample = metrics['clients'][client]
                assert sample['startup_measurement_method'] == 'launch-to-addin-ready-v1', sample
                assert sample['startup_memory_private_bytes'] > 0 and sample['startup_memory_working_set_bytes'] > 0, sample
            results[f'{name}_startup'] = metrics['clients']
            print(name, 'captured', flush=True)

        commandlines = subprocess.check_output([
            'powershell.exe', '-NoProfile', '-Command',
            "Get-CimInstance Win32_Process -Filter \"name = 'msedgewebview2.exe'\" | "
            "Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress"],
            creationflags=0x08000000, text=True)
        (root / 'webview-processes.json').write_text(commandlines)
        for session, _ in sessions:
            profile = read(session / 'metrics.json')['webview2_user_data_folder']
            assert profile in commandlines.replace('\\\\', '\\'), profile
        survivor, peer = sessions[-1]
        survivor_name = ('a', 'b', 'c', 'd')[args.groups - 1]
        for index, (session, process) in enumerate(sessions[:-1], 2):
            # Dispatch real image uploads before closing the other job. This
            # exercises both in-flight work and subsequent peer re-use.
            overlapping = queue_images(survivor,results[survivor_name],index) if args.image_batches else None
            publish(session / 'finish.json', {'complete': True})
            exit_code = process.wait(timeout=40)
            metrics = read(session / 'metrics.json')
            assert exit_code == 0 and metrics['status'] == 'complete', metrics
            print(session.name, 'cleaned up', flush=True)
            if overlapping:
                results[f'during_cleanup_{session.name}'] = check_images(survivor,peer,results[survivor_name],overlapping)
            command_id = f'{index:04d}'
            for client in client_ids:
                publish(survivor / f'commands/{client}/{command_id}.json', dict(
                    id=command_id, kind='capture', sheets=[sheet], tile_megapixels=32))
            capture = {client:response(survivor / f'responses/{client}/{command_id}.json', peer) for client in client_ids}
            results[f'after_cleanup_{session.name}'] = capture
            for client in client_ids:
                assert capture[client]['tiles'] == results[survivor_name][client]['tiles'], results
            print(survivor_name, 'captured identical pixels after cleanup', flush=True)
        if args.image_batches:
            samples = []
            for index in range(args.image_batches):
                command_id = queue_images(survivor,results[survivor_name],100 + index)
                samples.append(check_images(survivor,peer,results[survivor_name],command_id))
            for client in client_ids:
                publish(survivor / f'commands/{client}/release.json',dict(id='release',kind='release'))
            released = {client:response(survivor / f'responses/{client}/release.json',peer) for client in client_ids}
            for client,value in released.items():
                assert value['pane_resources']['retained_tiles'] == 0 and value['pane_resources']['geometry_rows'] == 0, value
            results['memory_samples'] = samples
            results['after_release'] = released
            print('Image-first batches and metadata release passed:',args.image_batches,flush=True)
        results['passed'] = True
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
