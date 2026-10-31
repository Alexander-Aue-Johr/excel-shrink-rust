"""Compare two rainbow workbooks through byte-selected Office.js images.

Run with Windows Python. Fixtures, screenshots, and telemetry stay in TEMP.
"""

import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import xml.etree.ElementTree as ET
import zipfile

from sheet_bytes import changed_cells, make_variant


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
RAINBOW = 'RGB sweep'
OBJECTS = 'Objects'
DRAWING_NS = '{http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing}'
CHART_NS = '{http://schemas.openxmlformats.org/drawingml/2006/chart}'


def image_limit_fixture():
    """Reuse the numeric rainbow generator without running its experiment."""
    source = HERE.parent / 'officejs-image-limit/run.py'
    spec = importlib.util.spec_from_file_location('image_limit_fixture', source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.create_workbook


def publish(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix('.pending')
    pending.write_text(json.dumps(value, indent=2), encoding='utf-8')
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


def excel_save(mode, source, target):
    """Use Excel itself to create object markup or recalculate a changed copy."""
    started = time.monotonic()
    command = ['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
               '-File', str(HERE / 'excel_fixture.ps1'), '-Mode', mode,
               '-Source', str(source), '-Target', str(target)]
    result = subprocess.run(command, capture_output=True, text=True, timeout=600, check=True)
    lines = [line for line in result.stdout.splitlines() if line.startswith('{')]
    if len(lines) != 1:
        raise RuntimeError(f'Excel did not return object bounds: {result.stdout} {result.stderr}')
    return {'seconds': round(time.monotonic() - started, 3),
            'objects': json.loads(lines[0]), 'bytes': target.stat().st_size}


def prepare_worker(directory):
    """Add two diagnosis operations to a disposable copy of the resident."""
    copy = directory / 'excel-shrink-cli/office-capture'
    shutil.copytree(ROOT / 'excel-shrink-cli/office-capture', copy)
    helper = directory / 'excel-shrink-cli/src/regression/archive_processes.ps1'
    helper.parent.mkdir(parents=True)
    shutil.copy2(ROOT / 'excel-shrink-cli/src/regression/archive_processes.ps1', helper)
    target = copy / 'web/resident-diagnosis.js'
    script = target.read_text(encoding='utf-8')
    function_anchor = '  async function execute(command, {office}) {'
    command_anchor = "    if (command.operation === 'read_cells') {"
    if script.count(function_anchor) != 1 or script.count(command_anchor) != 1:
        raise RuntimeError('Resident diagnosis insertion point changed')
    script = script.replace(function_anchor,
                            (HERE / 'targeted-operation.js').read_text(encoding='utf-8') +
                            '\n' + function_anchor, 1)
    dispatch = (
        "    if (command.operation === 'probe_targeted_image') {\n"
        '      return probeTargetedImage(command, office);\n'
        '    }\n'
        "    if (command.operation === 'probe_object_bounds') {\n"
        '      return probeObjectBounds(command, office);\n'
        '    }\n'
    )
    target.write_text(script.replace(command_anchor, dispatch + command_anchor, 1), encoding='utf-8')
    return copy


def start_client(directory, workbook, worker, binary, rows, timeout):
    """Open one private Excel instance and wait until its add-in is ready."""
    session = directory / 'session'
    control = directory / 'control'
    publish(control / 'control.json', {'paused': False, 'hide_excel': True})
    publish(session / 'capacity.json', {'reserved_clients': 1})
    publish(session / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(workbook), 'workbook': 'one.xlsx',
        'sheets': [
            {'index': 1, 'name': OBJECTS, 'bottom_row': 20,
             'base_right_column': 10, 'right_column': 10,
             'horizontal_overflow_required': False, 'drawing_fallback_required': True},
            {'index': 2, 'name': RAINBOW, 'bottom_row': rows - 1,
             'base_right_column': 0, 'right_column': 0,
             'horizontal_overflow_required': False, 'drawing_fallback_required': False},
        ],
    })
    environment = {**os.environ, 'EXCEL_SHRINK_CONTROL_ROOM_DIR': str(control),
                   'EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID': 'targeted-change-probe',
                   'EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID': 'targeted-change-probe'}
    with (directory / 'stdout.log').open('wb') as stdout, (directory / 'stderr.log').open('wb') as stderr:
        started = time.monotonic()
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            str(worker / 'resident.ps1'), '-SessionPath', str(session),
            '-HostExecutable', str(binary), '-StartupTimeoutSeconds', '300',
            '-CaptureTimeoutSeconds', str(timeout), '-HideExcel',
        ], stdout=stdout, stderr=stderr, env=environment)
    try:
        wait_until(lambda: True if (read_json(session / 'metrics.json') or {}).get(
            'primary_clients_ready') else None, process, 300)
    except Exception:
        close_client(session, process)
        raise
    return process, session, round(time.monotonic() - started, 3)


