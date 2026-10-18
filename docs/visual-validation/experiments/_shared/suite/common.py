"""Shared identities, immutable inputs, and pixel-quality contracts."""
from __future__ import annotations
import copy, hashlib, json, math, os, shutil, subprocess, time
from pathlib import Path
from datetime import datetime, timezone
from typing import Any
TERMINAL={'complete','failed','partial','timeout','unavailable','blocked','cancelled','unverified','skipped','no-results'}
SOURCE_SUFFIXES={'.ps1','.py','.cs','.rs','.js','.bas','.json','.toml','.lock','.sh','.xml','.csproj','.xaml'}
def utc() -> str:
    return datetime.now(timezone.utc).isoformat()

def read_json(path: Path, default=None):
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except FileNotFoundError:
        return default

def atomic_json(path: Path, obj: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + '.new')
    with temp.open('w', encoding='utf-8', newline='\n') as out:
        json.dump(obj, out, ensure_ascii=False, allow_nan=False, indent=2)
        out.write('\n'); out.flush(); os.fsync(out.fileno())
    os.replace(temp, path)

def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024*1024), b''):
            h.update(block)
    return h.hexdigest()

def obj_digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',',':')).encode()).hexdigest()

def windows_path(path: Path) -> str:
    absolute = str(path.resolve())
    if os.name == 'nt': return absolute
    p = subprocess.run(['wslpath','-w',absolute], check=True, capture_output=True, text=True)
    return p.stdout.strip()

def local_path(path: str) -> Path:
    if os.name == 'nt': return Path(path)
    p = subprocess.run(['wslpath','-u',path], check=True, capture_output=True, text=True)
    return Path(p.stdout.strip())

def safe_walk(root: Path):
    """Never traverse symlinks/junctions. File names remain identities, not commands."""
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if not (Path(directory)/d).is_symlink() and not (getattr((Path(directory)/d),'is_junction',lambda:False)()))
        for name in sorted(files):
            p=Path(directory)/name
            if not p.is_symlink(): yield p

def discover(repo: Path, cfg: dict) -> list[dict]:
    root=Path(cfg['corpus_root'])
    if not root.is_absolute(): root=repo/root
    root=root.resolve()
    if not root.is_dir(): raise ValueError('Corpus directory is missing: '+str(root))
    available=[p for p in safe_walk(root) if p.suffix.lower()=='.xlsx' and not p.name.startswith('~$')]
    requested=cfg['workbooks']
    if requested!=['*']:
        wanted=set(str(x).replace('\\','/') for x in requested)
        available=[p for p in available if p.relative_to(root).as_posix() in wanted]
        missing=wanted-{p.relative_to(root).as_posix() for p in available}
        if missing: raise ValueError('Missing exact relative workbook paths: '+', '.join(sorted(missing)))
    if not available: raise ValueError('No XLSX workbooks found')
    out=[]
    for p in available:
        rel=p.relative_to(root).as_posix()
        out.append({'id':hashlib.sha256(rel.encode()).hexdigest()[:16], 'relative_path':rel,'name':p.name,
                    'source':str(p),'bytes':p.stat().st_size,'mtime_ns':p.stat().st_mtime_ns,'sha256':None})
    if len({r['id'] for r in out})!=len(out): raise ValueError('Input identity collision')
    return out

def candidate(host: str, mode: str, mp: int) -> dict:
    return {'id':f'f-{host}-{mode}-{mp}','kind':'capture','strategy':'F','host_kind':host,
            'overflow_mode':mode,'tile_megapixels':mp,'group':'current-capture',
            'label':f'Office.js / {host} / {mode} / {mp} MP','scope':'Decision candidate'}

def job_key(stage: str, c: dict, f: dict | None, repeat: int) -> str:
    return obj_digest([stage,c['id'],f['id'] if f else None,repeat])[:20]

def make_job(stage: str, c: dict, f: dict | None, repeat: int=1) -> dict:
    return {'key':job_key(stage,c,f,repeat),'stage':stage,'candidate':c,'candidate_id':c['id'],
            'input_id':f['id'] if f else None,'workbook':f['relative_path'] if f else 'Synthetic fixtures',
            'phase':'warmup' if stage=='setup' else 'measured','round':repeat,'status':'queued','metrics':None,
            'report':None,'error':None,'case_seconds':None,'timeout_seconds':None,'quality':None,'id':None}

def successful(job: dict) -> bool:
    return job.get('status')=='complete' and (job.get('metrics') or {}).get('status')=='complete'

