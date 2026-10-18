"""Resolve exact scopes, pinned inputs and a bounded, blocked experimental design."""
from __future__ import annotations
import copy,json,re,zipfile
from pathlib import Path
import xml.etree.ElementTree as ET
import common as C

EXPERIMENTS={'range-image':'Office.js capture','vba-picture':'VBA CopyPicture / chart',
 'shape-bounds':'Drawing bounds','units':'Renderer units','markers':'Renderer markers','row-heights':'Row-height geometry',
 'height-plateau':'Renderer height plateau','native-bitmap':'VBA-local GDI+ bitmap','native-emf':'VBA-local GDI+ EMF',
 'native-memory':'VBA pair / memory','native-png-memory':'VBA pair / PNG + memory','native-rows':'VBA row heights'}
BOUNDS={'setup_timeout_seconds':(30,1800),'startup_timeout_seconds':(10,1800),'capture_timeout_seconds':(30,7200),
 'native_call_timeout_seconds':(10,600),'inactivity_timeout_seconds':(10,1800),'postprocess_timeout_seconds':(30,1800),'session_timeout_seconds':(60,86400)}

def sheet_catalogue(path):
    with zipfile.ZipFile(path) as z:
        entry=z.getinfo('xl/workbook.xml')
        if entry.file_size>4194304:raise ValueError('Workbook catalogue exceeds 4 MiB')
        raw=z.read(entry)
    if b'<!DOCTYPE' in raw.replace(b'\0',b'').upper() or b'<!ENTITY' in raw.replace(b'\0',b'').upper():raise ValueError('DTD/entity in workbook catalogue')
    doc=ET.fromstring(raw);sheets=doc.find('{*}sheets')
    if sheets is None:raise ValueError('No worksheet catalogue')
    rows=[{'index':i+1,'name':s.get('name'),'state':s.get('state','visible'),'sheet_id':s.get('sheetId')} for i,s in enumerate(sheets)]
    if not rows or any(not r['name'] for r in rows) or len({r['name'] for r in rows})!=len(rows):raise ValueError('Invalid worksheet catalogue')
    return rows

def select_sheets(catalog,names=None,indexes=None):
    if names is not None and indexes is not None:raise ValueError('Use exact names OR indexes')
    if names is not None:
        if not isinstance(names,list) or not names or len(set(names))!=len(names) or any(not isinstance(x,str) or not x for x in names):raise ValueError('Expected unique exact worksheet names')
        missing=set(names)-{r['name'] for r in catalog}
        if missing:raise ValueError('Unknown worksheet names: '+json.dumps(sorted(missing),ensure_ascii=False))
        wanted=set(names)
    elif indexes is not None:
        if not isinstance(indexes,list) or not indexes or len(set(indexes))!=len(indexes) or any(type(i)is not int or i<1 or i>len(catalog) for i in indexes):raise ValueError('Invalid 1-based worksheet indexes')
        wanted={r['name'] for r in catalog if r['index'] in indexes}
    else:wanted={r['name'] for r in catalog if r['state']=='visible'}
    if any(r['state']!='visible' and r['name'] in wanted for r in catalog):raise ValueError('Hidden worksheets are not unhidden by the experiment')
    out=[r['name'] for r in catalog if r['name'] in wanted]
    if not out:raise ValueError('No visible worksheets selected')
    return out

def limits(base,override=None):
    out=dict(base);override=override or {}
    if set(override)-set(BOUNDS):raise ValueError('Unknown deadline fields')
    out.update(override)
    for k,(lo,hi) in BOUNDS.items():
        if type(out.get(k))is not int or not lo<=out[k]<=hi:raise ValueError(k+' out of range')
    out['workload_timeout_seconds']=sum(out[k] for k in ('setup_timeout_seconds','startup_timeout_seconds','capture_timeout_seconds'))+30
    return out