def close_client(session, process):
    publish(session / 'finish.json', {'complete': True})
    try:
        process.wait(timeout=45)
    except subprocess.TimeoutExpired:
        subprocess.run(['taskkill', '/F', '/T', '/PID', str(process.pid)], timeout=20,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def diagnosis(session, process, command_id, operation, timeout, **arguments):
    """Run one resident command and reject an Office.js diagnostic error."""
    publish(session / f'commands/one/{command_id}.json',
            {'id': command_id, 'kind': 'diagnosis', 'operation': operation, **arguments})
    started = time.monotonic()
    response = wait_until(lambda: read_json(session / f'responses/one/{command_id}.json'),
                          process, timeout + 30)
    if response.get('diagnostic_error') or response.get('ok') is not True:
        raise RuntimeError(f'{operation}: {response.get("diagnostic_error") or response.get("error")}')
    return response, round(time.monotonic() - started, 3)


def capture(directory, session, process, command_id, item, timeout):
    """Save one small PNG and verify that its bytes match the add-in hash."""
    response, elapsed = diagnosis(session, process, command_id, 'probe_targeted_image', timeout,
                                  sheet_name=item['sheet'], range=item['range'])
    base64_image = response.pop('image_base64')
    image = base64.b64decode(base64_image)
    if hashlib.sha256(image).hexdigest() != response['sha256'] or len(image) != response['png_bytes']:
        raise RuntimeError(f'Image bytes differ from Office.js at {item["key"]}')
    path = directory / f'{item["key"]}.png'
    path.write_bytes(image)
    metrics = read_json(session / 'metrics.json') or {}
    client = metrics.get('clients', {}).get('one', {})
    return {'key': item['key'], 'sheet': item['sheet'], 'range': item['range'],
            'reason': item['reason'], 'width': response['width'], 'height': response['height'],
            'png_bytes': len(image), 'sha256': response['sha256'],
            'capture_ms': round(response['capture_ms'], 1),
            'decode_ms': round(response['decode_ms'], 1),
            'hash_ms': round(response['hash_ms'], 1), 'elapsed_seconds': elapsed,
            'peak_private_bytes': client.get('peak_private_bytes'),
            'peak_working_set_bytes': client.get('peak_working_set_bytes'),
            'js_heap_used_bytes': response.get('pane_resources', {}).get('js_heap_used_bytes'),
            'png_windows_path': str(path)}


def column_name(index):
    name = ''
    while index >= 0:
        index, digit = divmod(index, 26)
        name = chr(ord('A') + digit) + name
        index -= 1
    return name


def drawing_anchors(path):
    """Read two simple OOXML anchors to cross-check Excel's object bounds."""
    with zipfile.ZipFile(path) as archive:
        root = ET.fromstring(archive.read('xl/drawings/drawing1.xml'))
    found = {}
    for anchor in root.findall(f'{DRAWING_NS}twoCellAnchor'):
        identity = anchor.find(f'.//{DRAWING_NS}cNvPr')
        start = anchor.find(f'{DRAWING_NS}from')
        end = anchor.find(f'{DRAWING_NS}to')
        if identity is None or start is None or end is None:
            raise ValueError('Incomplete object anchor')
        address = lambda marker: column_name(int(marker.findtext(f'{DRAWING_NS}col'))) + str(
            int(marker.findtext(f'{DRAWING_NS}row')) + 1)
        found[identity.get('name')] = {'top_left': address(start), 'bottom_right': address(end)}
    return found


def chart_data(path):
    """Check that Excel persisted the chart's source and recalculated cache."""
    with zipfile.ZipFile(path) as archive:
        root = ET.fromstring(archive.read('xl/charts/chart1.xml'))
    formulas = [item.text for item in root.findall(f'.//{CHART_NS}f')]
    points = root.findall(f'.//{CHART_NS}pt')
    changed_point = next((point.findtext(f'{CHART_NS}v') for point in points
                          if point.get('idx') == '4'), None)
    if formulas != ["'RGB sweep'!$A$1:$A$16"] or changed_point is None:
        raise ValueError('Chart source or cached fifth value is unavailable')
    return {'source': formulas[0], 'cached_A5': changed_point}


def targets(changes, objects):
    """Turn scanned cell coordinates into halos, then add complete objects."""
    selected = []
    for change in changes:
        address = change['address']
        if not address.startswith('A') or not address[1:].isdigit():
            raise ValueError(f'Unexpected fixture cell: {address}')
        row = int(address[1:])
        key = 'removed-cell' if change['kind'] == 'removed' else 'changed-value'
        selected.append({'key': key, 'sheet': RAINBOW,
                         'range': f'A{max(1, row - 1)}:B{row + 1}',
                         'reason': f'{address} {change["kind"]}; one-cell halo'})
    if len({item['key'] for item in selected}) != len(selected):
        raise ValueError('Fixture has multiple changes of the same kind')
    selected.extend([
        {'key': 'chart', 'sheet': OBJECTS,
         'range': f'{objects["chart"]["top_left"]}:{objects["chart"]["bottom_right"]}',
         'reason': 'chart references RGB sweep A1:A16'},
        {'key': 'drawing', 'sheet': OBJECTS,
         'range': f'{objects["drawing"]["top_left"]}:{objects["drawing"]["bottom_right"]}',
         'reason': 'capture the complete drawing anchor'},
    ])
    return selected


def capture_workbook(directory, workbook, label, worker, binary, rows, items,
                     expected_objects, timeout):
    process, session, startup = start_client(directory, workbook, worker, binary, rows, timeout)
    result = {'startup_seconds': startup, 'captures': []}
    try:
        bounds, elapsed = diagnosis(session, process, 'bounds', 'probe_object_bounds', timeout,
                                    sheet_name=OBJECTS)
        for kind, collection in (('chart', bounds['charts']), ('drawing', bounds['shapes'])):
            expected = expected_objects[kind]
            match = next((item for item in collection if item['name'] == expected['name']), None)
            if match is None or any(abs(match[field] - expected[field]) > 0.5
                                    for field in ('left', 'top', 'width', 'height')):
                raise RuntimeError(f'Office.js and Excel bounds differ for {expected["name"]}')
        result['officejs_objects'] = {'charts': bounds['charts'], 'shapes': bounds['shapes'],
                                      'elapsed_seconds': elapsed, 'officejs_ms': round(bounds['elapsed_ms'], 1)}
        for index, item in enumerate(items):
            captured = capture(directory, session, process, f'image{index:03d}', item, timeout)
            result['captures'].append(captured)
            print(json.dumps({'workbook': label, 'capture': captured}), flush=True)
    finally:
        close_client(session, process)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--rows', type=int, default=100_000)
    parser.add_argument('--changed-row', type=int, default=50_000)
    parser.add_argument('--timeout', type=int, default=300)
    args = parser.parse_args()
    if os.name != 'nt' or not args.binary.is_file():
        parser.error('Run with Windows Python and a Windows capture-host executable')
    if not 16 < args.changed_row < args.rows or not 20 <= args.rows <= 1_048_576:
        parser.error('Use at least 20 rows and a changed row after the chart source')

    directory = Path(tempfile.gettempdir()) / f'officejs-targeted-change-{time.time_ns()}'
    directory.mkdir(parents=True)
    report = {'rows': args.rows, 'changed_row': args.changed_row,
              'base_capture_budget': 0, 'directory': str(directory)}
    source = image_limit_fixture()(directory / 'rgb-sweep.xlsx', args.rows)
    baseline = directory / 'baseline.xlsx'
    variant_before = directory / 'variant-before-recalc.xlsx'
    variant = directory / 'variant.xlsx'
    report['fixture_bytes'] = source.stat().st_size
    report['prepare'] = excel_save('Prepare', source, baseline)
    changes = {'A5': b'50000', f'A{args.changed_row}': None}
    started = time.monotonic()
    make_variant(baseline, variant_before, changes)
    report['mutation_seconds'] = round(time.monotonic() - started, 3)
    started = time.monotonic()
    report['changes_before_recalc'] = list(changed_cells(baseline, variant_before))
    report['scan_before_seconds'] = round(time.monotonic() - started, 3)
    report['recalculate'] = excel_save('Recalculate', variant_before, variant)
    report['chart_data'] = {'baseline': chart_data(baseline), 'variant': chart_data(variant)}
    if (report['chart_data']['baseline']['cached_A5'],
        report['chart_data']['variant']['cached_A5']) != ('5', '50000'):
        raise RuntimeError('Recalculated chart cache does not contain the changed value')
    started = time.monotonic()
    report['changes_after_recalc'] = list(changed_cells(baseline, variant))
    report['scan_after_seconds'] = round(time.monotonic() - started, 3)
    expected = set(changes)
    if {item['address'] for item in report['changes_before_recalc']} != expected or (
        {item['address'] for item in report['changes_after_recalc']} != expected
    ):
        raise RuntimeError(f'Unexpected cell changes: {report["changes_after_recalc"][:10]}')
    if report['prepare']['objects'] != report['recalculate']['objects']:
        raise RuntimeError('Excel moved an object after recalculation')
    report['xml_anchors'] = drawing_anchors(baseline)
    for kind, name in (('chart', 'Rainbow values chart'), ('drawing', 'Static rectangle')):
        if report['xml_anchors'][name] != {
            'top_left': report['prepare']['objects'][kind]['top_left'],
            'bottom_right': report['prepare']['objects'][kind]['bottom_right'],
        }:
            raise RuntimeError(f'OOXML and Excel object anchors differ for {name}')
    publish(directory / 'results.json', report)
    items = targets(report['changes_after_recalc'], report['prepare']['objects'])
    worker = prepare_worker(directory)
    for label, workbook in (('baseline', baseline), ('variant', variant)):
        report[label] = capture_workbook(directory / label, workbook, label, worker,
                                         args.binary, args.rows, items,
                                         report['prepare']['objects'], args.timeout)
        publish(directory / 'results.json', report)
    report['comparison'] = [
        {'key': left['key'], 'same_sha256': left['sha256'] == right['sha256']}
        for left, right in zip(report['baseline']['captures'], report['variant']['captures'])
    ]
    publish(directory / 'results.json', report)
    print(json.dumps({'results': str(directory / 'results.json'),
                      'comparison': report['comparison']}), flush=True)


if __name__ == '__main__':
    main()
