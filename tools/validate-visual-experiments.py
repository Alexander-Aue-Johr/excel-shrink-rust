#!/usr/bin/env python3
"""Portable code validation and a read-only Git membership audit; never starts Excel."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from repository_policy import audit


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--audit-only', action='store_true', help='Inspect Git membership without running tests')
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[1]
    exp = root / 'docs/visual-validation/experiments'
    if not args.audit_only:
        commands = [
            [sys.executable, '-m', 'unittest', 'discover', '-s', str(exp/'tests'), '-p', 'test_*.py'],
            [sys.executable, '-m', 'unittest', 'discover', '-s', str(exp/'range-image-calibration/tests'), '-p', 'test_*.py'],
            ['node', '--test'] + [str(p) for folder in (exp/'tests', exp/'range-image-calibration/tests')
                                 for p in sorted(folder.glob('*.test.cjs'))],
        ]
        for command in commands:
            try:
                process = subprocess.run(command, cwd=root, check=False)
            except FileNotFoundError as exc:
                print(str(exc), file=sys.stderr)
                return 1
            if process.returncode:
                return process.returncode
        print('Portable checks passed. Windows/Excel execution is a separate check.')
    result = audit(root)
    directory = root / '.tmp/visual-validation'
    directory.mkdir(parents=True, exist_ok=True)
    destination = directory / 'repository-audit.json'
    descriptor, name = tempfile.mkstemp(prefix='repository-audit-', suffix='.new', dir=directory)
    try:
        with os.fdopen(descriptor, 'w', encoding='utf-8') as out:
            json.dump(result, out, ensure_ascii=True, indent=2)
            out.write('\n')
        os.replace(name, destination)
    finally:
        if os.path.exists(name):
            os.unlink(name)
    print('REPOSITORY AUDIT:', result['status'])
    print('AUDIT REPORT:', destination)
    if result.get('reason'):
        print(result['reason'])
    for row in result['findings'][:12]:
        print('  ' + row['code'] + ': ' + row['path'])
    if len(result['findings']) > 12:
        print('  Further findings are listed in the audit report.')
    # An export without .git cannot be certified. Keep portable unit validation separate.
    if args.audit_only:
        return 0 if result['status'] == 'passed' else 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