def settings(base,extra=None):
    out={'strategy':'F','host_kind':'dotnet','overflow_mode':'selective','tile_megapixels':32,'batch_size':1,'tile_packing':'budget','excel_launch_mode':'excel-x',
         'context_validation':'off','diagnostic_level':'normal','wait_for_start':False,'use_office_debug_js':False,'overflow_probe_width_px':1920,'overflow_probe_height_px':1080}
    for patch in (base,extra or {}):
        if set(patch)-set(out):raise ValueError('Unknown capture setting')
        out.update(patch)
    for k,allowed in {'strategy':set('ABCDEF'),'host_kind':{'dotnet','rust'},'overflow_mode':{'heuristic','selective'},'tile_megapixels':{16,32,64},'batch_size':{1,2,4,8,16,32,64},'tile_packing':{'budget','balanced'},'excel_launch_mode':{'excel-x','rust-com'},'context_validation':{'off','first-seam'},'diagnostic_level':{'normal','detailed'}}.items():
        if type(out[k])is bool or out[k] not in allowed:raise ValueError('Invalid '+k)
    for k in ('overflow_probe_width_px','overflow_probe_height_px'):
        if type(out[k])is not int or not 64<=out[k]<=8192:raise ValueError('Invalid '+k)
    for k in ('wait_for_start','use_office_debug_js'):
        if type(out[k])is not bool:raise ValueError('Invalid '+k)
    if out['host_kind']!='rust' and out['excel_launch_mode']=='rust-com':raise ValueError('rust-com requires the Rust host')
    return out

def find_workbook(value,files):
    if not isinstance(value,str) or not value:raise ValueError('Select a workbook explicitly')
    normalized=value.replace('\\','/')
    found=[f for f in files if normalized in (f['relative_path'],f['name'],str(f['source']).replace('\\','/'))]
    if not found and re.match(r'^[A-Za-z]:|^\\\\',value) and C.os.name!='nt':
        target=str(C.local_path(value).resolve());found=[f for f in files if str(Path(f['source']).resolve())==target]
    if len(found)!=1:raise ValueError('Workbook is missing or ambiguous: '+value)
    return found[0]

