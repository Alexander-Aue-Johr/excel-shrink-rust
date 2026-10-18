"""Validated review bundles. Full local evidence is never removed by packaging."""
from __future__ import annotations

import hashlib
import html
import json
import os
import zipfile
from pathlib import Path

from common import atomic_json, digest, safe_walk, utc

TEXT_SUFFIXES = {'.json', '.jsonl', '.txt', '.log', '.csv', '.md', '.html', '.css', '.js', '.xml'}
REQUIRED = ['run.json', 'report.html', 'evaluation-state.json', 'resolved-plan.json',
            'source-manifest.json', 'case-results.csv', 'summary.json']
LOG_LIMIT = 96 * 1024


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _case_view(job: dict) -> bytes:
    """A labelled review projection, not a replacement of the full local inspector."""
    e = lambda x: html.escape(str(x), quote=True)
    metrics = job.get('metrics') or {}
    rows = []
    for workbook in metrics.get('workbooks') or []:
        for sheet in workbook.get('sheets') or []:
            error = sheet.get('error') or {}
            error = error if isinstance(error, dict) else {}
            rows.append('<tr>' + ''.join('<td>' + e(x) + '</td>' for x in (
                sheet.get('sheet_index'), json.dumps(sheet.get('sheet'), ensure_ascii=False),
                sheet.get('address') or '—', sheet.get('status'),
                error.get('code') or sheet.get('warning') or '')) + '</tr>')
    text = ('<!doctype html><html lang="en"><meta charset="utf-8">'
            '<meta name="viewport" content="width=device-width,initial-scale=1">'
            '<title>Case metadata</title><style>body{max-width:1100px;margin:30px auto;'
            'padding:0 20px;font:15px/1.6 system-ui;color:#20364a}table{width:100%;'
            'border-collapse:collapse}td,th{text-align:left;padding:8px;border-bottom:1px solid #dae2e9}'
            'aside{padding:15px;background:#f2f6fa}code{overflow-wrap:anywhere}</style>'
            '<h1>' + e(job['id']) + '</h1><aside><strong>Metadata review view.</strong> '
            'The complete local workbook inspector, original images, full plans and logs remain '
            'in the run directory. This archive uses an explicit compact projection; see '
            '<a href="../../bundle-manifest.json">the bundle manifest</a> for omissions and hashes.</aside>'
            '<p>' + e(job.get('workbook')) + ' · ' + e(job.get('status')) + '</p>'
            '<p>' + e(job.get('error') or '') + '</p>'
            '<nav><a href="../../report.html">Suite report</a> · '
            '<a href="case-metrics.json">Full case metrics</a> · '
            '<a href="case-spec.json">Exact case specification</a> · '
            '<a href="supervisor-result.json">Supervisor outcome</a></nav>'
            '<table><tr><th>Index</th><th>Exact sheet</th><th>Final range</th>'
            '<th>Capture</th><th>Issue</th></tr>' + ''.join(rows) + '</table></html>')
    return text.encode('utf-8')


def _excerpt(path: Path, original_size: int, original_sha: str) -> bytes:
    half = LOG_LIMIT // 2
    with path.open('rb') as source:
        first = source.read(half)
        source.seek(max(half, original_size - half))
        last = source.read(half)
    header = (f'Bounded log excerpt; this is NOT the complete log.\n'
              f'Original bytes: {original_size}\nOriginal SHA-256: {original_sha}\n'
              'UTF-8 display decoding replaces undecodable bytes. Full original remains local.\n\n')
    return (header + first.decode('utf-8', errors='replace') +
            f'\n\n[... {max(0, original_size-len(first)-len(last))} middle bytes omitted ...]\n\n' +
            last.decode('utf-8', errors='replace')).encode('utf-8')


def _compact_kind(relative: Path, capture_cases: set[str]) -> str:
    parts = relative.parts
    if len(parts) == 1:
        return 'include'
    if parts[0] != 'cases' or len(parts) < 3:
        return 'include'  # Suite report assets and other explicit measurements.
    if parts[1] not in capture_cases:
        return 'include'  # Do not drop native/renderer diagnostic measurements.
    within = parts[2:]
    if within == ('report.html',):
        return 'case-view'
    if len(within) == 1:
        if relative.suffix in {'.log', '.jsonl'}:
            return 'log'
        return 'include'
    if within[0] in {'artifacts', 'inputs', 'report-assets'}:
        return 'local-support'
    if within[0] == 'data':
        # Complete case metrics contain receipts/outcomes. Detailed producer state,
        # repeated plans and journals stay local, not copied once per report layer.
        return 'include' if relative.name.endswith('.options.json') else 'local-support'
    return 'include'


