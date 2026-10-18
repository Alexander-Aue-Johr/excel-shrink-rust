"""Paired comparisons with complete-scope, repeatability and environment gates."""
from datetime import datetime
import itertools,statistics
import common as C

def interval(m,a,b):
    try:
        n=(datetime.fromisoformat(m[b].replace('Z','+00:00'))-datetime.fromisoformat(m[a].replace('Z','+00:00'))).total_seconds()
        return n if n>=0 else None
    except (KeyError,TypeError,ValueError):return None

def environment(j):
    values=[]
    for w in C.effective_metrics(j).get('workbooks',[]):
        if w.get('kind')!='capture':continue
        e=w.get('environment') or {};o=e.get('office') or {}
        if not o.get('version') or e.get('device_pixel_ratio') is None:return None
        values.append({k:e.get(k) for k in ('office','device_pixel_ratio','screen','browser')})
    return C.obj_digest(values) if values else None

def comparable(a,b):
    q=C.check_same(a,b)['status']
    if q!='equal':return q
    x,y=environment(a),environment(b)
    return 'environment-unverified' if x is None or y is None else 'equal' if x==y else 'environment-changed'

def is_qualified(j):
    if not C.policy_success(j) or C.quality_fingerprint(j) is None or environment(j) is None:return False
    m=C.effective_metrics(j);wanted=j['candidate'].get('sheet_names')
    actual=[s.get('sheet') for w in m.get('workbooks',[]) if w.get('kind')=='capture' for s in w.get('sheets') or []]
    if wanted is not None and (len(actual)!=len(wanted) or set(actual)!=set(wanted)):return False
    if j['candidate'].get('context_validation')=='first-seam':
        for w in m.get('workbooks',[]):
            if w.get('kind')!='capture':continue
            if w.get('context_policy')!='visible-cell-halo-v1':return False
            tests={c['sheet']:c for c in w.get('context_validations') or []}
            for s in m.get('canonical_sheets') or []:
                if s.get('workbook')!=w.get('workbook') or s.get('tiles',0)<2:continue
                c=tests.get(s.get('sheet')) or {}
                if c.get('status')!='complete' or not c.get('full_comparison',{}).get('equal') or not c.get('repeat_comparison',{}).get('equal') or c.get('overlap_mismatches',1):return False
    return True

def case_diagnostic(root,j):
    receipt=j.get('supervisor') or {};error=str(j.get('error') or '')
    category='infrastructure / report I/O' if receipt.get('failure_category')=='report-io' or 'VV_SNAPSHOT_IO:' in error else j['status']
    return {'case_id':j['id'],'category':category,'error':j.get('error')}

def worksheet_issues(jobs):
    """Group operational worksheet failures without counting repeats as new defects."""
    grouped = {}
    for job in jobs:
        metrics = job.get('metrics') or {}
        for workbook in metrics.get('workbooks') or []:
            for sheet in workbook.get('sheets') or []:
                if sheet.get('status') in {'complete', 'not-selected'}:
                    continue
                error = sheet.get('error') or {}
                error = error if isinstance(error, dict) else {}
                detail = error.get('detail') or {}
                detail = detail if isinstance(detail, dict) else {}
                code = error.get('code') or 'WORKSHEET_INCOMPLETE'
                key = (workbook.get('workbook'), sheet.get('sheet_index'), sheet.get('sheet'), code)
                entry = grouped.setdefault(key, dict(workbook=key[0], sheet_index=key[1], sheet=key[2],
                    code=key[3], cases=[], groups=[], observations=[]))
                if job['id'] not in entry['cases']:
                    entry['cases'].append(job['id'])
                if job['stage'] not in entry['groups']:
                    entry['groups'].append(job['stage'])
                edges = [p.get('edge_points') for p in detail.get('final_checks') or []]
                if not edges:
                    edges = [p.get('edge_points') for p in detail.get('probes') or []][-2:]
                observation = dict(axis=detail.get('axis'), target_points=detail.get('target_points'),
                                   checked_edge_points=edges)
                if observation not in entry['observations']:
                    entry['observations'].append(observation)
    return sorted(grouped.values(), key=lambda row: (str(row['workbook']), row['sheet_index'] or 0, str(row['sheet'])))

def run_row(j):
    c=j['candidate'];m=j.get('metrics') or {};life=m.get('focus_lifecycle') or (j.get('supervisor') or {}).get('focus_lifecycle') or {};mark=life.get('milestones') or {};ms=C.policy_workload(j)
    return {'case_id':j['id'],'group':j['stage'],'block':c.get('block_id'),'variant':c.get('comparison_variant'),'workbook':j['workbook'],'sheets':c.get('sheet_names'),'round':j['round'],'status':j['status'],
            'primary_complete':C.successful(j),'qualified':is_qualified(j) if c['kind']=='capture' else False,'recovery_case_id':(j.get('recovery_policy') or {}).get('recovery_case_id'),
            'workload_seconds':m['workload_ms']/1000 if m.get('workload_ms') is not None else None,'policy_seconds':ms/1000 if ms is not None else None,'elapsed_seconds':j.get('case_seconds'),
            'startup_seconds':interval(mark,'startup','measurement_started'),'capture_seconds':interval(mark,'measurement_started','result_received'),'report':j.get('report'),'error':j.get('error')}

