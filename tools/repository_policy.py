"""Read-only Git/worktree audit for reproducible source and compact evidence.

Does not stage, remove, untrack, commit or rewrite repository content. The result
separates tracked paths from non-ignored new paths; it is not a staged-content review.
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess

EXPERIMENTS = 'docs/visual-validation/experiments/'
FIXTURES = {
    EXPERIMENTS + 'range-image-calibration/fixtures/' + n
    for n in ('vv-render-arial10.xlsx', 'vv-render-calibri11.xlsx',
              'vv-render-calibri16.xlsx', 'vv-render-markers.xlsx')
} | {
    EXPERIMENTS + 'range-image-calibration/row-height-fixtures/' + n
    for n in ('vv-row-height-calibri11.xlsx', 'vv-row-height-calibri16.xlsx',
              'vv-row-height-geometry.xlsx')
}
HELPERS = {'tools/excel-height-check/HeightProbeHost.xlsm',
           'tools/excel-height-check/JsonDiffProbeHost.xlsm'}
GENERATED_PREFIXES = ('.tmp/', 'logs/', 'corpus-local/', 'regression/tmp/',
                      'regression/reports/', 'regression/measurements/',
                      'docs/visual-validation/results/')


def generated_reason(path: str) -> str | None:
    p = PurePosixPath(path)
    if path in FIXTURES or path in HELPERS:
        return None  # Required input assets, not measurement output.
    if path.startswith(GENERATED_PREFIXES):
        return 'local inputs or generated execution state'
    if path.startswith(EXPERIMENTS) and 'results' in p.parts:
        return 'per-experiment local results'
    if any(x in p.parts for x in ('__pycache__', 'target', 'obj', 'bin')):
        return 'build/cache directory'
    if p.suffix.lower() in {'.pyc', '.pyo', '.pdb'}:
        return 'compiled/cache artifact'
    if len(p.parts) == 1 and (p.name.startswith(('sce-request-', 'sce-patch-', 'review-metadata'))
                              or p.name.startswith('patch_visual_validation_')):
        return 'local exchange/installer artifact'
    return None


def _git(root: Path, *args: str, data: bytes | None = None, allowed=(0,)) -> bytes:
    proc = subprocess.run(['git', '-C', str(root), *args], input=data,
                          capture_output=True, timeout=30, check=False)
    if proc.returncode not in allowed:
        raise RuntimeError(proc.stderr.decode('utf-8', 'replace').strip() or 'Git query failed')
    return proc.stdout


def _paths(data: bytes) -> set[str]:
    return {os.fsdecode(p) for p in data.split(b'\0') if p}


def _safe_file(root: Path, rel: str) -> Path | None:
    p = PurePosixPath(rel)
    if p.is_absolute() or not p.parts or any(x in {'..', '.', ''} for x in p.parts) or ':' in rel or '\\' in rel:
        return None
    dest = root.joinpath(*p.parts)
    if any(x.is_symlink() or getattr(x, 'is_junction', lambda: False)()
           for x in (dest, *dest.parents) if x != root.parent):
        return None
    return dest


def audit(root: Path) -> dict:
    root = root.resolve()
    result = dict(schema='vv-repository-audit-v1', status='unavailable',
                  scope='Git path membership and current working files; staged bytes are not certified.',
                  index_modified=False, counts={}, findings=[], required_inputs=[])
    try:
        top = Path(os.fsdecode(_git(root, 'rev-parse', '--show-toplevel')).strip()).resolve()
        if top != root:
            raise RuntimeError('Run the audit at the repository root, not in a nested export.')
        tracked = _paths(_git(root, 'ls-files', '--cached', '-z'))
        new = _paths(_git(root, 'ls-files', '--others', '--exclude-standard', '-z'))
        input_paths = b''.join(os.fsencode(p) + b'\0' for p in sorted(tracked))
        ignored_tracked = _paths(_git(root, 'check-ignore', '--no-index', '-z', '--stdin',
                                     data=input_paths, allowed=(0, 1))) if tracked else set()
    except (FileNotFoundError, RuntimeError, subprocess.TimeoutExpired, OSError) as exc:
        result['reason'] = str(exc)
        return result
    result['counts'] = dict(tracked=len(tracked), new_nonignored=len(new),
                            tracked_matching_ignore=len(ignored_tracked))
    result['new_nonignored'] = sorted(new)
    seen = set()

    def finding(path, code, detail):
        key = (path, code)
        if key not in seen:
            seen.add(key)
            result['findings'].append(dict(path=path, code=code, detail=detail))

    for path in sorted(tracked):
        why = generated_reason(path)
        if why:
            finding(path, 'tracked-generated', why + '; review removal from the index, not the local file')
        elif path in ignored_tracked:
            finding(path, 'tracked-but-ignored', 'Ignore rules do not remove an existing index entry; review intentionally tracked inputs separately')
    for path in sorted(new):
        why = generated_reason(path)
        if why:
            finding(path, 'new-generated', why + '; do not add as source')

    required = set(FIXTURES | HELPERS)
    # Standalone executable workspaces need their resolved dependency graph.
    for rel in tracked | new:
        if PurePosixPath(rel).name != 'Cargo.toml' or generated_reason(rel):
            continue
        path = _safe_file(root, rel)
        if path and path.is_file():
            text = path.read_text(encoding='utf-8-sig')
            if rel == 'Cargo.toml' or '[workspace]' in text:
                required.add(str(PurePosixPath(rel).with_name('Cargo.lock')))
    for rel in sorted(required):
        path = _safe_file(root, rel)
        exists = bool(path and path.is_file())
        result['required_inputs'].append(dict(path=rel, exists=exists, tracked=rel in tracked,
                                             new_nonignored=rel in new))
        if not exists:
            finding(rel, 'missing-input', 'Required reproducibility asset is absent; recover/build the intended input before running')
        elif rel not in tracked:
            finding(rel, 'input-not-tracked', 'Required asset exists locally but is not in the Git index; review and add this exact file')
    evidence = 'docs/visual-validation/evidence/'
    for rel in sorted(p for p in tracked | new if p.startswith(evidence) and p.endswith('/manifest.json')):
        path = _safe_file(root, rel)
        if not path or not path.is_file():
            finding(rel, 'evidence-missing', 'Curated manifest is absent or uses a symbolic path')
            continue
        try:
            manifest = json.loads(path.read_text(encoding='utf-8-sig'))
            for row in manifest['files']:
                dest = _safe_file(root, str(PurePosixPath(rel).parent / row['path']))
                if not dest or not dest.is_file() or dest.stat().st_size != row['bytes'] or hashlib.sha256(dest.read_bytes()).hexdigest() != row['sha256']:
                    raise ValueError('Evidence bytes differ from the integrity manifest')
        except (OSError, ValueError, KeyError, TypeError) as exc:
            finding(rel, 'evidence-integrity', str(exc))
    result['status'] = 'needs-review' if result['findings'] else 'passed'
    return result
