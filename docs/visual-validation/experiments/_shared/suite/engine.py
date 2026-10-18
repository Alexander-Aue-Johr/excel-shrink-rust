"""Sequential execution; the existing Windows supervisor owns process lifetimes."""
from __future__ import annotations
import copy,json,os,shutil,subprocess,time,traceback
from pathlib import Path
import common as C
from common import *
import report
SCHEMA='vv-evaluation-run-v1'

class Runner:
    def __init__(self,repo,root,plan,parent=0):
        self.repo,self.root=repo.resolve(),root.resolve();self.root.mkdir(parents=True,exist_ok=True)
        self.windows_parent=parent;self.clock=time.monotonic();self.current=None;self.cancelled=False
        self.specdir=self.repo/'docs/visual-validation/experiments/_shared/suite';self.plan=plan
        self.cfg={'capture':plan['capture'],'limits':dict(plan['limits'],session_seconds=plan['limits']['session_timeout_seconds'],minimum_free_gib=plan['minimum_free_gib'],postprocess_seconds=plan['limits']['postprocess_timeout_seconds']),
                  'native':plan['native'],'verify_pixels':True,'references':{'build_lab_host_if_needed':True}}
        source=C.source_manifest(self.repo);old=C.read_json(self.root/'evaluation-state.json')
        if old:
            if old.get('schema')!=SCHEMA or old['source_version']!=source['fingerprint'] or old['plan_hash']!=C.obj_digest(plan):raise ValueError('Source or plan changed; start a new run')
            self.state=old
            for j in self.state['jobs']:
                if j['status']=='running':j.update(status='cancelled',error='Previous session ended without a completion receipt')
                if j['status']=='cancelled':j['resume_pending']=True
        else:
            self.state={'schema':SCHEMA,'run_id':self.root.name,'started_utc':C.utc(),'mode':plan['mode'],'source_version':source['fingerprint'],'plan_hash':C.obj_digest(plan),
                        'plan':copy.deepcopy(plan),'files':copy.deepcopy(plan['files']),'jobs':copy.deepcopy(plan['jobs']),'errors':[],'sessions':[],'status':'running'}
            C.atomic_json(self.root/'source-manifest.json',source);C.atomic_json(self.root/'resolved-plan.json',plan)
        # Validate inputs on resumption as well; completed results cannot mask changed bytes.
        for f in self.state['files']:
            p=Path(f['source']);st=p.stat()
            if st.st_size!=f['bytes'] or st.st_mtime_ns!=f['mtime_ns'] or (f.get('sha256') and C.digest(p)!=f['sha256']):raise ValueError('Input changed: '+f['relative_path'])
        self.files={f['id']:f for f in self.state['files']};self.state['sessions'].append({'started_utc':C.utc(),'budget_seconds':self.cfg['limits']['session_seconds']})
        self.state['status']='running';self.save()
    def save(self):
        self.state['updated_utc']=C.utc();C.atomic_json(self.root/'evaluation-state.json',self.state);report.render(self.root,self.state)
    def time_left(self,stage):return max(0,self.cfg['limits']['session_seconds']-(time.monotonic()-self.clock))
    def has_time(self,stage):return self.time_left(stage)>=30
    def input_file(self,ident):return self.files[ident] if ident else None
    def ensure_job(self,stage,c,f,repeat=1):
        key=C.job_key(stage,c,f,repeat);found=next((j for j in self.state['jobs'] if j['key']==key),None)
        if found:return found
        j=C.make_job(stage,c,f,repeat);j['sequence']=len(self.state['jobs'])+1;j['id']=f"{j['sequence']:06d}-{c['id']}";self.state['jobs'].append(j);return j

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

    def run_job(self,stage,c,f=None,repeat=1,reference_job=None):
        j=self.ensure_job(stage,c,f,repeat)
        if not self.has_time(stage): return j
        if j.get('resume_pending'):
            prior=copy.deepcopy({k:v for k,v in j.items() if k not in {'prior_attempts','resume_pending'}})
            j.setdefault('prior_attempts',[]).append(prior)
            j['id']=f"{j['sequence']:06d}-{c['id']}-attempt{len(j['prior_attempts'])+1}"
            j.update(status='queued',metrics=None,case_seconds=None,report=None,error=None,quality=None,resume_pending=False)
        if j['status'] in TERMINAL: return j
        if shutil.disk_usage(self.root).free<self.cfg['limits']['minimum_free_gib']*1024**3:
            raise RuntimeError('Free-disk safety budget reached. No screenshots were deleted; queue retained.')
        source=self.stage_input(f) if f else None
        budget={'seconds':c['phase_limits']['workload_timeout_seconds'],'source':'Declared phase budgets','baseline_seconds':None}
        remaining=max(1,int(self.time_left(stage)))
        j['timeout_seconds']=budget['seconds'];j['timeout_basis']=budget
        case_root=self.root/'cases'/j['id'];case_root.mkdir(parents=True,exist_ok=True)
        j['report']='cases/'+j['id']+'/report.html';j['status']='running';j['started_utc']=utc()
        # Compact identities in directories; the source basename remains unchanged inside its per-file corpus.
        spec={'schema':'vv-decision-case-v1','repo_root':windows_path(self.repo),'suite_root':windows_path(self.root),
              'case_root':windows_path(case_root),'job':dict(j),'inputs':([{'name':f['name'],'sha256':f['sha256'],'bytes':f['bytes'],'source':windows_path(Path(f['source']))}] if f else []),
              'staged_corpus':windows_path(source.parent if source else self.repo/'corpus-local'),
              'capture':dict(c['settings']),
              'verify_final_pixels':self.cfg['verify_pixels'],'decision':True,'native':self.cfg['native'],
              'build_lab_host_if_needed':self.cfg['references']['build_lab_host_if_needed'],
              'workload_timeout_seconds':min(budget['seconds'],remaining),'postprocess_timeout_seconds':c['phase_limits']['postprocess_timeout_seconds'],
              'total_remaining_seconds':remaining,'controller_parent_pid':self.windows_parent,
              'reference_case_root':windows_path(self.root/'cases'/reference_job['id']) if reference_job else None,
              'release_token':os.urandom(16).hex(),'parent_pid':0}
        if c['kind']=='capture':
            spec['focus_parameters'] = dict(c['phase_limits'], **{k:c['settings'][k] for k in ('wait_for_start','use_office_debug_js','diagnostic_level')})
            spec['focus_target_id'] = c.get('block_id')
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
            if (metrics.get('report_publication') or {}).get('status')=='failed':j['status']='failed'
            j['error']=receipt.get('error') or metrics.get('invocation_error')
            j['case_seconds']=time.monotonic()-start;j['supervisor']=receipt
            if not receipt: j['status']='failed';j['error']='No supervisor completion receipt. See retained stdout/stderr.'
            if j['status']=='timeout':j['censored']=True
            if c['kind']=='capture' and c.get('strategy')=='F':
                j['quality']='verified' if quality_fingerprint(j) is not None else 'unverified'
            if receipt.get('safe_to_continue') is False: raise RuntimeError('Owned cleanup could not be established; suite paused. '+str(j['error']))
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
        fallback=copy.deepcopy(c)
        fallback['tile_megapixels']=16
        fallback['settings']=dict(c['settings'],tile_megapixels=16)
        fallback['recovery_enabled']=False
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

    def run(self):
        from archive import write_bundle
        from analysis import is_qualified
        try:
            prerequisite=None;refs={}
            for j in list(self.state['jobs']):
                if not self.has_time(j['stage']):break
                c=j['candidate'];f=self.input_file(j['input_id']);ref=refs.get((j['input_id'],j['round']))
                if c['kind'] in {'lab-capture','lab-memory','lab-rows'}:
                    if not prerequisite or not C.successful(prerequisite):j.update(status='blocked',error='Native helper prerequisite did not complete');self.save();continue
                    if c['kind']!='lab-rows' and (not ref or not C.successful(ref)):j.update(status='blocked',error='Complete reference bounds are missing');self.save();continue
                result=self.run_policy(j['stage'],c,f,j['round']) if c.get('recovery_enabled') else self.run_job(j['stage'],c,f,j['round'],reference_job=ref)
                if c['kind']=='lab-setup':prerequisite=result
                if j['stage']=='reference-bounds':refs[(j['input_id'],j['round'])]=result
                failure=startup_failure(result,C.read_json(self.root/'cases'/result['id']/'asset-preflight.json',{}))
                if failure:
                    self.state['errors'].append(dict(failure,message='Shared web assets unavailable',case_id=result['id']))
                    for pending in self.state['jobs']:
                        if pending['status']=='queued':pending.update(status='blocked',error='Shared web-asset deployment failed')
                    break
                time.sleep(2)
            pending=any(j['status']=='queued' for j in self.state['jobs'])
            bad=any(not C.successful(j) or (j['candidate']['kind']=='capture' and not is_qualified(j)) for j in self.state['jobs'])
            self.state['status']='partial' if pending else 'complete-with-gaps' if bad else 'complete'
        except KeyboardInterrupt:self.state['status']='interrupted';self.state['errors'].append({'message':'User interruption; completed evidence retained'})
        except Exception as exc:
            self.state['status']='interrupted';self.state['errors'].append({'message':str(exc),'trace':traceback.format_exc()});print('EVALUATION ERROR: '+str(exc),flush=True)
        finally:
            self.state['ended_utc']=C.utc();self.state['resumable']=self.state['status'] in {'partial','interrupted'}
            self.state['sessions'][-1].update(ended_utc=C.utc(),elapsed_seconds=time.monotonic()-self.clock)
            self.save();write_bundle(self.root,self.state,report.render)
        return 0 if self.state['status']=='complete' else 2

