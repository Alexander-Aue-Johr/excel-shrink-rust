"""Pure Python/controller and source-contract tests; no Windows/Excel execution."""
import copy, importlib.util, json, sys, tempfile, unittest, zipfile
from pathlib import Path
BASE=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(BASE/'_shared/decision'))
import campaign as C
import focus as F

class FocusTests(unittest.TestCase):
    def config(self): return json.loads((BASE/'capture-followup.json').read_text())
    def test_finite_six_cases_no_parameter_matrix(self):
        cfg=self.config();C.validate(cfg);F.validate_targets(cfg)
        self.assertEqual(len(cfg['focused_targets']),6)
        self.assertEqual(cfg['limits']['session_seconds'],1800)
        self.assertEqual(cfg['diagnostics'],[]);self.assertEqual(C.reference_candidates(cfg),[])
    def test_names_not_trimmed(self):
        self.assertIn('Erläuterung zu CSV-Tabellen ',self.config()['focused_targets'][1]['sheets'])
        cfg=self.config();cfg['focused_targets'][0]['sheets']=['A','A']
        with self.assertRaises(ValueError):F.validate_targets(cfg)
    def test_unique_inventory(self):
        cfg=self.config();cfg['focused_targets'][1]['workbook']=cfg['focused_targets'][0]['workbook']
        with self.assertRaises(ValueError):F.validate_targets(cfg)
    def test_no_reference_expansion(self):
        cfg=self.config();cfg['references']['vba_chart_export']=True
        with self.assertRaises(ValueError):F.validate_targets(cfg)
    def setup_run(self,tmp):
        repo=Path(tmp)/'repo';root=repo/'docs/visual-validation/results/runs/test-run';root.mkdir(parents=True)
        cfg=self.config()
        for f in cfg['workbooks']:
            p=repo/cfg['corpus_root']/f;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b'mocked source only; not opened as Excel')
        C.atomic_json(root/'run.json',{'schema':'vv-report-v1','experiment':'capture-followup','status':'running'})
        return repo,root,cfg
    def test_selected_sheet_quality_is_not_full_corpus_claim(self):
        with tempfile.TemporaryDirectory() as td:
            repo,root,cfg=self.setup_run(td);c=F.FocusCampaign(repo,root,cfg)
            self.assertNotIn('Deployment recommendation',(root/'report.html').read_text())
            self.assertIn('not full-corpus coverage',(root/'report.html').read_text())
            self.assertIsNone(C.read_json(root/'decision-summary.json')['deployment_selection'])
    def test_controller_simulation_completes_all_six_without_excel(self):
        with tempfile.TemporaryDirectory() as td:
            repo,root,cfg=self.setup_run(td)
            class MockCampaign(F.FocusCampaign):
                def run_job(self,stage,c,f=None,repeat=1,reference_job=None):
                    j=self.ensure_job(stage,c,f,repeat)
                    if j['status']=='complete':return j
                    sheets=c['sheet_names'] or ['Mock visible sheet'];canonical=[];results=[]
                    for i,name in enumerate(sheets,1):
                        canonical.append({'sheet_index':i,'sheet':name,'status':'complete','algorithm':'mock','width':10,'height':10,'sha256':'a'*64})
                        results.append({'sheet_index':i,'sheet':name,'status':'complete','address':'A1:B3'})
                    j.update(status='complete',case_seconds=.1,metrics={'status':'complete','workload_ms':50,'canonical_sheets':canonical,
                        'workbooks':[{'workbook':f['name'],'kind':'capture','sheets':results}]});self.save();return j
            c=MockCampaign(repo,root,cfg);self.assertEqual(c.run(),0)
            self.assertEqual(len(c.state['jobs']),6);self.assertEqual(c.state['status'],'complete')
            with zipfile.ZipFile(root/'review-metadata.zip') as z:
                self.assertIsNone(z.testzip());self.assertIn('focus-findings.json',z.namelist())
                manifest=json.loads(z.read('bundle-manifest.json'));self.assertTrue(all(x in z.namelist() for x in manifest['required']))
                self.assertFalse(any(n.endswith('.png') for n in z.namelist()))
                self.assertIn('Capture readiness',z.read('report.html').decode())
            self.assertEqual(C.read_json(root/'bundle-status.json')['status'],'ready')
    def test_partial_case_does_not_turn_into_passed_calibration(self):
        with tempfile.TemporaryDirectory() as td:
            repo,root,cfg=self.setup_run(td);c=F.FocusCampaign(repo,root,cfg)
            f=c.state['files'][0];j=c.ensure_job('focus',C.candidate('dotnet','selective',16),f)
            j.update(status='partial',metrics={'status':'partial','workbooks':[{'workbook':f['name'],'kind':'capture','sheets':[{'sheet':'A','sheet_index':1,'status':'failed','error':{'code':'CALIBRATION_MODEL_REJECTED'}},{'sheet':'B','sheet_index':2,'status':'complete'}]}]})
            c.save();r=(root/'report.html').read_text();self.assertIn('CALIBRATION_MODEL_REJECTED',r);self.assertIn('unverified',r)
            self.assertEqual(F.findings(c.state)[0]['status'],'failed');self.assertIsNone(C.quality_fingerprint(j))
    def test_seam_failure_remains_strict_and_has_structured_diagnostics(self):
        cs=(BASE/'_shared/decision/StripDigest.cs').read_text();ps=(BASE/'_shared/decision/extras.ps1').read_text()
        for s in ['changed_pixels','current_tile_local_row_0based','previous_bgra_hex','throw mismatch;']:self.assertIn(s,cs)
        self.assertIn('seam_diagnostic',ps);self.assertIn('GetBaseException()',ps)
    def test_sheet_names_pass_as_array_not_shell_string(self):
        ps=(BASE/'range-image/run.ps1').read_text();worker=(BASE/'_shared/suite/worker.ps1').read_text()
        self.assertIn('[string[]]$SheetNames',ps);self.assertIn('sheet_names=$SheetNames',ps)
        self.assertIn('$params.SheetNames=[string[]]@($c.sheet_names)',worker)
    def test_core_keeps_pixel_gates_and_before_capture_events(self):
        s=(BASE/'_shared/office-addin/web/safe-capture.js').read_text()
        self.assertIn('tolerance_px: 2',s);self.assertIn('CALIBRATION_MODEL_REJECTED',s)
        self.assertIn("await this.event(stage + '_started'",s)
        self.assertIn('this.calibrations.get(sheet).observations[0].address',s)
        self.assertNotIn("getRange('A1:A1').getImage()",s)

if __name__=='__main__': unittest.main()
