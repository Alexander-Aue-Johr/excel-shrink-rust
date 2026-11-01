"""Compare native Excel startup on one versus two private Windows desktops."""
import argparse
import hashlib
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback
import zipfile

from fixture import ROWS, package_workbook, write_numeric_sheet
from resources import available_memory, machine_info, process_cpu_seconds
from report import write_report

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
MEMORY_RESERVE = 8 * 1024**3


def publish(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix('.writing')
    temporary.write_text(json.dumps(value, indent=2), encoding='utf-8')
    temporary.replace(path)


def read_json(path):
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except (OSError, ValueError):
        return None  # Launcher files can be observed during a write.


def progress(directory, phase, **details):
    publish(directory / 'progress.json', {'phase': phase, **details})


def prepare_worker(directory):
    """Add a scheduled-start barrier to a disposable copy, never to the real runner."""
    worker = directory / 'worker/excel-shrink-cli/office-capture'
    shutil.copytree(ROOT / 'excel-shrink-cli/office-capture', worker)
    regression = worker.parent / 'src/regression'
    regression.mkdir(parents=True)
    shutil.copy2(ROOT / 'excel-shrink-cli/src/regression/archive_processes.ps1', regression)
    launcher = worker / 'resident.ps1'
    source = launcher.read_text(encoding='utf-8')
    replacements = {
        '        $desktopName = $null': '        $desktopName = $env:EXCEL_SHRINK_EXPERIMENT_DESKTOP',
        '        $openClock = [Diagnostics.Stopwatch]::StartNew()': (
            '\n'.join('        ' + line for line in (HERE / 'launch-barrier.ps1').read_text().splitlines())
            + '\n        $openClock = [Diagnostics.Stopwatch]::StartNew()'),
        '        $process = $owned.Start($excelExecutable, $arguments, [bool]$hideClient, $desktopName)': (
            '        $process = $owned.Start($excelExecutable, $arguments, [bool]$hideClient, $desktopName)\n'
            '        $launchEvidence = @{requested_at=$experimentRequestedAt; pid=$process.Id; '
            'desktop=$desktopName; target_start_at=$gate.start_at}\n'
            '        [IO.File]::WriteAllText($env:EXCEL_SHRINK_EXPERIMENT_LAUNCHED, '
            '($launchEvidence | ConvertTo-Json), $utf8)')
    }
    for anchor, replacement in replacements.items():
        if source.count(anchor) != 1:
            raise RuntimeError(f'Launcher extension point changed: {anchor}')
        source = source.replace(anchor, replacement)
    launcher.write_text(source, encoding='utf-8')
    return worker


@dataclass
class Resident:
    directory: Path
    process: subprocess.Popen
    stdout: object
    stderr: object


def start_resident(directory, worker, fixture, desktop, gate):
    publish(directory / 'session/capacity.json', {'reserved_clients': 1})
    publish(directory / 'session/clients/one.json', {
        'client_id': 'one', 'source_path': str(fixture), 'workbook': 'one.xlsx',
        'sheets': [{'index': 1, 'name': 'Data 1', 'bottom_row': ROWS - 1,
                    'base_right_column': 2, 'right_column': 2,
                    'horizontal_overflow_required': False, 'drawing_fallback_required': False}]})
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith('EXCEL_SHRINK_')}
    environment.pop('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', None)
    environment.update({
        'EXCEL_SHRINK_SYSTEM_RESERVE_MIB': str(MEMORY_RESERVE // 1024**2),
        'EXCEL_SHRINK_EXPERIMENT_DESKTOP': desktop,
        'EXCEL_SHRINK_EXPERIMENT_GATE': str(gate),
        'EXCEL_SHRINK_EXPERIMENT_PREPARED': str(directory / 'prepared.json'),
        'EXCEL_SHRINK_EXPERIMENT_LAUNCHED': str(directory / 'launched.json')})
    stdout = (directory / 'stdout.log').open('wb')
    stderr = (directory / 'stderr.log').open('wb')
    try:
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-STA', '-ExecutionPolicy', 'Bypass',
            '-File', str(worker / 'resident.ps1'), '-SessionPath', str(directory / 'session'),
            '-HostExecutable', str(ROOT / 'target/release/excel-shrink-cli.exe'),
            '-StartupTimeoutSeconds', '600', '-CaptureTimeoutSeconds', '120', '-Visible'
        ], env=environment, stdout=stdout, stderr=stderr, creationflags=subprocess.CREATE_NO_WINDOW)
    except BaseException:
        stdout.close()
        stderr.close()
        raise
    return Resident(directory, process, stdout, stderr)


