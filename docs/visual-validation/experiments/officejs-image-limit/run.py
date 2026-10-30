"""Probe Excel's raw Office.js Range.getImage() limit in an isolated resident.

Run with Windows Python. The generated workbook and resident session live in
Windows TEMP. Only a temporary copy of the add-in gains the probe operation.
"""

import argparse
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import zipfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SHEET_NAME = "RGB sweep"
MAX_ROWS = 1_048_576
COPY = ROOT / "excel-shrink-cli/office-capture-image-limit-probe"
SOURCE = ROOT / "excel-shrink-cli/office-capture"


def rgb_cycle():
    """Return a 1,530-step hue cycle with one channel changing per step."""
    segments = (
        lambda n: (255, n, 0),
        lambda n: (255 - n, 255, 0),
        lambda n: (0, 255, n),
        lambda n: (0, 255 - n, 255),
        lambda n: (n, 0, 255),
        lambda n: (255, 0, 255 - n),
    )
    return [segment(step) for segment in segments for step in range(255)]


def create_workbook(path, rows):
    """Stream actual numeric cells and repeated RGB fills into a small XLSX."""
    ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    colors = rgb_cycle()
    fills = ['<fill><patternFill patternType="none"/></fill>',
             '<fill><patternFill patternType="gray125"/></fill>']
    fills += [f'<fill><patternFill patternType="solid"><fgColor rgb="FF{red:02X}{green:02X}{blue:02X}"/>'
              '<bgColor indexed="64"/></patternFill></fill>' for red, green, blue in colors]
    xfs = ['<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>']
    xfs += [f'<xf numFmtId="0" fontId="0" fillId="{index + 2}" borderId="0" '
            'xfId="0" applyFill="1"/>' for index in range(len(colors))]
    styles = (f'<styleSheet xmlns="{ns}"><fonts count="1"><font><sz val="11"/>'
              '<name val="Calibri"/></font></fonts>'
              f'<fills count="{len(fills)}">{"".join(fills)}</fills>'
              '<borders count="1"><border><left/><right/><top/><bottom/><diagonal/>'
              '</border></borders><cellStyleXfs count="1"><xf numFmtId="0" '
              'fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
              f'<cellXfs count="{len(xfs)}">{"".join(xfs)}</cellXfs>'
              '<cellStyles count="1"><cellStyle name="Normal" xfId="0" '
              'builtinId="0"/></cellStyles></styleSheet>')
    parts = {
        '[Content_Types].xml': (
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
            '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
            '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>'
            '</Types>'),
        '_rels/.rels': (
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
            '</Relationships>'),
        'xl/workbook.xml': (
            f'<workbook xmlns="{ns}" xmlns:r="{rel}"><sheets>'
            f'<sheet name="{SHEET_NAME}" sheetId="1" r:id="rId1"/>'
            '</sheets></workbook>'),
        'xl/_rels/workbook.xml.rels': (
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
            '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
            '</Relationships>'),
        'xl/styles.xml': styles,
    }
    declaration = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name, xml in parts.items():
            archive.writestr(name, declaration + xml)
        with archive.open('xl/worksheets/sheet1.xml', 'w', force_zip64=True) as sheet:
            sheet.write((declaration + f'<worksheet xmlns="{ns}"><dimension ref="A1:A{rows}"/>'
                         '<sheetFormatPr defaultRowHeight="15"/>'
                         '<cols><col min="1" max="1" width="12" customWidth="1"/></cols>'
                         '<sheetData>').encode())
            for row in range(1, rows + 1):
                sheet.write(f'<row r="{row}"><c r="A{row}" s="{(row - 1) % len(colors) + 1}">'
                            f'<v>{row}</v></c></row>'.encode())
            sheet.write(b'</sheetData></worksheet>')
    return path


def prepare_worker_copy():
    """Add one direct-image diagnosis command to a disposable add-in copy."""
    if COPY.exists():
        raise RuntimeError(f'Temporary worker copy already exists: {COPY}')
    shutil.copytree(SOURCE, COPY)
    target = COPY / 'web/resident-diagnosis.js'
    source = target.read_text(encoding='utf-8')
    operation = (HERE / 'probe-operation.js').read_text(encoding='utf-8')
    anchor = "  async function execute(command, {office}) {"
    if source.count(anchor) != 1:
        raise RuntimeError('Diagnosis insertion point changed')
    source = source.replace(anchor, operation + '\n' + anchor)
    branch = "    if (command.operation === 'read_cells') {"
    if source.count(branch) != 1:
        raise RuntimeError('Diagnosis command insertion point changed')
    source = source.replace(branch, "    if (command.operation === 'probe_range_image') {\n"
                            "      return probeRangeImage(command, office);\n    }\n" + branch)
    source = source.replace(branch, "    if (command.operation === 'hash_sweep') {\n"
                            "      return probeHashSweep(command, office);\n    }\n" + branch)
    target.write_text(source, encoding='utf-8')


