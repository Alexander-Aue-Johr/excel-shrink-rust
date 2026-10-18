"""Compact versioned evidence. No images, machine paths, tokens or cell values are copied."""
from __future__ import annotations
import csv,hashlib,html,json,os,re,shutil,statistics,tempfile,zipfile
from pathlib import Path
import common as C
from report import STYLE

def observation(job,group=None):
    c=job['candidate'];m=job.get('metrics') or {};effective=C.effective_metrics(job)
    canonical=[{k:s.get(k) for k in ('sheet','sheet_index','status','width','height','algorithm','sha256','tiles','overlap_rows_checked')} for s in effective.get('canonical_sheets') or []]
    env=[];sheets=[]
    for w in effective.get('workbooks') or []:
        x=w.get('environment') or {};env.append({k:x.get(k) for k in ('office','device_pixel_ratio','screen','browser')})
        sheets += [{k:s.get(k) for k in ('sheet','sheet_index','address','status')} for s in w.get('sheets') or []]
    value=C.policy_workload(job)
    return {'case_id':job['id'],'group':group or job['stage'],'variant':c.get('comparison_variant') or c['host_kind']+'-'+c['overflow_mode']+'-'+str(c['tile_megapixels']),
            'block':c.get('block_id',c.get('target_id',job.get('input_id'))),'workbook':Path(job['workbook']).name,'requested_sheets':c.get('sheet_names'),'round':job['round'],
            'status':job['status'],'metrics_status':m.get('status'),'host':c.get('host_kind'),'overflow':c.get('overflow_mode'),'tile_megapixels':c.get('tile_megapixels'),
            'workload_seconds':m['workload_ms']/1000 if m.get('workload_ms') is not None else None,'policy_seconds':value/1000 if value is not None else None,
            'elapsed_seconds':job.get('case_seconds'),'recovery_status':(job.get('recovery_policy') or {}).get('status'),'canonical_sheets':canonical,'sheets':sheets,'environment':env,
            'pixels_verified':C.quality_fingerprint(job) is not None,'capture_count':sum(w.get('capture_count',0) or 0 for w in m.get('workbooks') or []),
            'report_published':(m.get('report_publication') or {}).get('status')}

def write_record(root,record):
    root.mkdir(parents=True,exist_ok=True)
    C.atomic_json(root/'results.json',record)
    rows=record.get('observations',[]);fields=['source_run','case_id','group','block','variant','workbook','round','status','metrics_status','workload_seconds','policy_seconds','elapsed_seconds','pixels_verified','capture_count']
    with (root/'results.csv').open('w',newline='',encoding='utf-8') as f:
        writer=csv.DictWriter(f,fieldnames=fields);writer.writeheader()
        for r in rows:writer.writerow({k:r.get(k) for k in fields})
    e=lambda x:html.escape(str(x),quote=True)
    def fmt(x):return '—' if x is None else f'{x:.2f}'
    blocks=[]
    for group in dict.fromkeys(r['group'] for r in rows):
        observations=[r for r in rows if r['group']==group]
        table=[]
        for (block,variant) in sorted({(r['block'],r['variant']) for r in observations},key=str):
            obs=[r for r in observations if r['block']==block and r['variant']==variant]
            good=[r for r in obs if r['status']=='complete' and r['metrics_status']=='complete' and r['pixels_verified']]
            times=[r['policy_seconds'] for r in good if r['policy_seconds']is not None]
            hashes={C.obj_digest(r['canonical_sheets']) for r in good}
            # Hash comparison excludes tile counts: canonical pixels are layout-independent.
            hashes={C.obj_digest([(s['sheet_index'],s['sheet'],s['width'],s['height'],s['algorithm'],s['sha256']) for s in r['canonical_sheets']]) for r in good}
            table.append('<tr>'+''.join('<td>'+e(v)+'</td>' for v in (obs[0]['workbook'],variant,f'{len(good)}/{len(obs)}',fmt(statistics.median(times) if times else None),'equal' if len(good)>1 and len(hashes)==1 else 'not fully established'))+'</tr>')
        blocks.append('<section class="card"><h2>'+e(group)+'</h2><div class="table"><table><tr><th>Workbook scope</th><th>Configuration</th><th>Complete / recorded</th><th>Median s</th><th>Repeat pixels</th></tr>'+''.join(table)+'</table></div></section>')
    sources=''.join('<li>'+e(s['run_id'])+' · '+e(s.get('scope',''))+'<small>Source '+e(s.get('source_fingerprint'))+'; archive SHA-256 '+e(s.get('archive_sha256'))+'</small></li>' for s in record['sources'])
    ledger=''.join('<tr>'+''.join('<td>'+e(x)+'</td>' for x in (r['source_run'],r['case_id'],r['workbook'],json.dumps(r['requested_sheets'],ensure_ascii=False),r['variant'],r['round'],r['status'],fmt(r['policy_seconds'])))+'</tr>' for r in rows)
    decision=record.get('decision') or {};decisiontext=decision.get('text','No automatic change to the selected method. Review the measurements and completeness conditions.')
    (root/'index.html').write_text('<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Capture evidence · Excel Shrink</title><style>'+STYLE+'</style></head><body><header><div class="eyebrow">Excel Shrink · Evidence</div><h1>Capture-method evaluation</h1><p>'+e(record['id'])+'</p></header><main><nav><a class="button" href="results.json">Structured results</a><a class="button" href="results.csv">Measurements CSV</a><a class="button" href="manifest.json">Integrity manifest</a><a class="button" href="../../decisions.md">Method decision</a></nav><section class="card"><h2>Selected method</h2><p>'+e(decisiontext)+'</p><p class="note">'+e(record['scope'])+'</p></section>'+''.join(blocks)+'<details class="card"><summary>Individual observations ('+str(len(rows))+')</summary><div class="table"><table><tr><th>Run</th><th>Case</th><th>Workbook</th><th>Exact sheets</th><th>Variant</th><th>Round</th><th>Status</th><th>Policy s</th></tr>'+ledger+'</table></div></details><section class="card"><h2>Source records</h2><ul>'+sources+'</ul><p>Workload includes setup and Excel startup, capture, analysis and export; offline pixel checks and report generation are excluded. Missing and failed attempts remain in the ledger. Summary medians of successful cases are descriptive when a row is incomplete. Screenshots and original logs remain in the local run archive. No image bytes are redistributed here.</p></section></main></body></html>',encoding='utf-8')
    manifest={'schema':'vv-evidence-manifest-v1','id':record['id'],'status':'complete','files':[{'path':p.name,'bytes':p.stat().st_size,'sha256':C.digest(p)} for p in sorted(root.iterdir()) if p.is_file() and p.name!='manifest.json']}
    C.atomic_json(root/'manifest.json',manifest);verify_record(root)

