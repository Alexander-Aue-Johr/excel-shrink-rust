"""Test fixed-height invariance and automatic-height controls on private Excel copies."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
CASES = ('fixed-cell', 'fixed-column', 'automatic-cell', 'automatic-column',
         'stale-ht-cell', 'fixed-row-deleted', 'fixed-hidden-cell')
NS = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'


def load_helpers():
    """Reuse only the earlier experiment's package and resident protocol helpers."""
    source = HERE.parent / 'officejs-million-row-heights/run.py'
    spec = importlib.util.spec_from_file_location('height_helpers', source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sheet_xml(case, before):
    """Change only a font source, except for the explicit deleted-row negative control."""
    fixed = case.startswith('fixed-')
    column = case.endswith('column')
    columns = '<cols><col min="1" max="1" width="18" customWidth="1"/>'
    columns += '<col min="2" max="2" width="12" customWidth="1"'
    columns += (' style="1"' if column and before else '') + '/></cols>'
    rows = []
    for row in range(1, 33):
        if row == 8 and case == 'fixed-row-deleted' and not before:
            continue
        attributes = ''
        if (fixed and row == 8) or case == 'fixed-column':
            attributes = ' ht="40" customHeight="1"'
        elif case == 'stale-ht-cell' and row == 8:
            attributes = ' ht="40" customHeight="0"'
        if case == 'fixed-hidden-cell' and row == 8:
            attributes += ' hidden="1"'
        content = f'<c r="A{row}" s="0"><v>{row}</v></c>' if row != 8 else ''
        if row == 8 and before and not column:
            content += '<c r="B8" s="1"/>'
        rows.append(f'<row r="{row}"{attributes}>{content}</row>')
    return (f'<worksheet xmlns="{NS}"><dimension ref="A1:B32"/>'
            '<sheetFormatPr defaultRowHeight="15"/>' + columns + '<sheetData>'
            + ''.join(rows) + '</sheetData></worksheet>')


def create_workbook(directory, helpers):
    """Package paired small sheets with anchored content below the empty test row."""
    parts = helpers.fixture_parts(directory)
    names = [f'{case}-{role}' for case in CASES for role in ('before', 'after')]
    rel_ns = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
    parts['xl/workbook.xml'] = (f'<workbook xmlns="{NS}" xmlns:r="{rel_ns}"><sheets>'
        + ''.join(f'<sheet name="{name}" sheetId="{i}" r:id="rId{i}"/>'
                  for i, name in enumerate(names, 1))
        + '</sheets><calcPr calcMode="manual"/></workbook>').encode()
    parts['xl/_rels/workbook.xml.rels'] = (
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + ''.join(f'<Relationship Id="rId{i}" Type="{rel_ns}/worksheet" Target="worksheets/sheet{i}.xml"/>'
                  for i in range(1, len(names) + 1))
        + f'<Relationship Id="rIdStyles" Type="{rel_ns}/styles" Target="styles.xml"/></Relationships>').encode()
    types = ET.fromstring(parts['[Content_Types].xml'])
    for i in range(2, len(names) + 1):
        ET.SubElement(types, '{http://schemas.openxmlformats.org/package/2006/content-types}Override', {
            'PartName': f'/xl/worksheets/sheet{i}.xml',
            'ContentType': 'application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml'})
    parts['[Content_Types].xml'] = ET.tostring(types)
    for i, (case, before) in enumerate(((case, before) for case in CASES for before in (True, False)), 1):
        parts[f'xl/worksheets/sheet{i}.xml'] = sheet_xml(case, before).encode()
    path = directory / 'input.xlsx'
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            archive.writestr(name, content)
    return path, names


def prepare_worker(directory):
    """Inject the read/save/rebuild probe into a disposable worker copy."""
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
    dispatch = "\n    if (command.operation === 'probe_fixed_heights') return probeFixedRowHeights(command, office);"
    script.write_text(source.replace(anchor, (HERE / 'probe.js').read_text() + '\n' + anchor + dispatch),
                      encoding='utf-8')
    return worker


def retain_saved_xml(directory, response):
    """Copy a completed save, never an in-progress workbook write."""
    location = response['document_url']
    if location.startswith('file:'):
        location = unquote(urlsplit(location).path).lstrip('/')
    source = Path(location)
    if not source.is_file():
        raise RuntimeError(f'Cannot inspect saved resident copy: {location}')
    target = directory / f'{response["action"]}.xlsx'
    shutil.copyfile(source, target)
    return str(target)


def run_probe(directory, workbook, names, worker, binary, helpers):
    """Measure every pair in one resident before save, after save and after rebuild/save."""
    session = directory / 'session'
    control = directory / 'control'
    helpers.publish(control / 'control.json', {'paused': False, 'hide_excel': True})
    helpers.publish(session / 'capacity.json', {'reserved_clients': 1})
    helpers.publish(session / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(workbook), 'workbook': 'one.xlsx',
        'sheets': [{'index': i, 'name': name, 'bottom_row': 31,
                    'base_right_column': 1, 'right_column': 1,
                    'horizontal_overflow_required': False, 'drawing_fallback_required': False}
                   for i, name in enumerate(names, 1)],
    })
    environment = {**os.environ, 'EXCEL_SHRINK_CONTROL_ROOM_DIR': str(control),
                   'EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID': 'fixed-height-probe',
                   'EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID': 'fixed-height-probe'}
    started = time.monotonic()
    result = {'measurements': []}
    with (directory / 'stdout.log').open('wb') as stdout, (directory / 'stderr.log').open('wb') as stderr:
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(worker / 'resident.ps1'),
            '-SessionPath', str(session), '-HostExecutable', str(binary),
            '-StartupTimeoutSeconds', '120', '-CaptureTimeoutSeconds', '60', '-HideExcel',
        ], stdout=stdout, stderr=stderr, env=environment)
    try:
        helpers.wait_until(lambda: True if (helpers.read_json(session / 'metrics.json') or {}).get('primary_clients_ready') else None,
                           process, 120)
        result['startup_seconds'] = round(time.monotonic() - started, 3)
        for action in ('read', 'save', 'rebuild_save'):
            helpers.publish(session / f'commands/one/{action}.json', {
                'id': action, 'kind': 'diagnosis', 'operation': 'probe_fixed_heights',
                'action': action, 'sheets': names,
            })
            response = helpers.wait_until(lambda: helpers.read_json(session / f'responses/one/{action}.json'), process, 60)
            if response.get('ok') is not True or response.get('diagnostic_error'):
                raise RuntimeError(str(response))
            if action != 'read':
                response['saved_copy'] = retain_saved_xml(directory, response)
            result['measurements'].append(response)
            print(json.dumps({'action': action, 'elapsed_ms': response['elapsed_ms'], 'measured_sheets': len(response['rows'])}), flush=True)
    finally:
        helpers.close_client(session, process)
        result['final_metrics'] = helpers.read_json(session / 'metrics.json')
        helpers.publish(directory / 'results.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/excel-shrink-cli.exe')
    args = parser.parse_args()
    directory = args.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    helpers = load_helpers()
    workbook, names = create_workbook(directory, helpers)
    source_hash = hashlib.sha256(workbook.read_bytes()).hexdigest()
    result = run_probe(directory, workbook, names, prepare_worker(directory), args.binary.resolve(), helpers)
    assert hashlib.sha256(workbook.read_bytes()).hexdigest() == source_hash
    verify(directory, result)


