'''Bounded repair checks using the existing campaign worker, locks, timeouts and reports.
Not a strategy race; no old result is upgraded or used as a new quality measurement.
'''
from __future__ import annotations
import copy
import csv
import html
import json
import time
from pathlib import Path
import campaign as C


def validate_targets(cfg):
    targets=cfg.get('focused_targets')
    if cfg.get('focus_protocol') == 'exact-scope-phase-timeouts-v1':
        if not isinstance(targets,list) or not 1<=len(targets)<=32:raise ValueError('Expected 1..32 exact focused targets')
        ids=set()
        for t in targets:
            if not t.get('id') or t['id'] in ids or t.get('workbook') not in cfg['workbooks']:raise ValueError('Invalid exact target identity')
            ids.add(t['id'])
            ss=t.get('sheets')
            if ss is not None and (not ss or len(ss)>512 or len(ss)!=len(set(ss)) or any(not isinstance(x,str) or not x for x in ss)):raise ValueError('Invalid exact sheets')
        if {t['workbook'] for t in targets}!=set(cfg['workbooks']):raise ValueError('Focused inventory mismatch')
        if cfg['diagnostics'] or C.reference_candidates(cfg):raise ValueError('Focused mode cannot run reference sweeps')
        return
    if not isinstance(targets,list) or not 1<=len(targets)<=12:
        raise ValueError('Expected 1..12 focused targets')
    names=[]
    for target in targets:
        name=target.get('workbook');sheets=target.get('sheets')
        if not isinstance(name,str) or name not in cfg['workbooks'] or name in names:
            raise ValueError('Focused targets require unique explicit corpus paths')
        names.append(name)
        if sheets is not None and (not isinstance(sheets,list) or not sheets or len(sheets)>16 or
            any(not isinstance(s,str) or not s for s in sheets) or len(set(sheets))!=len(sheets)):
            raise ValueError('Use exact unique sheet names; whitespace is significant')
    if set(names)!=set(cfg['workbooks']):raise ValueError('Every selected workbook needs a focused target')
    if cfg['diagnostics'] or C.reference_candidates(cfg):raise ValueError('Focused mode does not run reference sweeps')


def findings(state):
    result=[]
    for j in state['jobs']:
        m=j.get('metrics') or {}
        for w in m.get('workbooks') or []:
            for s in w.get('sheets') or []:
                row=dict(s,case_id=j['id'],workbook=w.get('workbook'),case_report=j.get('report'))
                matches=[x for x in m.get('canonical_sheets',[]) if x.get('sheet')==s.get('sheet') and x.get('sheet_index')==s.get('sheet_index')]
                row['canonical']=matches[0] if len(matches)==1 else None
                result.append(row)
    return result


def case_diagnostic(root, job):
    metrics=job.get('metrics') or {}
    receipt=job.get('supervisor') or {}
    local=root/'cases'/job['id']
    life=metrics.get('focus_lifecycle') or receipt.get('focus_lifecycle') or C.read_json(local/'capture-lifecycle.json',{})
    cause=receipt.get('timeout_kind') or life.get('timeout_kind')
    category=('timeout / '+cause) if cause else ('timeout / '+str(receipt.get('phase','unclassified'))) if job['status']=='timeout' else None
    if category is None:
        codes=[(s.get('error') or {}).get('code') for w in metrics.get('workbooks',[]) for s in w.get('sheets') or [] if isinstance(s.get('error'),dict)]
        if receipt.get('failure_category')=='report-io' or 'VV_SNAPSHOT_IO:' in str(job.get('error') or ''):category='infrastructure / report I/O'
        elif any(x in ('DRAWING_BOUND_UNRESOLVED','AXIS_GEOMETRY_BUDGET') for x in codes):category='geometry / drawing bounds'
        elif any(x and x.startswith('CALIBRATION_') for x in codes):category='calibration'
        elif job['status']=='complete':category='complete' if C.quality_fingerprint(job) else 'canonical pixels unverified'
        else:category=job['status']
    return {'case_id':job['id'],'status':job['status'],'category':category,'error':job.get('error'),
        'phase':life.get('phase') or receipt.get('phase'),'last_sheet':life.get('last_sheet'),
        'last_address':life.get('last_address'),'last_native_start':life.get('last_native_start'),
        'normal_images_received':life.get('normal_images_received'),'milestones':life.get('milestones'),
        'process':life.get('process')}


