"""Reproduce Office.js startup with a stale or consistent worksheet selection."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
from zipfile import ZipFile, ZIP_DEFLATED

from pinned_runtime import RUNTIME_REF, capture_plan, export_runtime, windows_path
from run import patch_desktop_launcher, publish_signal

MAIN = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
RELS = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'


def write_fixture(path, consistent, sheet_count=2, target=2):
    """Uncompressed XML differs only in the two tabSelected attributes."""
    with ZipFile(path, 'w', ZIP_DEFLATED) as archive:
        content_types = '''<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>'''
        extra = ''.join(f'<Override PartName="/xl/worksheets/sheet{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' for i in range(3, sheet_count + 1))
        archive.writestr('[Content_Types].xml', content_types.replace('</Types>', extra + '</Types>'))
        archive.writestr('_rels/.rels', f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="book" Type="{RELS}/officeDocument" Target="xl/workbook.xml"/></Relationships>')
        names = ['First', 'Second'] + [f'Sheet{i}' for i in range(3, sheet_count + 1)]
        sheets = ''.join(f'<sheet name="{name}" sheetId="{i}" r:id="sheet{i}"/>' for i, name in enumerate(names, 1))
        archive.writestr('xl/workbook.xml', f'<workbook xmlns="{MAIN}" xmlns:r="{RELS}"><bookViews><workbookView activeTab="{target - 1}"/></bookViews><sheets>{sheets}</sheets></workbook>')
        relationships = ''.join(f'<Relationship Id="sheet{i}" Type="{RELS}/worksheet" Target="worksheets/sheet{i}.xml"/>' for i in range(1, sheet_count + 1))
        archive.writestr('xl/_rels/workbook.xml.rels', f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{relationships}</Relationships>')
        for index in range(1, sheet_count + 1):
            selected = index == (target if consistent else 1)
            archive.writestr(f'xl/worksheets/sheet{index}.xml', f'<worksheet xmlns="{MAIN}"><sheetViews><sheetView workbookViewId="0" tabSelected="{int(selected)}"/></sheetViews><sheetData><row r="1"><c r="A1"><v>123</v></c><c r="B1" t="inlineStr"><is><t>Probe</t></is></c></row></sheetData></worksheet>')


def probe(output, runtime, launcher, consistent, timeout, workbook=None, scope=None):
    output.mkdir()
    fixture = output / 'fixture.xlsx'
    if workbook:
        shutil.copyfile(workbook, fixture)
    else:
        write_fixture(fixture, consistent)
    capture = capture_plan(fixture, [scope or 'Second!A1:B3'])
    # The other sides are never opened, but still need a valid target index.
    placeholder = output / 'unused-sides.xlsx'
    target = capture['sheets'][0]['index']
    write_fixture(placeholder, True, max(2, target), target)
    signals = {name: output / filename for name, filename in {
        'stage_ready_signal': 'stage-ready.signal', 'launch_signal': 'launch.signal',
        'ready_signal': 'ready.signal', 'measurement_signal': 'measurement.signal',
        'start_signal': 'start.signal', 'release_signal': 'release.signal'}.items()}
    request = {
        'workbook': 'fixture.xlsx', 'capture': capture,
        'transport': 'hash', 'batch_size': 1, 'max_batch_pixels': 64_000_000,
        **{name: windows_path(path) for name, path in signals.items()},
        'original': windows_path(fixture),
        **{side: windows_path(placeholder) for side in ('default', 'maximum', 'too-aggressive')},
    }
    request_path = output / 'request.json'
    request_path.write_text(json.dumps(request, indent=2), encoding='utf-8')
    # Open only Original. Once Office.js has announced itself, abort before the
    # second Excel instance. A phase-0.ready file is the startup success evidence.
    for name, content in [('launch.signal', 'go\n'), ('phase-0.launch', 'go\n'),
                          ('phase-0.signal', 'go\n'), ('phase-1.launch', 'abort\n')]:
        publish_signal(output / name, content)
    command = ['powershell.exe', '-STA', '-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass',
               '-File', windows_path(launcher), '-RequestPath', windows_path(request_path),
               '-RuntimeRoot', windows_path(runtime), '-OutputDir', windows_path(output),
               '-StartupTimeoutSeconds', str(timeout), '-CaptureTimeoutSeconds', '60',
               '-MemoryMiB', '4096', '-Minimized']
    with (output / 'launcher.log').open('wb') as log:
        process = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    result = json.loads((output / 'result.json').read_text(encoding='utf-8-sig'))
    return {
        'consistent_selection': consistent, 'office_js_started': (output / 'phase-0.ready').exists(),
        'launcher_exit': process.returncode, 'launcher_error': result.get('error'),
        'total_seconds': result.get('total_seconds'),
        'fixture_sha256': hashlib.sha256(fixture.read_bytes()).hexdigest(),
        'runtime_helpers_sha256': {name: hashlib.sha256((runtime / name).read_bytes()).hexdigest()
                                  for name in ('run.ps1', 'CaptureWorkbook.cs')},
        'launcher_sha256': hashlib.sha256(launcher.read_bytes()).hexdigest(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--runtime', type=Path, help='Use an already exported runtime instead of exporting one')
    parser.add_argument('--runtime-ref', help='Export this Git revision; defaults to the reviewed runtime pin')
    parser.add_argument('--repo-root', type=Path, default=Path.cwd())
    parser.add_argument('--launcher', type=Path, default=Path(__file__).with_name('run.ps1'))
    parser.add_argument('--startup-timeout', type=int, default=20)
    parser.add_argument('--workbook', type=Path, help='Probe one real workbook instead of the tiny A/B pair')
    parser.add_argument('--scope', help='Required for a real workbook, e.g. Sheet1!A1:G400')
    args = parser.parse_args()
    if bool(args.workbook) != bool(args.scope):
        parser.error('--workbook and --scope are required together')
    if args.runtime and args.runtime_ref:
        parser.error('Choose --runtime or --runtime-ref')
    args.output.mkdir(parents=True)
    if not args.runtime:
        args.runtime_ref = args.runtime_ref or RUNTIME_REF
        args.runtime = export_runtime(args.repo_root.resolve(), args.runtime_ref, args.output / 'runtime')
        patch_desktop_launcher(args.runtime)
        from web.patch import patch_runtime
        patch_runtime(args.runtime)
    observations = []
    cases = [('real-workbook', None)] if args.workbook else [('stale-selection', False), ('consistent-selection', True)]
    for label, consistent in cases:
        observation = probe(args.output / label, args.runtime, args.launcher, consistent, args.startup_timeout, args.workbook, args.scope)
        observation['runtime_commit'] = args.runtime_ref
        observations.append(observation)
        (args.output / 'observations.json').write_text(json.dumps(observations, indent=2), encoding='utf-8')
        print(label, observation, flush=True)


if __name__ == '__main__':
    main()