def startup_failure(job, probe=None):
    """Classify only local web deployment failures, never pixel/geometry failures."""
    if (isinstance(probe, dict) and probe.get('scope') == 'local-web-assets'
            and probe.get('status') == 'failed'
            and probe.get('code') == 'VV_ASSET_PREFLIGHT_FAILED'):
        return {'code': probe['code'], 'phase': probe.get('phase'),
                'route': probe.get('current'), 'reason': probe.get('error'),
                'evidence': 'asset-preflight.json'}
    metrics = job.get('metrics') or {}
    messages = [job.get('error'), metrics.get('invocation_error')]
    messages += [x for x in metrics.get('errors') or [] if isinstance(x, str)]
    known = ('capture-geometry.js', 'context-validation.js', 'safe-capture.js',
             'capture-policy.js', 'selective-overflow.js', 'mode.js')
    for message in messages:
        if not isinstance(message, str):
            continue
        if 'VV_ASSET_PREFLIGHT_FAILED:' in message:
            return {'code': 'VV_ASSET_PREFLIGHT_FAILED', 'reason': message,
                    'evidence': 'invocation_error'}
        if any('Could not load /' + name in message for name in known):
            return {'code': 'VV_LOCAL_SCRIPT_LOAD_FAILED', 'reason': message,
                    'evidence': 'invocation_error'}
    return None
