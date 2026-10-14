#!/usr/bin/env python3
"""Bounded, resumable decision campaign. Standard-library only; Windows Excel stays
inside the existing PowerShell workers and a Windows Job Object supervisor.
This module does not implement a second Excel renderer or change shrink baselines.
"""
from __future__ import annotations
import argparse
import copy
import csv
import hashlib
import html
import io
import json
import math
import os
from pathlib import Path
import random
import re
import shutil
import statistics
import subprocess
import sys
import threading
import time
import traceback
import zipfile
from datetime import datetime, timezone
from typing import Any

SCHEMA = 'vv-decision-campaign-v1'
TERMINAL = {'complete','failed','partial','timeout','unavailable','blocked','skipped-budget','pruned','cancelled','unverified','skipped','no-results'}
BINARY_SUFFIXES = {'.png','.bmp','.jpg','.jpeg','.webp','.xlsx','.xlsm','.dll','.exe','.pdb','.xll','.7z','.zip'}
TEXT_SUFFIXES = {'.json','.jsonl','.txt','.log','.csv','.md','.html','.css','.js','.xml'}
SOURCE_SUFFIXES = {'.ps1','.py','.cs','.rs','.js','.bas','.json','.toml'}

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

def validate(cfg: dict) -> None:
    if cfg.get('schema') != 'vv-decision-config-v1': raise ValueError('Unsupported decision config schema')
    for field in ('screen_max_files','reference_max_files','confirmation_max_files'):
        if type(cfg.get(field)) is not int or not 1 <= cfg[field] <= 64: raise ValueError(field+' must be 1..64')
    if cfg['reference_max_files'] > cfg['screen_max_files']: raise ValueError('Reference files must be a subset of screening files')
    if cfg['confirmation_max_files'] > cfg['screen_max_files']: raise ValueError('Confirmation files must be a subset of screening files')
    c=cfg['capture']
    if c.get('batch_size')!=1 or c.get('excel_launch_mode')!='excel-x' or c.get('tile_packing')!='budget':
        raise ValueError('Decision campaign fixes batch=1, excel-x and budget-aware packing')
    for field in ('overflow_probe_width_px','overflow_probe_height_px'):
        if type(c.get(field)) is not int or not 64 <= c[field] <= 8192: raise ValueError('Invalid '+field)
    for key,val in cfg['limits'].items():
        if not isinstance(val,(int,float)) or isinstance(val,bool) or not math.isfinite(val) or val <= 0: raise ValueError('Invalid limit '+key)
    limits=cfg['limits']
    if not 30 <= limits['case_min_seconds'] <= limits['case_default_seconds'] <= limits['case_max_seconds'] <= 3600:
        raise ValueError('Require 30 <= min <= default <= maximum case seconds <= 3600')
    if limits['session_seconds'] > 86400: raise ValueError('Session limit must not exceed one day')
    if not 1 <= limits['baseline_multiplier'] <= 10: raise ValueError('Invalid baseline multiplier')
    if cfg.get('verify_pixels') is not True: raise ValueError('Pixel verification must remain enabled for decisions')
    if set(cfg.get('diagnostics',[])) - {'units','markers','row-heights','height-plateau'}: raise ValueError('Unknown diagnostic')
    if set(cfg['references']['bounds_strategies'])-set('ABCDE'): raise ValueError('Unknown historical bounds strategy')
    for key in ('prefer_rust_within_fraction','prefer_selective_within_fraction','prefer_smaller_budget_within_fraction'):
        if not 0 <= cfg['decision'][key] <= .5: raise ValueError('Invalid decision preference '+key)
    if cfg['decision']['required_screen_success_fraction'] != 1: raise ValueError('Screening requires complete coverage; do not drop failures from the gate')
    if not isinstance(cfg.get('workbooks'), list) or not cfg['workbooks']: raise ValueError('Select workbooks or ["*"]')
    if cfg['native']['tile_points'] not in range(72,6001) or not 1 <= cfg['native']['max_tiles'] <= 20000: raise ValueError('Invalid native limits')
    if cfg['native']['max_pixels'] != 32000000: raise ValueError('Native reference keeps its existing 32-MP safety policy')

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

def stratified(files: list[dict], limit: int, seed: str, mandatory=True) -> list[dict]:
    """Size-quantile + seeded within-bin sampling, with named prior stress cases.
    This is coverage-oriented engineering screening, not a random corpus sample.
    """
    if len(files)<=limit: return sorted(files,key=lambda f:(f['bytes'],f['relative_path']))
    chosen=[]
    if mandatory:
        for token in ('krankenhausverzeichnis-','verkehrsunfaelle-','wanderungen-','kernhaushalt-'):
            hits=sorted((f for f in files if token in f['name']),key=lambda f:f['relative_path'])
            if hits and hits[0] not in chosen and len(chosen)<limit: chosen.append(hits[0])
    remain=sorted((f for f in files if f not in chosen),key=lambda f:(f['bytes'],f['relative_path']))
    slots=limit-len(chosen)
    for b in range(slots):
        lo=b*len(remain)//slots; hi=(b+1)*len(remain)//slots
        bucket=remain[lo:hi]
        chosen.append(min(bucket,key=lambda f:obj_digest([seed,b,f['relative_path']])))
    return sorted(chosen,key=lambda f:(f['bytes'],f['relative_path']))

def candidate(host: str, mode: str, mp: int) -> dict:
    return {'id':f'f-{host}-{mode}-{mp}','kind':'capture','strategy':'F','host_kind':host,
            'overflow_mode':mode,'tile_megapixels':mp,'group':'current-capture',
            'label':f'Office.js / {host} / {mode} / {mp} MP','scope':'Decision candidate'}

def screen_candidates() -> list[dict]:
    return [candidate('rust','selective',16),candidate('rust','selective',32),candidate('rust','selective',64),
            candidate('dotnet','selective',16),candidate('rust','heuristic',16),candidate('dotnet','heuristic',16)]

def reference_candidates(cfg: dict) -> list[dict]:
    refs=cfg['references']; result=[]
    for s in refs['bounds_strategies']:
        result.append({'id':s.lower()+'-dotnet','kind':'capture','strategy':s,'host_kind':'dotnet','overflow_mode':'heuristic','tile_megapixels':16,
            'label':'Legacy bounds '+s+' / Office.js .NET','scope':'Different bounds policy; not an equivalent quality baseline','group':'historical-bounds'})
    if refs['vba_chart_export']:
        result.append({'id':'vba-picture','kind':'vba-picture','label':'VBA CopyPicture / chart export','group':'native-picture',
            'scope':'Existing guarded/capped baseline; not full Strategy F coverage'})
    for setting,ident,kind,label in [
        ('vba_gdi_bitmap','lab-bitmap','lab-capture','VBA-local CopyPicture / bitmap / GDI+ PNG'),
        ('vba_gdi_emf','lab-emf','lab-capture','VBA-local CopyPicture / EMF / GDI+ PNG'),
        ('vba_gdi_memory_compare','lab-memory','lab-memory','VBA-local CopyPicture / owned GDI+ buffers / memory comparison'),
        ('vba_row_heights','lab-rows','lab-rows','VBA-local row heights / used range'),
        ('native_drawing_bounds','shape-native-base','shape-native','Drawing bounds / VBA, Excel-DNA, PowerShell COM, Rust COM'),
        ('vsto_drawing_bounds','shape-vsto','shape-vsto','Drawing bounds / VSTO')]:
        if refs[setting]: result.append({'id':ident,'kind':kind,'label':label,'scope':'Existing native component; separate question','group':'native-reference'})
    if refs['vba_gdi_memory_compare']:
        result.append({'id':'lab-disk-pair','kind':'lab-memory','label':'VBA-local CopyPicture / identical pair / PNG + memory comparison','scope':'Same operations as lab-memory, with PNG persistence enabled','group':'native-reference'})
    if refs['officejs_drawing_bounds']:
        for host in ('rust','dotnet'): result.append({'id':'shape-officejs-'+host,'kind':'shape-officejs','host_kind':host,'label':'Drawing bounds / Office.js / '+host,'scope':'Geometry, not screenshots','group':'drawing-bounds'})
    result.sort(key=lambda c:(c['group']=='historical-bounds',c['kind']=='shape-vsto'))
    return result

def job_key(stage: str, c: dict, f: dict | None, repeat: int) -> str:
    return obj_digest([stage,c['id'],f['id'] if f else None,repeat])[:20]

