"""Pure decision/controller tests. Native Excel conformance is NOT simulated evidence."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile
BASE=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(BASE/'_shared/decision'))
import campaign as C
import finalize as F


def result(name='test.xlsx',tiled=True,check=True):
    canonical={'workbook':name,'sheet':'S','sheet_index':1,'status':'complete','width':30,'height':100,
               'sha256':'a'*64,'algorithm':'mock-only','tiles':2 if tiled else 1}
    wb={'workbook':name,'kind':'capture','status':'complete','context_policy':'visible-cell-halo-v1',
        'sheets':[{'sheet':'S','sheet_index':1,'address':'A1:B10','status':'complete'}],
        'context_validations':([{'sheet':'S','status':'complete','overlap_mismatches':0,
            'full_comparison':{'equal':True},'repeat_comparison':{'equal':True}}] if check else [])}
    return {'status':'complete','metrics':{'status':'complete','workload_ms':100,'canonical_sheets':[canonical],'workbooks':[wb]}}


class FinalizationTests(unittest.TestCase):
    def config(self):return C.read_json(BASE/'capture-finalize.json')
    def setup(self,td):
        cfg=self.config();repo=Path(td)/'repo';root=repo/'docs/visual-validation/results/runs/mock';root.mkdir(parents=True)
        for i in range(3):
            p=repo/cfg['corpus_root']/('folder'+str(i))/('mock'+str(i)+'.xlsx');p.parent.mkdir(parents=True);p.write_bytes(b'NOT EXCEL; pure controller test '+bytes([i]))
        C.atomic_json(root/'run.json',{'schema':'vv-report-v1','experiment':'capture-finalization','status':'running'})
        return repo,root,cfg
    def test_configuration_is_bounded_and_no_historical_families(self):
        c=self.config();C.validate(c);self.assertEqual(c['workbooks'],['*']);self.assertEqual(c['diagnostics'],[])
        self.assertEqual(C.reference_candidates(c),[]);self.assertEqual(c['limits']['session_seconds'],10800)
        self.assertEqual(c['limits']['readiness_seconds'],2700);self.assertEqual(c['limits']['case_max_seconds'],600)
    def test_six_candidates_not_full_cross_product(self):
        cs=C.screen_candidates();self.assertEqual(len(cs),6)
        self.assertEqual({x['tile_megapixels'] for x in cs if x['host_kind']=='rust' and x['overflow_mode']=='selective'},{16,32,64})
    def test_multitile_needs_reference(self):self.assertFalse(F.readiness(result(check=False))['ready'])
    def test_monolithic_does_not_fabricate_reference(self):self.assertTrue(F.readiness(result(tiled=False,check=False))['ready'])
    def test_complete_reference_is_ready(self):self.assertTrue(F.readiness(result())['ready'])
    def test_pixel_mismatch_does_not_qualify(self):
        j=result();j['metrics']['workbooks'][0]['context_validations'][0]['full_comparison']['equal']=False
        self.assertFalse(F.readiness(j)['ready'])
    def test_nonrepeatable_reference_does_not_qualify(self):
        j=result();j['metrics']['workbooks'][0]['context_validations'][0]['repeat_comparison']['equal']=False
        self.assertFalse(F.readiness(j)['ready'])
    def test_one_overlap_pixel_is_not_tolerated(self):
        j=result();j['metrics']['workbooks'][0]['context_validations'][0]['overlap_mismatches']=1
        self.assertFalse(F.readiness(j)['ready'])
    def test_old_capture_policy_does_not_qualify(self):
        j=result();j['metrics']['workbooks'][0]['context_policy']='legacy-unpadded';self.assertFalse(F.readiness(j)['ready'])
    def test_missing_canonical_remains_open(self):
        j=result();j['metrics']['canonical_sheets'][0]['status']='unverified';self.assertFalse(F.readiness(j)['ready'])
    def test_failed_common_gate_does_not_start_matrix_and_publishes_bundle(self):
        with tempfile.TemporaryDirectory() as td:
            repo,root,cfg=self.setup(td)
            class Broken(F.FinalCampaign):
                def run_job(self,stage,c,f=None,repeat=1,reference_job=None):
                    self.assert_stage=stage;j=self.ensure_job(stage,c,f,repeat)
                    j.update(result(f['name'],check=False),case_seconds=.01);return j
            c=Broken(repo,root,cfg);self.assertEqual(c.run(),2)
            self.assertEqual(c.state['status'],'quality-blocked');self.assertEqual({j['stage'] for j in c.state['jobs']},{'readiness'})
            self.assertEqual(C.read_json(root/'bundle-status.json')['status'],'ready')
            with zipfile.ZipFile(root/'review-metadata.zip') as z:
                self.assertIsNone(z.testzip());self.assertIn('bundle-manifest.json',z.namelist())
                self.assertIn('Common capture readiness',z.read('report.html').decode())
    def test_successful_gate_uses_existing_race_and_no_native_reruns(self):
        with tempfile.TemporaryDirectory() as td:
            repo,root,cfg=self.setup(td)
            class Working(F.FinalCampaign):
                def run_job(self,stage,c,f=None,repeat=1,reference_job=None):
                    j=self.ensure_job(stage,c,f,repeat)
                    if j['status']!='queued':return j
                    j.update(result(f['name']),case_seconds=.01)
                    j['metrics']['workload_ms']=100+(10 if c.get('host_kind')=='dotnet' else 0)+abs(c.get('tile_megapixels',16)-32)
                    return j
            c=Working(repo,root,cfg);self.assertEqual(c.run(),0)
            self.assertEqual(c.state['stages']['readiness'],'complete');self.assertEqual(len(c.state['coverage']),3)
            self.assertTrue(all(j['candidate']['kind']=='capture' for j in c.state['jobs']))
            self.assertIn(c.state['decisions']['confirmation']['status'],['supported-on-tested-inputs','provisional'])
            self.assertEqual(C.read_json(root/'bundle-status.json')['status'],'ready')
    def test_time_budget_retains_pending_without_launching(self):
        with tempfile.TemporaryDirectory() as td:
            repo,root,cfg=self.setup(td)
            class NoTime(F.FinalCampaign):
                def has_time(self,stage):return False
            c=NoTime(repo,root,cfg);self.assertEqual(c.run(),2);self.assertEqual(c.state['status'],'partial')
            self.assertTrue(c.state['resumable']);self.assertEqual(len(c.state['jobs']),3)
    def test_historical_families_cannot_accidentally_expand_profile(self):
        with tempfile.TemporaryDirectory() as td:
            repo,root,cfg=self.setup(td);cfg['diagnostics']=['units']
            with self.assertRaises(ValueError):F.FinalCampaign(repo,root,cfg)
    def test_rust_and_dotnet_crop_at_nonzero_origin_without_resampling(self):
        cs=(BASE/'_shared/office-addin/hosts/dotnet/FinalImageEndpoints.cs').read_text()
        rs=(BASE/'_shared/office-addin/hosts/rust/src/final_image.rs').read_text()
        self.assertIn('new Rectangle(x, y, width, height)',cs);self.assertNotIn('Mutate(x =>',cs)
        self.assertIn('crop_imm(&decoded, x, y, width, height)',rs)
        self.assertIn('"crop_rect_px"',rs)
    def test_context_support_is_loaded_and_required(self):
        js=(BASE/'_shared/office-addin/web/shell.js').read_text();mode=(BASE/'_shared/office-addin/web/modes/range-image.js').read_text()
        self.assertLess(js.index("load('/capture-geometry.js"),js.index("load('/safe-capture.js"))
        self.assertIn("typeof CaptureGeometry === 'undefined'",mode);self.assertIn("typeof ContextValidation === 'undefined'",mode)
    def test_full_plan_archived_not_deleted_or_retransmitted(self):
        runner=(BASE/'_shared/office-addin/run.ps1').read_text();mode=(BASE/'_shared/office-addin/web/modes/range-image.js').read_text()
        self.assertIn('inputs\\plans\\visual-plan.json',runner);self.assertIn('compactRustPlan(rustVisualPlan)',mode)
        self.assertIn('precomputed_wsl_rust_visual_plan:undefined',(BASE/'_shared/office-addin/web/safe-capture.js').read_text())
    def test_qualifier_flag_reaches_host_options(self):
        self.assertIn('context_validation=$ContextValidation',(BASE/'range-image/run.ps1').read_text())
        self.assertIn('$params.ContextValidation=[string]$c.context_validation',(BASE/'_shared/suite/worker.ps1').read_text())

if __name__=='__main__':unittest.main()
