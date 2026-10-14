"""Pure planning, recovery-policy, evidence and publication tests; no Windows/Excel."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

EXP=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('decision_campaign',EXP/'_shared/decision/campaign.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
CFG=json.loads((EXP/'decision-campaign.json').read_text())

def good_job(c,f,stage='screen',r=1,seconds=100,pixels='same'):
    j=m.make_job(stage,c,f,r);j.update(id='000001-'+c['id'],sequence=1,status='complete',case_seconds=seconds+1,report='cases/x/report.html')
    j['metrics']={'status':'complete','workload_ms':seconds*1000,'workbooks':[{'kind':'capture','sheets':[{'sheet_index':1,'sheet':'Sheet','address':'A1:B20'}]}],
       'canonical_sheets':[{'status':'complete','sheet_index':1,'sheet':'Sheet','width':100,'height':400,'algorithm':'vv-bgra32-strip-v1','sha256':pixels}]}
    return j

def state_for(candidates=None,stage='screen',rounds=(1,)):
    fs=[{'id':str(i),'relative_path':str(i)+'.xlsx','name':str(i)+'.xlsx','bytes':i+10} for i in range(3)]
    cs=candidates or m.screen_candidates()
    state={'schema':m.SCHEMA,'run_id':'test','source_version':'abc','status':'running','config':copy.deepcopy(CFG),'files':fs,
           'screen_ids':[f['id'] for f in fs],'reference_ids':[f['id'] for f in fs],'confirmation_ids':[f['id'] for f in fs],
           'jobs':[],'decisions':{},'stages':{},'coverage':{},'errors':[],'limitations':[],'sessions':[]}
    for r in rounds:
        for f in fs:
            for c in cs:state['jobs'].append(good_job(c,f,stage,r))
    for i,j in enumerate(state['jobs'],1):j['sequence']=i;j['id']=f'{i:06d}-'+j['candidate_id']
    return state

class PlanTests(unittest.TestCase):
    def test_configuration(self):m.validate(CFG)
    def test_not_cartesian(self):
        cs=m.screen_candidates();self.assertEqual(len(cs),6);self.assertEqual(len({c['id'] for c in cs}),6)
        self.assertEqual([c['tile_megapixels'] for c in cs if c['host_kind']=='rust' and c['overflow_mode']=='selective'],[16,32,64])
    def test_diagnostics_are_once(self):self.assertEqual(len(CFG['diagnostics']),4)
    def test_native_kinds_distinct(self):
        cs=m.reference_candidates(CFG);self.assertEqual(len(cs),15)
        self.assertEqual(len([c for c in cs if c['kind']=='shape-native']),1)
        self.assertTrue(any(c['kind']=='lab-memory' for c in cs))
        self.assertFalse(any(c['kind']=='csharp-picture' for c in cs))
    def test_runtime_ceiling(self):
        self.assertEqual(CFG['limits']['session_seconds'],21600)
        self.assertLessEqual(sum(CFG['limits'][k] for k in ('screen_seconds','reference_seconds','diagnostic_seconds','confirmation_seconds')),21600)
    def test_different_identical_basenames(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);(root/'corpus-local/a').mkdir(parents=True);(root/'corpus-local/b').mkdir()
            for d in ('a','b'):(root/f'corpus-local/{d}/same.xlsx').write_bytes(b'fixture')
            fs=m.discover(root,CFG);self.assertEqual(len(fs),2);self.assertNotEqual(fs[0]['id'],fs[1]['id'])
    def test_discovery_skips_locks_and_symlinks(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);d=root/'corpus-local';d.mkdir();(d/'a.xlsx').write_bytes(b'a');(d/'~$a.xlsx').write_bytes(b'lock')
            (d/'link.xlsx').symlink_to(d/'a.xlsx');self.assertEqual(len(m.discover(root,CFG)),1)
    def test_no_hidden_corpus_cap(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);d=root/'corpus-local';d.mkdir()
            for i in range(101):(d/f'{i}.xlsx').write_bytes(b'x')
            self.assertEqual(len(m.discover(root,CFG)),101)
    def test_stratification_deterministic(self):
        fs=[{'id':str(i),'bytes':i*10,'name':f'{i}.xlsx','relative_path':f'd/{i}.xlsx'} for i in range(100)]
        a=m.stratified(fs,12,'s');self.assertEqual(a,m.stratified(list(reversed(fs)),12,'s'));self.assertEqual(len(a),12)
        self.assertGreater(max(f['bytes'] for f in a),800)
    def test_prior_stress_included(self):
        fs=[{'id':str(i),'bytes':i,'name':f'{i}.xlsx','relative_path':f'{i}.xlsx'} for i in range(100)]
        fs[50]['name']='krankenhausverzeichnis-test.xlsx'
        self.assertIn(fs[50],m.stratified(fs,12,'s'))
    def test_timeout_same_input_formula_and_clamps(self):
        self.assertEqual(m.deadline(CFG,[100,110,90],'screen')['seconds'],295)
        self.assertEqual(m.deadline(CFG,[.1],'screen')['seconds'],120)
        self.assertEqual(m.deadline(CFG,[100000],'screen')['seconds'],1200)
        self.assertEqual(m.deadline(CFG,[],'screen')['seconds'],360)
        self.assertEqual(m.deadline(CFG,[100000],'references')['seconds'],300)
    def test_invalid_config_rejected(self):
        c=copy.deepcopy(CFG);c['capture']['batch_size']=64
        with self.assertRaises(ValueError):m.validate(c)
    def test_stage_separate_identity(self):
        c=m.screen_candidates()[0];f={'id':'f'}
        self.assertNotEqual(m.job_key('screen',c,f,1),m.job_key('confirm',c,f,1))
        self.assertNotEqual(m.job_key('confirm',c,f,1),m.job_key('confirm',c,f,2))

class QualityTests(unittest.TestCase):
    def test_stable_candidates_qualified(self):
        state=state_for();d=m.select_candidate(state,m.screen_candidates(),'screen')
        self.assertEqual(d['status'],'supported-on-tested-inputs');self.assertEqual(d['candidate']['id'],'f-rust-selective-16')
    def test_two_confirmation_rounds_required(self):
        st=state_for(stage='confirm');d=m.select_candidate(st,m.screen_candidates(),'confirm');self.assertEqual(d['status'],'provisional')
        st=state_for(stage='confirm',rounds=(1,2));self.assertEqual(m.select_candidate(st,m.screen_candidates(),'confirm')['status'],'supported-on-tested-inputs')
    def test_pixel_outlier_never_erased(self):
        st=state_for(stage='confirm',rounds=(1,2));st['jobs'][0]['metrics']['canonical_sheets'][0]['sha256']='outlier'
        d=m.select_candidate(st,m.screen_candidates(),'confirm');row=next(x for x in d['statistics'] if x['candidate']['id']==st['jobs'][0]['candidate_id'])
        self.assertFalse(row['repeat_stable'])
    def test_missing_png_unverified(self):
        j=state_for()['jobs'][0];j['metrics']['canonical_sheets'][0]['status']='unverified';self.assertIsNone(m.quality_fingerprint(j))
    def test_missing_sheet_is_not_verified(self):
        j=state_for()['jobs'][0];j['metrics']['workbooks'][0]['sheets'].append({'sheet_index':2,'sheet':'Missing','address':'A1:B20'})
        self.assertIsNone(m.quality_fingerprint(j))
    def test_pixel_disagreement_no_forced_winner(self):
        st=state_for()
        for j in st['jobs']:
            if j['candidate_id']=='f-rust-selective-16':j['metrics']['canonical_sheets'][0]['sha256']='different'
        self.assertEqual(m.select_candidate(st,m.screen_candidates(),'screen')['status'],'provisional')
    def test_different_bounds_not_equal(self):
        st=state_for();a,b=st['jobs'][:2];b['metrics']['workbooks'][0]['sheets'][0]['address']='A1:C20'
        self.assertEqual(m.check_same(a,b)['status'],'different-bounds')
    def test_different_input_not_comparable(self):
        st=state_for();a,b=st['jobs'][0],st['jobs'][6];self.assertEqual(m.check_same(a,b)['status'],'not-comparable')
    def test_no_hash_algorithm_mixing(self):
        st=state_for();a,b=st['jobs'][:2];b['metrics']['canonical_sheets'][0]['algorithm']='other';self.assertEqual(m.check_same(a,b)['status'],'different-pixels')
    def test_failure_not_zero_fast_time(self):
        st=state_for();j=st['jobs'][0];j['status']='timeout';j['metrics']['status']='partial';j['metrics']['workload_ms']=None;j['case_seconds']=400
        d=m.select_candidate(st,m.screen_candidates(),'screen');r=next(x for x in d['statistics'] if x['candidate']['id']==j['candidate_id']);self.assertFalse(r['all_complete'])
        r=next(x for x in m.aggregate(st) if x['candidate']==j['candidate_id']);self.assertEqual(r['timeouts'],1);self.assertGreater(r['spent_seconds'],400)
    def test_real_recovery_cost_admitted(self):
        st=state_for();j=st['jobs'][1];metrics=copy.deepcopy(j['metrics']);j['status']='timeout';j['metrics']['status']='failed'
        j['recovery_policy']={'status':'recovered','effective_metrics':metrics,'total_workload_ms':480000}
        self.assertTrue(m.policy_success(j));self.assertEqual(m.policy_workload(j),480000)
        d=m.select_candidate(st,m.screen_candidates(),'screen');r=next(x for x in d['statistics'] if x['candidate']['id']==j['candidate_id']);self.assertEqual(r['recovered'],1);self.assertGreater(r['total_file_medians_seconds'],300)
    def test_preference_does_not_compound(self):
        cs=[m.candidate('dotnet','heuristic',16),m.candidate('rust','heuristic',16),m.candidate('rust','selective',16)]
        st=state_for(cs)
        for j in st['jobs']:j['metrics']['workload_ms']={'f-dotnet-heuristic-16':100000,'f-rust-heuristic-16':114000,'f-rust-selective-16':130000}[j['candidate_id']]
        d=m.select_candidate(st,cs,'screen');self.assertEqual(d['candidate']['id'],'f-rust-heuristic-16')
    def test_larger_budget_can_win(self):
        cs=[m.candidate('rust','selective',n) for n in (16,32,64)];st=state_for(cs)
        for j in st['jobs']:j['metrics']['workload_ms']={16:100000,32:70000,64:69000}[j['candidate']['tile_megapixels']]
        d=m.select_candidate(st,cs,'screen');self.assertEqual(d['candidate']['tile_megapixels'],32)
    def test_single_ready_candidate_not_agreement(self):
        cs=[m.candidate('rust','selective',16)];st=state_for(cs);self.assertEqual(m.select_candidate(st,cs,'screen')['status'],'provisional')
    def test_phase_pairing_keeps_files(self):
        st=state_for();p=m.pairing(st['jobs'],'f-rust-selective-16','f-rust-selective-32','screen')
        self.assertEqual(p['equal_pairs'],3);self.assertEqual(p['median_equal_ratio_b_over_a'],1)

class PersistenceTests(unittest.TestCase):
    def root(self,t):
        p=Path(t);m.atomic_json(p/'run.json',{'schema':'vv-report-v1','experiment':'decision-campaign','status':'running'});return p
    def test_atomic_json_unicode(self):
        with tempfile.TemporaryDirectory() as t:
            p=Path(t)/'x.json';m.atomic_json(p,{'a':'Ämter'});self.assertEqual(m.read_json(p)['a'],'Ämter');self.assertFalse(p.with_name('x.json.new').exists())
    def test_report_escapes_input(self):
        with tempfile.TemporaryDirectory() as t:
            root=self.root(t);st=state_for();st['jobs'][0]['workbook']='<script>alert(1)</script>';m.render_report(root,st)
            self.assertIn('&lt;script&gt;', (root/'report.html').read_text());self.assertNotIn('<script>alert(1)</script>',(root/'report.html').read_text())
    def test_metadata_bundle_manifest(self):
        with tempfile.TemporaryDirectory() as t:
            root=self.root(t);st=state_for();m.atomic_json(root/'campaign-state.json',st);(root/'large.png').write_bytes(b'no image needed')
            m.render_report(root,st);m.write_bundle(root,st)
            with zipfile.ZipFile(root/'review-metadata.zip') as z:
                self.assertIsNone(z.testzip());self.assertNotIn('large.png',z.namelist());manifest=json.loads(z.read('bundle-manifest.json'))
                self.assertEqual(manifest['status'],'complete');self.assertTrue(any(x['path']=='large.png' for x in manifest['omitted']))
                for x in manifest['included']:self.assertEqual(m.hashlib.sha256(z.read(x['path'])).hexdigest(),x['sha256'])
            self.assertEqual(m.read_json(root/'bundle-status.json')['status'],'ready')
    def test_failed_zip_never_publishes_partial(self):
        with tempfile.TemporaryDirectory() as t:
            root=self.root(t);st=state_for();m.atomic_json(root/'campaign-state.json',st);(root/'review-metadata.zip').write_bytes(b'previous good bundle')
            with patch.object(m.zipfile,'ZipFile',side_effect=OSError('disk full')):
                with self.assertRaises(OSError):m.write_bundle(root,st)
            self.assertEqual((root/'review-metadata.zip').read_bytes(),b'previous good bundle')
            self.assertEqual(m.read_json(root/'bundle-status.json')['status'],'failed');self.assertNotIn('href="review-metadata.zip"',(root/'report.html').read_text())
    def test_state_metrics_remain_compact(self):
        v={'status':'complete','captures':[{'image':'long'}]*10000,'canonical_sheets':[],'workbooks':[]}
        self.assertNotIn('captures',m.compact_metrics(v))
    def test_scoped_sources_hash_does_not_read_generated_results(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);(root/'docs/visual-validation/experiments/results').mkdir(parents=True);(root/'tools/excel-lab').mkdir(parents=True)
            (root/'tools/excel-lab/a.bas').write_text('a');before=m.source_version(root)
            (root/'docs/visual-validation/experiments/results/a.json').write_text('changed');self.assertEqual(before,m.source_version(root))
            (root/'tools/excel-lab/a.bas').write_text('b');self.assertNotEqual(before,m.source_version(root))

class WorkflowTests(unittest.TestCase):
    class Fake(m.Campaign):
        def run_job(self,stage,c,f=None,repeat=1,reference_job=None):
            j=self.ensure_job(stage,c,f,repeat)
            if j['status'] in m.TERMINAL or not self.has_time(stage):return j
            j.update(status='complete',case_seconds=2,report='cases/'+j['id']+'/report.html')
            if c['kind']=='capture':
                metric=good_job(c,f,stage,repeat,seconds={16:2,32:1.5,64:1.49}.get(c.get('tile_megapixels'),2))['metrics']
                j['metrics']=metric
            else:j['metrics']={'status':'complete','workload_ms':2000,'workbooks':[],'canonical_sheets':[]}
            return j
    def fixture(self,root,count=5):
        (root/'corpus-local').mkdir();(root/'docs/visual-validation/experiments').mkdir(parents=True);(root/'tools/excel-lab').mkdir(parents=True)
        for n in range(count):(root/'corpus-local'/f'{n}.xlsx').write_bytes(bytes([n])*100)
        run=root/'docs/visual-validation/results/runs/example';run.mkdir(parents=True)
        m.atomic_json(run/'run.json',{'schema':'vv-report-v1','experiment':'decision-campaign','status':'running'})
        cfg=copy.deepcopy(CFG);cfg['screen_max_files']=3;cfg['reference_max_files']=3
        return cfg,run
    def test_complete_campaign_uses_selected_only_on_remaining_files(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);cfg,run=self.fixture(root)
            campaign=self.Fake(root,run,cfg);self.assertEqual(campaign.run(),0)
            st=campaign.state;self.assertEqual(len(st['coverage']),5)
            self.assertEqual(st['decisions']['budget']['candidate']['tile_megapixels'],32)
            self.assertEqual(len([j for j in st['jobs'] if j['stage']=='screen']),18)
            self.assertLessEqual(len([j for j in st['jobs'] if j['stage']=='confirm']),18)
            self.assertEqual(len(st['confirmation_candidates']),2)
            self.assertEqual(len([j for j in st['jobs'] if j['stage']=='coverage']),2)
            self.assertEqual(len([j for j in st['jobs'] if j['candidate']['kind']=='lab-setup']),1)
            self.assertEqual(len([j for j in st['jobs'] if j['stage']=='diagnostics']),4)
    def test_resume_keeps_existing_measurements(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);cfg,run=self.fixture(root);a=self.Fake(root,run,cfg);a.run();jobs=copy.deepcopy(a.state['jobs'])
            b=self.Fake(root,run,cfg);b.run();self.assertEqual(jobs,b.state['jobs'])
    def test_comparison_budgets_mark_unexecuted_and_still_cover_files(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);cfg,run=self.fixture(root);cfg['limits']['reference_seconds']=1
            campaign=self.Fake(root,run,cfg);campaign.run()
            refs=[j for j in campaign.state['jobs'] if j['stage']=='references']
            self.assertTrue(refs);self.assertTrue(all(j['status']=='skipped-budget' for j in refs))
            self.assertEqual(campaign.state['stages']['references'],'budget-limited');self.assertEqual(len(campaign.state['coverage']),5)
    def test_resume_rejects_source_or_config_changes(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);cfg,run=self.fixture(root);a=self.Fake(root,run,cfg)
            c=copy.deepcopy(cfg);c['limits']['session_seconds']+=1
            with self.assertRaises(ValueError):self.Fake(root,run,c)
            (root/'tools/excel-lab/a.bas').write_text('changed')
            with self.assertRaises(ValueError):self.Fake(root,run,cfg)
    def test_actual_recovery_controller_retains_primary(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);cfg,run=self.fixture(root);campaign=self.Fake(root,run,cfg);f=campaign.state['files'][0];c=m.candidate('rust','selective',64)
            j=campaign.ensure_job('screen',c,f);j.update(status='timeout',case_seconds=100,metrics={'status':'failed','workload_ms':None})
            answer=campaign.run_policy('screen',c,f)
            self.assertEqual(answer['status'],'timeout');self.assertEqual(answer['recovery_policy']['status'],'recovered')
            self.assertEqual(answer['recovery_policy']['total_workload_ms'],102000)
            self.assertEqual(len(campaign.state['jobs']),2)

if __name__=='__main__':unittest.main()