def make_job(stage: str, c: dict, f: dict | None, repeat: int=1) -> dict:
    return {'key':job_key(stage,c,f,repeat),'stage':stage,'candidate':c,'candidate_id':c['id'],
            'input_id':f['id'] if f else None,'workbook':f['relative_path'] if f else 'Synthetic fixtures',
            'phase':'warmup' if stage=='setup' else 'measured','round':repeat,'status':'queued','metrics':None,
            'report':None,'error':None,'case_seconds':None,'timeout_seconds':None,'quality':None,'id':None}

def deadline(cfg: dict, samples: list[float], stage: str) -> dict:
    lim=cfg['limits']
    valid=[float(x) for x in samples if isinstance(x,(float,int)) and math.isfinite(x) and x>0]
    if stage=='setup': return {'seconds':int(lim['setup_seconds']),'source':'setup allowance','baseline_seconds':None}
    if stage=='diagnostics': return {'seconds':int(lim['diagnostic_case_seconds']),'source':'diagnostic allowance','baseline_seconds':None}
    base=statistics.median(valid) if valid else None
    seconds=lim['case_default_seconds'] if base is None else base*lim['baseline_multiplier']+lim['baseline_slack_seconds']
    seconds=max(lim['case_min_seconds'],min(lim['case_max_seconds'],seconds))
    if stage=='references': seconds=min(seconds,lim['reference_case_seconds'])
    return {'seconds':int(math.ceil(seconds)),'baseline_seconds':base,
            'source':'successful same-input workload baseline (allowance only)' if base else 'no matching timing baseline; conservative fallback'}

def successful(job: dict) -> bool:
    return job.get('status')=='complete' and (job.get('metrics') or {}).get('status')=='complete'

def source_version(repo: Path) -> str:
    paths=[]
    for base in (repo/'docs/visual-validation/experiments',repo/'tools/excel-lab'):
        for p in safe_walk(base):
            if p.suffix.lower() not in SOURCE_SUFFIXES: continue
            if any(part in {'results','target','bin','obj','__pycache__'} for part in p.relative_to(base).parts): continue
            if p.name=='ExcelLabHost.manifest.json': continue  # generated host build metadata, not source
            paths.append((p.relative_to(repo).as_posix(),digest(p)))
    return obj_digest(sorted(paths))

def compact_metrics(value: dict) -> dict:
    """Do not embed every PNG receipt in a 10,000-file campaign state."""
    fields=('schema','status','workload_ms','pixel_verification_status','pixel_verified_captures','pixel_unverified_captures',
            'pixel_verification_ms','invocation_error','report_publication','canonical_sheets','native_summary','reference_details','errors')
    out={k:value[k] for k in fields if k in value}
    out['workbooks']=[]
    for w in value.get('workbooks',[]):
        if not isinstance(w,dict): continue
        out['workbooks'].append({k:w.get(k) for k in ('workbook','status','kind','complete_sheets','sheet_count','failed_sheets','inconclusive_sheets',
                'capture_count','render_attempts','timings','compute','environment','capture_policy','capture_options','verification_groups','confirmed_blockers','sheets')})
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

def pairing(jobs: list[dict], a: str, b: str, stage: str | None=None) -> dict:
    groups={}
    for j in jobs:
        if j['candidate_id'] not in (a,b) or j.get('phase')=='warmup' or (stage and j['stage']!=stage): continue
        key=(j['input_id'],j['round'])
        groups.setdefault(key,{})[j['candidate_id']]=j
    pairs=[]
    for (f,r), g in groups.items():
        if a not in g or b not in g: continue
        aa,bb=g[a],g[b]; q=check_same(aa,bb)
        ma=policy_workload(aa);mb=policy_workload(bb)
        pairs.append({'input_id':f,'round':r,'a':aa['id'],'b':bb['id'],'quality':q['status'],
                      'ratio_b_over_a':mb/ma if ma and mb and policy_success(aa) and policy_success(bb) else None,
                      'a_status':aa['status'],'b_status':bb['status']})
    ratios=[p['ratio_b_over_a'] for p in pairs if p['ratio_b_over_a'] is not None and p['quality']=='equal']
    return {'a':a,'b':b,'pairs':pairs,'paired_files':len({p['input_id'] for p in pairs}),
            'equal_pairs':sum(p['quality']=='equal' for p in pairs),'median_equal_ratio_b_over_a':statistics.median(ratios) if ratios else None}

def select_candidate(state: dict, candidates: list[dict], stage: str) -> dict:
    """Deployment recommendation is never upgraded to proven visual correctness.
    Rank complete common-input executions; require canonical quality agreement for
    confirmed preference. If the gate cannot be closed, retain the declared fallback.
    """
    cfg=state['config']; jobs=[j for j in state['jobs'] if j['stage']==stage and j.get('phase')!='warmup']
    required=state['screen_ids'] if stage=='screen' else state.get('confirmation_ids',[])
    stats=[]; ready=[]
    for c in candidates:
        js=[j for j in jobs if j['candidate_id']==c['id']]
        byfile={f:[] for f in required}
        for j in js:
            if j['input_id'] in byfile: byfile[j['input_id']].append(j)
        required_rounds={1} if stage=='screen' else {1,2}
        ok=all(rows and required_rounds.issubset({j['round'] for j in rows}) and all(policy_success(j) and policy_workload(j) is not None for j in rows) for rows in byfile.values()) and bool(required)
        # Every admitted candidate must have stable repeats and fully validated pixel digests.
        verified=ok and all(quality_fingerprint(j) is not None for j in js)
        repeat_stable=verified and all(len({quality_fingerprint(j) for j in rows})==1 for rows in byfile.values())
        medians={f:statistics.median(policy_workload(j) for j in rows if j['round'] in required_rounds and policy_success(j) and policy_workload(j) is not None) for f,rows in byfile.items() if any(j['round'] in required_rounds and policy_success(j) and policy_workload(j) is not None for j in rows)}
        row={'candidate':c,'attempted':len(js),'complete':sum(successful(j) for j in js),'recovered':sum((j.get('recovery_policy') or {}).get('status')=='recovered' for j in js),'covered_files':len(medians),
             'required_files':len(required),'all_complete':ok,'pixels_verified':verified,'repeat_stable':repeat_stable,
             'total_file_medians_seconds':sum(medians.values())/1000,'file_medians_ms':medians}
        stats.append(row)
        if ok and verified and repeat_stable: ready.append(row)
    # Agreement gate on the intersection of all eligible candidates (reference is not assumed to be truth).
    compatible=[]
    for row in ready:
        conflict=False
        for other in ready:
            for f in required:
                aa=next(j for j in jobs if j['candidate_id']==row['candidate']['id'] and j['input_id']==f)
                bb=next(j for j in jobs if j['candidate_id']==other['candidate']['id'] and j['input_id']==f)
                if check_same(aa,bb)['status']!='equal': conflict=True
        if not conflict: compatible.append(row)
    enough=len(required)>=cfg['decision']['minimum_paired_files']
    if len(compatible)<2 or not enough:
        # A bounded pilot still needs an execution choice, but does not invent a successful decision.
        fallback=next((c for c in candidates if c['id']==cfg['decision']['fallback']),candidate('dotnet','selective',16))
        return {'candidate':fallback,'status':'provisional','reason':'Quality, completeness, repeatability or paired-coverage gate remains open; conservative fallback retained.', 'statistics':stats}
    best=min(compatible,key=lambda r:r['total_file_medians_seconds'])
    fastest=best['total_file_medians_seconds']
    # Preferences use a single total overhead cap against the fastest, not chained 15% tolerances.
    maximum=fastest*(1+max(cfg['decision']['prefer_rust_within_fraction'],cfg['decision']['prefer_selective_within_fraction']))
    near=[r for r in compatible if r['total_file_medians_seconds']<=maximum]
    rust=[r for r in near if r['candidate']['host_kind']=='rust' and r['total_file_medians_seconds']<=fastest*(1+cfg['decision']['prefer_rust_within_fraction'])]
    if rust: best=min(rust,key=lambda r:r['total_file_medians_seconds'])
    selective=[r for r in near if r['candidate']['host_kind']==best['candidate']['host_kind'] and r['candidate']['overflow_mode']=='selective' and r['total_file_medians_seconds']<=fastest*(1+cfg['decision']['prefer_selective_within_fraction'])]
    if selective: best=min(selective,key=lambda r:r['total_file_medians_seconds'])
    budget_near=[r for r in compatible if r['candidate']['host_kind']==best['candidate']['host_kind'] and r['candidate']['overflow_mode']==best['candidate']['overflow_mode'] and r['total_file_medians_seconds']<=best['total_file_medians_seconds']*(1+cfg['decision']['prefer_smaller_budget_within_fraction']) and r['total_file_medians_seconds']<=maximum]
    if budget_near: best=min(budget_near,key=lambda r:(r['candidate']['tile_megapixels'],r['total_file_medians_seconds']))
    return {'candidate':best['candidate'],'status':'supported-on-tested-inputs','reason':'Complete tested policy coverage (including measured fresh-session recovery costs), verified canonical pixel agreement and stable required repeats. Preference thresholds are engineering policy, not statistical significance.',
            'statistics':stats,'fastest_candidate':min(compatible,key=lambda r:r['total_file_medians_seconds'])['candidate'],
            'chosen_total_seconds':best['total_file_medians_seconds'],'fastest_total_seconds':fastest}