def summarize(state):
    plan=state['plan'];lookup={j['key']:j for j in state['jobs']};primary=[lookup.get(j['key'],j) for j in plan['jobs']];groups=[]
    for g in plan['groups']:
        jobs=[j for j in primary if j['stage']==g['id']];buckets={};rows=[];pairs=[];reference=g.get('question')=='reference'
        for j in jobs:buckets.setdefault((j['candidate']['block_id'],j['candidate']['comparison_variant']),[]).append(j)
        for (block,variant),js in sorted(buckets.items()):
            full=all(C.successful(j) if reference else is_qualified(j) for j in js)
            times=[C.policy_workload(j)/1000 for j in js if (C.successful(j) if reference else is_qualified(j)) and C.policy_workload(j)is not None]
            repeat='not-tested' if len(js)<2 or reference else 'unverified'
            if not reference and len(js)>=2 and full:repeat='equal' if all(comparable(js[0],j)=='equal' for j in js[1:]) else 'different-or-environment-changed'
            rows.append(dict(block=block,variant=variant,planned=len(js),complete=sum(C.successful(j) for j in js),qualified=sum(is_qualified(j) for j in js) if not reference else None,all_complete=full,repeat_pixels=repeat,
                             median_seconds=statistics.median(times) if times else None,min_seconds=min(times) if times else None,max_seconds=max(times) if times else None,recovered=sum(j.get('recovery_policy',{}).get('status')=='recovered' for j in js),successful_only=not full))
        if not reference:
            for block in {j['candidate']['block_id'] for j in jobs}:
                for r in {j['round'] for j in jobs}:
                    for a,b in itertools.combinations([j for j in jobs if j['candidate']['block_id']==block and j['round']==r],2):pairs.append(dict(block=block,round=r,a=a['id'],b=b['id'],quality=comparable(a,b)))
        totals={};required={r['block'] for r in rows}
        if not reference:
            for variant in {r['variant'] for r in rows}:
                obs=[r for r in rows if r['variant']==variant]
                if {r['block'] for r in obs}==required and all(r['all_complete'] and r['repeat_pixels']=='equal' and r['median_seconds']is not None for r in obs):totals[variant]=sum(r['median_seconds'] for r in obs)
        conflicts=[p for p in pairs if p['quality']not in {'equal','not-comparable','not-started','unverified'}]
        fastest=None;chosen=None
        if len(totals)>=2 and not conflicts:
            fastest=min(totals,key=totals.get);chosen=fastest;p=plan['decision_policy']
            if g['id']=='tile-budget':chosen=min((v for v,t in totals.items() if t<=totals[fastest]*(1+p['prefer_smaller_budget_within_fraction'])),key=lambda v:int(v.rsplit('-',1)[1]))
            elif g['id']=='host-overflow':
                rust=[v for v,t in totals.items() if v.startswith('rust-') and t<=totals[fastest]*(1+p['prefer_rust_within_fraction'])]
                if rust:chosen=min(rust,key=totals.get)
                selective=[v for v,t in totals.items() if v.startswith(chosen.split('-')[0]+'-selective-') and t<=totals[fastest]*(1+p['prefer_selective_within_fraction'])]
                if selective:chosen=min(selective,key=totals.get)
        groups.append(dict(id=g['id'],description=g['description'],question=g.get('question','capture'),rows=rows,pairs=pairs,totals=totals,fastest=fastest,selected=chosen,conflicts=conflicts,status='complete' if rows and all(r['all_complete'] for r in rows) and not conflicts else 'with-gaps'))
    primary_keys={j['key'] for j in primary}
    return dict(schema='vv-evaluation-summary-v1',run_id=state['run_id'],status=state['status'],mode=plan['mode'],protocol_id=plan['protocol_id'],source_fingerprint=state['source_version'],selected_policy=plan['selected_policy'],decision_policy=plan['decision_policy'],groups=groups,runs=[run_row(j) for j in primary],recoveries=[run_row(j) for j in state['jobs'] if j['key']not in primary_keys],
                worksheet_issues=worksheet_issues(state['jobs']),
                scope='Unchanged-input capture evaluation, not original-versus-shrunk equivalence.',timing_note='Workload includes preparation, Excel startup, capture, analysis and final export; offline pixel checks and reporting are excluded. Recovery includes failed elapsed time plus fresh 16-MP workload. Incomplete rows show successful-time medians descriptively, not as an eligible ranking.')
