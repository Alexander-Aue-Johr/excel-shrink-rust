"""Benchmark four prepared workbook quartets with all 16 Excel workers resident."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import csv
import hashlib
import html
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time
from zipfile import ZipFile
import xml.etree.ElementTree as ET
import posixpath
import re

HERE = Path(__file__).resolve().parent
from pinned_runtime import capture_plan, export_runtime, windows_path, RUNTIME_REF
from quartet_controls import create_controls

MAXIMUM = [
    '--remove-textless-default-font-representatives',
    '--remove-textless-style-representatives',
    '--remove-textless-column-style-representatives',
    '--remove-unreferenced-whitespace-cells',
]
PROFILES = {'default': [], 'maximum': MAXIMUM,
            'too-aggressive': [*MAXIMUM, '--cell-formula-references-only']}
PRIORITY = ('kernhaushalt', 'wanderungen', 'stromsteuerstatistik', 'verkehrsunfaelle')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def prepare(corpus, cli, directory, only=None):
    """Finish every shrink operation before any measured Excel process starts."""
    directory.mkdir(parents=True, exist_ok=True)
    manifest_path = directory / 'manifest.json'
    sources = sorted(corpus.glob('*.xlsx'), key=lambda path: (
        next((index for index, token in enumerate(PRIORITY) if token in path.name),
             len(PRIORITY)), path.name))
    if only is not None:
        if only:
            selected = []
            for token in only:
                matches = [source for source in sources if token in source.name]
                if len(matches) != 1:
                    raise ValueError(f'--only {token!r} must match one workbook')
                if matches[0] not in selected:
                    selected.append(matches[0])
            sources = selected
        else:
            sources = sources[:4]
        if len(sources) != 4:
            raise ValueError('Select exactly four distinct corpus workbooks')
    cli_sha256 = digest(cli)
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
        if (not isinstance(manifest, dict) or manifest.get('schema') != 1 or
            manifest.get('cli_sha256') != cli_sha256 or
            manifest.get('profile_flags') != PROFILES or
            not isinstance(manifest.get('entries'), list) or
            any(not isinstance(entry, dict) for entry in manifest['entries'])):
            raise RuntimeError(f'Prepared manifest lacks matching CLI and profile provenance: {manifest_path}')
        entries = manifest['entries']
        if [entry.get('name') for entry in entries] != [source.name for source in sources]:
            raise RuntimeError(f'Prepared manifest does not match corpus originals: {manifest_path}')
        for entry, source in zip(entries, sources):
            original = entry.get('original', {})
            if (not isinstance(original, dict) or
                original.get('path') != str(source.resolve()) or
                original.get('sha256') != digest(source)):
                raise RuntimeError(f'Prepared original changed: {source}')
            for profile, flags in PROFILES.items():
                target = directory / profile / source.name
                record = entry.get(profile, {})
                if (not isinstance(record, dict) or
                    record.get('path') != str(target.resolve()) or
                    record.get('flags') != flags or not target.is_file() or
                    record.get('sha256') != digest(target)):
                    raise RuntimeError(f'Prepared shrink input changed: {target}')
        return entries
    existing = [directory / profile / source.name
                for source in sources for profile in PROFILES
                if (directory / profile / source.name).exists()]
    if existing:
        raise RuntimeError(f'Prepared shrink file has no complete manifest: {existing[0]}')
    entries = []
    for source in sources:
        entry = {'name': source.name, 'original': {
            'path': str(source.resolve()), 'bytes': source.stat().st_size,
            'sha256': digest(source)}}
        for profile, flags in PROFILES.items():
            target_dir = directory / profile
            target_dir.mkdir(exist_ok=True)
            target = target_dir / source.name
            with (directory / f'{profile}-{source.stem}.log').open('wb') as log:
                subprocess.run([str(cli), str(source), str(target_dir), *flags],
                               check=True, stdout=log, stderr=subprocess.STDOUT)
            if not target.is_file():
                raise RuntimeError(f'Shrink output missing: {target}')
            entry[profile] = {'path': str(target.resolve()), 'bytes': target.stat().st_size,
                              'sha256': digest(target), 'flags': flags}
        entries.append(entry)
    pending = manifest_path.with_suffix('.json.pending')
    pending.write_text(json.dumps({'schema': 1, 'cli_sha256': cli_sha256,
                                   'profile_flags': PROFILES, 'entries': entries}, indent=2) + '\n',
                       encoding='utf-8')
    os.replace(pending, manifest_path)
    return entries


def worksheet_info(path, name=None):
    """Return a visible sheet's name, dimension, hidden columns and ZIP member."""
    with ZipFile(path) as archive:
        rels = {item.get('Id'): (item.get('Target').lstrip('/') if item.get('Target').startswith('/')
                                 else posixpath.normpath(posixpath.join('xl', item.get('Target'))))
                for item in ET.fromstring(archive.read('xl/_rels/workbook.xml.rels'))}
        sizes = {item.filename: item.file_size for item in archive.infolist()}
        sheets = ET.fromstring(archive.read('xl/workbook.xml')).find('{*}sheets')
        candidates = []
        for sheet in sheets:
            if sheet.get('state', 'visible') != 'visible':
                continue
            relation = next(value for key, value in sheet.attrib.items() if key.endswith('}id'))
            candidates.append((sizes.get(rels[relation], 0), sheet.get('name'), rels[relation]))
        if not candidates:
            raise RuntimeError(f'No visible worksheet: {path}')
        selected = (next((item for item in candidates if item[1] == name), None)
                    if name else max(candidates))
        if selected is None:
            raise ValueError(f'Unknown visible worksheet {name!r} in {path}')
        dimension = None
        hidden = []
        with archive.open(selected[2]) as stream:
            for _, element in ET.iterparse(stream, events=('start',)):
                tag = element.tag.rsplit('}', 1)[-1]
                if tag == 'dimension':
                    dimension = element.get('ref')
                elif tag == 'col' and element.get('hidden') == '1':
                    hidden.append((int(element.get('min')), int(element.get('max'))))
                elif tag == 'sheetData':
                    break
        return selected[1], dimension, hidden, selected[2]


