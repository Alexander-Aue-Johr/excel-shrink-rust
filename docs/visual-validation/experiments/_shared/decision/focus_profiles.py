"""Bounded question profiles using the existing focused worker and evidence rules.

No production choice, no old timings substituted, and no full-workbook expansion
when a selected-sheet policy falls back to 16 MP.
"""
from __future__ import annotations
import copy
import csv
import html
import itertools
import json
import statistics
from datetime import datetime
from pathlib import Path
import campaign as C


def load_profile(repo: Path, name: str) -> dict:
    data=C.read_json(repo/'docs/visual-validation/experiments/capture-question-profiles.json', {})
    if data.get('schema')!='vv-focus-question-profiles-v1':
        raise ValueError('Unsupported question profile catalogue')
    rows=[p for p in data.get('profiles',[]) if p.get('id')==name]
    if len(rows)!=1: raise ValueError('Unknown question profile: '+name)
    p=copy.deepcopy(rows[0])
    variants=p.get('variants')
    if not isinstance(variants,list) or not 1<=len(variants)<=6 or len({C.obj_digest(x) for x in variants})!=len(variants):
        raise ValueError('Expected 1..6 unique profile variants')
    allowed={'host_kind':{'dotnet','rust'},'overflow_mode':{'heuristic','selective'},'tile_megapixels':{16,32,64}}
    for v in variants:
        if not isinstance(v,dict) or set(v)-allowed.keys() or any(type(value) is bool or value not in allowed[k] for k,value in v.items()):
            raise ValueError('Invalid profile variant')
    if not isinstance(p.get('targets'),list) or not 1<=len(p['targets'])<=8:
        raise ValueError('Expected 1..8 bounded target scopes')
    return p


def planned_jobs(cfg: dict):
    """Pure deterministic schedule. Rotate arms, not the input's worksheet order."""
    p=cfg['focus_parameters'];question=cfg.get('focus_comparison') or {}
    targets=cfg['focused_targets'];variants=question.get('variants',[{}])
    def build(target, variant, repeat):
        settings=dict(p,**variant)
        c=C.candidate(settings['host_kind'],settings['overflow_mode'],settings['tile_megapixels'])
        base_id=c['id'];token=C.obj_digest(target['id'])[:8]
        c.update(id='focus-'+token+'-'+base_id,target_id=target['id'],sheet_names=target['sheets'],
            comparison_variant=base_id,question_profile=question.get('id'),
            scope='Exact selected worksheets; no production recommendation',
            context_validation=settings['context_validation'],diagnostic_level=settings['diagnostic_level'])
        return c,target,repeat
    if not question:
        return [build(t,{},r) for t in targets for r in range(1,p['repetitions']+1)]
    out=[]
    for r in range(1,p['repetitions']+1):
        for i,t in enumerate(targets):
            shift=(r-1+i)%len(variants)
            for v in variants[shift:]+variants[:shift]: out.append(build(t,v,r))
    return out


def interval(milestones: dict, a: str, b: str):
    try:
        seconds=(datetime.fromisoformat(milestones[b].replace('Z','+00:00'))-
                 datetime.fromisoformat(milestones[a].replace('Z','+00:00'))).total_seconds()
        return seconds if seconds>=0 else None
    except (KeyError,TypeError,ValueError): return None


def environment(job):
    values=[]
    for w in C.effective_metrics(job).get('workbooks',[]):
        e=w.get('environment') or {};office=e.get('office') or {}
        if not e or e.get('device_pixel_ratio') is None or not office.get('version'): return None
        values.append((office.get('version'),office.get('platform'),e['device_pixel_ratio']))
    return tuple(values) or None


def comparable(a,b):
    quality=C.check_same(a,b)
    if quality['status']!='equal': return quality['status']
    ea,eb=environment(a),environment(b)
    if ea is None or eb is None:return 'environment-unverified'
    return 'equal' if ea==eb else 'environment-changed'