def journal(root: Path, stage: str, detail: Any) -> None:
    with (root/'campaign-events.jsonl').open('a',encoding='utf-8') as f:
        f.write(json.dumps({'timestamp_utc':utc(),'stage':stage,'detail':detail},ensure_ascii=False)+'\n');f.flush()

class Campaign:
    def __init__(self, repo: Path, root: Path, cfg: dict, windows_parent: int=0):
        self.repo,self.root,self.cfg=repo.resolve(),root.resolve(),cfg
        self.windows_parent=windows_parent; self.clock=time.monotonic();self.current=None;self.cancelled=False
        self.specdir=self.repo/'docs/visual-validation/experiments/_shared/decision'
        self.state=read_json(root/'campaign-state.json')
        version=source_version(repo)
        if self.state:
            if self.state['config_hash']!=obj_digest(cfg) or self.state['source_version']!=version:
                raise ValueError('Config or source changed. Start a new campaign; existing measurements will not be mixed.')
            now=discover(repo,cfg)
            if [(x['relative_path'],x['bytes'],x['mtime_ns']) for x in now]!=[(x['relative_path'],x['bytes'],x['mtime_ns']) for x in self.state['files']]:
                raise ValueError('Corpus inventory changed. Start a new campaign; no existing evidence was replaced.')
            for j in self.state['jobs']:
                if j['status']=='running': j['status']='cancelled';j['error']='Prior session ended without controller completion'
                if j['status']=='cancelled':j['resume_pending']=True
            self.state['sessions'].append({'started_utc':utc(),'budget_seconds':cfg['limits']['session_seconds']})
        else:
            fs=discover(repo,cfg); screened=stratified(fs,cfg['screen_max_files'],cfg['selection_seed']); references=stratified(screened,cfg['reference_max_files'],cfg['selection_seed']+'-references')
            self.state={'schema':SCHEMA,'run_id':root.name,'status':'running','started_utc':utc(),'config':cfg,'config_hash':obj_digest(cfg),
                        'source_version':version,'files':fs,'screen_ids':[x['id'] for x in screened],'reference_ids':[x['id'] for x in references],
                        'confirmation_ids':[],'jobs':[],'stages':{},'decisions':{},'coverage':{},'baselines':[],
                        'sessions':[{'started_utc':utc(),'budget_seconds':cfg['limits']['session_seconds']}], 'errors':[],
                        'limitations':['Agreement between implementations is not an external visual oracle.',
                        'Historical timings size a timeout only; no performance rows are replaced by old data.',
                        'Fresh-instance qualification times are not steady-state throughput predictions for a persistent 10,000-file runner.',
                        'One-at-a-time screening misses some interactions; the selected budget is crossed with both hosts and both overflow modes on an interaction probe; two finalists are repeated on separate file blocks.',
                        'Some externally created COM processes may be outside the Windows job. Without a verified ownership receipt the campaign pauses instead of killing them.',
                        'Clipboard corruption is recorded only if observed. Clipboard use itself is a known constraint, not an inferred failure.',
                        'This is strategy qualification, not the original-versus-shrunk 10,000-file regression run.']}
            self.import_baselines()
        self.state['status']='running';self.files={f['id']:f for f in self.state['files']}
        self.save()

    def save(self):
        self.state['updated_utc']=utc();atomic_json(self.root/'campaign-state.json',self.state)
        render_report(self.root,self.state)

    def import_baselines(self):
        reports=self.repo/'docs/visual-validation/results/runs'
        paths=sorted(reports.glob('*/suite-state.json'),reverse=True)[:50]
        rows=[]
        for path in paths:
            try:
                state=read_json(path,{})
                inputs={x['name']:x for x in state.get('inputs',[]) if x.get('sha256')}
                for j in state.get('jobs',[]):
                    m=j.get('metrics') or {}; ws=m.get('workbooks') or []
                    if j.get('status')!='complete' or j.get('phase')!='measured' or len(ws)!=1 or not m.get('workload_ms'): continue
                    w=ws[0]; inp=inputs.get(w.get('workbook'))
                    if not inp or w.get('status')!='complete': continue
                    rows.append({'sha256':inp['sha256'],'workbook':w['workbook'],'seconds':m['workload_ms']/1000,
                                 'candidate':j['candidate_id'],'source':path.relative_to(self.repo).as_posix(),'environment':w.get('environment'),
                                 'purpose':'timeout-sizing-only; machine/DPI/source differences may exist'})
            except (ValueError,OSError,KeyError,TypeError): continue
        self.state['baselines']=rows

    def time_left(self, stage: str) -> float:
        lim=self.cfg['limits']
        remaining=max(0.0,lim['session_seconds']-(time.monotonic()-self.clock))
        name={'screen':'screen_seconds','references':'reference_seconds','diagnostics':'diagnostic_seconds','confirm':'confirmation_seconds'}.get(stage)
        used=sum((j.get('case_seconds') or 0)+sum(a.get('case_seconds') or 0 for a in j.get('prior_attempts',[])) for j in self.state['jobs'] if j['stage']==stage)
        return min(remaining,max(0.0,lim[name]-used)) if name else remaining

    def has_time(self, stage: str) -> bool:
        # Do not start Excel with only seconds of the declared budget left.
        return self.time_left(stage)>=30

    def input_file(self, ident): return self.files[ident] if ident else None

    def stage_input(self, f: dict) -> Path:
        src=Path(f['source']); st=src.stat()
        if st.st_size!=f['bytes'] or st.st_mtime_ns!=f['mtime_ns']: raise ValueError('Input changed: '+f['relative_path'])
        sha=digest(src)
        if f['sha256'] is not None and f['sha256']!=sha: raise ValueError('Input hash changed: '+f['relative_path'])
        f['sha256']=sha
        target=self.repo/'.tmp/vv-decision-inputs'/self.root.name/f['id']/f['name']
        target.parent.mkdir(parents=True,exist_ok=True)
        if not target.exists():
            tmp=target.with_name(target.name+'.new');shutil.copyfile(src,tmp)
            if digest(tmp)!=sha: raise ValueError('Input changed during staging')
            os.replace(tmp,target)
        elif digest(target)!=sha: raise ValueError('Staged input was changed: '+str(target))
        return target

    def baseline(self, f: dict | None, stage: str) -> dict:
        samples=[]
        if f:
            for j in self.state['jobs']:
                if j['input_id']==f['id'] and j['candidate']['kind']=='capture' and j['candidate'].get('strategy')=='F' and successful(j):
                    ms=(j.get('metrics') or {}).get('workload_ms')
                    if ms: samples.append(ms/1000)
            if not samples:
                samples=[r['seconds'] for r in self.state['baselines'] if r['sha256']==f.get('sha256')]
        return deadline(self.cfg,samples,stage)

    def ensure_job(self, stage,c,f,repeat=1):
        key=job_key(stage,c,f,repeat)
        existing=next((j for j in self.state['jobs'] if j['key']==key),None)
        if existing: return existing
        j=make_job(stage,c,f,repeat);j['sequence']=len(self.state['jobs'])+1
        j['id']=f"{j['sequence']:06d}-{c['id']}";self.state['jobs'].append(j)
        return j

    def circuit(self,c):
        bad=[j for j in self.state['jobs'] if j['candidate_id']==c['id'] and j['input_id'] and j['status'] in {'failed','timeout','unavailable'}]
        good=[j for j in self.state['jobs'] if j['candidate_id']==c['id'] and successful(j)]
        return not good and len({j['input_id'] for j in bad})>=self.cfg['limits']['failure_circuit_distinct_files']

    def run_job(self,stage,c,f=None,repeat=1,reference_job=None):
        j=self.ensure_job(stage,c,f,repeat)
        if not self.has_time(stage): return j
        if j.get('resume_pending'):
            prior=copy.deepcopy({k:v for k,v in j.items() if k not in {'prior_attempts','resume_pending'}})
            j.setdefault('prior_attempts',[]).append(prior)
            j['id']=f"{j['sequence']:06d}-{c['id']}-attempt{len(j['prior_attempts'])+1}"
            j.update(status='queued',metrics=None,case_seconds=None,report=None,error=None,quality=None,resume_pending=False)
        if j['status'] in TERMINAL: return j
        if self.circuit(c) and stage not in {'coverage','confirm'}:
            j['status']='pruned';j['error']='Circuit breaker: repeated independent input failures with no successful case. Not a speed measurement.';self.save();return j
        if shutil.disk_usage(self.root).free<self.cfg['limits']['minimum_free_gib']*1024**3:
            raise RuntimeError('Free-disk safety budget reached. No screenshots were deleted; queue retained.')
        source=self.stage_input(f) if f else None
        budget=self.baseline(f,stage)
        remaining=max(1,int(self.time_left(stage)))
        j['timeout_seconds']=budget['seconds'];j['timeout_basis']=budget
        case_root=self.root/'cases'/j['id'];case_root.mkdir(parents=True,exist_ok=True)
        j['report']='cases/'+j['id']+'/report.html';j['status']='running';j['started_utc']=utc()
        # Compact identities in directories; the source basename remains unchanged inside its per-file corpus.
        spec={'schema':'vv-decision-case-v1','repo_root':windows_path(self.repo),'suite_root':windows_path(self.root),
              'case_root':windows_path(case_root),'job':dict(j),'inputs':([{'name':f['name'],'sha256':f['sha256'],'bytes':f['bytes'],'source':windows_path(Path(f['source']))}] if f else []),
              'staged_corpus':windows_path(source.parent if source else self.repo/'corpus-local'),
              'capture':dict(self.cfg['capture'],tile_megapixels=c.get('tile_megapixels',16)),
              'verify_final_pixels':self.cfg['verify_pixels'],'decision':True,'native':self.cfg['native'],
              'build_lab_host_if_needed':self.cfg['references']['build_lab_host_if_needed'],
              'workload_timeout_seconds':min(budget['seconds'],remaining),'postprocess_timeout_seconds':self.cfg['limits']['postprocess_seconds'],
              'total_remaining_seconds':remaining,'controller_parent_pid':self.windows_parent,
              'reference_case_root':windows_path(self.root/'cases'/reference_job['id']) if reference_job else None,
              'release_token':os.urandom(16).hex(),'parent_pid':0}
        spec['job']['metrics']=None
        atomic_json(case_root/'case-spec.json',spec)
        self.save();journal(self.root,'case_started',{'id':j['id'],'workbook':j['workbook'],'deadline':budget})
        print(f"[{j['sequence']}] {stage} | {c['label']} | {j['workbook']} | limit {spec['workload_timeout_seconds']} s",flush=True)
        supervisor=windows_path(self.specdir/'supervise.ps1')
        args=['powershell.exe','-NoLogo','-NoProfile','-STA','-ExecutionPolicy','Bypass','-File',supervisor,'-SpecPath',windows_path(case_root/'case-spec.json')]
        start=time.monotonic(); self.current=case_root;proc=None;last=0
        try:
            # Both handles drain directly to files; no PIPE buffer can stall a verbose child.
            with (case_root/'supervisor.stdout.log').open('wb') as out,(case_root/'supervisor.stderr.log').open('wb') as err:
                proc=subprocess.Popen(args,stdout=out,stderr=err)
                while proc.poll() is None:
                    (case_root/'.lease').write_text(utc(),encoding='ascii')
                    elapsed=time.monotonic()-start
                    if elapsed-last>=15: print(f"  {j['id']} | {elapsed:.0f} s | {j['workbook']}",flush=True);last=elapsed
                    if elapsed>remaining or self.cancelled:
                        (case_root/'.cancel').write_text('Controller session budget/cancellation\n')
                    if elapsed>remaining+45:
                        raise RuntimeError('Windows supervisor did not acknowledge cancellation. Stop the task; no further Excel process will be started.')
                    time.sleep(.5)
            self.recover_native(case_root,spec['release_token'])
            receipt=read_json(case_root/'supervisor-result.json',{})
            metrics=read_json(case_root/'case-metrics.json',{})
            j['metrics']=compact_metrics(metrics)
            j['status']=receipt.get('status') or ('failed' if proc.returncode else metrics.get('status','failed'))
            if j['status']=='complete' and metrics.get('status')!='complete':j['status']=metrics.get('status','failed')
            j['error']=receipt.get('error') or metrics.get('invocation_error')
            j['case_seconds']=time.monotonic()-start;j['supervisor']=receipt
            if not receipt: j['status']='failed';j['error']='No supervisor completion receipt. See retained stdout/stderr.'
            if j['status']=='timeout':j['censored']=True
            if c['kind']=='capture' and c.get('strategy')=='F':
                j['quality']='verified' if quality_fingerprint(j) is not None else 'unverified'
            if receipt.get('safe_to_continue') is False: raise RuntimeError('Owned cleanup could not be established; campaign paused. '+str(j['error']))
        except KeyboardInterrupt:
            self.cancelled=True
            (case_root/'.cancel').write_text('User cancelled\n');j['status']='cancelled';j['error']='User cancelled; owned Windows supervisor is stopping the case.'
            if proc:
                try:proc.wait(timeout=35)
                except subprocess.TimeoutExpired:pass
            raise
        finally:
            self.current=None;j['ended_utc']=utc()
            if j['case_seconds'] is None: j['case_seconds']=time.monotonic()-start
            if j['status']=='running':j['status']='failed'
            if source and (not source.exists() or digest(source)!=f['sha256']):
                j['status']='failed';j['error']='Frozen source was changed; evidence cannot be admitted.'
                self.save();raise RuntimeError(j['error'])
            self.save();journal(self.root,'case_finished',{'id':j['id'],'status':j['status'],'seconds':j['case_seconds']})
        print('  '+j['status'].upper()+' | '+j['report'],flush=True)
        return j

    def recover_native(self, case_root: Path, token: str):
        location=read_json(case_root/'native-location.json',{})
        if location.get('owner_token')!=token or not location.get('artifacts'): return
        try:
            src=local_path(location['artifacts'])
            # Only the private directory declared by our native adapter is copied.
            if 'ExcelShrinkDecision' not in str(src) or token[:8] not in str(src):
                raise ValueError('Native recovery workspace identity mismatch')
            if not src.is_dir(): return
            for p in safe_walk(src):
                target=case_root/'native'/p.relative_to(src)
                target.parent.mkdir(parents=True,exist_ok=True)
                if not target.exists(): shutil.copyfile(p,target)
        except (OSError,ValueError,subprocess.SubprocessError) as e:
            atomic_json(case_root/'native-recovery-error.json',{'error':str(e),'workspace':location})

    def run_policy(self, stage, c, f, repeat=1):
        primary=self.run_job(stage,c,f,repeat)
        if primary['status']=='queued': return primary
        if c.get('tile_megapixels',16)<=16: return primary
        if primary.get('recovery_policy'): return primary
        if successful(primary) and quality_fingerprint(primary) is not None: return primary
        # No fallback can turn a pixel mismatch into equality: missing/invalid evidence
        # remains part of the primary attempt, and changed repeats still fail the gate.
        if not self.has_time(stage): return primary
        fallback=candidate(c['host_kind'],c['overflow_mode'],16)
        fallback['id']='rescue-'+c['id'];fallback['label']='Recovery for '+c['label']+' / fresh 16 MP'
        fallback['group']='recovery';fallback['scope']='Explicit fresh-session recovery, included in parent policy cost'
        rescue=self.run_job(stage,fallback,f,repeat)
        if rescue['status']=='queued': return primary
        good=successful(rescue) and quality_fingerprint(rescue) is not None
        primary_ms=(primary.get('metrics') or {}).get('workload_ms')
        if primary_ms is None: primary_ms=1000*(primary.get('case_seconds') or 0)
        # Failed attempt includes its teardown and waiting cost (never a zero-time failure).
        primary_cost=max(primary_ms,1000*(primary.get('case_seconds') or 0))
        rescued_ms=(rescue.get('metrics') or {}).get('workload_ms')
        if rescued_ms is None: rescued_ms=1000*(rescue.get('case_seconds') or 0)
        primary['recovery_policy']={'status':'recovered' if good else 'failed','recovery_case_id':rescue['id'],
            'effective_metrics':copy.deepcopy(rescue.get('metrics')) if good else None,
            'total_workload_ms':primary_cost+rescued_ms,
            'primary_status':primary['status'],'note':'Failed primary elapsed time plus fresh 16-MP workload. Both attempts remain visible; no simulation or cached substitute.'}
        self.save();return primary

    def reference_for(self,f):
        choices=[j for j in self.state['jobs'] if j['input_id']==f['id'] and j['candidate']['kind']=='capture' and j['candidate'].get('strategy')=='F' and successful(j)]
        return next((j for j in choices if j['candidate_id']=='f-rust-selective-16'),next(iter(choices),None))

    def stage_screen(self):
        cs=screen_candidates(); fs=[self.files[i] for i in self.state['screen_ids']]
        for f in fs:
            for c in cs:self.ensure_job('screen',c,f)
        self.save()
        for host in ('rust','dotnet'):
            self.run_job('setup',candidate(host,'selective',16),fs[0])
        for n,f in enumerate(fs):
            order=cs[n%len(cs):]+cs[:n%len(cs)]
            for c in order: self.run_policy('screen',c,f)
            if not self.has_time('screen'):break
        # Choose budget from the controlled Rust/selective slice, not from a mixed host ranking.
        budget_cs=[c for c in cs if c['host_kind']=='rust' and c['overflow_mode']=='selective']
        budget=select_candidate(self.state,budget_cs,'screen')
        if budget['status']=='provisional':budget['candidate']=candidate('rust','selective',16)
        self.state['decisions']['budget']=budget
        self.state['decisions']['screen']=select_candidate(self.state,cs,'screen');self.save()

    def stage_confirm(self):
        mp=self.state['decisions']['budget']['candidate']['tile_megapixels']
        screened=[self.files[x] for x in self.state['screen_ids']]
        fs=stratified(screened,self.cfg['confirmation_max_files'],self.cfg['selection_seed']+'-confirm')
        self.state['confirmation_ids']=[f['id'] for f in fs]
        cs=[candidate(h,m,mp) for h in ('rust','dotnet') for m in ('selective','heuristic')]
        # Keep the best actual 16-MP control, so a selected Rust budget is not assumed
        # to beat the previously faster .NET/16 configuration without a comparison.
        old16=select_candidate(self.state,[c for c in screen_candidates() if c['tile_megapixels']==16],'screen')
        control=old16.get('fastest_candidate',old16['candidate'])
        for c in (control,candidate('dotnet','selective',16)):
            if c['id'] not in {x['id'] for x in cs}:cs.append(c)
        pilot=sorted(screened,key=lambda f:(f['bytes'],f['relative_path']))[len(screened)//2]
        for c in cs:self.ensure_job('confirm',c,pilot,0)
        self.save()
        for c in cs:self.run_policy('confirm',c,pilot,0)
        # One interaction spot-check, then repeat only the two finalists. This is
        # explicitly a bounded engineering race, not a full-factorial significance test.
        pilot_state=copy.deepcopy(self.state);pilot_state['screen_ids']=[pilot['id']]
        pilot_state['config']['decision']['minimum_paired_files']=1
        pilot_state['jobs']=[copy.deepcopy(j) for j in self.state['jobs'] if j['stage']=='confirm' and j['round']==0]
        for j in pilot_state['jobs']:j['stage']='screen';j['round']=1
        choice=select_candidate(pilot_state,cs,'screen')
        self.state['decisions']['interaction_probe']=choice
        promoted=[choice['candidate']]
        fastest=choice.get('fastest_candidate')
        if fastest and fastest['id']!=promoted[0]['id']:promoted.append(fastest)
        ordered=sorted(choice['statistics'],key=lambda r:(not(r['all_complete'] and r['pixels_verified']),r['total_file_medians_seconds']))
        for row in ordered:
            c=row['candidate']
            if c['id'] not in {x['id'] for x in promoted}:promoted.append(c)
            if len(promoted)==2:break
        promoted=promoted[:2];self.state['confirmation_candidates']=promoted
        for r in (1,2):
            for f in fs:
                for c in promoted:self.ensure_job('confirm',c,f,r)
        self.save()
        for r in (1,2):
            for n,f in enumerate(fs):
                order=promoted if (r+n)%2 else list(reversed(promoted))
                for c in order:self.run_policy('confirm',c,f,r)
                if not self.has_time('confirm'):break
            if not self.has_time('confirm'):break
        self.state['decisions']['confirmation']=select_candidate(self.state,promoted,'confirm');self.save()

    def stage_references(self):
        fs=[self.files[x] for x in self.state['reference_ids']];cs=reference_candidates(self.cfg)
        for f in fs:
            for c in cs:self.ensure_job('references',c,f)
        labs=any(c['kind'].startswith('lab-') for c in cs)
        setup=None
        if not self.has_time('references'):self.save();return
        if labs:
            c={'id':'lab-setup','kind':'lab-setup','label':'Build/reuse reviewed VBA helper','group':'setup','scope':'No measured corpus work; prerequisite only'}
            setup=self.run_job('setup',c,None)
        self.save()
        # First pass touches every reference family, then fill the remaining file blocks.
        ordering=[(c,fs[i%len(fs)]) for i,c in enumerate(cs)]
        ordering += [(c,f) for f in fs for c in cs if (c,f) not in ordering]
        for c,f in ordering:
            ref=self.reference_for(f)
            if c['kind'].startswith('lab-') and (not setup or not successful(setup)):
                j=self.ensure_job('references',c,f);j['status']='unavailable';j['error']='Native helper prerequisite did not complete; see lab-setup evidence.';continue
            if c['kind'] in {'lab-capture','lab-memory'} and not ref:
                j=self.ensure_job('references',c,f);j['status']='blocked';j['error']='No complete F bounds available. Native comparison will not invent a range.';continue
            self.run_job('references',c,f,reference_job=ref)
            if not self.has_time('references'):break
        self.save()

    def stage_diagnostics(self):
        hospital=next((f for f in self.state['files'] if f['name']=='krankenhausverzeichnis-3500100227005.xlsx'),None)
        jobs=[]
        for name in self.cfg['diagnostics']:
            c={'id':name,'kind':'diagnostic','suite':name,'label':'Renderer diagnostic / '+name,'group':'renderer-diagnostics','scope':'Once per campaign, not per corpus file'}
            f=hospital if name=='height-plateau' else None
            j=self.ensure_job('diagnostics',c,f);jobs.append((c,f))
            if name=='height-plateau' and not hospital:j['status']='blocked';j['error']='The fixed hospital diagnostic workbook is not in this corpus.'
        self.save()
        for c,f in jobs:
            self.run_job('diagnostics',c,f)
            if not self.has_time('diagnostics'):break
        self.save()

    def stage_coverage(self):
        chosen=self.state['decisions'].get('confirmation',self.state['decisions']['screen'])['candidate']
        fallback=candidate('dotnet','selective',16)
        self.state['deployment_candidate']=chosen
        # Every input receives an outcome. No factorial matrix over the remaining 10,000 files.
        for f in self.state['files']:
            if f['id'] in self.state['coverage'] and self.state['coverage'][f['id']]['status'] in {'complete','fallback-complete','failed'}:continue
            prior=[j for j in self.state['jobs'] if j['input_id']==f['id'] and j['candidate_id']==chosen['id'] and policy_success(j) and quality_fingerprint(j) is not None and j['phase']!='warmup']
            if prior:
                fingerprints={quality_fingerprint(j) for j in prior}
                if len(fingerprints)==1:
                    self.state['coverage'][f['id']]={'status':'fallback-complete' if prior[-1].get('recovery_policy',{}).get('status')=='recovered' else 'complete','job_id':prior[-1]['id'],'reused_same_campaign':True};continue
                # Preserve nondeterminism; do not turn a later majority into success.
                self.state['coverage'][f['id']]={'status':'failed','reason':'Repeated unchanged-input captures disagree','job_ids':[j['id'] for j in prior]};continue
            if not self.has_time('coverage'):break
            j=self.run_policy('coverage',chosen,f)
            ok=policy_success(j) and quality_fingerprint(j) is not None
            record={'status':('fallback-complete' if j.get('recovery_policy',{}).get('status')=='recovered' else 'complete') if ok else 'failed','job_id':j['id'],'reused_same_campaign':False,'capture_status':j['status'],'quality':'verified' if quality_fingerprint(j) is not None else 'unverified'}
            if not ok and chosen['id']!=fallback['id'] and self.has_time('coverage'):
                rescue=self.run_job('coverage',fallback,f)
                record['fallback_job_id']=rescue['id']
                if successful(rescue) and quality_fingerprint(rescue) is not None:record['status']='fallback-complete'
                record['recovery_seconds']=(j.get('case_seconds') or 0)+(rescue.get('case_seconds') or 0)
            self.state['coverage'][f['id']]=record;self.save()
        self.save()

    def run(self):
        try:
            for name,method in [('screen',self.stage_screen),('confirm',self.stage_confirm),('references',self.stage_references),('diagnostics',self.stage_diagnostics),('coverage',self.stage_coverage)]:
                if self.state['stages'].get(name) in {'complete','budget-limited'}: continue
                if not self.has_time('coverage'):break
                self.state['stages'][name]='running';self.save();method()
                pending=[j for j in self.state['jobs'] if j['stage']==name and j['status']=='queued']
                if name=='coverage':
                    self.state['stages'][name]='complete' if len(self.state['coverage'])==len(self.state['files']) else 'pending'
                elif pending:
                    # Finite comparison budgets are campaign budgets, not a hidden invitation
                    # to repeat screening forever. Unexecuted comparisons remain explicit.
                    for j in pending:j['status']='skipped-budget';j['error']='Declared comparison/session budget exhausted; not measured.'
                    self.state['stages'][name]='budget-limited'
                else:self.state['stages'][name]='complete'
            total=len(self.state['files']);done=len(self.state['coverage'])
            if done<total:self.state['status']='partial'
            else:
                gaps=any(s!='complete' for s in self.state['stages'].values()) or any(v['status']=='failed' for v in self.state['coverage'].values())
                gaps=gaps or any(j['status'] not in {'complete'} or j.get('prior_attempts') for j in self.state['jobs'])
                self.state['status']='complete-with-gaps' if gaps else 'complete'
        except KeyboardInterrupt:
            self.state['status']='interrupted';self.state['errors'].append({'time':utc(),'message':'User interruption; resume retains completed cases.'})
        except Exception as e:
            self.state['status']='interrupted';self.state['errors'].append({'time':utc(),'message':str(e),'trace':traceback.format_exc()});print(str(e),file=sys.stderr)
        finally:
            self.state['resumable']=self.state['status'] in {'partial','interrupted'}
            self.state['sessions'][-1]['ended_utc']=utc();self.state['sessions'][-1]['elapsed_seconds']=time.monotonic()-self.clock
            self.state['ended_utc']=utc();self.save();write_bundle(self.root,self.state)
        return 0 if self.state['status'] in {'complete','complete-with-gaps'} else 2

def paired_results(state: dict) -> list[dict]:
    ids={j['candidate_id'] for j in state['jobs']}
    pairs=[('f-rust-selective-16','f-rust-selective-32'),('f-rust-selective-16','f-rust-selective-64'),
           ('f-dotnet-selective-16','f-rust-selective-16'),('f-dotnet-heuristic-16','f-rust-heuristic-16'),
           ('f-rust-heuristic-16','f-rust-selective-16'),('f-dotnet-heuristic-16','f-dotnet-selective-16')]
    return [pairing(state['jobs'],a,b,'screen') for a,b in pairs if a in ids and b in ids]

def aggregate(state: dict) -> list[dict]:
    grouped={}
    for j in state['jobs']:
        if j['phase']=='warmup':continue
        grouped.setdefault((j['stage'],j['candidate_id']),[]).append(j)
    out=[]
    for (stage,ident),rows in grouped.items():
        times=[j['metrics']['workload_ms']/1000 for j in rows if successful(j) and (j.get('metrics') or {}).get('workload_ms') is not None]
        wall=sum((j.get('case_seconds') or 0)+sum(a.get('case_seconds') or 0 for a in j.get('prior_attempts',[])) for j in rows)
        out.append({'stage':stage,'candidate':ident,'label':rows[0]['candidate']['label'],'planned':len(rows),
            'attempted':sum(j['status'] not in {'queued','blocked','pruned','skipped-budget'} for j in rows),
            'complete':sum(successful(j) for j in rows),'timeouts':sum(j['status']=='timeout' for j in rows)+sum(a['status']=='timeout' for j in rows for a in j.get('prior_attempts',[])),
            'restarts':sum(len(j.get('prior_attempts',[])) for j in rows),'recovered_policies':sum(j.get('recovery_policy',{}).get('status')=='recovered' for j in rows),
            'unverified':sum(j.get('quality')=='unverified' for j in rows),'median_seconds':statistics.median(times) if times else None,
            'min_seconds':min(times) if times else None,'max_seconds':max(times) if times else None,
            'spent_seconds':wall,'seconds_per_completed_case_including_failed_attempts':wall/len(times) if times else None,
            'note':'Timing statistics use completed workloads only. Failure/timeout cost is also shown; timeout is censored, not a completion time.'})
    return out

def decision_document(state: dict) -> str:
    selection=state['decisions'].get('confirmation') or state['decisions'].get('screen')
    coverage=state['coverage'];n=len(state['files'])
    strict=sum(x['status']=='complete' for x in coverage.values());rescued=sum(x['status']=='fallback-complete' for x in coverage.values())
    failed=sum(x['status']=='failed' for x in coverage.values());pending=n-len(coverage)
    lines=['# Visual-validation strategy decision','',f"Campaign: `{state['run_id']}`",f"Source fingerprint: `{state['source_version']}`",'',
           '## Decision status','']
    eligible=selection and selection['status']=='supported-on-tested-inputs' and strict==n and pending==0 and all(state['stages'].get(x)=='complete' for x in ('screen','confirm'))
    if selection:
        lines += [f"Proposed deployment configuration: **{selection['candidate']['label']}**.",f"Evidence status: **{'ready for author review' if eligible else 'provisional / exceptions require review'}**.",selection['reason'],'']
    else:lines+=['No deployment candidate selected yet.','']
    lines += [f'Corpus: {n} files inventoried; {strict} candidate-complete, {rescued} fallback-complete, {failed} failed, {pending} pending.',
              'This qualifies capture strategies on unchanged input. It does **not** run or certify original-versus-shrunk regression equivalence.',
              'No statement of reliability for 10,000 files is inferred from a small screening sample.','',
              '## Predeclared engineering policy','',
              f"Rust may be preferred within {100*state['config']['decision']['prefer_rust_within_fraction']:.0f}% of the fastest quality-compatible configuration to avoid a second production host stack.",
              f"Selective verification may be preferred within {100*state['config']['decision']['prefer_selective_within_fraction']:.0f}% overhead for explicit counterfactual evidence.",
              f"The smaller tile budget is preferred within {100*state['config']['decision']['prefer_smaller_budget_within_fraction']:.0f}% overhead. Tolerances do not compound.",
              'These thresholds are practical policy, not a significance test. Performance is compared within workbook blocks.', '',
              '## Results', '', '| Stage | Candidate | Complete / attempted | Timeouts | Median completed workload (s) | Total spent (s) |',
              '|---|---|---:|---:|---:|---:|']
    for r in aggregate(state):
        med='—' if r['median_seconds'] is None else f"{r['median_seconds']:.2f}"
        lines.append(f"| {r['stage']} | {r['candidate']} | {r['complete']}/{r['attempted']} | {r['timeouts']} | {med} | {r['spent_seconds']:.1f} |")
    lines+=['','## Tile-budget policy and paired comparisons','', 'A 32/64-MP policy includes at most one measured fresh-session 16-MP recovery after a failed or unverified primary attempt. Primary failures remain in the execution table; recovered-policy cost is not replaced by the successful retry time alone.','']
    for pair in paired_results(state):
        ratio=pair['median_equal_ratio_b_over_a']
        lines.append('- '+pair['a']+' versus '+pair['b']+': '+str(pair['equal_pairs'])+' equal-output paired observations; median ratio B/A '+('unverified' if ratio is None else format(ratio,'.3f'))+'.')
    budget=state['decisions'].get('budget',{})
    for row in budget.get('statistics',[]):
        lines.append('- Budget '+row['candidate']['id']+': '+str(row['complete'])+' primary completions, '+str(row.get('recovered',0))+' recovered, required files '+str(row['required_files'])+', sum of per-file policy medians '+format(row['total_file_medians_seconds'],'.2f')+' s; gate '+('closed' if row['all_complete'] and row['pixels_verified'] else 'open')+'.')
    lines+=['','## Scope of native references','',
        '- C# Excel-DNA/VSTO references measure drawing geometry. They are not an implemented C# screenshot acquisition benchmark in this source snapshot.',
        '- VBA chart export uses its historical capped plan. It must not be ranked against complete Strategy F coverage as if the work were identical.',
        '- VBA-local GDI+ bitmap/EMF routes receive the completed F sheet bounds and retain their documented native tiling and raster policy.',
        '- The memory case uses two unchanged working copies and owned GDI+ buffers; it measures capture plus buffer comparison, not PNG persistence.',
        '- CopyPicture uses the system clipboard. Do not use it concurrently. A clipboard-corruption assertion requires an observed, recorded failure; none is assumed.',
        '', '## Diagnostics and unresolved evidence','']
    for name,status in state['stages'].items():lines.append(f'- {name}: {status}')
    for item in state['limitations']:lines.append('- '+item)
    lines+=['','## Interpretation','',
        'Different tile layouts are compared only by the validated, unscaled canonical strip stream. Gaps, inconsistent overlaps, unsupported horizontal layouts and missing PNGs remain unverified.',
        'Raw and final artifacts remain in each case report. No previous failure or nondeterministic screenshot is removed to make a candidate win.',
        'Ready for author review does not automatically edit decisions.md or select a production regression implementation.', '',
        '## Next operational step','',
        'Review the exceptions and this decision, then freeze one production configuration and run the actual original-versus-shrunk regression corpus. Do not automatically launch another parameter sweep.','']
    return '\n'.join(lines)

def render_report(root: Path, state: dict) -> None:
    e=lambda x:html.escape(str(x),quote=True)
    rows=aggregate(state);selected=state['decisions'].get('confirmation') or state['decisions'].get('screen')
    chosen=selected['candidate']['label'] if selected else 'Pending screening'
    coverage=state['coverage'];counts={k:sum(v['status']==k for v in coverage.values()) for k in ('complete','fallback-complete','failed')}
    numeric=lambda v:'—' if v is None else f'{v:.2f}'
    row_html=''.join(f"<tr><td>{e(r['stage'])}</td><td>{e(r['label'])}</td><td>{r['complete']} / {r['attempted']}<small>{r['planned']} planned</small></td><td>{r['timeouts']}</td><td>{numeric(r['median_seconds'])}</td><td>{numeric(r['spent_seconds'])}</td></tr>" for r in rows)
    jobs=''.join(f"<tr data-search='{e(j['workbook']+' '+j['candidate']['label']+' '+j['status'])}'><td>{j['sequence']}</td><td>{e(j['stage'])}</td><td>{e(j['workbook'])}</td><td>{e(j['candidate']['label'])}</td><td><span class='badge {e(j['status'])}'>{e(j['status'])}</span></td><td>{numeric((j.get('metrics') or {}).get('workload_ms',0)/1000) if (j.get('metrics') or {}).get('workload_ms') is not None else '—'}</td><td>{('<a href="'+e(j['report'])+'">Open case</a>') if j.get('report') else 'Not started'}<small>{e(j.get('error') or '')}</small></td></tr>" for j in state['jobs'])
    cov=''.join(f"<tr><td>{e(f['relative_path'])}</td><td>{e((coverage.get(f['id']) or {}).get('status','pending'))}</td><td>{e((coverage.get(f['id']) or {}).get('job_id',''))}</td></tr>" for f in state['files'])
    stages=''.join(f"<span class='badge'>{e(k)}: {e(v)}</span> " for k,v in state['stages'].items())
    errors=''.join('<p class="warn">'+e(x['message'])+'</p>' for x in state['errors'][-10:])
    reason=selected['reason'] if selected else 'No automatic decision before the predeclared quality gates have been evaluated.'
    status=read_json(root/'bundle-status.json',{})
    bundle='<a class="button" href="review-metadata.zip">Review bundle · no images</a>' if status.get('status')=='ready' else '<span class="button disabled">Review bundle · '+e(status.get('status','available after finalization'))+'</span>'
    reference_links=''.join('<li>'+e(j['workbook'])+' · '+e(j['candidate']['label'])+' · <a href="cases/'+e(j['id'])+'/'+e(d['path'])+'">'+e(d['name'])+'</a></li>' for j in state['jobs'] for d in (j.get('metrics') or {}).get('reference_details',[]))
    budget_rows=''.join('<tr><td>'+e(r['candidate']['label'])+'</td><td>'+str(r['complete'])+'</td><td>'+str(r.get('recovered',0))+'</td><td>'+numeric(r['total_file_medians_seconds'])+'</td><td>'+e('complete/verified' if r['all_complete'] and r['pixels_verified'] else 'open')+'</td></tr>' for r in state['decisions'].get('budget',{}).get('statistics',[]))
    content=f'''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Strategy decision · Excel Shrink</title><style>
:root{{--ink:#172b42;--muted:#5c6f82;--line:#dce5ec;--accent:#125b88;--bg:#f4f7fa}}*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--ink);font:15px/1.55 system-ui,sans-serif}}header{{background:#142c43;color:white;padding:36px max(28px,calc((100vw - 1440px)/2));}}header p{{color:#cbd8e5}}main{{max-width:1440px;margin:auto;padding:26px}}h1{{font-size:32px;line-height:1.2;margin:8px 0}}h2{{font-size:21px;margin:0 0 14px}}.eyebrow{{letter-spacing:.15em;text-transform:uppercase;font-size:12px;color:#92c9ec}}.card{{background:white;border:1px solid var(--line);border-radius:12px;padding:24px;margin:20px 0;box-shadow:0 2px 4px #172b4205}}.stats{{display:grid;grid-template-columns:repeat(4,1fr);gap:14px}}.stat{{background:white;padding:20px;border:1px solid var(--line);border-radius:12px}}.stat strong{{display:block;font-size:27px}}.stat span,small{{color:var(--muted);font-size:12px}}small{{display:block;max-width:480px;overflow-wrap:anywhere}}a{{color:var(--accent);text-decoration:none}}a:hover{{text-decoration:underline}}.button{{display:inline-block;padding:10px 14px;border:1px solid var(--line);border-radius:7px;background:white;margin:6px 8px 6px 0}}.disabled{{opacity:.6}}.badge{{display:inline-block;border-radius:5px;padding:3px 7px;font-size:12px;background:#edf2f6;color:#34495d}}.complete{{background:#e4f4ec;color:#176442}}.failed,.timeout{{background:#fce7e5;color:#9a3028}}.warn{{border-left:4px solid #d99b2f;background:#fff7e7;padding:12px}}.table-scroll{{overflow:auto}}table{{width:100%;border-collapse:collapse;text-align:left}}th{{font-size:12px;text-transform:uppercase;color:var(--muted);background:#f7f9fb}}td,th{{padding:11px 12px;border-bottom:1px solid var(--line);vertical-align:top}}td:nth-child(3){{overflow-wrap:anywhere}}.note{{color:var(--muted)}}input{{padding:10px 14px;border:1px solid var(--line);border-radius:8px;width:min(600px,100%);margin:8px 0 18px}}details summary{{cursor:pointer;font-weight:600}}code{{background:#eef3f7;padding:2px 4px;border-radius:4px}}@media(max-width:800px){{.stats{{grid-template-columns:repeat(2,1fr)}}main{{padding:12px}}}}
</style></head><body><header><div class="eyebrow">Excel Shrink · Engineering decision</div><h1>Strategy qualification</h1><p>{e(state['run_id'])} · {e(state['status'])}</p></header><main>
<nav>{bundle}<a class="button" href="decision.md">Decision memorandum</a><a class="button" href="case-results.csv">Case results CSV</a><a class="button" href="../../index.html">Run history</a></nav>
<div class="stats"><div class="stat"><strong>{len(state['files'])}</strong><span>Files inventoried · no hidden corpus limit</span></div><div class="stat"><strong>{len(state['screen_ids'])}</strong><span>Screening blocks</span></div><div class="stat"><strong>{counts['complete']+counts['fallback-complete']}</strong><span>Covered · {counts['fallback-complete']} required fallback</span></div><div class="stat"><strong>{len(state['files'])-len(coverage)}</strong><span>Pending files · {counts['failed']} failed</span></div></div>
<section class="card"><h2>Deployment recommendation</h2><h3>{e(chosen)}</h3><p>{e(reason)}</p><p class="warn">This campaign qualifies capture methods on unchanged inputs. It does not certify the original-versus-shrunk regression corpus. A timeout is a censored observation, not a successful fast run.</p>{stages}{errors}</section>
<section class="card"><h2>Comparisons by question</h2><p class="note">Completed-workload medians exclude setup. Total spent includes failure, timeout and post-processing costs. Different reference tasks are never pooled into a screenshot speed ranking.</p><div class="table-scroll"><table><thead><tr><th>Stage</th><th>Configuration</th><th>Complete / attempted</th><th>Timeouts</th><th>Median seconds</th><th>Total spent seconds</th></tr></thead><tbody>{row_html}</tbody></table></div></section>
<section class="card"><h2>16 / 32 / 64 MP · recovery-aware policy comparison</h2><p class="note">Sum of per-file policy medians, not a mean over successful survivors. A larger-budget failure includes its elapsed cost plus one fresh 16-MP recovery. An open gate is not a proven faster option.</p><div class="table-scroll"><table><thead><tr><th>Configuration</th><th>Primary complete</th><th>Recovered</th><th>Policy seconds</th><th>Evidence gate</th></tr></thead><tbody>{budget_rows}</tbody></table></div></section>
<section class="card"><h2>Execution ledger</h2><input id="search" placeholder="Filter by workbook, strategy or status" aria-label="Filter cases"><div class="table-scroll"><table id="jobs"><thead><tr><th>#</th><th>Stage</th><th>Input</th><th>Configuration</th><th>Outcome</th><th>Workload seconds</th><th>Evidence</th></tr></thead><tbody>{jobs}</tbody></table></div></section>
<details class="card"><summary>Every-file coverage ({len(state['files'])})</summary><div class="table-scroll"><table><thead><tr><th>Workbook</th><th>Coverage outcome</th><th>Evidence case</th></tr></thead><tbody>{cov}</tbody></table></div></details>
<details class="card"><summary>Native drawing-bound timings and validation</summary><ul>{reference_links}</ul><p class="note">These producer timers measure drawing geometry. They are not screenshot acquisition timings.</p></details>
<section class="card"><h2>Scope and decision rules</h2><p>Screening isolates the Rust tile budget (16/32/64 MP) and the 2 × 2 host/overflow comparison at 16 MP. An interaction spot-check crosses the selected budget with both hosts and both overflow modes and retains a 16-MP control. Only two finalists receive repeated multi-file confirmation. Remaining corpus files run only the deployment candidate, with one explicit fresh-session fallback.</p><p>Units, row-height and limit fixtures run once. C# Excel-DNA/VSTO are drawing-bounds references in this repository, not independent screenshot implementations. Clipboard effects are recorded rather than presumed.</p><p class="note">Raw case data and original images stay local. The metadata bundle intentionally excludes image bytes. No old reports, baselines, inputs or alternative implementations are deleted.</p></section>
</main><script>document.getElementById('search').addEventListener('input',function(){{var q=this.value.toLowerCase();document.querySelectorAll('#jobs tbody tr').forEach(function(r){{r.hidden=!r.dataset.search.toLowerCase().includes(q);}});}});</script></body></html>'''
    (root/'report.html').write_text(content,encoding='utf-8')
    (root/'decision.md').write_text(decision_document(state),encoding='utf-8')
    fields=['sequence','stage','candidate','workbook','phase','round','status','workload_seconds','case_seconds','timeout_seconds','censored','quality','report','error']
    with (root/'case-results.csv').open('w',encoding='utf-8-sig',newline='') as f:
        writer=csv.DictWriter(f,fieldnames=fields);writer.writeheader()
        for j in state['jobs']:
            row={k:j.get(k) for k in fields};row['candidate']=j['candidate_id'];ms=(j.get('metrics') or {}).get('workload_ms');row['workload_seconds']=ms/1000 if ms is not None else None;writer.writerow(row)
    atomic_json(root/'decision-summary.json',{'schema':'vv-decision-summary-v1','state_status':state['status'],'selection':selected,'coverage':{'files':len(state['files']),**counts,'pending':len(state['files'])-len(coverage)},'comparisons':rows,'paired_comparisons':paired_results(state),'tile_budget_selection':state['decisions'].get('budget')})
    meta=read_json(root/'run.json',{})
    if meta:
        meta['status']=state['status'];meta['completed_utc']=state.get('ended_utc');atomic_json(root/'run.json',meta)
    atomic_json(root/'result.json',{'schema':'vv-run-result-v1','status':state['status'],'run':meta,'outputs':[{'path':'decision.md'},{'path':'decision-summary.json'},{'path':'campaign-state.json'},{'path':'case-results.csv'}],
        'errors':state['errors'],'note':'See distinct execution, quality and every-file coverage outcomes.'})

def write_bundle(root: Path, state: dict) -> None:
    destination=root/'review-metadata.zip';tmp=root/('review-metadata.zip.'+os.urandom(6).hex()+'.new')
    atomic_json(root/'bundle-status.json',{'status':'building','updated_utc':utc()});render_report(root,state)
    included=[];omitted=[]
    try:
        files=list(safe_walk(root))
        with zipfile.ZipFile(tmp,'x',compression=zipfile.ZIP_DEFLATED,compresslevel=6,allowZip64=True) as z:
            for p in files:
                rel=p.relative_to(root).as_posix()
                if p.name.startswith('review') and '.zip' in p.name:continue
                if p.name.startswith('.') or p.suffix in {'.new','.previous'} or 'recovery-backup' in p.parts:continue
                if p.suffix.lower() not in TEXT_SUFFIXES:
                    omitted.append({'path':rel,'bytes':p.stat().st_size,'reason':'binary-artifact'});continue
                data=p.read_bytes()
                if p.name=='report.html':data=data.replace(b'<body>',b'<body><aside style="padding:12px;background:#fff3cd">Metadata review copy. Images are intentionally omitted.</aside>',1)
                z.writestr(rel,data);included.append({'path':rel,'stored_bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()})
            manifest={'schema':'vv-review-metadata-v2','status':'complete','run_id':root.name,'images_included':False,'included':included,'omitted':omitted,
                      'required':['run.json','report.html','campaign-state.json','decision.md','case-results.csv','decision-summary.json']}
            z.writestr('bundle-manifest.json',json.dumps(manifest,ensure_ascii=False,indent=2))
        with zipfile.ZipFile(tmp) as z:
            if z.testzip() is not None:raise ValueError('ZIP CRC validation failed')
            if len(z.namelist())!=len(set(z.namelist())):raise ValueError('Duplicate ZIP entries')
            for row in included:
                data=z.read(row['path'])
                if len(data)!=row['stored_bytes'] or hashlib.sha256(data).hexdigest()!=row['sha256']:raise ValueError('Bundle content validation failed')
            for name in manifest['required']:
                if name not in z.namelist():raise ValueError('Missing required review file '+name)
        os.replace(tmp,destination)
        atomic_json(root/'bundle-status.json',{'status':'ready','updated_utc':utc(),'path':'review-metadata.zip','bytes':destination.stat().st_size})
        render_report(root,state)
        print('REVIEW BUNDLE (NO IMAGES): '+str(destination),flush=True)
    except Exception as error:
        atomic_json(root/'bundle-status.json',{'status':'failed','updated_utc':utc(),'message':str(error),'temporary':str(tmp)})
        render_report(root,state);raise

def main(argv=None):
    if sys.version_info < (3,10):raise RuntimeError('Python 3.10+ is required')
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo-root',type=Path,required=True)
    parser.add_argument('--run-root',type=Path)
    parser.add_argument('--config',type=Path)
    parser.add_argument('--parent-pid',type=int,default=0)
    parser.add_argument('--plan',action='store_true')
    parser.add_argument('--report-only',action='store_true')
    args=parser.parse_args(argv)
    config=args.config or args.repo_root/'docs/visual-validation/experiments/decision-campaign.json'
    cfg=read_json(config);validate(cfg)
    if args.plan:
        fs=discover(args.repo_root,cfg);ss=stratified(fs,cfg['screen_max_files'],cfg['selection_seed'])
        print(json.dumps({'files':len(fs),'screen_files':[f['relative_path'] for f in ss],
            'core_screen_configurations':[c['id'] for c in screen_candidates()],
            'references':[c['id'] for c in reference_candidates(cfg)],'diagnostics':cfg['diagnostics'],
            'session_budget_seconds':cfg['limits']['session_seconds'],'scope':'All files inventoried; expensive comparisons restricted to the declared screening blocks, then selected candidate covers all files.'},indent=2));return 0
    if args.run_root is None:parser.error('--run-root is required except with --plan')
    if args.report_only:
        state=read_json(args.run_root/'campaign-state.json')
        if not state or state.get('schema')!=SCHEMA:raise ValueError('No decision campaign state')
        render_report(args.run_root,state);write_bundle(args.run_root,state);return 0
    campaign=Campaign(args.repo_root,args.run_root,cfg,args.parent_pid)
    return campaign.run()

if __name__=='__main__':
    try:sys.exit(main())
    except Exception as exc:print('DECISION CAMPAIGN ERROR: '+str(exc),file=sys.stderr);sys.exit(1)