def column_number(letters):
    value = 0
    for letter in letters:
        value = value * 26 + ord(letter) - ord('A') + 1
    return value


def column_letters(number):
    result = ''
    while number:
        number, digit = divmod(number - 1, 26)
        result = chr(ord('A') + digit) + result
    return result


def scope_and_overflow(path, requested=None):
    name = requested.rsplit('!', 1)[0] if requested else None
    sheet, dimension, hidden, _ = worksheet_info(path, name)
    if requested:
        scope = requested
    else:
        match = re.search(r'([A-Z]+)(\d+)$', dimension or '')
        right = min(172, column_number(match[1])) if match else 52
        bottom = min(400, int(match[2])) if match else 400
        scope = f'{sheet}!A1:{column_letters(right)}{bottom}'
    end = re.search(r'([A-Z]+)\d+$', scope)
    right = column_number(end[1])
    visible_after = any(not any(start <= column <= stop for start, stop in hidden)
                        for column in range(right + 1, min(16384, right + 32) + 1))
    return scope, visible_after


def occupied_cell_right_of_scope(path, scope):
    """Find content in the column immediately right of an explicit scope."""
    sheet_name, address = scope.rsplit('!', 1)
    match = re.fullmatch(r'A1:([A-Z]+)([1-9][0-9]*)', address)
    if not match:
        raise ValueError(f'Invalid scope for right-edge check: {scope}')
    target_column = column_letters(column_number(match[1]) + 1)
    last_row = int(match[2])
    _, _, _, member = worksheet_info(path, sheet_name)
    with ZipFile(path) as archive, archive.open(member) as stream:
        for _, cell in ET.iterparse(stream, events=('end',)):
            if cell.tag.rsplit('}', 1)[-1] != 'c':
                continue
            address = cell.get('r', '')
            found = re.fullmatch(r'([A-Z]+)([1-9][0-9]*)', address)
            if found and found[1] == target_column and int(found[2]) <= last_row:
                occupied = any(
                    child.tag.rsplit('}', 1)[-1] == 'f' or
                    (child.tag.rsplit('}', 1)[-1] == 'v' and child.text is not None) or
                    (child.tag.rsplit('}', 1)[-1] == 'is' and
                     any(text.text is not None for text in child.iter()
                         if text.tag.rsplit('}', 1)[-1] == 't'))
                    for child in cell)
                if occupied:
                    return address
            cell.clear()
    return None


def validate_explicit_scope(entry, scope, overflow):
    if not overflow:
        return
    sheet_name, address = scope.rsplit('!', 1)
    match = re.fullmatch(r'A1:([A-Z]+)([1-9][0-9]*)', address)
    if not match:
        raise ValueError(f'Invalid explicit scope: {scope}')
    next_column = column_letters(column_number(match[1]) + 1)
    for profile in ('original', *PROFILES):
        occupied = occupied_cell_right_of_scope(Path(entry[profile]['path']), scope)
        if occupied:
            suggested = f'{sheet_name}!A1:{next_column}{match[2]}'
            raise ValueError(
                f"Explicit scope {scope} for {entry['name']} ends before occupied "
                f"{occupied} in {profile}. Extend its right edge to at least "
                f"{suggested} before starting Excel.")


def available_mib():
    script = '[math]::Floor((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory/1024)'
    return int(subprocess.check_output(['powershell.exe', '-NoProfile', '-Command', script],
                                       text=True).strip())


