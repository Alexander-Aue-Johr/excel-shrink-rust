"""Refresh Default outputs while an independent Excel preparation run progresses."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import time
import hashlib
import zipfile
from datetime import datetime, timezone


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path, value):
    temporary = path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n', encoding='utf8')
    temporary.replace(path)


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    binary = args.output / 'excel-shrink-cli.exe'
    shutil.copy2(args.binary, binary)
    identity = {'binary_sha256': sha256(binary), 'source_commit': args.commit,
                'comparison_run': str(args.run), 'profile': 'default',
                'input': 'raw prepared Original XLSX', 'visual_validation': 'pending'}
    write_json(args.output / 'provenance.json', identity)
    spec = importlib.util.spec_from_file_location('sizes', Path(__file__).with_name('analyze-prepared-sizes.py'))
    sizes = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sizes)
    completed, processed, failed = [], set(), set()
    defaults = args.output / 'default'
    while True:
        changed = False
        for record in sorted((args.run / 'results').glob('*.json')):
            result = json.loads(record.read_bytes())
            case = result['id']
            if result['status'] != 'prepared' or case in processed:
                continue
            destination = defaults / case
            destination.mkdir(parents=True)
            original = Path(result['attempt']) / 'inputs/original.xlsx'
            started = time.monotonic()
            evidence = {'case_id': case, 'original_path': str(original),
                        'original_sha256': sha256(original), **identity}
            try:
                if evidence['original_sha256'] != result['original_sha256']:
                    raise ValueError('Original XLSX identity differs from preparation record')
                with (destination / 'shrink.log').open('w', encoding='utf8') as log:
                    subprocess.run([str(binary.resolve()), str(original), str(destination),
                                    '--rayon-threads', '2', '--batch-report', str(destination / 'batch.json')],
                                   stdout=log, stderr=subprocess.STDOUT, timeout=120, check=True)
                output = destination / 'original.xlsx'
                with zipfile.ZipFile(output) as archive:
                    archive.infolist()
                evidence.update(status='completed', output_sha256=sha256(output), output_bytes=output.stat().st_size)
                completed.append(record)
            except (OSError, ValueError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
                evidence.update(status='failed', error=str(error))
                failed.add(case)
            processed.add(case)
            evidence['elapsed_seconds'] = time.monotonic() - started
            write_json(destination / 'result.json', evidence)
            changed = True
        if changed:
            sizes.compare_controls(args.run, args.output / 'comparison', defaults, completed)
        source_state = json.loads((args.run / 'status.json').read_bytes()).get('state')
        write_json(args.output / 'status.json', {'state': 'following' if args.follow and source_state in ('running', 'waiting_for_resources') else 'completed',
                   'completed': len(completed), 'failed': len(failed), 'source_state': source_state,
                   'updated_utc': datetime.now(timezone.utc).isoformat()})
        if not args.follow or source_state not in ('running', 'waiting_for_resources'):
            break
        time.sleep(60)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--follow', action='store_true')
    run(parser.parse_args())
