"""Inspect or execute the declared visual-capture protocol."""
from __future__ import annotations
import argparse,json,sys
from pathlib import Path
import common as C
import plan as P

def main(argv=None):
    if sys.version_info<(3,10):raise RuntimeError('Python 3.10 or newer is required')
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--repo-root',type=Path,required=True)
    for key in ('config','request','resolved','write-resolved','run-root','publish-run'):p.add_argument('--'+key,type=Path)
    p.add_argument('--parent-pid',type=int,default=0)
    for key in ('plan','list-sheets','list-experiments','report-only','full-metadata'):p.add_argument('--'+key,action='store_true')
    a=p.parse_args(argv);repo=a.repo_root.resolve()
    if a.full_metadata and not a.report_only:p.error('--full-metadata requires --report-only')
    if a.list_experiments:print(json.dumps([dict(id=k,label=v) for k,v in P.EXPERIMENTS.items()],indent=2));return 0
    if a.publish_run:
        from publication import publish
        print('PUBLISHED EVIDENCE: '+str(publish(repo,a.publish_run)));return 0
    if a.report_only:
        if not a.run_root:p.error('--run-root is required')
        state=C.read_json(a.run_root/'evaluation-state.json')
        if not state or state.get('schema')!='vv-evaluation-run-v1':raise ValueError('Not an evaluation run')
        if state.get('status')=='running':state['status']='interrupted';state['resumable']=True;C.atomic_json(a.run_root/'evaluation-state.json',state)
        import report
        from archive import write_bundle
        report.render(a.run_root,state);write_bundle(a.run_root,state,report.render,detail='full' if a.full_metadata else 'compact');return 0
    cfg=C.read_json(a.config or repo/'docs/visual-validation/experiments/reproduce.json');req=C.read_json(a.request,{}) if a.request else {}
    if a.list_sheets:
        root=req.get('corpus_root',cfg['corpus_root'])
        if C.os.name!='nt' and P.re.match(r'^[A-Za-z]:|^\\\\',str(root)):root=C.local_path(str(root))
        root=Path(root);root=root if root.is_absolute() else repo/root
        f=P.find_workbook(req.get('workbook'),C.discover(repo,{'corpus_root':str(root),'workbooks':['*']}));rows=P.sheet_catalogue(Path(f['source']))
        if a.write_resolved:C.atomic_json(a.write_resolved,rows)
        print(json.dumps(rows,ensure_ascii=False,indent=2));return 0
    resolved=C.read_json(a.resolved) if a.resolved else P.resolve(repo,cfg,req)
    if not resolved or resolved.get('schema')!='vv-resolved-evaluation-v1':raise ValueError('Invalid resolved plan')
    if a.write_resolved:C.atomic_json(a.write_resolved,resolved)
    if a.plan:
        print(json.dumps(dict(mode=resolved['mode'],protocol=resolved['protocol_id'],case_count=len(resolved['jobs']),workbooks=len(resolved['files']),
          session_timeout_seconds=resolved['limits']['session_timeout_seconds'],jobs=[dict(sequence=j['sequence'],group=j['stage'],workbook=j['workbook'],sheets=j['candidate']['sheet_names'],variant=j['candidate']['comparison_variant'],round=j['round'],limits=j['candidate']['phase_limits']) for j in resolved['jobs']]),ensure_ascii=False,indent=2));return 0
    if not a.run_root:p.error('--run-root is required except when inspecting the plan')
    from engine import Runner
    return Runner(repo,a.run_root,resolved,a.parent_pid).run()
if __name__=='__main__':
    try:sys.exit(main())
    except Exception as e:print('EVALUATION ERROR: '+str(e),file=sys.stderr);sys.exit(1)
