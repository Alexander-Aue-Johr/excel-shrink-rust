"""Probe Office.js width and high-entropy PNG cost with one synthetic workbook.

Run with Windows Python. All generated files remain under Windows TEMP.
"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import zipfile

import run as resident


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SHEET_NAME = 'Base62 rainbow'
DIGITS = '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ'
DEFAULT_COLUMNS = 2600
DEFAULT_ROWS = 320
PALETTE = resident.rgb_cycle()


def base62(value):
    """Encode a nonnegative integer in the compact alphabet visible in cells."""
    if value == 0:
        return DIGITS[0]
    encoded = ''
    while value:
        value, digit = divmod(value, len(DIGITS))
        encoded = DIGITS[digit] + encoded
    return encoded


def column_name(index):
    """Convert a zero-based column index to its XLSX address letters."""
    name = ''
    while index >= 0:
        index, digit = divmod(index, 26)
        name = chr(ord('A') + digit) + name
        index -= 1
    return name


def create_workbook(path, columns, rows):
    """Stream a two-dimensional fine rainbow with unique Base62 cell labels."""
    ns = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
    rel = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
    fills = ['<fill><patternFill patternType="none"/></fill>',
             '<fill><patternFill patternType="gray125"/></fill>']
    fills.extend(
        f'<fill><patternFill patternType="solid"><fgColor rgb="FF{red:02X}{green:02X}{blue:02X}"/>'
        '<bgColor indexed="64"/></patternFill></fill>'
        for red, green, blue in PALETTE
    )
    xfs = ['<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>']
    xfs.extend(
        f'<xf numFmtId="0" fontId="0" fillId="{index + 2}" borderId="0" xfId="0" applyFill="1"/>'
        for index in range(len(PALETTE))
    )
    styles = (f'<styleSheet xmlns="{ns}"><fonts count="1"><font><sz val="5"/>'
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
    column_names = [column_name(column) for column in range(columns)]
    # Separate rows by whole rainbow cycles plus one step: labels stay unique,
    # while the fill shifts one color step across either axis.
    row_stride = ((columns - 1) // len(PALETTE) + 1) * len(PALETTE) + 1
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name, xml in parts.items():
            archive.writestr(name, declaration + xml)
        with archive.open('xl/worksheets/sheet1.xml', 'w', force_zip64=True) as sheet:
            sheet.write((declaration + f'<worksheet xmlns="{ns}"><dimension ref="A1:{column_name(columns - 1)}{rows}"/>'
                         '<sheetFormatPr defaultRowHeight="8"/>'
                         f'<cols><col min="1" max="{columns}" width="3.5" customWidth="1"/></cols>'
                         '<sheetData>').encode())
            for row in range(rows):
                cells = []
                for col in range(columns):
                    value = row * row_stride + col
                    style = value % len(PALETTE) + 1
                    address = f'{column_names[col]}{row + 1}'
                    cells.append(f'<c r="{address}" s="{style}" t="inlineStr"><is><t>{base62(value)}</t></is></c>')
                sheet.write((f'<row r="{row + 1}" ht="8" customHeight="1">'
                             + ''.join(cells) + '</row>').encode())
            sheet.write(b'</sheetData></worksheet>')
    return path


def prepare_worker_copy(directory):
    """Add one diagnosis operation to a disposable copy of the production add-in."""
    copy = directory / 'excel-shrink-cli/office-capture'
    shutil.copytree(ROOT / 'excel-shrink-cli/office-capture', copy)
    archive_helper = directory / 'excel-shrink-cli/src/regression/archive_processes.ps1'
    archive_helper.parent.mkdir(parents=True)
    shutil.copy2(ROOT / 'excel-shrink-cli/src/regression/archive_processes.ps1', archive_helper)
    target = copy / 'web/resident-diagnosis.js'
    source = target.read_text(encoding='utf-8')
    function_anchor = '  async function execute(command, {office}) {'
    command_anchor = "    if (command.operation === 'read_cells') {"
    if source.count(function_anchor) != 1 or source.count(command_anchor) != 1:
        raise RuntimeError('Resident diagnosis insertion point changed')
    operation = (HERE / 'entropy-operation.js').read_text(encoding='utf-8')
    source = source.replace(function_anchor, operation + '\n' + function_anchor, 1)
    source = source.replace(command_anchor,
                            "    if (command.operation === 'probe_entropy_image') {\n"
                            '      return probeEntropyImage(command, office);\n'
                            '    }\n' + command_anchor, 1)
    target.write_text(source, encoding='utf-8')
    return copy


def start_client(directory, workbook, binary, worker_copy, columns, rows, timeout):
    """Launch one private Excel instance without touching an existing runner."""
    session = directory / 'session'
    control = directory / 'control'
    resident.publish(control / 'control.json', {'paused': False, 'hide_excel': True})
    resident.publish(session / 'capacity.json', {'reserved_clients': 1})
    resident.publish(session / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(workbook), 'workbook': 'one.xlsx',
        'sheets': [{'index': 1, 'name': SHEET_NAME, 'bottom_row': rows - 1,
                    'base_right_column': columns - 1, 'right_column': columns - 1,
                    'horizontal_overflow_required': False, 'drawing_fallback_required': False}],
    })
    environment = {**os.environ, 'EXCEL_SHRINK_CONTROL_ROOM_DIR': str(control),
                   'EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID': 'officejs-entropy-probe',
                   'EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID': 'officejs-entropy-probe'}
    stdout = (directory / 'stdout.log').open('wb')
    stderr = (directory / 'stderr.log').open('wb')
    try:
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            str(worker_copy / 'resident.ps1'), '-SessionPath', str(session),
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


def close_client(session, process):
    """Close only the Excel instance started for this experiment."""
    resident.publish(session / 'finish.json', {'complete': True})
    try:
        process.wait(timeout=45)
    except subprocess.TimeoutExpired:
        subprocess.run(['taskkill', '/F', '/T', '/PID', str(process.pid)], timeout=20,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def capture(directory, session, process, report, columns, rows, timeout,
            expected_width=None, expected_height=None, save_png=False, hash_png=False):
    """Capture one range and checkpoint its dimensions, cost, and optional PNG."""
    command_id = f'probe{len(report["captures"]):03d}'
    resident.publish(session / f'commands/one/{command_id}.json', {
        'id': command_id, 'kind': 'diagnosis', 'operation': 'probe_entropy_image',
        'sheet_name': SHEET_NAME, 'columns': columns, 'rows': rows,
        'hash': hash_png or save_png, 'save_png': save_png,
    })
    started = time.monotonic()
    try:
        response = resident.wait_until(lambda: resident.read_json(
            session / f'responses/one/{command_id}.json'), process, timeout + 30)
    except (TimeoutError, RuntimeError) as error:
        result = {'columns': columns, 'rows': rows, 'error': str(error),
                  'elapsed_seconds': round(time.monotonic() - started, 3)}
        report['captures'].append(result)
        resident.publish(directory / 'results.json', report)
        raise
    image = response.pop('image_base64', None)
    fields = ('ok', 'width', 'height', 'png_bytes', 'capture_ms', 'decode_ms',
              'hash_ms', 'sha256', 'diagnostic_error', 'error')
    result = {'columns': columns, 'rows': rows,
              'elapsed_seconds': round(time.monotonic() - started, 3),
              'expected_width': expected_width, 'expected_height': expected_height,
              **{key: response[key] for key in fields if key in response}}
    result['ok'] = response.get('ok') is True and not response.get('diagnostic_error')
    result['js_heap_used_bytes'] = response.get('pane_resources', {}).get('js_heap_used_bytes')
    result['complete'] = (result['ok'] and
                          (expected_width is None or response.get('width') == expected_width) and
                          (expected_height is None or response.get('height') == expected_height))
    if response.get('width') and response.get('height'):
        pixels = response['width'] * response['height']
        result['bitmap_rgba_bytes'] = pixels * 4
        result['png_bits_per_pixel'] = round(8 * response['png_bytes'] / pixels, 3)
    if image:
        png = base64.b64decode(image)
        if hashlib.sha256(png).hexdigest() != result.get('sha256'):
            raise RuntimeError('Saved PNG differs from the worker hash')
        destination = directory / ('maximum-complete.png' if result['complete']
                                   else f'incomplete-{command_id}.png')
        destination.write_bytes(png)
        result['png'] = str(destination)
        result['png_bytes'] = len(png)
    metrics = resident.read_json(session / 'metrics.json') or {}
    client = metrics.get('clients', {}).get('one', {})
    result['peak_private_bytes'] = client.get('peak_private_bytes')
    result['peak_working_set_bytes'] = client.get('peak_working_set_bytes')
    report['captures'].append(result)
    resident.publish(directory / 'results.json', report)
    print(json.dumps(result), flush=True)
    return result


def run_probe(directory, workbook, binary, columns, rows, timeout):
    """Find the width boundary with one row, then test the full-height image."""
    report = {'workbook': str(workbook), 'workbook_sha256': hashlib.sha256(workbook.read_bytes()).hexdigest(),
              'columns': columns, 'rows': rows, 'palette_steps': len(PALETTE),
              'alphabet': DIGITS, 'captures': []}
    worker_copy = prepare_worker_copy(directory)
    process, session = start_client(directory, workbook, binary, worker_copy, columns, rows, timeout)
    try:
        first = capture(directory, session, process, report, 1, 1, timeout)
        second = capture(directory, session, process, report, 2, 1, timeout)
        sample = capture(directory, session, process, report, 32, 1, timeout)
        height = capture(directory, session, process, report, 1, 2, timeout)
        width_step = second['width'] - first['width']
        height_step = height['height'] - first['height']
        if not all(item['complete'] for item in (first, second, sample, height)) or (
            first['width'] != width_step + 1 or sample['width'] != 32 * width_step + 1 or
            first['height'] != height_step + 1):
            raise RuntimeError('Cell dimensions were not linear; width boundary cannot be inferred')
        report['cell_pixels'] = {'width': width_step, 'height': height_step}
        expected = lambda count: count * width_step + 1
        if expected(columns) <= 32766:
            raise RuntimeError('Fixture is narrower than the known image edge; increase --columns')
        over = capture(directory, session, process, report, columns, rows, timeout,
                       expected(columns), rows * height_step + 1)
        if over['complete']:
            raise RuntimeError('Fixture did not exceed the image width; increase --columns')
        wide_row = capture(directory, session, process, report, columns, 1, timeout,
                           expected(columns), height_step + 1)
        if wide_row['complete']:
            raise RuntimeError('One-row width did not truncate; no width boundary was found')
        low, high = 32, columns
        while high - low > 1:
            middle = (low + high) // 2
            probe = capture(directory, session, process, report, middle, 1, timeout,
                            expected(middle), height_step + 1)
            if probe['complete']:
                low = middle
            else:
                high = middle
        report['maximum_complete_columns'] = low
        report['maximum_complete_width'] = expected(low)
        resident.publish(directory / 'results.json', report)
        for requested_rows in (rows, rows // 2, rows // 4):
            if requested_rows < 1:
                continue
            final = capture(directory, session, process, report, low, requested_rows,
                            timeout, expected(low), requested_rows * height_step + 1,
                            save_png=True)
            if final['complete']:
                report['maximum_saved_capture'] = final
                break
        resident.publish(directory / 'results.json', report)
    finally:
        close_client(session, process)
    return report


def run_single_capture(directory, workbook, binary, columns, rows, timeout):
    """Hash one large image without transporting its base64 PNG through JSON."""
    report = {'workbook': str(workbook), 'workbook_sha256': hashlib.sha256(workbook.read_bytes()).hexdigest(),
              'columns': columns, 'rows': rows, 'palette_steps': len(PALETTE),
              'alphabet': DIGITS, 'captures': []}
    worker_copy = prepare_worker_copy(directory)
    process, session = start_client(directory, workbook, binary, worker_copy, columns, rows, timeout)
    try:
        first = capture(directory, session, process, report, 1, 1, timeout)
        second = capture(directory, session, process, report, 2, 1, timeout)
        taller = capture(directory, session, process, report, 1, 2, timeout)
        width_step = second['width'] - first['width']
        height_step = taller['height'] - first['height']
        if first['width'] != width_step + 1 or first['height'] != height_step + 1:
            raise RuntimeError('Cell dimensions were not linear')
        report['cell_pixels'] = {'width': width_step, 'height': height_step}
        result = capture(directory, session, process, report, columns, rows, timeout,
                         columns * width_step + 1, rows * height_step + 1, hash_png=True)
        report['large_capture'] = result
        resident.publish(directory / 'results.json', report)
    finally:
        close_client(session, process)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='Windows capture-host executable')
    parser.add_argument('--columns', type=int, default=DEFAULT_COLUMNS)
    parser.add_argument('--rows', type=int, default=DEFAULT_ROWS)
    parser.add_argument('--timeout', type=int, default=600)
    parser.add_argument('--run', action='store_true', help='Open Excel and run the Office.js probe')
    parser.add_argument('--single-capture', action='store_true',
                        help='Hash one image without returning its PNG through JSON')
    args = parser.parse_args()
    if not 1 <= args.columns <= 16_384 or not 2 <= args.rows <= 1_048_576:
        parser.error('columns and rows lie outside the Excel grid')
    if not 60 <= args.timeout <= 1800:
        parser.error('--timeout must be 60..1800 seconds')
    if args.run and os.name != 'nt':
        parser.error('--run requires Windows Python')
    if args.run and (args.binary is None or not args.binary.is_file()):
        parser.error('A Windows --binary is required for --run')
    directory = Path(tempfile.gettempdir()) / f'officejs-entropy-probe-{time.time_ns()}'
    directory.mkdir(parents=True)
    workbook = create_workbook(directory / 'base62-rainbow.xlsx', args.columns, args.rows)
    print(json.dumps({'workbook': str(workbook), 'bytes': workbook.stat().st_size}), flush=True)
    if args.run:
        if args.single_capture:
            report = run_single_capture(directory, workbook, args.binary, args.columns, args.rows, args.timeout)
            print(json.dumps({'results': str(directory / 'results.json'),
                              'complete': report['large_capture']['complete']}), flush=True)
        else:
            report = run_probe(directory, workbook, args.binary, args.columns, args.rows, args.timeout)
            print(json.dumps({'results': str(directory / 'results.json'),
                              'maximum_complete_width': report['maximum_complete_width'],
                              'png': report.get('maximum_saved_capture', {}).get('png')}), flush=True)


if __name__ == '__main__':
    main()