def verify_record(root):
    m=C.read_json(root/'manifest.json')
    if not m or m.get('schema')!='vv-evidence-manifest-v1' or m.get('status')!='complete':raise ValueError('Missing evidence manifest')
    expected={x['path'] for x in m['files']}
    if expected!={p.name for p in root.iterdir() if p.is_file() and p.name!='manifest.json'}:raise ValueError('Evidence entry set differs')
    for row in m['files']:
        p=root/row['path']
        if p.name!=row['path'] or p.is_symlink() or not p.is_file() or p.stat().st_size!=row['bytes'] or C.digest(p)!=row['sha256']:raise ValueError('Evidence hash mismatch')
    return m

def index(evidence,ident):
    C.atomic_json(evidence/'current.json',{'schema':'vv-current-evidence-v1','id':ident,'report':ident+'/index.html','manifest':ident+'/manifest.json'})
    items=[]
    for d in sorted(evidence.iterdir(),reverse=True):
        if d.is_dir() and (d/'manifest.json').is_file():
            try:verify_record(d)
            except (OSError,ValueError):continue
            items.append('<li><a href="'+html.escape(d.name,quote=True)+'/index.html">'+html.escape(d.name)+'</a></li>')
    (evidence/'index.html').write_text('<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Capture evidence</title><style>'+STYLE+'</style><header><h1>Capture evidence</h1></header><main><section class="card"><p><a href="'+html.escape(ident,quote=True)+'/index.html">Current published report</a></p><ul>'+''.join(items)+'</ul><p><a href="../decisions.md">Method decision</a></p></section></main></html>',encoding='utf-8')

def publish(repo,run,replace=False):
    run=run.resolve();managed=(repo/'docs/visual-validation/results/runs').resolve()
    if run.parent!=managed or not re.fullmatch(r'[A-Za-z0-9_-]+',run.name):raise ValueError('Select an immediate managed run directory')
    state=C.read_json(run/'evaluation-state.json')
    if not state or state.get('schema')!='vv-evaluation-run-v1' or state.get('status') in {'running','queued'}:raise ValueError('Only finished evaluation records can be published')
    archive=run/'review-metadata.zip'
    with zipfile.ZipFile(archive) as z:
        if len(z.namelist())!=len(set(z.namelist())):raise ValueError('Duplicate archive paths')
        m=json.loads(z.read('bundle-manifest.json'))
        if m.get('status')!='complete':raise ValueError('Incomplete metadata manifest')
        rows=m.get('included',[])
        if {x['path'] for x in rows}|{'bundle-manifest.json'}!=set(z.namelist()):raise ValueError('Archive entry set differs')
        for x in rows:
            data=z.read(x['path'])
            if len(data)!=x['stored_bytes'] or hashlib.sha256(data).hexdigest()!=x['sha256']:raise ValueError('Archive content mismatch')
        if z.read('evaluation-state.json')!=(run/'evaluation-state.json').read_bytes():raise ValueError('Published state differs from the verified archive')
    obs=[]
    for j in state['jobs']:
        if j['candidate']['kind']!='capture':continue
        obs.append(dict(observation(j),source_run=state['run_id']))
    record={'schema':'vv-published-evidence-v1','id':run.name,'scope':'Current protocol execution on unchanged inputs. This is not a production regression certificate. Status: '+state['status'],
            'sources':[{'run_id':state['run_id'],'source_fingerprint':state['source_version'],'archive_sha256':C.digest(archive),'state_sha256':C.digest(run/'evaluation-state.json'),'scope':state['plan']['protocol_id']}],
            'observations':obs,'decision':{'text':'Office.js / Strategy F / .NET / Selective / 32 MP. Updated observations do not automatically amend decisions.md.'},'summary':__import__('analysis').summarize(state)}
    # Summary contains no machine process data; observations are explicitly allowlisted.
    record['summary']={k:record['summary'][k] for k in ('schema','status','groups','selected_policy','decision_policy','scope','timing_note')}
    dest=repo/'docs/visual-validation/evidence'/run.name;dest.parent.mkdir(parents=True,exist_ok=True)
    if dest.exists():raise ValueError('Evidence ID already exists; prior published evidence is immutable')
    staging=Path(tempfile.mkdtemp(prefix='.publish-',dir=dest.parent))
    try:write_record(staging,record);os.replace(staging,dest);index(dest.parent,dest.name)
    finally:
        if staging.exists():shutil.rmtree(staging)
    return dest