def verify(directory, result):
    """Check known effective geometry and whether fixed row attributes survive saves."""
    assert [m['action'] for m in result['measurements']] == ['read', 'save', 'rebuild_save']
    geometry_fields = ('top', 'height', 'format_row_height', 'hidden', 'next_top')
    opening = {row['sheet']: row for row in result['measurements'][0]['rows']}
    for measurement in result['measurements']:
        by_sheet = {row['sheet']: row for row in measurement['rows']}
        for row in measurement['rows']:
            assert all(row[field] == opening[row['sheet']][field] for field in geometry_fields)
        for case in CASES:
            before, after = (by_sheet[f'{case}-{role}'] for role in ('before', 'after'))
            if case in ('fixed-cell', 'fixed-column'):
                assert before['format_row_height'] == after['format_row_height'] == 40
                assert all(before[field] == after[field] for field in geometry_fields)
            elif case == 'fixed-hidden-cell':
                assert before['height'] == after['height'] == 0
                assert before['hidden'] is True and after['hidden'] is True
            elif case == 'fixed-row-deleted':
                assert before['format_row_height'] == 40 and after['format_row_height'] == 15
                assert before['height'] > after['height']
            else:
                assert (before['height'], after['height']) == (39, 15)
        if measurement['action'] != 'read':
            with zipfile.ZipFile(measurement['saved_copy']) as archive:
                for index, case in enumerate(CASES):
                    if case not in ('fixed-cell', 'fixed-column', 'fixed-hidden-cell'):
                        continue
                    pair = []
                    for offset in (1, 2):
                        sheet = ET.fromstring(archive.read(f'xl/worksheets/sheet{index * 2 + offset}.xml'))
                        row = sheet.find(f'{{{NS}}}sheetData/{{{NS}}}row[@r="8"]')
                        assert row is not None and row.attrib.get('customHeight') == '1'
                        pair.append((float(row.attrib['ht']), row.attrib.get('hidden', '0')))
                    assert pair[0] == pair[1], (measurement['action'], case, pair)
    assert result['final_metrics']['clients']['one']['process_alive'] is False
    print('Fixed-height invariants, automatic-height controls, saved XML and cleanup verified.')


if __name__ == '__main__':
    main()