def ensure_safe_and_running(residents):
    if available_memory() < MEMORY_RESERVE:
        raise RuntimeError('Free RAM fell below the 8 GiB experiment reserve; stopping owned sessions.')
    for resident in residents:
        if resident.process.poll() is not None:
            raise RuntimeError(f'Experiment launcher exited early; see {resident.directory / "stderr.log"}')


def verified_ready_at(client):
    """Use Office.js verification, independently of the launcher's later COM sample."""
    monitor = client.get('monitor') or {}
    for event in monitor.get('events') or []:
        if event.get('kind') == 'addin_ready':
            return event['at']
    return None


def release_start_gate(residents, gate):
    """Wait until staging is complete, then give all launchers one shared UTC target."""
    deadline = time.monotonic() + 120
    while not all((resident.directory / 'prepared.json').exists() for resident in residents):
        ensure_safe_and_running(residents)
        if time.monotonic() > deadline:
            raise TimeoutError('Launchers did not reach the prepared barrier.')
        time.sleep(0.1)
    start = datetime.now(timezone.utc) + timedelta(seconds=2)
    publish(gate, {'start_at': start.isoformat()})


def observe_readiness(residents):
    """Sample real Excel CPU activity until every add-in has verified its workbook."""
    deadline = time.monotonic() + 600
    samples = []
    while True:
        ensure_safe_and_running(residents)
        observations = []
        for resident in residents:
            metrics = read_json(resident.directory / 'session/metrics.json') or {}
            client = metrics.get('clients', {}).get('one', {})
            pid = client.get('pid')
            observations.append({
                'client': resident.directory.name, 'pid': pid,
                'ready': verified_ready_at(client) is not None,
                'cpu_seconds': process_cpu_seconds(pid) if pid else None,
                'private_bytes': client.get('private_bytes'),
                'peak_private_bytes': client.get('peak_private_bytes')})
        samples.append({'at': datetime.now(timezone.utc).isoformat(),
                        'available_bytes': available_memory(), 'clients': observations})
        if all(observation['ready'] for observation in observations):
            return samples
        if time.monotonic() > deadline:
            raise TimeoutError('Workbook readiness exceeded 600 seconds.')
        time.sleep(0.25)


def close_residents(residents):
    for resident in residents:
        publish(resident.directory / 'session/finish.json', {'complete': True})
    for resident in residents:
        try:
            try:
                resident.process.wait(timeout=25)
            except subprocess.TimeoutExpired:
                resident.process.kill()
                resident.process.wait(timeout=10)
        finally:
            resident.stdout.close()
            resident.stderr.close()


def measure_openings(directory, worker, fixture, desktops):
    """Stage independent residents, release one timed gate, and retain readiness evidence."""
    residents = []
    gate = directory / 'start.json'
    try:
        for index, desktop in enumerate(desktops, 1):
            residents.append(start_resident(directory / f'client-{index}', worker, fixture, desktop, gate))
        release_start_gate(residents, gate)
        samples = observe_readiness(residents)
        clients = []
        for resident in residents:
            launch = read_json(resident.directory / 'launched.json')
            metrics = read_json(resident.directory / 'session/metrics.json')
            client = metrics['clients']['one']
            ready_at = verified_ready_at(client)
            if ready_at is None:
                raise RuntimeError('Verified add-in readiness disappeared before recording the result.')
            opening_seconds = (datetime.fromisoformat(ready_at.replace('Z', '+00:00'))
                - datetime.fromisoformat(launch['requested_at'].replace('Z', '+00:00'))).total_seconds()
            clients.append({**launch, 'ready_at': ready_at,
                            'startup_seconds': opening_seconds,
                            'launcher_startup_sampled': client.get('startup_seconds') is not None,
                            'peak_private_bytes': client['peak_private_bytes'],
                            'process_started_unix_ms': client['process_started_unix_ms']})
        starts = [datetime.fromisoformat(client['requested_at'].replace('Z', '+00:00')) for client in clients]
        ends = [datetime.fromisoformat(client['ready_at'].replace('Z', '+00:00')) for client in clients]
        report = {'clients': clients, 'launch_skew_seconds': (max(starts) - min(starts)).total_seconds(),
                  'batch_seconds': (max(ends) - min(starts)).total_seconds(),
                  'minimum_available_bytes': min(sample['available_bytes'] for sample in samples)}
        publish(directory / 'samples.json', samples)
        publish(directory / 'result.json', report)
        return report
    finally:
        close_residents(residents)