def render_focus(root,state):
    # Shared projections provide the CSV and current run metadata. The view below
    # is deliberately not the campaign's ranking/deployment recommendation.
    C.render_report(root,state)
    e=lambda x:html.escape(str(x),quote=True)
    rows=findings(state);jobs=state['jobs']
    done=sum(j['status']=='complete' for j in jobs);verified=sum(j['status']=='complete' and C.quality_fingerprint(j) is not None for j in jobs)
    bad=sum(j['status'] not in {'queued','running'} and (j['status']!='complete' or C.quality_fingerprint(j) is None) for j in jobs)
    ledger=''
    for j in jobs:
        ms=(j.get('metrics') or {}).get('workload_ms')
        scope=j['candidate'].get('sheet_names')
        diag=case_diagnostic(root,j)
        evidence=('<a href="'+e(j['report'])+'">Open workbook report</a>') if j.get('report') else 'Not started'
        for name,label in [('focus-request.json','Selection JSON'),('reproduce.ps1','Replay this case'),('capture-lifecycle.json','Phase / process evidence'),('capture-lifecycle.jsonl','Waiting timeline')]:
            if (root/'cases'/j['id']/name).is_file():evidence+='<br><a href="cases/'+e(j['id'])+'/'+name+'">'+label+'</a>'
        ledger+='<tr><td>'+e(j['workbook'])+'</td><td>'+e(', '.join(json.dumps(s,ensure_ascii=False) for s in scope) if scope else 'All visible worksheets')+'</td><td>'+e(j['candidate']['label'])+'<br>'+e(diag['category'])+'<small>'+e(j.get('error') or '')+'</small></td><td>'+('—' if ms is None else format(ms/1000,'.2f'))+'</td><td>'+evidence+'</td></tr>'
    details=''
    for r in rows:
        code=(r.get('error') or {}).get('code') or r.get('warning') or ''
        q=r.get('canonical') or {}
        details+='<tr><td>'+e(r['workbook'])+'</td><td>'+e(str(r.get('sheet_index'))+' · '+json.dumps(r.get('sheet'),ensure_ascii=False))+'</td><td>'+e(r.get('status'))+'</td><td>'+e(code)+'</td><td>'+e(q.get('status','unverified'))+('<br>'+e(q.get('error')) if q.get('error') else '')+'</td></tr>'
    p=state['config'].get('focus_parameters')
    scope_card=''
    if p:
        timeout_rows=''.join('<tr><td>'+e(k)+'</td><td>'+e(v)+'</td></tr>' for k,v in p.items())
        scope_card='<section class="card"><h2>Effective configuration and deadlines</h2><div class="table"><table>'+timeout_rows+'</table></div><p>Startup ends at measurement_started. Capture gets its own fixed allowance. Native-call, inactivity and absolute deadlines still apply. A heartbeat cannot extend the absolute budget.</p><p><a href="focus-request.json">Replay selection JSON</a> · <a href="reproduce.ps1">Replay script</a> · <a href="focus-resolved.json">Resolved configuration</a></p></section>'
    status=C.read_json(root/'bundle-status.json',{})
    bundle='<a class="button" href="review-metadata.zip">Review bundle · no images</a>' if status.get('status')=='ready' else '<span class="button">Review bundle · '+e(status.get('status','pending'))+'</span>'
    text='''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Focused capture checks · Excel Shrink</title><style>
body{margin:0;background:#f4f7fa;color:#172b42;font:15px/1.6 system-ui,sans-serif}header{background:#142c43;color:white;padding:32px max(24px,calc((100vw - 1400px)/2))}main{max-width:1400px;padding:24px;margin:auto}.card{background:white;border:1px solid #dce5ec;border-radius:12px;padding:24px;margin:20px 0}.table{overflow:auto}table{width:100%;border-collapse:collapse;text-align:left}td,th{padding:12px;border-bottom:1px solid #dce5ec;vertical-align:top;overflow-wrap:anywhere}th{background:#f7f9fb;color:#5c6f82;font-size:12px;text-transform:uppercase}a{color:#125b88;text-decoration:none}a:hover{text-decoration:underline}.button{display:inline-block;padding:10px 14px;background:white;border:1px solid #dce5ec;border-radius:8px;margin:5px}.note{border-left:4px solid #d99b2f;background:#fff7e7;padding:14px}small{color:#5c6f82}pre{white-space:pre-wrap;overflow-wrap:anywhere}h1{margin:4px 0}input{padding:10px;width:min(500px,85%);border:1px solid #dce5ec;border-radius:8px}</style></head><body><header><small style="color:#92c9ec">EXCEL SHRINK · FOCUSED REPAIR CHECKS</small><h1>Capture readiness</h1><div>RUNID · STATUS</div></header><main>
<nav>BUNDLE <a class="button" href="focus-findings.json">Structured findings</a><a class="button" href="decision.md">Interpretation</a><a class="button" href="../../index.html">Run history</a></nav>
<section class="card"><h2>Scope</h2><p>COUNTS</p><p class="note">This is a selected-sheet repair check, not full-corpus coverage or a speed ranking. An isolated worksheet failure stays failed. A native timeout stops that session. No hidden row or column is resized or made visible.</p><p>Configuration: .NET / Strategy F / Selective / 16 MP. Synthetic diagnostics, clipboard references, and the 16/32/64 MP matrix are intentionally not repeated.</p></section>
<section class="card"><h2>Workbooks</h2><div class="table"><table><thead><tr><th>Input</th><th>Requested worksheets</th><th>Result</th><th>Workload seconds</th><th>Evidence</th></tr></thead><tbody>LEDGER</tbody></table></div></section>
<section class="card"><h2>Worksheet outcomes</h2><input id="filter" placeholder="Filter by sheet, workbook or error"><div class="table"><table id="sheets"><thead><tr><th>Workbook</th><th>Sheet index and exact name</th><th>Capture</th><th>Issue</th><th>Canonical pixel check</th></tr></thead><tbody>DETAILS</tbody></table></div></section>
<section class="card"><h2>What remains undecided</h2><p>Strict geometry predictions and seam comparisons are not weakened. Failure records now include control rejection reasons, hidden flags, row geometry at two anchor columns and structured seam differences. Use these observations to repair the common capture layer before another strategy comparison.</p><p>Original capture PNGs and raw inputs remain unchanged. The metadata bundle omits images.</p></section></main><script>document.getElementById('filter').addEventListener('input',function(){const q=this.value.toLowerCase();document.querySelectorAll('#sheets tbody tr').forEach(r=>r.hidden=!r.textContent.toLowerCase().includes(q));});</script></body></html>'''
    for token,value in [('RUNID',e(state['run_id'])),('STATUS',e(state['status'])),('BUNDLE',bundle),('COUNTS',f'{len(jobs)} workbook checks · {done} capture-complete · {verified} canonically verified · {bad} with issues · {len(rows)} worksheet outcomes recorded'),('LEDGER',ledger),('DETAILS',details)]:text=text.replace(token,value)
    if p:
        text=text.replace('Configuration: .NET / Strategy F / Selective / 16 MP.', 'Configuration: '+e(p['host_kind'])+' / Strategy F / '+e(p['overflow_mode'])+' / '+str(p['tile_megapixels'])+' MP.')
        text=text.replace('<section class="card"><h2>Workbooks</h2>',scope_card+'<section class="card"><h2>Workbooks</h2>')
        text=text.replace('<h1>Capture readiness</h1>','<h1>Focused capture readiness</h1>')
        publish_replay_files(root,state)
        csv_path=root/'case-results.csv'
        with csv_path.open(encoding='utf-8-sig',newline='') as stream:
            reader=csv.DictReader(stream);columns=list(reader.fieldnames or []);csv_rows=list(reader)
        extra_columns=['target_id','requested_sheets','setup_timeout_seconds','startup_timeout_seconds','capture_timeout_seconds','native_call_timeout_seconds','postprocess_timeout_seconds','failure_category','last_sheet','last_address']
        for row,j in zip(csv_rows,jobs):
            diagnostic=case_diagnostic(root,j)
            row.update(target_id=j['candidate'].get('target_id'),requested_sheets=json.dumps(j['candidate'].get('sheet_names'),ensure_ascii=False),
                failure_category=diagnostic['category'],last_sheet=diagnostic['last_sheet'],last_address=diagnostic['last_address'])
            for key in extra_columns:
                if key in p:row[key]=p[key]
        with csv_path.open('w',encoding='utf-8-sig',newline='') as stream:
            writer=csv.DictWriter(stream,fieldnames=columns+extra_columns);writer.writeheader();writer.writerows(csv_rows)
    if state['config'].get('focus_comparison'):
        from focus_profiles import write_question_report
        card=write_question_report(root,state)
        text=text.replace('This is a selected-sheet repair check, not full-corpus coverage or a speed ranking.',
            'This profile compares only its declared question and selected worksheets, not full-corpus coverage.')
        text=text.replace('Synthetic diagnostics, clipboard references, and the 16/32/64 MP matrix are intentionally not repeated.',
            'Only the declared profile variants are run. Synthetic diagnostics and historical clipboard references are not repeated.')
        text=text.replace('Configuration: '+e(p['host_kind'])+' / Strategy F / '+e(p['overflow_mode'])+' / '+str(p['tile_megapixels'])+' MP.',
            'Base settings: '+e(p['host_kind'])+' / Strategy F / '+e(p['overflow_mode'])+' / '+str(p['tile_megapixels'])+' MP. Profile variants are listed below.')
        text=text.replace('<section class="card"><h2>Scope</h2>',card+'<section class="card"><h2>Scope</h2>',1)
    (root/'report.html').write_text(text,encoding='utf-8')
    lines=['# Focused capture repair checks','',f"Run: `{state['run_id']}`",f"Status: `{state['status']}`",'',
        '## Interpretation','', 'No deployment recommendation is selected by this run.',
        'Only the configured worksheet subset is tested. A completed case is not a full-workbook/corpus certificate.',
        'Model mismatches and inconsistent seams remain errors. Read-only worksheet failures may be isolated after a session-health check.',
        'Existing timing data can size timeouts only; it is not copied into new result rows.', '', '## Requested checks','']
    for t in state['config']['focused_targets']:lines+=['- '+t['workbook']+': '+t['question']]
    lines+=['','## Next step','', 'Review the small evidence bundle. Do not launch the full decision matrix to investigate a repeated shared calibration error.']
    (root/'decision.md').write_text('\n'.join(lines)+'\n',encoding='utf-8')
    C.atomic_json(root/'focus-findings.json',{'schema':'vv-focus-findings-v1','scope':'selected worksheets, not production coverage','jobs':[{k:j.get(k) for k in ('id','workbook','status','error','report','timeout_seconds')} for j in jobs],'sheets':rows,'case_diagnostics':[case_diagnostic(root,j) for j in jobs]})
    C.atomic_json(root/'decision-summary.json',{'schema':'vv-focus-summary-v1','status':state['status'],'deployment_selection':None,'completed_cases':done,'cases_with_issues':bad,'findings':rows})


