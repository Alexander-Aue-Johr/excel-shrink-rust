"""Inspect Office.js and Check Performance on one isolated disposable workbook."""
import argparse
from contextlib import contextmanager
from dataclasses import dataclass
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
from types import ModuleType
from typing import BinaryIO
import urllib.request
from urllib.parse import urlsplit
import xml.etree.ElementTree as ET
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


@dataclass
class Experiment:
    directory: Path
    worker: Path
    session: Path
    helper: ModuleType
    port: int
    environment: dict


@dataclass
class ExcelSession:
    process: subprocess.Popen
    stdout: BinaryIO
    stderr: BinaryIO
    debugger: subprocess.Popen | None = None


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


def parse_options():
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
    return args


def prepare_experiment(options):
    directory = options.output.resolve()
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
    return Experiment(directory, worker, session, helper, port, environment)


@contextmanager
def open_excel(experiment, binary):
    """Own the launcher, log handles, and debugger until all probes finish."""
    with (experiment.directory / 'stdout.log').open('wb') as stdout, \
            (experiment.directory / 'stderr.log').open('wb') as stderr:
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
            '-File', str(experiment.worker / 'resident.ps1'),
            '-SessionPath', str(experiment.session),
            '-HostExecutable', str(binary.resolve()),
            '-StartupTimeoutSeconds', '120', '-CaptureTimeoutSeconds', '120', '-Visible'
        ], stdout=stdout, stderr=stderr, env=experiment.environment,
            creationflags=subprocess.CREATE_NO_WINDOW)
        session = ExcelSession(process, stdout, stderr)
        try:
            wait_for_addin(experiment, session)
            yield session
        finally:
            close_experiment(experiment, session)


def wait_for_addin(experiment, session):
    experiment.helper.wait_until(
        lambda: True if (experiment.helper.read_json(experiment.session / 'metrics.json') or {})
            .get('primary_clients_ready') else None,
        session.process, 150)


def attach_runtime_debugger(experiment, session):
    # Debugging is optional; a disabled CDP endpoint must not prevent COM tests.
    deadline = time.monotonic() + 20
    pages = targets(experiment.port)
    while not pages and time.monotonic() < deadline:
        time.sleep(0.25)
        pages = targets(experiment.port)
    page = next((page for page in pages if 'taskpane' in page.get('url', '')), None)
    if not page:
        return
    node = Path(os.environ['LOCALAPPDATA']) / 'Programs/Microsoft VS Code/Code.exe'
    session.debugger = subprocess.Popen([
        str(node), str(HERE / 'debugger.cjs'), page['webSocketDebuggerUrl'],
        str(experiment.directory / 'debugger.json')
    ], env={**experiment.environment, 'ELECTRON_RUN_AS_NODE': '1'},
        stdout=session.stdout, stderr=session.stderr, creationflags=subprocess.CREATE_NO_WINDOW)
    experiment.helper.wait_until(
        lambda: True if (experiment.directory / 'debugger.json.ready').exists() else None,
        session.debugger, 15)


def inspect_officejs_runtime(experiment, session):
    attach_runtime_debugger(experiment, session)
    experiment.helper.publish(experiment.session / 'commands/one/inventory.json', {
        'id': 'inventory', 'kind': 'diagnosis', 'operation': 'probe_check_performance',
        'breakpoint': session.debugger is not None})
    response = experiment.helper.wait_until(
        lambda: experiment.helper.read_json(experiment.session / 'responses/one/inventory.json'),
        session.process, 60)
    experiment.helper.publish(experiment.directory / 'officejs.json', response)
    print('Office.js inventory:', response.get('ok'), flush=True)


def probe_native_optimization(experiment, session, options):
    command = [
        'powershell.exe', '-NoProfile', '-STA', '-ExecutionPolicy', 'Bypass',
        '-File', str(HERE / 'probe-native.ps1'), '-Session', str(experiment.session),
        '-Worker', str(experiment.worker), '-Output', str(experiment.directory / 'native.json')
    ]
    if options.optimize:
        command.append('-Optimize')
    if options.hidden_optimize:
        command.append('-HiddenOptimize')
    if options.click_optimize:
        command.append('-ClickOptimize')
    subprocess.run(command, stdout=session.stdout, stderr=session.stderr,
        timeout=90, check=True, creationflags=subprocess.CREATE_NO_WINDOW)
    native = experiment.helper.read_json(experiment.directory / 'native.json') or {}
    print('Native probe:', native.get('error') or native.get('optimization'), flush=True)


def verify_saved_workbook(experiment):
    before = experiment.directory / 'before-optimization.xlsx'
    after = experiment.directory / 'after-optimization.xlsx'
    if not before.exists() or not after.exists():
        return
    with zipfile.ZipFile(before) as archive:
        original_xml = archive.read('xl/worksheets/sheet1.xml')
    with zipfile.ZipFile(after) as archive:
        optimized_xml = archive.read('xl/worksheets/sheet1.xml')
    cell_tag = '{http://schemas.openxmlformats.org/spreadsheetml/2006/main}c'
    experiment.helper.publish(experiment.directory / 'xml-comparison.json', {
        'worksheet_xml_identical': original_xml == optimized_xml,
        'z10000_before': any(cell.get('r') == 'Z10000'
            for cell in ET.fromstring(original_xml).iter(cell_tag)),
        'z10000_after': any(cell.get('r') == 'Z10000'
            for cell in ET.fromstring(optimized_xml).iter(cell_tag))})


def record_webview_origins(experiment):
    # URLs may contain tokens. Preserve only titles and origins for panel classification.
    experiment.helper.publish(experiment.directory / 'webviews.json', [
        {'title': page.get('title'),
         'origin': urlsplit(page.get('url', ''))._replace(path='', query='', fragment='').geturl()}
        for page in targets(experiment.port)])


def close_experiment(experiment, session):
    try:
        experiment.helper.publish(experiment.session / 'finish.json', {'complete': True})
        try:
            session.process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            subprocess.run(['taskkill', '/F', '/T', '/PID', str(session.process.pid)],
                timeout=15, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    finally:
        if session.debugger:
            session.debugger.terminate()
            session.debugger.wait(timeout=10)


def main():
    options = parse_options()
    experiment = prepare_experiment(options)
    with open_excel(experiment, options.binary) as session:
        inspect_officejs_runtime(experiment, session)
        if options.native:
            probe_native_optimization(experiment, session, options)
            verify_saved_workbook(experiment)
        record_webview_origins(experiment)
        print('Results:', experiment.directory, flush=True)


if __name__ == '__main__':
    main()
