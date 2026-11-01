"""Measure the unchanged production Office.js height scanner over all Excel rows.

Only a disposable worker copy gains the timing command. No corpus is started.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
ROWS = 1_048_576
CASES = ('uniform', 'outliers', 'alternating', 'column-font', 'column-font-removed')


def publish(path, value):
    """Publish complete JSON atomically for the existing resident protocol."""
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix('.pending')
    pending.write_text(json.dumps(value, indent=2), encoding='utf-8')
    os.replace(pending, path)


def read_json(path):
    """Read a resident snapshot that may not exist yet."""
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except (OSError, ValueError):
        return None


def wait_until(predicate, process, timeout):
    """Wait for one protocol response, with process-exit and wall-time bounds."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result is not None:
            return result
        if process.poll() is not None:
            raise RuntimeError(f'Resident exited: {process.returncode}')
        time.sleep(0.25)
    raise TimeoutError(f'Resident did not respond within {timeout} seconds')


def fixture_parts(directory):
    """Reuse the small valid XLSX package; replace only worksheet and style content."""
    source = HERE.parent / 'targeted-comparison/make_smoke_fixtures.py'
    spec = importlib.util.spec_from_file_location('height_fixture', source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.create_fixtures(directory / 'package-template')
    with zipfile.ZipFile(directory / 'package-template/default.xlsx') as archive:
        parts = {name: archive.read(name) for name in archive.namelist()}
    styles = parts['xl/styles.xml'].decode('utf-8')
    # The only comparison-pair difference will be B's inherited font, not borders.
    styles = styles.replace('fontId="1" fillId="0" borderId="1"', 'fontId="1" fillId="0" borderId="0"')
    parts['xl/styles.xml'] = styles.encode('utf-8')
    return parts


def create_workbook(path, case, parts):
    """Stream one million numeric rows; the empty B column optionally inherits 30pt font."""
    ns = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
    column_style = ' style="1"' if case == 'column-font' else ''
    prefix = (f'<worksheet xmlns="{ns}"><dimension ref="A1:B{ROWS}"/>'
              '<sheetFormatPr defaultRowHeight="15"/>'
              f'<cols><col min="1" max="1" width="18" customWidth="1"/>'
              f'<col min="2" max="2" width="12" customWidth="1"{column_style}/></cols><sheetData>')
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED, compresslevel=1) as archive:
        for name, content in parts.items():
            if name != 'xl/worksheets/sheet1.xml':
                archive.writestr(name, content)
        with archive.open('xl/worksheets/sheet1.xml', 'w', force_zip64=True) as sheet:
            sheet.write(prefix.encode())
            buffer = bytearray()
            for row in range(1, ROWS + 1):
                attributes = ''
                if case == 'alternating':
                    attributes = f' ht="{15 if row % 2 else 30}" customHeight="1"'
                elif case == 'outliers':
                    if row in (524288, ROWS):
                        attributes = ' ht="30" customHeight="1"'
                    elif row == 8192:
                        attributes = ' hidden="1"'
                buffer.extend(f'<row r="{row}"{attributes}><c r="A{row}" s="0"><v>{row}</v></c></row>'.encode())
                if len(buffer) >= 262144:
                    sheet.write(buffer)
                    buffer.clear()
            sheet.write(buffer)
            sheet.write(b'</sheetData></worksheet>')


def prepare_worker(directory):
    """Inject timing around the actual production scanner, leaving its algorithm untouched."""
    worker = directory / 'worker'
    shutil.copytree(ROOT / 'excel-shrink-cli/office-capture', worker)
    archive = directory / 'src/regression/archive_processes.ps1'
    archive.parent.mkdir(parents=True)
    shutil.copy2(ROOT / 'excel-shrink-cli/src/regression/archive_processes.ps1', archive)
    script = worker / 'web/resident-diagnosis.js'
    source = script.read_text(encoding='utf-8')
    anchor = '  async function execute(command, {office}) {'
    if source.count(anchor) != 1:
        raise RuntimeError('Diagnosis insertion point changed')
    operation = (HERE / 'probe.js').read_text(encoding='utf-8')
    dispatch = "\n    if (command.operation === 'probe_million_heights') return probeMillionRowHeights(command, office);"
    script.write_text(source.replace(anchor, operation + '\n' + anchor + dispatch), encoding='utf-8')
    return worker


