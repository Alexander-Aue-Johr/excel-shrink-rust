"""Measure native Office.js capture packages without increasing individual image limits."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path(__file__).resolve().parents[4]
spec = importlib.util.spec_from_file_location(
    'resident_probe', ROOT / 'docs/visual-validation/experiments/officejs-million-row-heights/run.py')
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--workbook', required=True, type=Path)
    parser.add_argument('--sheet', default='2020 E Cotton Lys 15-Min')
    parser.add_argument('--first-row', type=int, default=32816)
    parser.add_argument('--rows', type=int, default=138)
    parser.add_argument('--right-column', default='BU')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    with zipfile.ZipFile(args.workbook) as archive:
        workbook = ET.fromstring(archive.read('xl/workbook.xml'))
    sheets = list(workbook.find('{*}sheets'))
    index = next(i + 1 for i, sheet in enumerate(sheets) if sheet.attrib['name'] == args.sheet)
    session = output / 'session'
    control = output / 'control'
    helper.publish(control / 'control.json', {'paused': False, 'hide_excel': True})
    helper.publish(session / 'capacity.json', {'reserved_clients': 1})
    helper.publish(session / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(args.workbook.resolve()), 'workbook': 'one.xlsx',
        'sheets': [{'index': index, 'name': args.sheet, 'bottom_row': 114053,
                    'base_right_column': 72, 'right_column': 72,
                    'horizontal_overflow_required': False, 'drawing_fallback_required': False}]})
    environment = dict(os.environ, EXCEL_SHRINK_CONTROL_ROOM_DIR=str(control),
        EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID='capture-package-probe',
        EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID=output.name, EXCEL_SHRINK_SQLITE_METADATA='1')
    with (output / 'stdout.log').open('wb') as stdout, (output / 'stderr.log').open('wb') as stderr:
        process = subprocess.Popen([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            str(ROOT / 'excel-shrink-cli/office-capture/resident.ps1'), '-SessionPath', str(session),
            '-HostExecutable', str(args.binary.resolve()), '-StartupTimeoutSeconds', '180',
            '-CaptureTimeoutSeconds', '180', '-HideExcel'], stdout=stdout, stderr=stderr,
            env=environment, creationflags=subprocess.CREATE_NO_WINDOW)
    sequence = 0

    def command(request):
        nonlocal sequence
        sequence += 1
        identity = f'package-{sequence}'
        helper.publish(session / f'commands/one/{identity}.json', {
            'id': identity, 'kind': 'diagnosis', 'operation': 'targeted', 'request': request})
        result = helper.wait_until(
            lambda: helper.read_json(session / f'responses/one/{identity}.json'), process, 180)
        if result.get('diagnostic_error') or result.get('error') or result.get('ok') is False:
            raise RuntimeError(json.dumps(result))
        return result

    results = []
    try:
        helper.wait_until(lambda: (helper.read_json(session / 'metrics.json') or {}).get(
            'primary_clients_ready'), process, 180)
        ranges = [f'A{args.first_row + i * args.rows}:{args.right_column}'
                  f'{args.first_row + (i + 1) * args.rows - 1}' for i in range(8)]
        geometry = command({'kind': 'measure_batch', 'sheet': args.sheet, 'ranges': ranges})['ranges']
        estimated = [int((g['width'] * 2 + 2) * (g['height'] * 2 + 2)) for g in geometry]
        if max(estimated) > 32000000:
            raise RuntimeError(f'Individual images exceed the unchanged budget: {estimated}')
        command({'kind': 'capture_many', 'sheet': args.sheet, 'ranges': [{'range': ranges[0]}]})
        reference = None
        from PIL import Image
        for budget in [32, 64, 128, 256, 256, 128, 64, 32]:
            peaks = {'sampled_job_bytes': 0}
            stop = threading.Event()

            def sample():
                while not stop.wait(.25):
                    metrics = helper.read_json(session / 'metrics.json') or {}
                    peaks['sampled_job_bytes'] = max(peaks['sampled_job_bytes'],
                                                     metrics.get('current_job_bytes', 0))

            sampler = threading.Thread(target=sample, daemon=True)
            sampler.start()
            observed = []
            packages = []
            started = time.monotonic()
            try:
                offset = 0
                while offset < len(ranges):
                    end = offset + 1
                    while end < len(ranges) and sum(estimated[offset:end + 1]) <= budget * 1000000:
                        end += 1
                    package = ranges[offset:end]
                    response = command({'kind': 'capture_many', 'sheet': args.sheet,
                        'batch_megapixels': budget, 'ranges': [{'range': r} for r in package]})
                    if [image['range'] for image in response['images']] != package:
                        raise RuntimeError('Capture order changed')
                    packages.append(len(package))
                    for item in response['images']:
                        with Image.open(item['image_path']) as image:
                            observed.append({'range': item['range'], 'size': image.size,
                                'rgba_sha256': hashlib.sha256(image.convert('RGBA').tobytes()).hexdigest()})
                    offset = end
            finally:
                stop.set()
                sampler.join()
            if reference is None:
                reference = observed
            if observed != reference:
                raise RuntimeError('Decoded pixels or image dimensions changed')
            result = {'budget_mp': budget, 'packages': packages,
                'wall_seconds_including_rgba_verification': time.monotonic() - started,
                'all_pixels_equal': True, **peaks}
            results.append(result)
            helper.publish(output / 'results.json', {'source': str(args.workbook), 'sheet': args.sheet,
                'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                'individual_estimated_pixels': estimated, 'images': reference, 'results': results})
            print(json.dumps(result), flush=True)
    finally:
        helper.close_client(session, process)


if __name__ == '__main__':
    main()
