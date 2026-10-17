"""Final bounded qualification: common readiness first, then existing candidate race.
No clipboard/renderer-history reruns and no old timings promoted to new observations.
"""
from __future__ import annotations
import copy
import time
import campaign as C


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


def readiness(job):
    if not C.successful(job) or C.quality_fingerprint(job) is None:
        return {'ready':False,'reason':'Incomplete capture or canonical pixel evidence'}
    m=job.get('metrics') or {}
    for w in m.get('workbooks',[]):
        if w.get('context_policy')!='visible-cell-halo-v1':
            return {'ready':False,'reason':'Context policy not recorded'}
        checks={x['sheet']:x for x in w.get('context_validations') or []}
        for sheet in m.get('canonical_sheets') or []:
            if sheet.get('workbook')!=w.get('workbook') or sheet.get('tiles',0)<2:continue
            check=checks.get(sheet.get('sheet')) or {}
            if check.get('status')!='complete' or not check.get('full_comparison',{}).get('equal') or not check.get('repeat_comparison',{}).get('equal') or check.get('overlap_mismatches',1):
                return {'ready':False,'reason':'Missing or failed whole-versus-tiles reference check on '+str(sheet.get('sheet'))}
    return {'ready':True,'reason':'Complete capture, strict canonical seams, and all required local reference checks'}


class FinalCampaign(C.Campaign):
    def __init__(self,repo,root,cfg,parent=0):
        if cfg.get('diagnostics') or C.reference_candidates(cfg):
            raise ValueError('Final qualification does not rerun diagnostic/native historical matrices')
        super().__init__(repo,root,cfg,parent)
        self.state['run_kind']='capture-finalization'
        self.state.setdefault('readiness',{})
        note='The readiness pass contains extra whole-versus-tile reference pictures and is not used to rank performance. The candidate race measures the normal contextual pipeline.'
        if note not in self.state['limitations']:self.state['limitations'].append(note)
        self.save()
    def time_left(self,stage):
        remaining=super().time_left(stage)
        if stage=='readiness':
            used=sum((j.get('case_seconds') or 0)+sum(a.get('case_seconds') or 0 for a in j.get('prior_attempts',[])) for j in self.state['jobs'] if j['stage']==stage)
            return min(remaining,max(0,self.cfg['limits']['readiness_seconds']-used))
        return remaining
    def stage_readiness(self):
        files=[self.files[i] for i in self.state['screen_ids']]
        # Known seam and height cases first, followed by the remaining screened inputs.
        files.sort(key=lambda f:(not any(s in f['name'] for s in ('jahresbericht-','wanderungen-','verkehrsunfaelle-')),f['bytes'],f['relative_path']))
        c=C.candidate('dotnet','selective',16)
        c.update(id='ready-dotnet-selective-16',label='Readiness / .NET / Selective / 16 MP',context_validation='first-seam')
        for f in files:self.ensure_job('readiness',c,f)
        self.save()
        for f in files:
            if not self.has_time('readiness'):break
            j=self.run_job('readiness',c,f)
            probe = C.read_json(self.root/'cases'/j['id']/'asset-preflight.json', {})
            deployment = startup_failure(j, probe)
            if deployment:
                # The next workbook cannot fix a missing shared JavaScript resource.
                # Do not repeat the same startup error or count it as capture evidence.
                self.state['infrastructure_failure'] = dict(deployment, case_id=j['id'])
                self.state['readiness'][f['id']] = {
                    'ready':False, 'reason':'Local web-asset deployment failed; no quality result',
                    'case_id':j['id'], 'category':'infrastructure'}
                for pending in self.state['jobs']:
                    if pending['stage']=='readiness' and pending['status']=='queued':
                        pending['status']='blocked'
                        pending['error']='Shared local web-asset failure in '+j['id']+'; workbook not opened'
                C.journal(self.root, 'infrastructure_blocked', self.state['infrastructure_failure'])
                self.save()
                return False
            self.state['readiness'][f['id']]=dict(readiness(j),case_id=j['id'])
            self.save()
        return len(self.state['readiness'])==len(files) and all(x['ready'] for x in self.state['readiness'].values())
    def run(self):
        # Reuse the tested controller/locks/timeouts. An open common-quality gate
        # must not be sent through another host/MP Cartesian product.
        try:
            passed=self.stage_readiness()
        except KeyboardInterrupt:
            self.state['status']='interrupted';self.state['errors'].append({'time':C.utc(),'message':'Readiness interrupted'})
            passed=False
        except Exception as e:
            self.state['status']='interrupted';self.state['errors'].append({'time':C.utc(),'message':str(e),'trace':C.traceback.format_exc()})
            passed=False
        if not passed:
            pending=any(j['stage']=='readiness' and j['status']=='queued' for j in self.state['jobs'])
            blocked = 'infrastructure-blocked' if self.state.get('infrastructure_failure') else 'quality-blocked'
            self.state['stages']['readiness']='pending' if pending else blocked
            if self.state['status']!='interrupted':self.state['status']='partial' if pending else blocked
            self.state['resumable']=pending or self.state['status']=='interrupted'
            self.state['ended_utc']=C.utc();self.state['sessions'][-1].update(ended_utc=C.utc(),elapsed_seconds=time.monotonic()-self.clock)
            self.save();C.write_bundle(self.root,self.state)
            if self.state.get('infrastructure_failure'):
                print('INFRASTRUCTURE BLOCKED: local web assets failed; remaining workbooks were not opened. See asset-preflight.json in the failed case.',flush=True)
            else:
                print('QUALITY GATE OPEN: ranking was not started. Existing evidence is complete up to the stop; see the report.',flush=True)
            return 2
        self.state['stages']['readiness']='complete';self.save()
        return super().run()
