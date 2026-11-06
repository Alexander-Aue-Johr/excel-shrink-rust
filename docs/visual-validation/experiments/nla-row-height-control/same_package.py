"""Measure two byte-identical workbook copies through the production Excel host."""
import argparse
import hashlib
import json
import os
import queue
import shutil
import subprocess
import threading
import traceback
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path


class Host:
    """Keep one bounded request outstanding and retain responses and process events."""

    def __init__(self, repository, output):
        self.output = output
        self.responses = queue.Queue()
        self.stderr = (output / 'host.stderr.log').open('w', encoding='utf-8')
        self.process = subprocess.Popen(
            ['powershell.exe', '-NoProfile', '-STA', '-ExecutionPolicy', 'Bypass',
             '-File', str(repository / 'excel-shrink-cli/office-capture/targeted-comparison-host.ps1')],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
            text=True, encoding='utf-8', bufsize=1,
        )
        threading.Thread(target=self.read_responses, daemon=True).start()

    def read_responses(self):
        """Preserve the full transport stream while separating events from replies."""
        with (self.output / 'host.responses.jsonl').open('w', encoding='utf-8') as log:
            for line in self.process.stdout:
                log.write(line)
                log.flush()
                try:
                    response = json.loads(line)
                    if 'ok' in response:
                        self.responses.put(response)
                except ValueError as error:
                    self.responses.put({'ok': False, 'error': str(error)})

    def call(self, command, timeout_seconds=180):
        """Require an explicit successful reply within the operation deadline."""
        self.process.stdin.write(json.dumps(command) + '\n')
        self.process.stdin.flush()
        response = self.responses.get(timeout=timeout_seconds)
        if response.get('ok') is not True:
            raise RuntimeError(response)
        return response['result']

    def close(self):
        """Allow host cleanup, with the outer private-desktop job as the final guard."""
        self.process.stdin.close()
        try:
            self.process.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=10)
        self.stderr.close()


def sheet_inventory(source):
    """Read sheet names without modifying the workbook or inferring capture coverage."""
    namespace = '{http://schemas.openxmlformats.org/spreadsheetml/2006/main}'
    with zipfile.ZipFile(source) as archive:
        workbook = ET.fromstring(archive.read('xl/workbook.xml'))
    return [
        {'index': index, 'name': sheet.attrib['name'], 'bottom_row': 179,
         'base_right_column': 43, 'right_column': 43,
         'horizontal_overflow_required': False, 'drawing_fallback_required': False}
        for index, sheet in enumerate(workbook.find(namespace + 'sheets'), 1)
    ]


def observe_pair(host, arguments):
    """Measure paired copies before save, after save and after full rebuild."""
    digest = hashlib.sha256(arguments.original.read_bytes()).hexdigest()
    jobs = []
    input_hashes = {}
    for lane in ('O', 'SD'):
        path = arguments.output / f'{lane}.xlsx'
        source = arguments.default if lane == 'SD' and arguments.default else arguments.original
        shutil.copyfile(source, path)
        input_hashes[lane] = hashlib.sha256(path.read_bytes()).hexdigest()
        assert input_hashes[lane] == hashlib.sha256(source.read_bytes()).hexdigest()
        jobs.append({'kind': 'open', 'lane': lane, 'path': str(path),
                     'sheets': sheet_inventory(path), 'host_executable': str(arguments.binary),
                     'timeout_seconds': 180, 'manual_calculation': True,
                     'external_link_parts': 0})
    result = {'source_sha256': digest, 'input_sha256': input_hashes,
              'measurements': [], 'closed': {}}
    try:
        result['opened'] = host.call({'kind': 'open_batch', 'jobs': jobs})
        for phase, operation in [('open', None), ('save', 'save'), ('rebuild', 'recalculate')]:
            for lane in ('O', 'SD'):
                if operation:
                    host.call({'kind': operation, 'lane': lane, 'timeout_seconds': 180,
                               'linked_pending_timeout_ms': 15000})
                heights = host.call({'kind': 'heights', 'lane': lane,
                                     'sheet': arguments.sheet,
                                     'spans': [{'start': arguments.start_row, 'end': arguments.end_row}],
                                     'budget_ms': 180000})
                if heights.get('complete') is not True:
                    raise RuntimeError('Incomplete height coverage')
                geometry = host.call({'kind': 'measure', 'lane': lane,
                                      'sheet': arguments.sheet,
                                      'range': f'A{arguments.start_row}:AR{arguments.end_row}'})
                result['measurements'].append({'phase': phase, 'lane': lane,
                                               'heights': heights, 'geometry': geometry})
    finally:
        for lane in ('O', 'SD'):
            try:
                result['closed'][lane] = host.call({'kind': 'close', 'lane': lane}, 30)
            except Exception as error:
                result['closed'][lane] = {'error': str(error)}
        (arguments.output / 'result.json').write_text(json.dumps(result, indent=2), encoding='utf-8')


def main():
    """Run only inside the owned private desktop supplied by the companion launcher."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--original', type=Path, required=True)
    parser.add_argument('--default', type=Path, help='Optional counterfactual; omit for identical inputs')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sheet', default='2007 NLA QA Data')
    parser.add_argument('--start-row', type=int, default=6)
    parser.add_argument('--end-row', type=int, default=179)
    arguments = parser.parse_args()
    if not 1 <= arguments.start_row <= arguments.end_row <= 1048576:
        parser.error('Row bounds must satisfy 1 <= start <= end <= 1048576')
    desktop = os.environ.get('EXCEL_SHRINK_EXPECTED_DESKTOP')
    if not desktop or desktop.lower() == 'default':
        raise RuntimeError('Use the owned private-desktop launcher')
    arguments.output.mkdir(parents=True, exist_ok=False)
    host = Host(arguments.repository, arguments.output)
    try:
        observe_pair(host, arguments)
    except Exception:
        (arguments.output / 'failure.txt').write_text(traceback.format_exc(), encoding='utf-8')
        raise
    finally:
        host.close()


if __name__ == '__main__':
    main()