def publish_replay_files(root,state):
    cfg=state['config'];p=cfg['focus_parameters']
    allowed=('host_kind','overflow_mode','tile_megapixels','context_validation','diagnostic_level',
        'setup_timeout_seconds','startup_timeout_seconds','capture_timeout_seconds','inactivity_timeout_seconds',
        'native_call_timeout_seconds','postprocess_timeout_seconds','session_timeout_seconds','repetitions',
        'wait_for_start','use_office_debug_js')
    request={k:p[k] for k in allowed if k in p}
    request.update(schema='vv-focus-request-v1',corpus_root=cfg['corpus_root'],
        targets=[{k:t[k] for k in ('id','workbook','sheets','question') if k in t} for t in cfg['focused_targets']])
    if cfg.get('focus_comparison'):
        request=copy.deepcopy(cfg['focus_request'])
        request['corpus_root']=cfg['corpus_root']
        varied=set().union(*(v.keys() for v in cfg['focus_comparison']['variants']))
        for key in varied:request.pop(key,None)
    C.atomic_json(root/'focus-request.json',request)
    C.atomic_json(root/'focus-resolved.json',cfg)
    script = """# Exact focused rerun; starts a NEW report, never overwrites evidence.
$here=$PSScriptRoot
while($here -and -not(Test-Path -LiteralPath (Join-Path $here 'docs/visual-validation/experiments/decision-campaign.ps1'))){$here=Split-Path -Parent $here}
if(-not $here){throw 'Run this replay script from its original local report directory.'}
& (Join-Path $here 'docs/visual-validation/experiments/decision-campaign.ps1') -Focused -SelectionFile (Join-Path $PSScriptRoot 'focus-request.json') -NewRun
"""
    (root/'reproduce.ps1').write_text(script,encoding='utf-8-sig')
    for job in state['jobs']:
        if not job.get('id'):continue
        case=root/'cases'/job['id']
        if not case.is_dir():continue
        one=copy.deepcopy(request)
        one.pop('profile',None)
        one['targets']=[{k:t[k] for k in ('id','workbook','sheets','question') if k in t}
            for t in cfg['focused_targets'] if t['id']==job['candidate'].get('target_id')]
        for key in ('host_kind','overflow_mode','tile_megapixels','context_validation','diagnostic_level'):
            one[key]=job['candidate'].get(key,p[key])
        if not one['targets']:continue
        one['repetitions']=1
        C.atomic_json(case/'focus-request.json',one)
        (case/'reproduce.ps1').write_text(script,encoding='utf-8-sig')