def resolve(repo,cfg,request=None):
    if cfg.get('schema')!='vv-evaluation-config-v1':raise ValueError('Unknown evaluation config')
    req=copy.deepcopy(request or {});mode=req.get('mode','suite')
    if req.get('schema','vv-experiment-request-v1')!='vv-experiment-request-v1' or mode not in {'suite','single'}:raise ValueError('Invalid request schema or mode')
    allowed={'schema','mode','groups','experiment','implementation','corpus_root','workbook','sheets','sheet_indexes','targets','repetitions','settings','limits'}
    if set(req)-allowed:raise ValueError('Unknown request fields: '+','.join(sorted(set(req)-allowed)))
    if mode=='suite' and set(req)&{'experiment','implementation','workbook','sheets','sheet_indexes','targets','settings'}:raise ValueError('The reproduction suite uses its declared scopes and variants; use the single-experiment mode to change them')
    root=req.get('corpus_root',cfg['corpus_root'])
    if C.os.name!='nt' and re.match(r'^[A-Za-z]:|^\\\\',str(root)):root=C.local_path(str(root))
    root=Path(root);root=root if root.is_absolute() else repo/root
    # Fixture-only diagnostics do not need a local corpus.
    fixture=mode=='single' and req.get('experiment') in {'units','markers','row-heights'}
    files=[] if fixture else C.discover(repo,{'corpus_root':str(root),'workbooks':['*']})
    base_limits=limits(cfg['limits'],req.get('limits'));base_settings=settings(cfg['capture'],req.get('settings'))
    reps=req.get('repetitions',cfg['repetitions'] if mode=='suite' else 1)
    if type(reps)is not int or not 1<=reps<=5:raise ValueError('Repetitions must be 1..5')
    if type(cfg.get('minimum_free_gib')) not in (int,float) or not 0<=cfg['minimum_free_gib']<=1024:raise ValueError('Invalid disk reserve')
    jobs=[];groups=[];blocks=[];catalogues={};selected_files={}
    def target_info(t):
        f=find_workbook(t['workbook'],files);selected_files[f['id']]=f
        if f['id'] not in catalogues:catalogues[f['id']]=sheet_catalogue(Path(f['source']))
        ss=select_sheets(catalogues[f['id']],t.get('sheets'),t.get('sheet_indexes'))
        return f,ss
    def add(group,t,s,r=1,kind='capture',extra=None,phase_limits=None,recovery=False):
        f,ss=target_info(t) if t.get('workbook') else (None,None)
        tid=t['id'];variant=(extra or {}).get('native_id') or s['host_kind']+'-'+s['overflow_mode']+'-'+str(s['tile_megapixels'])
        c=C.candidate(s['host_kind'],s['overflow_mode'],s['tile_megapixels']);c.update(extra or {})
        c.update(kind=kind,strategy=s['strategy'],id=group+'-'+C.obj_digest([tid,variant,kind,extra])[:10],comparison_variant=variant,block_id=tid,sheet_names=ss,settings=copy.deepcopy(s),phase_limits=phase_limits or base_limits,
                 context_validation=s['context_validation'],diagnostic_level=s['diagnostic_level'],recovery_enabled=recovery)
        j=C.make_job(group,c,f,r);j['phase']='measured';j['sequence']=len(jobs)+1;j['id']=f"{j['sequence']:06d}-{c['id']}";jobs.append(j)
        if not any(b['id']==tid for b in blocks):blocks.append(dict(id=tid,workbook=f['relative_path'] if f else None,sheets=ss))
    if mode=='suite':
        chosen=req.get('groups',[g['id'] for g in cfg['groups']])
        if not isinstance(chosen,list) or not chosen or len(set(chosen))!=len(chosen) or set(chosen)-{g['id'] for g in cfg['groups']}:raise ValueError('Invalid comparison groups')
        for g in cfg['groups']:
            if g['id'] not in chosen:continue
            groups.append({'id':g['id'],'description':g['description'],'question':g.get('question','capture')})
            lim=limits(cfg['limits'],dict(g.get('limits',{}),**req.get('limits',{})))
            for r in range(1,(g.get('repetitions',reps))+1):
                for i,t in enumerate(g['targets']):
                    variants=g['variants'];shift=(r-1+i)%len(variants)
                    for v in variants[shift:]+variants[:shift]:add(g['id'],t,settings(base_settings,v),r,phase_limits=lim,recovery=g.get('recovery_16mp',False))
        expected={p['path']:p for p in cfg['inputs']}
        for f in selected_files.values():
            pin=expected.get(f['relative_path'])
            if not pin or f['bytes']!=pin['bytes'] or C.digest(Path(f['source']))!=pin['sha256']:raise ValueError('Input does not match the pinned reproduction corpus: '+f['relative_path'])
            f['sha256']=pin['sha256']
    else:
        kind=req.get('experiment','range-image')
        if kind not in EXPERIMENTS:raise ValueError('Unknown individual experiment')
        groups=[{'id':'individual','description':EXPERIMENTS[kind],'question':'capture' if kind=='range-image' else 'reference'}]
        if fixture:
            add('individual',{'id':kind},base_settings,kind='diagnostic',extra={'suite':kind,'native_id':kind})
        else:
            if kind=='height-plateau':targets=[{'id':'height-plateau','workbook':'krankenhausverzeichnis-3500100227005.xlsx'}]
            elif req.get('targets'):
                if set(req)&{'workbook','sheets','sheet_indexes'}:raise ValueError('Targets and one-workbook arguments are mutually exclusive')
                targets=req['targets']
            elif req.get('workbook') in {'all','*'}:
                if req.get('sheets') or req.get('sheet_indexes'):raise ValueError('Select one workbook for worksheet selection')
                targets=[{'id':f['id'],'workbook':f['relative_path']} for f in files]
            else:targets=[{'id':'selected','workbook':req.get('workbook'),'sheets':req.get('sheets'),'sheet_indexes':req.get('sheet_indexes')}]
            if not isinstance(targets,list) or not 1<=len(targets)<=10000:raise ValueError('Invalid targets')
            for t in targets:
                if not isinstance(t,dict) or set(t)-{'id','workbook','sheets','sheet_indexes'} or not re.fullmatch('[a-z0-9-]{1,64}',str(t.get('id',''))):raise ValueError('Invalid target identity')
                if kind in {'vba-picture','shape-bounds','height-plateau','native-rows'} and (t.get('sheets') or t.get('sheet_indexes')):raise ValueError('This native/diagnostic adapter measures its full declared workbook scope, not selected sheets')
            if kind.startswith('native-'):
                groups.insert(0,{'id':'prerequisites','description':'Build or reuse the declared VBA helper','question':'reference'})
                add('prerequisites',{'id':'helper'},base_settings,kind='lab-setup',extra={'native_id':'lab-setup'})
                if kind!='native-rows':groups.insert(1,{'id':'reference-bounds','description':'Obtain complete selected F bounds for native comparison','question':'capture'})
            for r in range(1,reps+1):
                for t in targets:
                    if kind=='range-image':add('individual',t,base_settings,r)
                    elif kind=='vba-picture':add('individual',t,base_settings,r,kind='vba-picture',extra={'native_id':'vba-picture'})
                    elif kind=='shape-bounds':
                        impl=req.get('implementation','native');table={'native':('shape-native',None),'vsto':('shape-vsto',None),'officejs-dotnet':('shape-officejs','dotnet'),'officejs-rust':('shape-officejs','rust')}
                        if impl not in table:raise ValueError('Unknown drawing implementation')
                        k,h=table[impl];s=dict(base_settings)
                        if h:s['host_kind']=h
                        add('individual',t,s,r,kind=k,extra={'native_id':'shapes-'+impl})
                    elif kind=='height-plateau':add('individual',t,base_settings,r,kind='diagnostic',extra={'suite':kind,'native_id':kind})
                    else:
                        if kind!='native-rows':add('reference-bounds',t,settings(base_settings,{'strategy':'F','host_kind':'dotnet','overflow_mode':'selective','excel_launch_mode':'excel-x'}),r)
                        k='lab-rows' if kind=='native-rows' else 'lab-memory' if kind in {'native-memory','native-png-memory'} else 'lab-capture'
                        ident={'native-bitmap':'lab-bitmap','native-emf':'lab-emf','native-memory':'lab-memory','native-png-memory':'lab-disk-pair','native-rows':'lab-rows'}[kind]
                        add('individual',t,base_settings,r,kind=k,extra={'native_id':ident})
    if len({j['key'] for j in jobs})!=len(jobs):raise ValueError('Duplicate case identity')
    required=max((sum(j['candidate']['phase_limits'][k] for k in ('setup_timeout_seconds','startup_timeout_seconds','capture_timeout_seconds','postprocess_timeout_seconds'))+60 for j in jobs),default=0)
    if base_limits['session_timeout_seconds']<required:raise ValueError('Session limit is smaller than one fully budgeted case')
    return {'schema':'vv-resolved-evaluation-v1','mode':mode,'protocol_id':cfg['protocol_id'],'corpus_root':str(root),'files':list(selected_files.values()),'catalogues':catalogues,'groups':groups,'blocks':blocks,'jobs':jobs,
            'limits':base_limits,'capture':base_settings,'minimum_free_gib':cfg['minimum_free_gib'],'native':cfg['native'],'selected_policy':cfg['selected_policy'],'decision_policy':cfg['decision_policy']}
