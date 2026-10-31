"""Inspect Office.js and Check Performance on one isolated disposable workbook."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import urllib.request
import xml.etree.ElementTree as ET
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


def load_helpers():
    path = HERE.parent / 'officejs-image-limit/run.py'
    spec = importlib.util.spec_from_file_location('image_probe', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def prepare_worker(directory):
    worker = directory / 'excel-shrink-cli/office-capture'
    shutil.copytree(ROOT / 'excel-shrink-cli/office-capture', worker)
    archive = directory / 'excel-shrink-cli/src/regression'
    archive.mkdir(parents=True)
    shutil.copy2(ROOT / 'excel-shrink-cli/src/regression/archive_processes.ps1', archive)
    path = worker / 'web/resident-diagnosis.js'
    source = path.read_text(encoding='utf-8')
    anchor = '  async function execute(command, {office}) {'
    if source.count(anchor) != 1:
        raise RuntimeError('The diagnosis extension point changed')
    operation = (HERE / 'runtime-probe.js').read_text(encoding='utf-8')
    source = source.replace(anchor, operation + '\n' + anchor + "\n"
        "    if (command.operation === 'probe_check_performance') {\n"
        "      return probeCheckPerformance(command, office);\n    }\n")
    path.write_text(source, encoding='utf-8')
    return worker


def create_fixture(helper, directory):
    original = helper.create_workbook(directory / 'fixture-source.xlsx', 20)
    fixture = directory / 'fixture.xlsx'
    with zipfile.ZipFile(original) as source, zipfile.ZipFile(fixture, 'w', zipfile.ZIP_DEFLATED) as target:
        for part in source.infolist():
            data = source.read(part)
            if part.filename == 'xl/worksheets/sheet1.xml':
                data = data.replace(b'ref="A1:A20"', b'ref="A1:Z10000"')
                data = data.replace(b'</sheetData>', b'<row r="10000"><c r="Z10000" s="2"/></row></sheetData>')
            target.writestr(part.filename, data)
    return fixture


def targets(port):
    try:
        with urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list', timeout=2) as response:
            return json.load(response)
    except (OSError, ValueError):
        return []


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/excel-shrink-cli.exe')
    parser.add_argument('--native', action='store_true')
    parser.add_argument('--optimize', action='store_true')
    parser.add_argument('--hidden-optimize', action='store_true')
    parser.add_argument('--click-optimize', action='store_true')
    args = parser.parse_args()
    if args.click_optimize and args.hidden_optimize:
        parser.error('--click-optimize requires visible Excel')
    if (args.optimize or args.hidden_optimize or args.click_optimize) and not args.native:
        parser.error('Native action options require --native')
    if (args.hidden_optimize or args.click_optimize) and not args.optimize:
        parser.error('Optimization options require --optimize')
    directory = args.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    helper = load_helpers()
    worker = prepare_worker(directory)
    fixture = create_fixture(helper, directory)
    session = directory / 'session'
    helper.publish(session / 'capacity.json', {'reserved_clients': 1})
    helper.publish(session / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(fixture), 'workbook': 'one.xlsx',
        'sheets': [{'index': 1, 'name': 'RGB sweep', 'bottom_row': 9999,
                    'base_right_column': 25, 'right_column': 25,
                    'horizontal_overflow_required': False, 'drawing_fallback_required': False}]})
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    environment = {**os.environ,
        'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS': f'--remote-debugging-port={port} --remote-debugging-address=127.0.0.1',
        'EXCEL_SHRINK_SYSTEM_RESERVE_MIB': '1024'}
    # Never inherit the production run's control-room identity.
    for name in list(environment):
        if name.startswith('EXCEL_SHRINK_CONTROL_ROOM'):
            del environment[name]
    debugger = None
    with (directory / 'stdout.log').open('wb') as stdout, (directory / 'stderr.log').open('wb') as stderr:
        process = subprocess.Popen(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
            '-File', str(worker / 'resident.ps1'), '-SessionPath', str(session),
            '-HostExecutable', str(args.binary.resolve()), '-StartupTimeoutSeconds', '120',
            '-CaptureTimeoutSeconds', '120', '-Visible'], stdout=stdout, stderr=stderr, env=environment,
            creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            helper.wait_until(lambda: True if (helper.read_json(session / 'metrics.json') or {})
                .get('primary_clients_ready') else None, process, 150)
            # Debugging is optional; a disabled CDP endpoint must not prevent COM tests.
            deadline = time.monotonic() + 20
            pages = targets(port)
            while not pages and time.monotonic() < deadline:
                time.sleep(0.25)
                pages = targets(port)
            page = next((page for page in pages if 'taskpane' in page.get('url', '')), None)
            if page:
                node = Path(os.environ['LOCALAPPDATA']) / 'Programs/Microsoft VS Code/Code.exe'
                debugger = subprocess.Popen([str(node), str(HERE / 'debugger.cjs'),
                    page['webSocketDebuggerUrl'], str(directory / 'debugger.json')],
                    env={**environment, 'ELECTRON_RUN_AS_NODE': '1'}, stdout=stdout, stderr=stderr,
                    creationflags=subprocess.CREATE_NO_WINDOW)
                helper.wait_until(lambda: True if (directory / 'debugger.json.ready').exists() else None,
                                  debugger, 15)
            helper.publish(session / 'commands/one/inventory.json', {
                'id': 'inventory', 'kind': 'diagnosis', 'operation': 'probe_check_performance',
                'breakpoint': debugger is not None})
            response = helper.wait_until(lambda: helper.read_json(session / 'responses/one/inventory.json'), process, 60)
            helper.publish(directory / 'officejs.json', response)
            print('Office.js inventory:', response.get('ok'), flush=True)
            if args.native:
                command = ['powershell.exe', '-NoProfile', '-STA', '-ExecutionPolicy', 'Bypass',
                    '-File', str(HERE / 'probe-native.ps1'), '-Session', str(session),
                    '-Worker', str(worker), '-Output', str(directory / 'native.json')]
                if args.optimize:
                    command.append('-Optimize')
                if args.hidden_optimize:
                    command.append('-HiddenOptimize')
                if args.click_optimize:
                    command.append('-ClickOptimize')
                subprocess.run(command, stdout=stdout, stderr=stderr, timeout=90, check=True,
                               creationflags=subprocess.CREATE_NO_WINDOW)
                native = helper.read_json(directory / 'native.json') or {}
                print('Native probe:', native.get('error') or native.get('optimization'), flush=True)
                before = directory / 'before-optimization.xlsx'
                after = directory / 'after-optimization.xlsx'
                if before.exists() and after.exists():
                    with zipfile.ZipFile(before) as archive:
                        original_xml = archive.read('xl/worksheets/sheet1.xml')
                    with zipfile.ZipFile(after) as archive:
                        optimized_xml = archive.read('xl/worksheets/sheet1.xml')
                    cell_tag = '{http://schemas.openxmlformats.org/spreadsheetml/2006/main}c'
                    helper.publish(directory / 'xml-comparison.json', {
                        'worksheet_xml_identical': original_xml == optimized_xml,
                        'z10000_before': any(cell.get('r') == 'Z10000'
                            for cell in ET.fromstring(original_xml).iter(cell_tag)),
                        'z10000_after': any(cell.get('r') == 'Z10000'
                            for cell in ET.fromstring(optimized_xml).iter(cell_tag))})
            # URLs may contain tokens. Preserve only titles and origins for panel classification.
            from urllib.parse import urlsplit
            helper.publish(directory / 'webviews.json', [{'title': p.get('title'),
                'origin': urlsplit(p.get('url', ''))._replace(path='', query='', fragment='').geturl()}
                for p in targets(port)])
            print('Results:', directory, flush=True)
        finally:
            helper.publish(session / 'finish.json', {'complete': True})
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                subprocess.run(['taskkill', '/F', '/T', '/PID', str(process.pid)], timeout=15,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if debugger:
                debugger.terminate()
                debugger.wait(timeout=10)


if __name__ == '__main__':
    main()