def patch_desktop_launcher(runtime):
    """Keep the pinned process/job cleanup while adding interactive launch modes."""
    desktop_source = runtime / 'CaptureDesktop.cs'
    desktop_code = desktop_source.read_text(encoding='utf-8')
    old_start = '    public Process Start(string executable, string arguments, CaptureProcesses owned)\n    {'
    new_start = '''    public Process Start(string executable, string arguments, CaptureProcesses owned)
    { return Start(executable, arguments, owned, false, false); }
    public Process StartVisible(string executable, string arguments, CaptureProcesses owned)
    { return Start(executable, arguments, owned, true, false); }
    public Process StartMinimized(string executable, string arguments, CaptureProcesses owned)
    { return Start(executable, arguments, owned, true, true); }
    Process Start(string executable, string arguments, CaptureProcesses owned,
        bool interactive, bool minimized)
    {'''
    if desktop_code.count(old_start) != 1 or desktop_code.count('startup.Desktop = name;') != 1:
        raise RuntimeError('Pinned desktop process launcher changed')
    desktop_source.write_text(desktop_code.replace(old_start, new_start).replace(
        'startup.Desktop = name;', '''startup.Desktop = interactive ? null : name;
        if (minimized)
        {
            startup.Flags = 0x00000001; // STARTF_USESHOWWINDOW
            startup.ShowWindow = 7;     // SW_SHOWMINNOACTIVE
        }'''), encoding='utf-8')


def run_one(entry, capture, runtime, launcher, trial, args):
    name = Path(entry['name']).stem
    directory = trial / name
    directory.mkdir()
    request = {'workbook': entry['name'], 'capture': capture,
               'transport': 'hash', 'batch_size': args.batch_size,
               'max_batch_pixels': args.max_batch_pixels,
               'stage_ready_signal': windows_path(directory / 'stage-ready.signal'),
               'launch_signal': windows_path(trial / 'launch.signal'),
               'ready_signal': windows_path(directory / 'ready.signal'),
               'measurement_signal': windows_path(trial / 'measurement.signal'),
               'start_signal': windows_path(directory / 'start.signal'),
               'release_signal': windows_path(directory / 'release.signal')}
    for side in ('original', *PROFILES):
        request[side] = windows_path(Path(entry[side]['path']))
    path = directory / 'request.json'
    path.write_text(json.dumps(request, indent=2, ensure_ascii=False), encoding='utf-8')
    command = ['powershell.exe', '-STA', '-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass',
               '-File', windows_path(launcher), '-RequestPath', windows_path(path),
               '-RuntimeRoot', windows_path(runtime), '-OutputDir', windows_path(directory),
               '-StartupTimeoutSeconds', str(args.startup_timeout),
               '-CaptureTimeoutSeconds', str(args.capture_timeout),
               '-MemoryMiB', str(args.memory_mib)]
    if getattr(args, 'ignore_memory_guard', False):
        command.append('-IgnoreMemoryGuard')
    if getattr(args, 'visible', False):
        command.append('-Visible')
    if getattr(args, 'minimized', False):
        command.append('-Minimized')
    if getattr(args, 'hidden_desktop', False):
        command.append('-HiddenDesktop')
    with (directory / 'launcher.log').open('wb') as log:
        process = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    result_path = directory / 'result.json'
    result = json.loads(result_path.read_text(encoding='utf-8-sig')) if result_path.exists() else {
        'status': 'failed', 'error': 'Launcher produced no result'}
    result.update(workbook=entry['name'], exit_code=process.returncode)
    return result


def live_view(output):
    """A file-based dashboard that refreshes while Excel is starting/capturing."""
    while True:
        trials = sorted(output.glob('r*-c*'), key=lambda path: path.stat().st_mtime)
        trial = trials[-1] if trials else None
        rows = []
        if trial:
            for group in sorted(path for path in trial.iterdir() if path.is_dir()):
                if (group / 'result.json').exists():
                    state = 'Abgeschlossen / Fehler – siehe Ergebnis'
                elif (group / 'capture-result.json').exists():
                    state = 'Capture fertig; Excel bleibt geöffnet'
                elif (group / 'start.signal').exists():
                    state = 'Capture läuft'
                elif (group / 'ready.signal').exists():
                    state = 'An der gemeinsamen Startlinie'
                else:
                    count = sum((group / f'phase-{phase}.ready').exists() for phase in range(4))
                    state = f'{count} von 4 Excel/Add-ins angemeldet' if count else (
                        'Dateien vorbereitet' if (group / 'stage-ready.signal').exists()
                        else 'Dateien/Host werden vorbereitet')
                rows.append(f'<tr><td>{html.escape(group.name)}</td><td>{html.escape(state)}</td></tr>')
        content = ('<!doctype html><html lang="de"><meta charset="utf-8">'
                   '<meta http-equiv="refresh" content="2"><title>Excel-Benchmark live</title>'
                   '<style>body{font:18px system-ui;max-width:1000px;margin:40px auto;'
                   'padding:0 20px;color:#203344}table{border-collapse:collapse;width:100%}'
                   'td,th{padding:12px;border-bottom:1px solid #ccd;text-align:left}</style>'
                   '<h1>Excel-Benchmark live</h1><p>Die Seite aktualisiert sich alle 2 Sekunden. '
                   'Die Messzeit beginnt erst, wenn alle 16 Worker bereit sind.</p>'
                   f'<h2>{html.escape(trial.name if trial else "Vorbereitung")}</h2>'
                   '<table><tr><th>Arbeitsmappe</th><th>Status</th></tr>' + ''.join(rows) +
                   '</table><p>Nach dem Lauf stehen Messwerte und Bilder in report.html.</p></html>')
        temporary = output / 'live.html.tmp'
        temporary.write_text(content, encoding='utf-8')
        temporary.replace(output / 'live.html')
        time.sleep(2)