def verify_bundle(path: Path) -> dict:
    with zipfile.ZipFile(path) as bundle:
        names = bundle.namelist()
        if len(names) != len(set(names)):
            raise ValueError('Duplicate ZIP entries')
        manifest = json.loads(bundle.read('bundle-manifest.json'))
        entries = manifest.get('included', [])
        if manifest.get('status') != 'complete':
            raise ValueError('Incomplete review manifest')
        if len(entries) != len({r['path'] for r in entries}) or {r['path'] for r in entries} | {'bundle-manifest.json'} != set(names):
            raise ValueError('Bundle entry set differs')
        for row in entries:
            name = row['path']
            rel = Path(name)
            if rel.is_absolute() or '..' in rel.parts or '\\' in name or ':' in name:
                raise ValueError('Unsafe bundle path')
            actual = hashlib.sha256()
            size = 0
            with bundle.open(name) as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b''):
                    actual.update(block)
                    size += len(block)
            if size != row['stored_bytes'] or actual.hexdigest() != row['sha256']:
                raise ValueError('Bundle content validation failed: ' + name)
        if any(name not in names for name in manifest.get('required', REQUIRED)):
            raise ValueError('Missing required review file')
        return manifest


def write_bundle(root: Path, state: dict, renderer=None, *, detail: str = 'compact') -> None:
    if renderer is None or detail not in {'compact', 'full'}:
        raise ValueError('Report renderer and a supported metadata detail are required')
    destination = root / 'review-metadata.zip'
    temporary = root / ('review-metadata.zip.' + os.urandom(6).hex() + '.new')
    atomic_json(root / 'bundle-status.json', {'status': 'building', 'updated_utc': utc(), 'detail': detail})
    renderer(root, state)
    included, omitted = [], []
    jobs = {j['id']: j for j in state.get('jobs', [])}
    capture_cases = {ident for ident, j in jobs.items() if j['candidate'].get('kind') == 'capture'}
    try:
        with zipfile.ZipFile(temporary, 'x', compression=zipfile.ZIP_DEFLATED,
                             compresslevel=9, allowZip64=True) as bundle:
            for path in safe_walk(root):
                relative = path.relative_to(root)
                name = relative.as_posix()
                if (path.name.startswith('review') and '.zip' in path.name) or path.name == 'bundle-manifest.json':
                    continue
                if path.name.startswith('.') or path.suffix in {'.new', '.previous'} or 'recovery-backup' in relative.parts:
                    continue
                if path.suffix.lower() not in TEXT_SUFFIXES and path.name != 'reproduce.ps1':
                    omitted.append({'path': name, 'bytes': path.stat().st_size, 'reason': 'binary-artifact'})
                    continue
                kind = _compact_kind(relative, capture_cases) if detail == 'compact' else 'include'
                original_size = path.stat().st_size
                if kind == 'local-support':
                    omitted.append({'path': name, 'bytes': original_size, 'sha256': digest(path),
                                    'reason': 'full-local-support; case metrics and outcomes included'})
                    continue
                transform = None
                if kind == 'case-view':
                    data = _case_view(jobs[relative.parts[1]])
                    transform = 'case-metadata-view'
                elif detail == 'compact' and path.suffix in {'.log', '.jsonl'} and original_size > LOG_LIMIT:
                    original_sha = digest(path)
                    data = _excerpt(path, original_size, original_sha)
                    omitted.append({'path': name, 'bytes': original_size, 'sha256': original_sha,
                                    'reason': 'full log retained locally', 'excerpt': name + '.excerpt.txt'})
                    name += '.excerpt.txt'
                    transform = 'bounded-head-tail-excerpt'
                else:
                    data = path.read_bytes()
                    if path.name == 'report.html':
                        data = data.replace(b'<body>', b'<body><aside style="padding:12px;background:#fff3cd">'
                                            b'Metadata review copy. Images and some local support files are omitted; '
                                            b'see bundle-manifest.json.</aside>', 1)
                        transform = 'review-banner'
                bundle.writestr(name, data)
                row = {'path': name, 'stored_bytes': len(data), 'sha256': _sha(data)}
                if transform:
                    row.update(transform=transform, source_path=relative.as_posix(),
                               source_bytes=original_size, source_sha256=digest(path))
                included.append(row)
            manifest = {'schema': 'vv-review-metadata-v2', 'status': 'complete', 'run_id': root.name,
                        'profile': detail + '-metadata-v1', 'images_included': False,
                        'included': included, 'omitted': omitted, 'required': REQUIRED,
                        'note': 'Original local files are unchanged. Compact case pages are labelled projections; '
                                'omitted support files and excerpt sources retain provenance hashes.'}
            bundle.writestr('bundle-manifest.json', json.dumps(manifest, ensure_ascii=False, indent=2))
        verify_bundle(temporary)
        os.replace(temporary, destination)
        atomic_json(root / 'bundle-status.json', {'status': 'ready', 'updated_utc': utc(),
                    'path': destination.name, 'bytes': destination.stat().st_size, 'detail': detail})
        renderer(root, state)
        print('REVIEW BUNDLE (NO IMAGES): ' + str(destination), flush=True)
    except Exception as error:
        atomic_json(root / 'bundle-status.json', {'status': 'failed', 'updated_utc': utc(),
                    'message': str(error), 'temporary': str(temporary)})
        renderer(root, state)
        raise