def calibrate_fixture(directory, worker, desktop, target_seconds):
    """Increase sheet count toward the target, bounded at eight to limit test size."""
    fixtures = directory / 'fixtures'
    fixtures.mkdir()
    progress(directory, 'Generating one million rows', columns=3)
    sheet_xml = fixtures / 'million-rows.xml'
    write_numeric_sheet(sheet_xml)
    sheet_count = 1
    while True:
        progress(directory, 'Calibrating workbook', sheets=sheet_count)
        fixture = package_workbook(fixtures / f'{sheet_count}-sheets.xlsx', sheet_xml, sheet_count)
        measured = measure_openings(directory / f'calibration-{sheet_count}', worker, fixture, [desktop])
        seconds = measured['clients'][0]['startup_seconds']
        if seconds >= target_seconds or sheet_count == 8:
            return fixture, sheet_count, seconds
        sheet_count = min(8, max(sheet_count + 1, math.ceil(sheet_count * target_seconds / max(seconds, 1))))


def compare_desktop_arrangements(directory, worker, fixture, desktops, repetitions, workbooks):
    """Reverse arrangement order on alternating rounds to expose ordering effects."""
    reports = []
    arrangements = ['sequential', 'same_desktop']
    if workbooks == 2:
        arrangements.append('separate_desktops')
    for repetition in range(1, repetitions + 1):
        order = arrangements if repetition % 2 else list(reversed(arrangements))
        for arrangement in order:
            progress(directory, 'Measuring startup', repetition=repetition, arrangement=arrangement)
            trial = directory / f'round-{repetition}-{arrangement}'
            if arrangement == 'sequential':
                openings = [measure_openings(trial / f'opening-{index}', worker, fixture, [desktops[0]])
                            for index in range(1, workbooks + 1)]
                report = {'batch_seconds': sum(opening['batch_seconds'] for opening in openings),
                          'clients': [client for opening in openings for client in opening['clients']],
                          'launch_skew_seconds': None}
            else:
                targets = [desktops[0]] * workbooks if arrangement == 'same_desktop' else desktops
                report = measure_openings(trial, worker, fixture, targets)
            reports.append({'repetition': repetition, 'arrangement': arrangement, **report})
            publish(directory / 'measurements.json', reports)
    return reports


def inspect_existing_fixture(path):
    """Reuse only the generated workbook geometry expected by this experiment."""
    with zipfile.ZipFile(path) as archive:
        sheets = [name for name in archive.namelist()
                  if name.startswith('xl/worksheets/sheet') and name.endswith('.xml')]
        if not sheets:
            raise ValueError('No synthetic sheets found')
        for sheet in sheets:
            with archive.open(sheet) as content:
                if f'<dimension ref="A1:C{ROWS}"/>'.encode() not in content.read(512):
                    raise ValueError('Existing fixture does not match the generated geometry')
    return len(sheets)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--desktop-one', required=True)
    parser.add_argument('--desktop-two', required=True)
    parser.add_argument('--target-seconds', type=float, default=45)
    parser.add_argument('--repetitions', type=int, default=2)
    parser.add_argument('--fixture', type=Path)
    parser.add_argument('--workbooks', type=int, choices=[2, 4], default=2)
    options = parser.parse_args()
    if not 1 <= options.repetitions <= 4 or not 10 <= options.target_seconds <= 120:
        parser.error('Use 1-4 repetitions and a 10-120 second sizing target')
    desktops = [options.desktop_one, options.desktop_two]
    if len(set(desktops)) != 2 or any(not name.startswith('es-capture-')
                                    or not name.replace('-', '').isalnum() for name in desktops):
        parser.error('Provide two distinct experiment-owned desktop names')
    directory = options.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    try:
        worker = prepare_worker(directory)
        if options.fixture:
            fixture = options.fixture.resolve()
            sheets = inspect_existing_fixture(fixture)
            seconds = None
        else:
            fixture, sheets, seconds = calibrate_fixture(directory, worker, options.desktop_one, options.target_seconds)
        report = {'fixture': str(fixture), 'rows_per_sheet': ROWS, 'columns': 3, 'sheets': sheets,
                  'calibration_seconds': seconds, 'workbooks_per_trial': options.workbooks, 'measurement': 'native-process-start-to-verified-addin-ready',
                  'cpu_count': os.cpu_count(), 'initial_available_bytes': available_memory()}
        publish(directory / 'fixture.json', report)
        results = compare_desktop_arrangements(directory, worker, fixture,
            [options.desktop_one, options.desktop_two], options.repetitions, options.workbooks)
        report['machine'] = machine_info()
        report['binary_sha256'] = hashlib.sha256((ROOT / 'target/release/excel-shrink-cli.exe').read_bytes()).hexdigest()
        report['resident_source_sha256'] = hashlib.sha256((worker / 'resident.ps1').read_bytes()).hexdigest()
        publish(directory / 'results.json', {**report, 'measurements': results})
        write_report(directory)
        progress(directory, 'Complete')
    except BaseException as error:
        progress(directory, 'Failed', error=str(error))
        (directory / 'error.log').write_text(traceback.format_exc(), encoding='utf-8')
        raise


if __name__ == '__main__':
    main()