def barrier(trial, entries, futures, ready, signal, timeout):
    """Release all launchers with 'abort' if any peer fails to reach a phase."""
    deadline = time.monotonic() + timeout
    while not all((trial / Path(entry['name']).stem / ready).exists() for entry in entries):
        if any(future.done() and not (trial / Path(entry['name']).stem / ready).exists()
               for future, entry in futures.items()) or time.monotonic() >= deadline:
            (trial / signal).write_text('abort\n')
            return False
        time.sleep(0.2)
    (trial / signal).write_text('go\n')
    return True


def wait_ready(trial, entries, futures, timeout):
    deadline = time.monotonic() + timeout
    while not all((trial / Path(entry['name']).stem / 'ready.signal').exists() for entry in entries):
        if any(future.done() for future in futures) or time.monotonic() >= deadline:
            return False
        time.sleep(0.2)
    return True


def launch_phase(trial, entries, futures, phase, timeout):
    """Start one Excel at a time; Office add-in loading can serialize globally."""
    signal = trial / f'phase-{phase}.signal'
    for entry in entries:
        group = trial / Path(entry['name']).stem
        (group / f'phase-{phase}.launch').write_text('go\n')
        deadline = time.monotonic() + timeout
        while not (group / f'phase-{phase}.ready').exists():
            if any(future.done() for future in futures) or time.monotonic() >= deadline:
                signal.write_text('abort\n')
                return False
            time.sleep(0.2)
    signal.write_text('go\n')
    return True


def memory_estimate_mib(entries):
    # Admission estimate, followed by physical-memory guards during launch/capture.
    # Excel's decompressed model varies by workbook; this is deliberately conservative.
    return sum(max(512, 8 * entry[side]['bytes'] / 2**20)
               for entry in entries for side in ('original', *PROFILES))


def run_trial(entries, plans, runtime, launcher, output, repeat, active_groups, args):
    trial = output / f'r{repeat}-c{active_groups}'
    trial.mkdir()
    available = available_mib()
    estimated = memory_estimate_mib(entries)
    row = {'repeat': repeat, 'active_groups': active_groups, 'excel_processes': 16,
           'available_mib': available, 'estimated_peak_mib': round(estimated),
           'memory_guard_ignored': getattr(args, 'ignore_memory_guard', False),
           'status': 'failed', 'measured_seconds': None, 'workbooks': []}
    if not getattr(args, 'ignore_memory_guard', False) and estimated + args.ram_reserve_mib > available:
        row.update(status='resource-limited', error='16-workbook RAM admission estimate exceeds free RAM')
        return row
    with ThreadPoolExecutor(max_workers=4) as pool:
        futures = {pool.submit(run_one, entry, plans[entry['name']]['capture'],
                               runtime, launcher, trial, args): entry for entry in entries}
        started = False
        try:
            if not barrier(trial, entries, futures, 'stage-ready.signal', 'launch.signal',
                           4 * args.startup_timeout + 300):
                raise RuntimeError('Staging barrier failed before all 16 Excel processes started')
            for phase in range(4):
                if not launch_phase(trial, entries, futures, phase,
                                    args.startup_timeout + 60):
                    raise RuntimeError(f'Excel launch phase {phase} failed')
            if not wait_ready(trial, entries, futures, 4 * args.startup_timeout + 300):
                raise RuntimeError('Fewer than 16 Office.js workers became capture-ready')
            # All sixteen Excel processes and workbooks remain open for the entire trial.
            # No ZIP preparation, Excel startup or sheet planning is inside this timer.
            start = time.monotonic()
            started = True
            (trial / 'measurement.signal').write_text('go\n')
            running = set()
            pending = [Path(entry['name']).stem for entry in entries]
            completed = set()
            while len(completed) < 4:
                while pending and len(running) < active_groups:
                    group = pending.pop(0)
                    (trial / group / 'start.signal').write_text('go\n')
                    running.add(group)
                finished = []
                for group in running:
                    result_path = trial / group / 'capture-result.json'
                    if result_path.exists():
                        result = json.loads(result_path.read_text(encoding='utf-8-sig'))
                        if result.get('status') != 'complete':
                            raise RuntimeError(f'{group} did not finish capture')
                        finished.append(group)
                for group in finished:
                    running.remove(group)
                    completed.add(group)
                if any(future.done() and Path(entry['name']).stem not in completed
                       for future, entry in futures.items()):
                    raise RuntimeError('An Excel group exited before capture completion')
                if time.monotonic() - start > args.capture_timeout * 4 + 120:
                    raise RuntimeError('Capture scheduler timed out')
                if len(completed) < 4:
                    time.sleep(0.02)
            row['measured_seconds'] = time.monotonic() - start
            row['status'] = 'complete'
        except Exception as error:
            row['error'] = str(error)
        finally:
            # Waiting groups must be unblocked before process cleanup, including failures.
            for entry in entries:
                group = trial / Path(entry['name']).stem
                for signal in ('start.signal', 'release.signal'):
                    path = group / signal
                    if not path.exists():
                        path.write_text('abort\n' if signal == 'start.signal' else 'release\n')
            for phase in range(3):
                signal = trial / f'phase-{phase}.signal'
                if not signal.exists():
                    signal.write_text('abort\n')
            for entry in entries:
                group = trial / Path(entry['name']).stem
                for phase in range(4):
                    token = group / f'phase-{phase}.launch'
                    if not token.exists():
                        token.write_text('abort\n')
            signal = trial / 'launch.signal'
            if not signal.exists():
                signal.write_text('abort\n')
            signal = trial / 'measurement.signal'
            if not signal.exists():
                signal.write_text('abort\n')
            for future, entry in futures.items():
                try:
                    row['workbooks'].append(future.result())
                except Exception as error:
                    row['workbooks'].append({'status': 'failed', 'workbook': entry['name'],
                                             'error': str(error)})
    if any(item['status'] != 'complete' for item in row['workbooks']):
        row['status'] = 'failed'
    if row['status'] == 'complete' and not started:
        row['status'] = 'failed'
    return row