def evidence(state):
    cfg=state['config'];profile=cfg['focus_comparison'];jobs=state['jobs']
    lookup={(j['candidate_id'],j['input_id'],j['round']):j for j in jobs}
    inputs={f['relative_path']:f for f in state['files']}
    planned=[]
    for c,t,r in planned_jobs(cfg):
        j=lookup.get((c['id'],inputs[t['workbook']]['id'],r))
        planned.append((c,t,r,j))
    groups={};runs=[]
    for c,t,r,j in planned:
        key=(t['id'],c['comparison_variant']);groups.setdefault(key,[]).append(j)
        m=(j or {}).get('metrics') or {};life=m.get('focus_lifecycle') or ((j or {}).get('supervisor') or {}).get('focus_lifecycle') or {}
        milestones=life.get('milestones') or {}
        runs.append({'scope':t['id'],'workbook':t['workbook'],'sheets':t['sheets'],'variant':c['comparison_variant'],'round':r,
            'case_id':(j or {}).get('id'),'status':(j or {}).get('status','not-started'),
            'primary_complete':bool(j and C.successful(j)),
            'policy_complete':bool(j and C.policy_success(j) and C.quality_fingerprint(j) is not None),
            'recovery_case_id':((j or {}).get('recovery_policy') or {}).get('recovery_case_id'),
            'primary_workload_ms':m.get('workload_ms'),'policy_workload_ms':C.policy_workload(j) if j else None,
            'startup_seconds':interval(milestones,'startup','measurement_started'),
            'capture_seconds':interval(milestones,'measurement_started','result_received'),
            'report':(j or {}).get('report'),'error':(j or {}).get('error')})
    rows=[]
    for (scope,variant),observations in groups.items():
        full=all(j and C.policy_success(j) and C.quality_fingerprint(j) is not None for j in observations)
        times=[C.policy_workload(j)/1000 for j in observations if j and C.policy_success(j) and C.policy_workload(j) is not None]
        repeat='not-tested' if len(observations)<2 else 'unverified'
        if len(observations)>=2 and full:
            repeat='equal' if all(comparable(observations[0],j)=='equal' for j in observations[1:]) else 'different-or-environment-changed'
        rows.append({'scope':scope,'variant':variant,'planned':len(observations),
            'attempted':sum(bool(j and j['status'] not in {'queued','blocked','not-started'}) for j in observations),
            'primary_complete':sum(bool(j and C.successful(j)) for j in observations),
            'policy_complete':sum(bool(j and C.policy_success(j) and C.quality_fingerprint(j) is not None) for j in observations),
            'recovered':sum(bool((j or {}).get('recovery_policy',{}).get('status')=='recovered') for j in observations),
            'all_complete':full,'repeat_pixels':repeat,
            'median_policy_seconds':statistics.median(times) if times else None,
            'min_policy_seconds':min(times) if times else None,'max_policy_seconds':max(times) if times else None,
            'successful_times_only':not full,
            'total_spent_seconds':sum((j or {}).get('case_seconds') or 0 for j in observations)+
                sum(next((k.get('case_seconds') or 0 for k in jobs if k.get('id')==((j or {}).get('recovery_policy') or {}).get('recovery_case_id')),0) for j in observations)})
    pairs=[]
    for t in cfg['focused_targets']:
        for r in range(1,cfg['focus_parameters']['repetitions']+1):
            selected=[j for c,tt,rr,j in planned if tt['id']==t['id'] and rr==r]
            for a,b in itertools.combinations(selected,2):
                pairs.append({'scope':t['id'],'round':r,'a':(a or {}).get('id'),'b':(b or {}).get('id'),
                    'quality':comparable(a,b) if a and b else 'not-started'})
    admitted=bool(rows) and all(r['all_complete'] and r['repeat_pixels']=='equal' for r in rows) and all(p['quality']=='equal' for p in pairs)
    totals={}
    if admitted:
        for row in rows:totals[row['variant']]=totals.get(row['variant'],0)+row['median_policy_seconds']
    fastest=min(totals,key=totals.get) if totals and len(totals)>1 and profile['id'] in {'tile-budget','hosts','overflow'} else None
    return {'schema':'vv-focused-question-v1','profile':profile['id'],'question':profile['question'],
        'scope':'Selected worksheets, unchanged inputs. No production or 10,000-file reliability certificate.',
        'evidence_status':('complete-selected-coverage' if all(r['all_complete'] for r in rows) else 'open') if profile['id']=='coverage-gaps' else ('complete-compatible-repeats' if admitted else 'open'),
        'fastest_on_tested_scopes':fastest,'summed_scope_median_seconds':totals,
        'rows':rows,'pairs':pairs,'runs':runs,
        'timing_note':'Primary workload includes preparation, startup and capture, not offline pixel/report work. A 16-MP recovery adds actual failed elapsed time plus fresh recovery workload. Failed, timed-out and missing cases are never zero-time successes. Different timing categories may overlap.',
        'order_note':'Arms rotate deterministically by target and repetition; not fully counterbalanced or randomized. Separate profiles must not be pooled as one controlled factorial experiment.'}


def write_question_report(root:Path,state:dict)->str:
    report=evidence(state);C.atomic_json(root/'question-summary.json',report)
    e=lambda x:html.escape(str(x),quote=True)
    fmt=lambda x:'—' if x is None else format(x,'.2f')
    head=['# Focused question evidence','',report['question'],'',
          '**Evidence:** '+report['evidence_status'],report['scope'],report['timing_note'],report['order_note'],'',
          '| Scope | Configuration | Primary / planned | Policy / planned | Repeat pixels | Median policy seconds |',
          '|---|---|---:|---:|---|---:|']
    table=''
    for r in report['rows']:
        head.append(f"| {r['scope']} | {r['variant']} | {r['primary_complete']}/{r['planned']} | {r['policy_complete']}/{r['planned']} | {r['repeat_pixels']} | {fmt(r['median_policy_seconds'])} |")
        table+='<tr>'+''.join('<td>'+e(v)+'</td>' for v in (r['scope'],r['variant'],f"{r['primary_complete']}/{r['planned']}",f"{r['policy_complete']}/{r['planned']}",r['repeat_pixels'],fmt(r['median_policy_seconds'])))+'</tr>'
    head += ['','Fastest quality-compatible measured configuration on these scopes: '+str(report['fastest_on_tested_scopes'] or 'not selected'),
        'This does not automatically edit decisions.md, discard failures, or trigger another profile.','']
    (root/'question.md').write_text('\n'.join(head),encoding='utf-8')
    with (root/'question-cases.csv').open('w',encoding='utf-8-sig',newline='') as f:
        if report['runs']:
            w=csv.DictWriter(f,fieldnames=list(report['runs'][0]));w.writeheader()
            for r in report['runs']:w.writerow(dict(r,sheets=json.dumps(r['sheets'],ensure_ascii=False)))
    return '<section class="card"><h2>'+e(state['config']['focus_comparison']['label'])+'</h2><p>'+e(report['question'])+'</p><p>Evidence: <strong>'+e(report['evidence_status'])+'</strong></p><p><a href="question.md">Question memorandum</a> · <a href="question-summary.json">Paired evidence</a> · <a href="question-cases.csv">Case timings</a></p><div class="table"><table><tr><th>Scope</th><th>Configuration</th><th>Primary / planned</th><th>Policy / planned</th><th>Repeat pixels</th><th>Median policy seconds</th></tr>'+table+'</table></div><p>'+e(report['timing_note'])+'</p></section>'
