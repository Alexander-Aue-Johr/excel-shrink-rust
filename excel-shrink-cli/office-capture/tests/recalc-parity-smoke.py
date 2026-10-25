"""Opt-in comparison of COM and Office.js saved recalculation values.

The JavaScript hook lives only in a private copy of the web assets. It uses a
release command as test transport; production workers never save capture inputs.
Requires Windows, Excel, openpyxl. Outputs and receipts remain under .tmp.
"""
import argparse
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import time
from zipfile import ZipFile, ZIP_DEFLATED
import xml.etree.ElementTree as ET

from openpyxl import Workbook, load_workbook
from openpyxl.chart import LineChart, Reference
from openpyxl.formatting.rule import CellIsRule
from openpyxl.styles import PatternFill
from openpyxl.workbook.properties import CalcProperties

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('peer', Path(__file__).with_name('cleanup-peer-smoke.py'))
peer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(peer)
PS = ['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File']


def fixture(path, mode):
    book = Workbook()
    data = book.active
    data.title = 'Data'
    data.append(['Input', 'Formula', 'Volatile'])
    data.append([21, '=A2*2', '=RAND()'])
    data.append([7, '=SUM(A2:A3)', '=NOW()'])
    data.append([3, '=IF(A4>0,"positive","negative")'])
    hidden = book.create_sheet('Hidden')
    hidden['A1'] = 11
    hidden.sheet_state = 'hidden'
    data['B5'] = '=Hidden!A1*2'
    other = book.create_sheet('Dependent')
    other['A1'] = '=Data!B2+Data!B5'
    other['A2'] = '=1/0'
    data.conditional_formatting.add('A2:A4', CellIsRule(operator='greaterThan', formula=['10'],
        fill=PatternFill('solid', fgColor='FF0000')))
    chart = LineChart()
    chart.add_data(Reference(data, min_col=2, min_row=2, max_row=3))
    data.add_chart(chart, 'E2')
    book.calculation = CalcProperties(calcMode=mode, fullCalcOnLoad=False, forceFullCalc=False)
    book.save(path)
    # Deliberately stale caches expose whether native opening itself recalculates.
    with ZipFile(path) as archive:
        entries = {i.filename: archive.read(i) for i in archive.infolist()}
    ns = {'s': 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'}
    for name, value in list(entries.items()):
        if name.startswith('xl/worksheets/sheet') and name.endswith('.xml'):
            xml = ET.fromstring(value)
            for cell in xml.findall('.//s:c', ns):
                if cell.find('s:f', ns) is not None:
                    cell.find('s:v', ns).text = '-999'
            entries[name] = ET.tostring(xml, encoding='utf-8')
    with ZipFile(path, 'w', ZIP_DEFLATED) as archive:
        for name, value in entries.items():
            archive.writestr(name, value)


def values(path):
    book = load_workbook(path, read_only=True, data_only=True)
    try:
        return {f'{sheet.title}!{cell.coordinate}': cell.value for sheet in book
                for row in sheet for cell in row if cell.value is not None}
    finally:
        book.close()


def staged_launcher(root):
    capture = ROOT / 'excel-shrink-cli/office-capture'
    automation = root / 'com-runtime'
    automation.mkdir()
    for name in ('excel_recalculate.ps1', 'archive_processes.ps1', 'ArchiveProcesses.cs'):
        shutil.copyfile(ROOT / 'excel-shrink-cli/src/regression' / name, automation / name)
    shutil.copyfile(capture / 'CaptureProcesses.cs', automation / 'CaptureProcesses.cs')
    web = root / 'web'
    shutil.copytree(capture / 'web', web)
    script = web / 'resident.js'
    source = script.read_text(encoding='utf-8')
    marker = "else if (command.kind === 'release') {"
    assert source.count(marker) == 1
    hook = """else if (command.kind === 'release' && command.diagnostic_recalc) {
        result = await office(async context => {
          const before = context.workbook.worksheets.getItem('Data').getRange('B2:C5');
          before.load('values');
          context.application.load('calculationMode');
          await context.sync();
          const beforeValues = before.values;
          const mode = context.application.calculationMode;
          context.application.calculate('FullRebuild');
          await context.sync();
          context.workbook.save('Save');
          await context.sync();
          return {diagnostic: 'Office.js FullRebuild + Save', before_values: beforeValues,
            calculation_mode: mode};
        });
      } """
    script.write_text(source.replace(marker, hook + marker), encoding='utf-8')
    launcher = (capture / 'resident.ps1').read_text(encoding='utf-8-sig')
    marker = "Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'web') -Destination $webStage -Recurse"
    assert launcher.count(marker) == 1
    launcher = launcher.replace(marker,
        "Copy-Item -LiteralPath '" + str(web).replace("'", "''") + "' -Destination $webStage -Recurse")
    # Keep the literal searched by the launcher's helper-function AST import.
    launcher = launcher.replace("'$PSScriptRoot'", "'__PROBE_ROOT_LITERAL__'")
    launcher = launcher.replace('$PSScriptRoot', '$probeSourceRoot')
    launcher = launcher.replace("'__PROBE_ROOT_LITERAL__'", "'$PSScriptRoot'")
    launcher = launcher.replace("$ErrorActionPreference = 'Stop'",
        "$probeSourceRoot = '" + str(capture).replace("'", "''") + "'\n$ErrorActionPreference = 'Stop'", 1)
    path = root / 'probe-launcher.ps1'
    path.write_text(launcher, encoding='utf-8-sig')
    return path


def run_case(root, launcher, mode):
    folder = root / mode
    folder.mkdir()
    source = folder / 'input.xlsx'
    fixture(source, mode)
    com = folder / 'com.xlsx'
    with (folder / 'com.stdout.log').open('w') as stdout, (folder / 'com.stderr.log').open('w') as stderr:
        subprocess.run(PS + [str(root / 'com-runtime/excel_recalculate.ps1'),
            '-InputPath', str(source), '-OutputPath', str(com), '-ProgressPath', str(folder / 'com.log')],
            check=True, timeout=120, creationflags=0x08000000, stdout=stdout, stderr=stderr)
    session = folder / 'session'
    peer.publish(session / 'clients/probe.json', dict(client_id='probe', source_path=str(source),
        workbook='probe.xlsx', sheets=[dict(index=1, name='Data', bottom_row=5,
            base_right_column=3, right_column=3, horizontal_overflow_required=False,
            drawing_fallback_required=False)]))
    peer.publish(session / 'commands/probe/0001.json', dict(id='0001', kind='release', diagnostic_recalc=True))
    with (folder / 'stdout.log').open('w') as stdout, (folder / 'stderr.log').open('w') as stderr:
        process = subprocess.Popen(PS + [str(launcher), '-SessionPath', str(session),
            '-StartupTimeoutSeconds', '90', '-CaptureTimeoutSeconds', '120', '-HideExcel'],
            stdout=stdout, stderr=stderr, creationflags=0x08000000)
        try:
            response = peer.response(session / 'responses/probe/0001.json', process)
            stage = Path(peer.read(session / 'launcher.json')['stage'])
            native = folder / 'native.xlsx'
            shutil.copyfile(stage / 'probe.xlsx', native)
            left, right = values(com), values(native)
            volatile = {'Data!C2', 'Data!C3'}
            different = []
            for key in sorted(left.keys() | right.keys()):
                if key in volatile:
                    continue
                a, b = left.get(key), right.get(key)
                equal = math.isclose(a, b, rel_tol=1e-12, abs_tol=1e-12) if isinstance(a, (int, float)) and isinstance(b, (int, float)) else a == b
                if not equal:
                    different.append(dict(cell=key, com=str(a), native=str(b)))
            result = dict(mode=mode, different=different, values=len(left),
                volatile_excluded=sorted(volatile), response=response,
                ready_metrics=peer.read(session / 'metrics.json'))
            assert not different, different
            return result
        finally:
            peer.publish(session / 'finish.json', {'complete': True})
            try:
                process.wait(timeout=40)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', action='store_true')
    args = parser.parse_args()
    if not args.run:
        parser.error('Pass --run to open disposable native and COM Excel instances')
    root = ROOT / '.tmp/recalc-parity' / str(time.time_ns())
    root.mkdir(parents=True)
    receipt = dict(passed=False, cases=[])
    try:
        launcher = staged_launcher(root)
        for mode in ('manual', 'auto'):
            receipt['cases'].append(run_case(root, launcher, mode))
            print(mode, 'cached values agree', flush=True)
        receipt['passed'] = True
    except Exception as error:
        receipt['error'] = repr(error)
        print(repr(error), flush=True)
    finally:
        peer.publish(root / 'receipt.json', receipt)
        print('Receipt:', root / 'receipt.json', flush=True)
    return 0 if receipt['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