def comparison_results(item):
    return item.get('capture', {}).get('quartet', {}).get('comparisons', [])


def range_contains(address, region):
    cell = re.fullmatch(r'([A-Z]+)([1-9][0-9]*)', address)
    bounds = re.fullmatch(r'\$?([A-Z]+)\$?([1-9][0-9]*):\$?([A-Z]+)\$?([1-9][0-9]*)',
                          region.rsplit('!', 1)[-1])
    if not cell or not bounds:
        return False
    return (column_number(bounds[1]) <= column_number(cell[1]) <= column_number(bounds[3])
            and int(bounds[2]) <= int(cell[2]) <= int(bounds[4]))


def validate_controls(row, controls, output):
    if row['status'] != 'complete':
        return
    for item in row['workbooks']:
        comparisons = comparison_results(item)
        if len(comparisons) != 3 or not item['capture']['complete']:
            raise ValueError(f"Incomplete comparisons: {item['workbook']}")
        expected = controls[item['workbook']]['cell']
        for comparison in comparisons:
            differences = comparison['result'].get('differences', [])
            root = (output / f"r{row['repeat']}-c{row['active_groups']}" /
                    Path(item['workbook']).stem / comparison['profile'])
            def proved(difference):
                bounds = difference.get('difference', {}).get('bounds')
                prefix = difference.get('evidence', '')
                if (not range_contains(expected, difference.get('range', '')) or
                    not isinstance(bounds, list) or len(bounds) != 4 or
                    bounds[0] >= bounds[2] or bounds[1] >= bounds[3] or
                    not re.fullmatch(r's\d+-t\d+(?:\.\d+)*', prefix)):
                    return False
                for side in ('original', 'shrunk'):
                    path = root / f'{prefix}-{side}.png'
                    if not path.is_file() or path.stat().st_size < 45:
                        return False
                    with path.open('rb') as image:
                        header = image.read(24)
                        image.seek(-12, 2)
                        trailer = image.read(12)
                    if (header[:8] != b'\x89PNG\r\n\x1a\n' or
                        header[12:16] != b'IHDR' or
                        int.from_bytes(header[16:20], 'big') < 1 or
                        int.from_bytes(header[20:24], 'big') < 1 or
                        trailer[:8] != b'\0\0\0\0IEND'):
                        return False
                return True
            if not any(proved(difference) for difference in differences):
                raise ValueError(f"Control cell {expected} lacks a pixel difference and two PNG "
                                 f"images in {item['workbook']} {comparison['profile']}")


def visual_signature(item):
    return {comparison['profile']: sorted(
        difference.get('range', '') for difference in comparison['result'].get('differences', []))
        for comparison in comparison_results(item)}


