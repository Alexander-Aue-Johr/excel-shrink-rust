"""Validated metadata bundle publication."""
from pathlib import Path
import os,json,hashlib,zipfile
from common import atomic_json,safe_walk,utc
TEXT_SUFFIXES={".json",".jsonl",".txt",".log",".csv",".md",".html",".css",".js",".xml"}
def write_bundle(root: Path, state: dict, renderer=None) -> None:
    if renderer is None: raise ValueError("Report renderer required")
    destination=root/'review-metadata.zip';tmp=root/('review-metadata.zip.'+os.urandom(6).hex()+'.new')
    atomic_json(root/'bundle-status.json',{'status':'building','updated_utc':utc()});renderer(root,state)
    included=[];omitted=[]
    try:
        files=list(safe_walk(root))
        with zipfile.ZipFile(tmp,'x',compression=zipfile.ZIP_DEFLATED,compresslevel=6,allowZip64=True) as z:
            for p in files:
                rel=p.relative_to(root).as_posix()
                if p.name.startswith('review') and '.zip' in p.name:continue
                if p.name.startswith('.') or p.suffix in {'.new','.previous'} or 'recovery-backup' in p.parts:continue
                if p.suffix.lower() not in TEXT_SUFFIXES and p.name != 'reproduce.ps1':
                    omitted.append({'path':rel,'bytes':p.stat().st_size,'reason':'binary-artifact'});continue
                data=p.read_bytes()
                if p.name=='report.html':data=data.replace(b'<body>',b'<body><aside style="padding:12px;background:#fff3cd">Metadata review copy. Images are intentionally omitted.</aside>',1)
                z.writestr(rel,data);included.append({'path':rel,'stored_bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()})
            manifest={'schema':'vv-review-metadata-v2','status':'complete','run_id':root.name,'images_included':False,'included':included,'omitted':omitted,
                      'required':['run.json','report.html','evaluation-state.json','resolved-plan.json','source-manifest.json','case-results.csv','summary.json']}
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
        renderer(root,state)
        print('REVIEW BUNDLE (NO IMAGES): '+str(destination),flush=True)
    except Exception as error:
        atomic_json(root/'bundle-status.json',{'status':'failed','updated_utc':utc(),'message':str(error),'temporary':str(tmp)})
        renderer(root,state);raise