class FocusCampaign(C.Campaign):
    def __init__(self,repo,root,cfg,parent=0):
        validate_targets(cfg)
        super().__init__(repo,root,cfg,parent)
        self.state['run_kind']='capture-followup'
        self.state['limitations']=['Selected-sheet repair checks, not a strategy ranking.',
            'No calibration constant or pixel tolerance was relaxed.',
            'Capture and canonical pixel verification are separate outcomes.']
        self.save()
    def save(self):
        self.state['updated_utc']=C.utc();C.atomic_json(self.root/'campaign-state.json',self.state)
        render_focus(self.root,self.state)
    def baseline(self,f,stage):
        p=self.cfg.get('focus_parameters')
        if p:return {'seconds':p['workload_timeout_seconds'],'source':'explicit focused phase budgets; no historical six-minute/adaptive clamp','baseline_seconds':None}
        return super().baseline(f,stage)
    def circuit(self,c):
        return False  # six bounded, distinct questions; one geometry fault must not prune the rest
    def run(self):
        try:
            index={f['relative_path']:f for f in self.state['files']}
            p=self.cfg.get('focus_parameters')
            if p:
                from focus_profiles import planned_jobs
                for c,target,repeat in planned_jobs(self.cfg):
                    self.ensure_job('focus',c,index[target['workbook']],repeat)
            else:
                for target in self.cfg['focused_targets']:
                    c=C.candidate('dotnet','selective',16)
                    c.update(id='focused-dotnet-selective-16',sheet_names=target['sheets'],scope='Selected-sheet repair check',label='Capture repair / .NET / Selective / 16 MP')
                    self.ensure_job('focus',c,index[target['workbook']])
            self.save()
            for j in list(self.state['jobs']):
                if not self.has_time('focus'):break
                if self.cfg.get('focus_comparison',{}).get('recovery_to_16'):
                    self.run_policy('focus',j['candidate'],index[j['workbook']],j['round'])
                else:
                    self.run_job('focus',j['candidate'],index[j['workbook']],j['round'])
                if self.cfg.get('focus_parameters'):
                    from finalize import startup_failure
                    failed=startup_failure(j,C.read_json(self.root/'cases'/j['id']/'asset-preflight.json',{}))
                    if failed:
                        self.state['infrastructure_failure']=dict(failed,case_id=j['id'])
                        for remaining in self.state['jobs']:
                            if remaining['status']=='queued':remaining.update(status='blocked',error='Shared local web deployment failed; not executed')
                        break
            pending=[j for j in self.state['jobs'] if j['status']=='queued']
            self.state['status']='partial' if pending else ('complete-with-gaps' if any(j['status']!='complete' or C.quality_fingerprint(j) is None for j in self.state['jobs']) else 'complete')
            self.state['stages']['focus']='pending' if pending else 'complete'
        except KeyboardInterrupt:
            self.state['status']='interrupted';self.state['errors'].append({'time':C.utc(),'message':'User cancelled; completed evidence retained.'})
        except Exception as e:
            self.state['status']='interrupted';self.state['errors'].append({'time':C.utc(),'message':str(e),'trace':C.traceback.format_exc()})
            print(str(e),file=C.sys.stderr)
        finally:
            self.state['ended_utc']=C.utc();self.state['resumable']=self.state['status'] in {'partial','interrupted'}
            self.state['sessions'][-1].update(ended_utc=C.utc(),elapsed_seconds=time.monotonic()-self.clock)
            self.save();C.write_bundle(self.root,self.state,renderer=render_focus)
        return 0 if self.state['status']=='complete' else 2