def close_client(session, process):
    """Request normal closure; bound fallback termination to this experiment's resident tree."""
    publish(session / 'finish.json', {'complete': True})
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        subprocess.run(['taskkill', '/F', '/T', '/PID', str(process.pid)], check=True,
                       capture_output=True, timeout=20)
        process.wait(timeout=10)


def measure_case(directory, workbook, worker, binary, budget):
    """Open one isolated resident, then measure cold and warm scans separately from startup."""
    session = directory / 'session'
    control = directory / 'control'
    publish(control / 'control.json', {'paused': False, 'hide_excel': True})
    publish(session / 'capacity.json', {'reserved_clients': 1})
    publish(session / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(workbook), 'workbook': 'one.xlsx',
        'sheets': [{'index': 1, 'name': 'Rows', 'bottom_row': ROWS - 1,
                    'base_right_column': 1, 'right_column': 1,
                    'horizontal_overflow_required': False, 'drawing_fallback_required': False}],
    })
    environment = {**os.environ, 'EXCEL_SHRINK_CONTROL_ROOM_DIR': str(control),
                   'EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID': 'million-height-probe',
                   'EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID': directory.name}
    result = {'case': directory.name, 'measurements': []}
    started = time.monotonic()
    with (directory / 'stdout.log').open('wb') as stdout, (directory / 'stderr.log').open('wb') as stderr:
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            str(worker / 'resident.ps1'), '-SessionPath', str(session),
            '-HostExecutable', str(binary), '-StartupTimeoutSeconds', '180',
            '-CaptureTimeoutSeconds', str(budget + 30), '-HideExcel',
        ], stdout=stdout, stderr=stderr, env=environment)
    try:
        wait_until(lambda: True if (read_json(session / 'metrics.json') or {}).get('primary_clients_ready') else None,
                   process, 180)
        result['startup_seconds'] = round(time.monotonic() - started, 3)
        for repeat in range(2):
            command_id = f'height{repeat}'
            publish(session / f'commands/one/{command_id}.json', {
                'id': command_id, 'kind': 'diagnosis', 'operation': 'probe_million_heights',
                'sheet_name': 'Rows', 'budget_ms': budget * 1000,
            })
            requested = time.monotonic()
            response = wait_until(lambda: read_json(session / f'responses/one/{command_id}.json'),
                                  process, budget + 30)
            response['host_wall_seconds'] = round(time.monotonic() - requested, 3)
            result['measurements'].append(response)
            fields = ('complete', 'elapsed_ms', 'requested_ranges', 'batches', 'run_count', 'error')
            print(json.dumps({'case': directory.name, 'repeat': repeat,
                              **{key: response.get(key) for key in fields}}), flush=True)
            if response.get('complete') is not True:
                break
    except Exception as error:
        result['error'] = str(error)
    finally:
        close_client(session, process)
        result['final_metrics'] = read_json(session / 'metrics.json')
        publish(directory / 'result.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/excel-shrink-cli.exe')
    parser.add_argument('--budget-seconds', type=int, default=30)
    args = parser.parse_args()
    if os.name != 'nt' or not args.binary.is_file() or not 1 <= args.budget_seconds <= 300:
        parser.error('Requires Windows, an existing capture-host executable, and a 1..300 second budget')
    directory = args.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    worker = prepare_worker(directory)
    parts = fixture_parts(directory)
    results = []
    for case in CASES:
        case_dir = directory / case
        case_dir.mkdir()
        workbook = case_dir / 'input.xlsx'
        print(f'Creating {case}: {ROWS} populated rows', flush=True)
        create_workbook(workbook, case, parts)
        result = measure_case(case_dir, workbook, worker, args.binary.resolve(), args.budget_seconds)
        results.append(result)
        publish(directory / 'results.json', {
            'rows': ROWS, 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
            'geometry_sha256': hashlib.sha256((worker / 'web/capture-geometry.js').read_bytes()).hexdigest(),
            'results': results,
        })


if __name__ == '__main__':
    main()
