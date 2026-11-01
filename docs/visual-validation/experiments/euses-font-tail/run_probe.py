"""Inspect geometry and bounded Office.js images on disposable private-desktop copies."""
import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


def load_resident_helpers():
    path = HERE.parent / 'check-performance-trigger/run.py'
    spec = importlib.util.spec_from_file_location('font_tail_resident', path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def inspect_workbook(resident, helper, worker, directory, item):
    directory.mkdir(parents=True)
    workbook = Path(item['path'])
    session_path = directory / 'session'
    helper.publish(session_path / 'capacity.json', {'reserved_clients': 1})
    helper.publish(session_path / 'clients/one.json', {
        'client_id': 'one', 'source_path': str(workbook.resolve()), 'workbook': 'one.xlsx',
        'sheets': [{'index': 1, 'name': 'Sheet1', 'bottom_row': 65445, 'base_right_column': 5,
                    'right_column': 5, 'horizontal_overflow_required': False,
                    'drawing_fallback_required': False}]})
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith('EXCEL_SHRINK_')}
    environment['EXCEL_SHRINK_EXPERIMENT_DESKTOP'] = os.environ['EXCEL_SHRINK_EXPERIMENT_DESKTOP']
    environment['EXCEL_SHRINK_SYSTEM_RESERVE_MIB'] = '8192'
    experiment = resident.Experiment(directory, worker, session_path, helper, 0, environment)
    with resident.open_excel(experiment, ROOT / 'target/release/excel-shrink-cli.exe') as session:
        command = {
            'id': 'probe', 'kind': 'diagnosis', 'operation': 'inspect_font_tail',
            'ranges': item['ranges'],
        }
        for key in ('sheet', 'rows', 'tail'):
            if key in item:
                command[key] = item[key]
        helper.publish(session_path / 'commands/one/probe.json', command)
        response = helper.wait_until(lambda: helper.read_json(session_path / 'responses/one/probe.json'),
                                     session.process, 120)
        if not response.get('ok'):
            raise RuntimeError(f'Font-tail probe failed: {response}')
        for index, capture in enumerate(response['captures']):
            image = base64.b64decode(capture.pop('image'))
            path = directory / f'range-{index}.png'
            path.write_bytes(image)
            capture['image_path'] = str(path.resolve())
        helper.publish(directory / 'result.json', response)
        return response


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--desktop', required=True)
    options = parser.parse_args()
    os.environ['EXCEL_SHRINK_EXPERIMENT_DESKTOP'] = options.desktop
    directory = options.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    resident = load_resident_helpers()
    helper = resident.load_helpers()
    worker = resident.prepare_worker(directory)
    source_path = worker / 'web/resident-diagnosis.js'
    source = source_path.read_text(encoding='utf-8')
    anchor = '  async function execute(command, {office}) {'
    if source.count(anchor) != 1:
        raise RuntimeError('The disposable worker no longer has the expected diagnosis entry point.')
    extension = (HERE / 'probe.js').read_text(encoding='utf-8')
    source = source.replace(anchor, extension + '\n' + anchor + '\n'
        "    if (command.operation === 'inspect_font_tail') return inspectFontTail(command, office);\n")
    source_path.write_text(source, encoding='utf-8')
    results = []
    for item in json.loads(options.plan.read_text(encoding='utf-8')):
        helper.publish(directory / 'progress.json', {'workbook': item['name'], 'completed': len(results)})
        result = inspect_workbook(resident, helper, worker, directory / item['name'], item)
        results.append({'name': item['name'], **result})
        helper.publish(directory / 'results.json', results)
    helper.publish(directory / 'progress.json', {'complete': True, 'completed': len(results)})


if __name__ == '__main__':
    main()
