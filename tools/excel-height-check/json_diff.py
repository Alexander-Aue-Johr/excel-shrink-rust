"""Select XLSX input and profile, shrink, collect Excel metadata, open VS Code diff."""
import argparse, hashlib, json, os, shutil, subprocess, time, uuid
from pathlib import Path
from run import ROOT, HERE, PROFILES, windows_path, linux_context

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--input', help='File or directory; omitted: Windows picker')
    p.add_argument('--profile', choices=PROFILES, default='default')
    p.add_argument('--no-diff', action='store_true', help='Collect only; useful for unattended checks')
    a=p.parse_args()
    run=ROOT/'.tmp/excel-json-diff'/(time.strftime('%Y%m%d-%H%M%S')+'-'+uuid.uuid4().hex[:8])
    run.mkdir(parents=True)
    def ps(script,*args):
        return subprocess.run(['powershell.exe','-NoProfile','-STA','-ExecutionPolicy','Bypass','-File',windows_path(HERE/script),*args])
    if not a.input:
        (run/'profiles.json').write_text(json.dumps(list(PROFILES)),encoding='utf-8')
        result=ps('select.ps1','-Destination',windows_path(run/'selection.json'),'-ProfilesFile',windows_path(run/'profiles.json'))
        if result.returncode==3:
            print('Cancelled.');return
        result.check_returncode()
        selection=json.loads((run/'selection.json').read_text(encoding='utf-8'))
        a.input=selection['path'];a.profile=selection['profile']
    source=a.input
    if os.name!='nt' and ('\\' in source or (len(source)>1 and source[1]==':')):
        source=subprocess.check_output(['wslpath','-u',source],text=True).strip()
    source=Path(source).expanduser().resolve()
    files=([source] if source.is_file() else sorted(source.glob('*.xlsx')))
    files=[f for f in files if f.suffix.lower()=='.xlsx' and not f.name.startswith('~$')]
    if not files:raise RuntimeError('No XLSX files selected (folders are not recursive)')
    prefix,linux_root=linux_context()
    cargo=(subprocess.check_output(prefix+['printenv','HOME'],text=True).strip()+'/.cargo/bin/cargo') if os.name=='nt' else str(Path.home()/'.cargo/bin/cargo')
    subprocess.run(prefix+[cargo,'build','--release','--locked','-p','excel-shrink-cli'],cwd=ROOT,check=True)
    (run/'input').mkdir();(run/'shrunk').mkdir()
    manifest={'profile':a.profile,'flags':PROFILES[a.profile],'files':[]}
    for i,f in enumerate(files):
        ident=f'{i:04d}'
        shutil.copy2(f,run/'input'/f'{ident}.xlsx')
        manifest['files'].append({'id':ident,'name':f.name,'source':str(f),'sha256':hashlib.sha256((run/'input'/f'{ident}.xlsx').read_bytes()).hexdigest()})
    linux_run=linux_root+'/'+run.relative_to(ROOT).as_posix()
    start=time.perf_counter()
    with (run/'shrink.log').open('wb') as log:
        subprocess.run(prefix+[linux_root+'/target/release/excel-shrink-cli',linux_run+'/input',linux_run+'/shrunk',*PROFILES[a.profile]],cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,check=True)
    manifest['shrink_seconds']=time.perf_counter()-start
    manifest['helper_sha256']=hashlib.sha256((HERE/'HeightProbeHost.xlsm').read_bytes()).hexdigest()
    (run/'manifest.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')
    ps('snapshot.ps1','-RunRoot',windows_path(run)).check_returncode()
    # Canonical formatting and ordering: no timestamps or variant paths in the diff.
    for name in ('original','shrunk'):
        path=run/(name+'.json')
        data=json.loads(path.read_text(encoding='utf-8-sig'))
        if len(data)!=len(files):raise RuntimeError('Incomplete workbook collection')
        path.write_text(json.dumps(data,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(f'JSON comparison: {run}',flush=True)
    if not a.no_diff:
        code=shutil.which('code') or shutil.which('code.cmd')
        if not code and os.name=='nt':
            candidate=Path(os.environ.get('LOCALAPPDATA',''))/'Programs/Microsoft VS Code/bin/code.cmd'
            if candidate.exists():code=str(candidate)
        if not code:raise RuntimeError(f'VS Code CLI not found. Open original.json and shrunk.json from {run} using Compare Selected.')
        subprocess.run([code,'--reuse-window','--diff',str(run/'original.json'),str(run/'shrunk.json')],check=True)

if __name__=='__main__':main()