def publish(output, rows, controls):
    (output / 'results.json').write_text(json.dumps(rows, indent=2) + '\n')
    baseline = {(row['repeat']): row['measured_seconds'] for row in rows
                if row['active_groups'] == 1 and row['status'] == 'complete'}
    records = []
    for row in rows:
        seconds = row.get('measured_seconds')
        cpu_seconds = sum(item.get('measured_cpu_seconds', 0) for item in row['workbooks'])
        logical_processors = max((item.get('logical_processors', 0)
                                  for item in row['workbooks']), default=0)
        metrics = [client.get('experiment_metrics', {})
                   for item in row['workbooks']
                   for client in item.get('capture', {}).get('clients', {}).values()]
        record = {'repeat': row['repeat'], 'active_groups': row['active_groups'],
                  'excel_processes': 16, 'status': row['status'],
                  'measured_seconds': seconds,
                  'speedup': round(baseline[row['repeat']] / seconds, 3)
                  if row['status'] == 'complete' and seconds and row['repeat'] in baseline else None,
                  'capture_ms': sum(m.get('capture_ms', 0) for m in metrics),
                  'hash_ms': sum(m.get('hash_ms', 0) for m in metrics),
                  'transfer_receive_ms': sum(item.get('capture', {}).get('quartet', {}).get(
                      'transfer_receive_ms', 0) for item in row['workbooks']),
                  'upload_wait_ms': sum(m.get('upload_wait_ms', 0) for m in metrics),
                  'refine_ms': sum(m.get('refine_ms', 0) for m in metrics),
                  'cpu_seconds': cpu_seconds,
                  'avg_cpu_cores': round(cpu_seconds / seconds, 2) if seconds else None,
                  'cpu_percent': round(100 * cpu_seconds / seconds / logical_processors, 1)
                  if seconds and logical_processors else None,
                  'peak_job_mib_sum': round(sum(item.get('peak_job_bytes', 0)
                                            for item in row['workbooks']) / 2**20),
                  'minimum_free_mib': round(min((item.get('minimum_available_bytes', 0)
                                                for item in row['workbooks']), default=0) / 2**20),
                  'error': row.get('error', '')}
        records.append(record)
    with (output / 'measurements.csv').open('w', newline='', encoding='utf-8') as stream:
        writer = csv.DictWriter(stream, fieldnames=list(records[0]) if records else ['status'])
        writer.writeheader()
        writer.writerows(records)
    rows_html = []
    for record in records:
        width = min(100, round(100 * record['measured_seconds'] /
                               max((r['measured_seconds'] or 0 for r in records), default=1))) if record['measured_seconds'] else 0
        cells = [record['repeat'], record['active_groups'], record['status'],
                 f"{record['measured_seconds']:.2f}" if record['measured_seconds'] else '—',
                 f"{record['speedup']:.2f}×" if record['speedup'] else '—',
                 round(record['capture_ms'] / 1000, 2),
                 round(record['hash_ms'] / 1000, 2),
                 round(record['transfer_receive_ms'] / 1000, 2),
                 round(record['refine_ms'] / 1000, 2),
                 record['cpu_seconds'], record['avg_cpu_cores'], record['cpu_percent'],
                 record['peak_job_mib_sum'], record['minimum_free_mib']]
        rows_html.append('<tr>' + ''.join(f'<td>{html.escape(str(cell))}</td>' for cell in cells) +
                         f'<td><div class="bar" style="width:{width}%"></div></td></tr>')
    control_evidence = []
    other_evidence = {}
    for row in rows:
        for item in row['workbooks']:
            if item.get('status') != 'complete':
                continue
            expected = controls.get(item['workbook'], {}).get('cell', '')
            for comparison in comparison_results(item):
                for difference in comparison['result'].get('differences', []):
                    prefix = difference.get('evidence')
                    if not prefix or not re.fullmatch(r's\d+-t\d+(?:\.\d+)*', prefix):
                        continue
                    name = Path(item['workbook']).stem
                    profile = comparison['profile']
                    root = output / f"r{row['repeat']}-c{row['active_groups']}" / name / profile
                    images = []
                    for suffix in ('original', 'shrunk'):
                        path = root / f'{prefix}-{suffix}.png'
                        if not path.is_file():
                            break
                        images.append(path.relative_to(output).as_posix())
                    if len(images) != 2:
                        continue
                    label = (f"r{row['repeat']} · {row['active_groups']} aktive Gruppen · "
                             f"{name} · {profile} · {difference.get('range', '')}" +
                             (' · Teilergebnis' if row['status'] != 'complete' else ''))
                    pair = ('<article><h3>' + html.escape(label) + '</h3>' + ''.join(
                        f'<figure><img src="{html.escape(src, quote=True)}" loading="lazy">'
                        f'<figcaption>{caption}</figcaption></figure>'
                        for src, caption in zip(images, ('Original', 'Shrink'))) + '</article>')
                    is_control = (range_contains(expected, difference.get('range', '')) and
                                  difference.get('difference', {}).get('bounds') is not None)
                    if is_control:
                        control_evidence.append(pair)
                    else:
                        key = (row['repeat'], row['active_groups'], name, profile)
                        other_evidence.setdefault(key, []).append(pair)
    other_details = []
    for (repeat, active, name, profile), pairs in other_evidence.items():
        noun = 'weiterer Befund' if len(pairs) == 1 else 'weitere Befunde'
        label = f'r{repeat} · {active} aktive Gruppen · {name} · {profile}: {len(pairs)} {noun}'
        other_details.append('<details><summary>' + html.escape(label) + '</summary>' +
                             ''.join(pairs) + '</details>')
    errors = [html.escape(f"r{row['repeat']} c{row['active_groups']}: {row.get('error', '')} "
                          f"(geschätzt {row['estimated_peak_mib']} MiB, "
                          f"frei {row['available_mib']} MiB)")
              for row in rows if row['status'] != 'complete']
    control_text = ', '.join(f"{name}: {value['cell']}" for name, value in controls.items())
    (output / 'report.html').write_text(
        '<!doctype html><html lang="de"><meta charset="utf-8">'
        '<title>Office.js Parallelität</title><style>body{font:16px system-ui;max-width:1200px;'
        'margin:36px auto;padding:0 20px;color:#203344}table{border-collapse:collapse;width:100%}'
        'td,th{padding:8px;border-bottom:1px solid #ccd;text-align:left}'
        '.bar{height:16px;background:#3a87a8}article{margin:18px 0;padding:8px;border:1px solid #ccd}'
        'figure{display:inline-block;vertical-align:top;max-width:45%;margin:12px}'
        'img{max-width:100%;border:1px solid #ccd}figcaption{font-size:13px}'
        'details{margin:12px 0;padding:10px;border:1px solid #ccd}'
        'summary{cursor:pointer}</style><h1>Parallele Office.js-Bildvergleiche</h1>'
        '<p>In jedem Lauf sind 16 Excel-Prozesse mit vier Arbeitsmappenquartetten geöffnet. '
        'Die Messung beginnt erst nach der Bereitschaft aller 16 Worker. Aktiv sind 1, 2 oder 4 '
        'Gruppen; fertige Gruppen geben ihren Platz sofort frei.</p>'
        '<p>Kontrollzellen: ' + html.escape(control_text) + '</p>'
        '<table><tr><th>Wiederholung</th><th>Aktive Gruppen</th><th>Status</th>'
        '<th>Messzeit s</th><th>Speedup</th><th>Capture s Σ</th><th>Hash s Σ</th>'
        '<th>Host-Empfang s Σ</th><th>Nachprüfung s Σ</th><th>CPU s</th>'
        '<th>Ø CPU-Kerne</th><th>CPU %</th><th>Σ Gruppenpeaks MiB</th>'
        '<th>Min. freier RAM MiB</th><th>Laufzeit</th></tr>' + ''.join(rows_html) +
        '</table><h2>Fehler / ausgelassene Läufe</h2><ul>' +
        ''.join('<li>' + value + '</li>' for value in errors) +
        '</ul><h2>Kontrollzellen mit Bildbelegen</h2>' +
        (''.join(control_evidence) or '<p>Noch keine vollständigen Bildbelege.</p>') +
        '<h2>Weitere Bildunterschiede</h2>' +
        (''.join(other_details) or '<p>Keine weiteren Bildunterschiede.</p>') +
        '<p>Die Summen der Worker-Zeiten überlappen; Nachprüfung enthält auch Capture, Hash und '
        'Übertragung. Sie sind keine Bestandteile, die man zur Messzeit addieren kann. '
        'CPU % bezieht sich auf die Excel- und Host-Prozesse des Versuchs und die verfügbaren '
        'logischen Prozessoren, nicht auf das gesamte System. '
        'Einzelwerte stehen in measurements.csv.</p>'
        '</html>', encoding='utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo-root', type=Path, required=True)
    parser.add_argument('--corpus-root', type=Path, required=True)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--prepared-dir', type=Path)
    parser.add_argument('--only', action='append', default=[],
                        help='Select exactly four source filenames/substrings; repeat four times')
    parser.add_argument('--scope-map', type=Path)
    parser.add_argument('--repeats', type=int, default=2)
    parser.add_argument('--batch-size', type=int, choices=(1, 2, 4), default=1)
    parser.add_argument('--max-batch-pixels', type=int, default=64_000_000)
    parser.add_argument('--startup-timeout', type=int, default=300)
    parser.add_argument('--capture-timeout', type=int, default=1200)
    parser.add_argument('--memory-mib', type=int, default=0)
    parser.add_argument('--ram-reserve-mib', type=int, default=2048)
    parser.add_argument('--ignore-memory-guard', action='store_true',
                        help='One-off run without RAM admission, job cap or low-RAM aborts')
    desktop = parser.add_mutually_exclusive_group()
    desktop.add_argument('--visible', action='store_true',
                         help='Show Excel windows on the interactive Windows desktop')
    desktop.add_argument('--minimized', action='store_true',
                         help='Start Excel minimized on the interactive Windows desktop (default)')
    desktop.add_argument('--hidden-desktop', action='store_true',
                         help='Diagnostic mode: use an isolated, invisible Windows desktop')
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    if args.repeats < 1 or (not args.ignore_memory_guard and args.ram_reserve_mib < 1024):
        parser.error('At least one repeat and 1024 MiB RAM reserve are required')
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    if (args.output / 'results.json').exists():
        parser.error('Choose a new output directory; earlier observations are preserved')
    if args.visible:
        threading.Thread(target=live_view, args=(args.output,), daemon=True).start()
    prepared_dir = args.prepared_dir.resolve() if args.prepared_dir else args.output / 'prepared'
    try:
        entries = prepare(args.corpus_root.resolve(), args.cli.resolve(), prepared_dir,
                          args.only)
    except ValueError as error:
        parser.error(str(error))
    plans = {}
    scope_map = json.loads(args.scope_map.read_text(encoding='utf-8')) if args.scope_map else {}
    if set(scope_map) - {entry['name'] for entry in entries}:
        parser.error('Scope map contains unselected workbooks')
    for entry in entries:
        source = Path(entry['original']['path'])
        explicit_scope = scope_map.get(entry['name'])
        scope, overflow = scope_and_overflow(source, explicit_scope)
        if explicit_scope:
            try:
                validate_explicit_scope(entry, scope, overflow)
            except ValueError as error:
                parser.error(str(error))
        capture = capture_plan(source, [scope])
        capture['sheets'][0]['horizontal_overflow_required'] = overflow
        plans[entry['name']] = {'scope': scope, 'capture': capture}
    plans_path = args.output / 'plans.json'
    if plans_path.exists() and json.loads(plans_path.read_text()) != plans:
        parser.error('Existing capture plans differ; choose a new output directory')
    plans_path.write_text(json.dumps(plans, indent=2) + '\n')
    controls_path = args.output / 'controls.json'
    prior_controls = json.loads(controls_path.read_text()) if controls_path.exists() else None
    if prior_controls is None and (args.output / 'controlled').exists():
        parser.error('Controlled copies lack provenance; choose a new output directory')
    controlled = []
    controls = {}
    for entry in entries:
        changed, provenance = create_controls(entry, plans[entry['name']]['scope'],
                                               args.output / 'controlled',
                                               reuse_existing=prior_controls is not None)
        controlled.append(changed)
        controls[entry['name']] = provenance
    if prior_controls is not None and prior_controls != controls:
        parser.error('Controlled copies differ from recorded provenance; choose a new output directory')
    entries = controlled
    controls_path.write_text(json.dumps(controls, indent=2) + '\n')
    if args.prepare_only:
        print(f'Prepared four controlled quartets under {args.output}')
        return
    (args.output / 'provenance.json').write_text(json.dumps({
        'runtime_commit': RUNTIME_REF, 'cli_sha256': digest(args.cli.resolve()),
        'prepared_manifest': str(prepared_dir / 'manifest.json'),
        'selected_workbooks': [entry['name'] for entry in entries],
    }, indent=2) + '\n')
    runtime = export_runtime(args.repo_root.resolve(), RUNTIME_REF, args.output / 'runtime')
    patch_desktop_launcher(runtime)
    from web.patch import patch_runtime
    patch_runtime(runtime)
    experiment = args.output / 'experiment'
    (experiment / 'host').mkdir(parents=True)
    for name in ('run.ps1', 'Memory.cs'):
        shutil.copyfile(HERE / name, experiment / name)
    for source in (HERE / 'host').iterdir():
        if source.suffix in ('.cs', '.csproj'):
            shutil.copyfile(source, experiment / 'host' / source.name)
    observations = []
    expected_visuals = {}
    orders = ((1, 2, 4), (4, 2, 1), (2, 4, 1))
    for repeat in range(args.repeats):
        for active_groups in orders[repeat % len(orders)]:
            row = run_trial(entries, plans, runtime, experiment / 'run.ps1', args.output,
                            repeat, active_groups, args)
            try:
                validate_controls(row, controls, args.output)
                if row['status'] == 'complete':
                    actuals = {item['workbook']: visual_signature(item)
                               for item in row['workbooks']}
                    for name, actual in actuals.items():
                        if name in expected_visuals and actual != expected_visuals[name]:
                            raise ValueError(f'Visual findings changed across trials: {name}')
                    expected_visuals.update(actuals)
            except ValueError as error:
                row.update(status='invalid', error=str(error))
            observations.append(row)
            publish(args.output, observations, controls)
            print(repeat, active_groups, row['status'], row['measured_seconds'], flush=True)


if __name__ == '__main__':
    main()