def publish(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix('.pending')
    pending.write_text(json.dumps(value), encoding='utf-8')
    os.replace(pending, path)


def read_json(path):
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except (OSError, ValueError):
        return None


def wait_until(predicate, process, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result is not None:
            return result
        if process.poll() is not None:
            raise RuntimeError(f'Resident exited with {process.returncode}')
        time.sleep(0.25)
    raise TimeoutError(f'No response after {timeout} seconds')


def probe_once(directory, session, process, command_id, start_row, rows, timeout):
    """Request one image and record whether its pixels cover every requested row."""
    publish(session / f'commands/one/{command_id}.json', {
        'id': command_id, 'kind': 'diagnosis',
        'operation': 'probe_range_image', 'sheet_name': SHEET_NAME,
        'start_row': start_row, 'rows': rows,
    })
    started = time.monotonic()
    try:
        response = wait_until(lambda: read_json(
            session / f'responses/one/{command_id}.json'), process, timeout + 15)
    except (TimeoutError, RuntimeError) as error:
        return {'start_row': start_row, 'rows': rows,
                'elapsed_seconds': round(time.monotonic() - started, 3),
                'ok': False, 'error': str(error), 'client_lost': True}

    image = response.pop('image_base64', None)
    image_path = directory / f'{command_id}.png'
    if image:
        image_path.write_bytes(base64.b64decode(image))
    returned = response.get('ok') and not response.get('diagnostic_error')
    complete = returned and response.get('height') == 20 * rows + 1
    result = {'start_row': start_row, 'rows': rows,
              'elapsed_seconds': round(time.monotonic() - started, 3),
              'returned': returned, 'complete': complete, 'ok': complete,
              'width': response.get('width'), 'height': response.get('height'),
              'error': response.get('diagnostic_error') or response.get('error'),
              'png': str(image_path) if image else None}
    metrics = read_json(session / 'metrics.json') or {}
    client = metrics.get('clients', {}).get('one', {})
    result['peak_private_bytes'] = client.get('peak_private_bytes')
    result['peak_working_set_bytes'] = client.get('peak_working_set_bytes')
    return result


def run_session(directory, workbook, workbook_rows, binary, attempts, timeout, start_row=1):
    """Open one private Excel client and try the requested range heights."""
    session = directory / 'session'
    control = directory / 'control'
    publish(control / 'control.json', {'paused': False, 'hide_excel': True})
    publish(session / 'capacity.json', {'reserved_clients': 1})
    publish(session / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(workbook), 'workbook': 'one.xlsx',
        'sheets': [{'index': 1, 'name': SHEET_NAME, 'bottom_row': workbook_rows - 1,
                    'base_right_column': 0, 'right_column': 0,
                    'horizontal_overflow_required': False,
                    'drawing_fallback_required': False}],
    })
    environment = {**os.environ, 'EXCEL_SHRINK_CONTROL_ROOM_DIR': str(control),
                   'EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID': 'officejs-image-limit',
                   'EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID': 'officejs-image-limit'}
    results = []
    with (directory / 'stdout.log').open('wb') as stdout, (directory / 'stderr.log').open('wb') as stderr:
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
            '-File', str(COPY / 'resident.ps1'), '-SessionPath', str(session),
            '-HostExecutable', str(binary), '-StartupTimeoutSeconds', '300',
            '-CaptureTimeoutSeconds', str(timeout), '-HideExcel',
        ], stdout=stdout, stderr=stderr, env=environment)
        try:
            wait_until(lambda: True if (read_json(session / 'metrics.json') or {}).get(
                'primary_clients_ready') else None, process, 300)
            last_failure = None
            lower_success = None
            for index, rows in enumerate(attempts):
                command_id = f'probe{index:03d}'
                result = probe_once(directory, session, process, command_id,
                                    start_row, rows, timeout)
                results.append(result)
                print(json.dumps(result), flush=True)
                if result.get('client_lost'):
                    break
                if result['ok']:
                    if lower_success is None:
                        lower_success = rows
                        del attempts[index + 1:]
                    else:
                        lower_success = max(lower_success, rows)
                elif last_failure is None or rows < last_failure:
                    last_failure = rows
                if lower_success is not None and last_failure is not None and last_failure - lower_success > 1:
                    attempts.append((lower_success + last_failure) // 2)
        finally:
            publish(session / 'finish.json', {'complete': True})
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                subprocess.run(['taskkill', '/F', '/T', '/PID', str(process.pid)], timeout=20,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            publish(directory / 'results.json', results)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', action='store_true', help='Start an isolated Windows Excel client')
    parser.add_argument('--rows', type=int, default=MAX_ROWS)
    parser.add_argument('--existing-workbook', type=Path,
                        help='Reuse a generated workbook instead of rebuilding it')
    parser.add_argument('--single-start-row', type=int, default=1,
                        help='Probe one range starting at this row')
    parser.add_argument('--single-rows', type=int,
                        help='Probe exactly this many rows, without boundary search')
    parser.add_argument('--timeout', type=int, default=120)
    parser.add_argument('--binary', type=Path, help='Windows capture host executable')
    args = parser.parse_args()
    if not 1 <= args.rows <= MAX_ROWS or not 30 <= args.timeout <= 600:
        parser.error('rows must be 1..1048576 and timeout 30..600 seconds')
    if args.run and os.name != 'nt':
        parser.error('--run requires Windows Python')
    directory = Path(tempfile.gettempdir()) / f'officejs-image-limit-{time.time_ns()}'
    if args.single_start_row != 1 and args.single_rows is None:
        parser.error('--single-start-row requires --single-rows')
    if args.single_rows is not None and (args.single_rows < 1 or args.single_start_row < 1 or
                                        args.single_start_row + args.single_rows - 1 > args.rows):
        parser.error('single range lies outside the workbook')
    workbook = args.existing_workbook or create_workbook(directory / 'rgb-sweep.xlsx', args.rows)
    print(json.dumps({'workbook': str(workbook), 'rows': args.rows,
                      'bytes': workbook.stat().st_size}), flush=True)
    if not args.run:
        return
    if args.binary is None or not args.binary.is_file():
        parser.error(f'Host executable missing: {args.binary}')
    prepare_worker_copy()
    try:
        if args.single_rows is not None:
            attempts = [args.single_rows]
        else:
            attempts = []
            rows = args.rows
            while rows >= 1:
                attempts.append(rows)
                rows //= 2
        run_session(directory, workbook, args.rows, args.binary, attempts, args.timeout,
                    args.single_start_row)
    finally:
        shutil.rmtree(COPY)


if __name__ == '__main__':
    main()