def compact_metrics(value: dict) -> dict:
    """Do not embed every PNG receipt in a 10,000-file campaign state."""
    fields=('schema','status','workload_ms','pixel_verification_status','pixel_verified_captures','pixel_unverified_captures',
            'pixel_verification_ms','invocation_error','report_publication','canonical_sheets','native_summary','reference_details','errors','focus_lifecycle')
    out={k:value[k] for k in fields if k in value}
    out['workbooks']=[]
    for w in value.get('workbooks',[]):
        if not isinstance(w,dict): continue
        out['workbooks'].append({k:w.get(k) for k in ('workbook','status','kind','complete_sheets','sheet_count','failed_sheets','inconclusive_sheets',
                'capture_count','render_attempts','timings','compute','environment','capture_policy','capture_options','capture_scope','context_validations','axis_geometry','context_policy','verification_groups','confirmed_blockers','sheets')})
    return out

def effective_metrics(job: dict) -> dict:
    return (job.get('recovery_policy') or {}).get('effective_metrics') or job.get('metrics') or {}

def policy_success(job: dict) -> bool:
    policy=job.get('recovery_policy') or {}
    return successful(job) or (policy.get('status')=='recovered' and policy.get('effective_metrics',{}).get('status')=='complete')

def policy_workload(job: dict) -> float | None:
    value=(job.get('recovery_policy') or {}).get('total_workload_ms', (job.get('metrics') or {}).get('workload_ms'))
    return None if value is None else value+1000*sum(x.get('case_seconds') or 0 for x in job.get('prior_attempts',[]))

def quality_fingerprint(job: dict) -> tuple | None:
    m=effective_metrics(job); entries=m.get('canonical_sheets') or []
    expected=sum(len(w.get('sheets') or []) for w in m.get('workbooks',[]) if w.get('kind')=='capture')
    if not entries or expected!=len(entries) or len({(x.get('sheet_index'),x.get('sheet')) for x in entries})!=len(entries) or any(x.get('status')!='complete' for x in entries): return None
    return tuple(sorted((x['sheet_index'],x['sheet'],x['width'],x['height'],x['algorithm'],x['sha256']) for x in entries))

def geometry_fingerprint(job: dict) -> tuple:
    out=[]
    for w in effective_metrics(job).get('workbooks',[]):
        for s in w.get('sheets') or []: out.append((s.get('sheet_index'),s.get('sheet'),s.get('address')))
    return tuple(sorted(out,key=str))

def check_same(a: dict, b: dict) -> dict:
    if a.get('input_id')!=b.get('input_id'):return {'status':'not-comparable','reason':'Different source file identities'}
    if not policy_success(a) or not policy_success(b): return {'status':'not-comparable','reason':'One capture did not complete'}
    af,bf=quality_fingerprint(a),quality_fingerprint(b)
    if geometry_fingerprint(a)!=geometry_fingerprint(b): return {'status':'different-bounds','reason':'Logical worksheet ranges differ'}
    if af is None or bf is None: return {'status':'unverified','reason':'Missing validated canonical strip digest'}
    if af==bf: return {'status':'equal','reason':'Same bounds and canonical decoded pixels; different tiling allowed'}
    return {'status':'different-pixels','reason':'Canonical decoded image differs; inspect corresponding cases'}

def journal(root: Path, stage: str, detail: Any) -> None:
    with (root/'events.jsonl').open('a',encoding='utf-8') as f:
        f.write(json.dumps({'timestamp_utc':utc(),'stage':stage,'detail':detail},ensure_ascii=False)+'\n');f.flush()

def source_manifest(repo):
    paths=[]
    for base in (repo/'docs/visual-validation/experiments',repo/'tools/excel-lab',repo/'tools/excel-height-check'):
        if not base.exists():continue
        for p in safe_walk(base):
            if p.suffix.lower() not in SOURCE_SUFFIXES or any(s in {'results','target','bin','obj','__pycache__'} for s in p.relative_to(base).parts) or p.name=='ExcelLabHost.manifest.json':continue
            paths.append({'path':p.relative_to(repo).as_posix(),'sha256':digest(p)})
    paths.sort(key=lambda r:r['path']);commit=None;dirty=None
    try:
        commit=subprocess.run(['git','rev-parse','HEAD'],cwd=repo,check=True,capture_output=True,text=True,timeout=10).stdout.strip()
        dirty=bool(subprocess.run(['git','status','--porcelain'],cwd=repo,check=True,capture_output=True,text=True,timeout=10).stdout)
    except (OSError,subprocess.SubprocessError):pass
    return {'schema':'vv-source-manifest-v1','fingerprint':obj_digest(paths),'git_commit':commit,'dirty':dirty,'files':paths}
