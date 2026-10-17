"""Pure regression tests for routing contracts and startup-failure isolation.

No Excel, Windows PowerShell, native host build or network request is made here.
Run from the repository root with:
python -m unittest discover -s docs/visual-validation/experiments/tests -p test_local_web_assets.py
"""
from __future__ import annotations
import importlib.util
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[4]
EXP = ROOT / 'docs/visual-validation/experiments'
SHARED = EXP / '_shared/office-addin'
sys.path.insert(0, str(EXP / '_shared/decision'))
import finalize as F


class AssetContractTests(unittest.TestCase):
    def setUp(self):
        self.dotnet = (SHARED/'hosts/dotnet/Program.cs').read_text(encoding='utf-8-sig')
        self.rust = (SHARED/'hosts/rust/src/main.rs').read_text(encoding='utf-8-sig')
        self.ps = (SHARED/'powershell/asset-preflight.ps1').read_text(encoding='utf-8-sig')
        self.runner = (SHARED/'run.ps1').read_text(encoding='utf-8-sig')
        shell = (SHARED/'web/shell.js').read_text()
        capture_branch = shell.split("if (config.experiment === 'range-image') {",1)[1].split('}',1)[0]
        self.required = set(re.findall(r"await load\('(/[^'?]+)",capture_branch)) | {'/mode.js'}

    def test_browser_capture_modules_are_served_by_dotnet(self):
        routes = set(re.findall(r'app\.MapGet\("([^"]+)"',self.dotnet))
        self.assertFalse(self.required-routes)

    def test_browser_capture_modules_are_served_by_rust(self):
        routes = set(re.findall(r'\.route\("([^"]+)"',self.rust))
        self.assertFalse(self.required-routes)

    def test_all_capture_modules_are_covered_by_preflight(self):
        routes = set(re.findall(r"route='([^']+)'",self.ps))
        self.assertFalse(self.required-routes)

    def test_route_files_exist(self):
        for name in ('capture-geometry.js','context-validation.js'):
            self.assertTrue((SHARED/'web'/name).is_file())
            self.assertIn(f'StaticFile("{name}", "application/javascript; charset=utf-8")',self.dotnet)
            self.assertIn(f'static_file(state,"{name}","application/javascript; charset=utf-8")',self.rust)

    def test_check_precedes_registration_and_workbook_loop(self):
        start = self.runner.index("Write-ExperimentLog 'Experiment host is healthy.'")
        check = self.runner.index('    Assert-VVWebAssets ',start)
        registration = self.runner.index('    $registrationState = Register-DeveloperAddIn',start)
        workbooks = self.runner.index('    foreach ($sourcePath in $paths)',start)
        self.assertLess(start,check)
        self.assertLess(check,registration)
        self.assertLess(registration,workbooks)

    def test_hash_mime_length_identity_and_bounds_guards_remain(self):
        for text in ('expected_sha256', 'received_sha256 -cne $asset.expected_sha256',
                     '$mime -ne $asset.expected_type', '$asset.received_bytes -ne $asset.expected_bytes',
                     '[string]$Config.capture_run_id -cne $LaunchId',
                     '[string]$Config.host_run_id -cne $LaunchId',
                     "[string]$Config.host_kind -cne $HostKind",'4194304','-TimeoutMs 8000'):
            self.assertIn(text,self.ps)

    def test_no_certificate_bypass_and_no_redirect(self):
        self.assertNotIn('ServerCertificateValidationCallback',self.ps)
        self.assertNotIn('SkipCertificateCheck',self.ps)
        self.assertIn('$request.AllowAutoRedirect = $false',self.ps)
        self.assertIn('$request.Proxy = $null',self.ps)
        self.assertIn('https://localhost:3000',self.ps)
        self.assertIn('http://localhost:3000',self.ps)

    def test_original_patch_missed_both_routes(self):
        # Mutation check: removing the additions reproduces the original routing gap.
        for text,pattern in [(self.dotnet,r'app\.MapGet\("([^"]+)"'),
                             (self.rust,r'\.route\("([^"]+)"')]:
            old='\n'.join(l for l in text.splitlines() if '/capture-geometry.js' not in l and '/context-validation.js' not in l)
            missing=self.required-set(re.findall(pattern,old))
            self.assertEqual(missing,{'/capture-geometry.js','/context-validation.js'})


class StartupClassifierTests(unittest.TestCase):
    def test_structured_asset_failure(self):
        probe={'scope':'local-web-assets','status':'failed','code':'VV_ASSET_PREFLIGHT_FAILED',
               'phase':'http-assets','current':'/capture-geometry.js','error':'HTTP 404'}
        result=F.startup_failure({},probe)
        self.assertEqual(result['route'],'/capture-geometry.js')

    def test_actual_reported_error_is_classified(self):
        text='Experiment host reported: {"message":"Error: Could not load /capture-geometry.js?run=1792023127888"}'
        result=F.startup_failure({'metrics':{'invocation_error':text}})
        self.assertEqual(result['code'],'VV_LOCAL_SCRIPT_LOAD_FAILED')

    def test_geometry_and_pixel_failures_are_not_classified(self):
        for message in ('CALIBRATION_NO_SAFE_CONTROLS','CONTEXT_DIFFERENCE','NATIVE_TIMEOUT',
                        'Inconsistent seam pixels at image row 5864','GeneralException'):
            self.assertIsNone(F.startup_failure({'error':message}))

    def test_successful_preflight_is_not_failure(self):
        self.assertIsNone(F.startup_failure({}, {'scope':'local-web-assets','status':'complete'}))

    def test_error_without_structured_record_still_stops(self):
        self.assertEqual(F.startup_failure({'error':'VV_ASSET_PREFLIGHT_FAILED: staged file missing'})['code'],
                         'VV_ASSET_PREFLIGHT_FAILED')

    def test_readiness_stops_after_first_shared_asset_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            obj=F.FinalCampaign.__new__(F.FinalCampaign)
            obj.root=Path(tmp)
            obj.files={str(i):{'id':str(i),'name':f'file{i}.xlsx','relative_path':f'file{i}.xlsx','bytes':i+1}
                       for i in range(8)}
            obj.state={'screen_ids':list(obj.files),'jobs':[],'readiness':{}}
            obj.save=lambda:None
            obj.has_time=lambda stage:True
            def ensure(stage,c,f):
                j={'id':f"{int(f['id'])+1:06d}-ready",'stage':stage,'status':'queued','input_id':f['id']}
                obj.state['jobs'].append(j)
                return j
            obj.ensure_job=ensure
            calls=[]
            def run(stage,c,f):
                calls.append(f['id'])
                j=next(j for j in obj.state['jobs'] if j['input_id']==f['id'])
                j.update(status='partial',error='Could not load /capture-geometry.js?run=x')
                return j
            obj.run_job=run
            with patch.object(F.C,'read_json',return_value={}),patch.object(F.C,'journal'):
                self.assertFalse(obj.stage_readiness())
            self.assertEqual(len(calls),1)
            self.assertEqual(sum(j['status']=='blocked' for j in obj.state['jobs']),7)
            self.assertEqual(obj.state['infrastructure_failure']['code'],'VV_LOCAL_SCRIPT_LOAD_FAILED')

    def test_geometry_failure_does_not_block_other_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            obj=F.FinalCampaign.__new__(F.FinalCampaign);obj.root=Path(tmp)
            obj.files={str(i):{'id':str(i),'name':f'f{i}.xlsx','relative_path':f'f{i}.xlsx','bytes':i+1} for i in range(2)}
            obj.state={'screen_ids':list(obj.files),'jobs':[],'readiness':{}}
            obj.save=lambda:None;obj.has_time=lambda stage:True
            def ensure(stage,c,f):
                obj.state['jobs'].append({'id':f['id'],'stage':stage,'status':'queued','input_id':f['id']})
            obj.ensure_job=ensure;calls=[]
            def run(stage,c,f):
                calls.append(f['id']);j=next(j for j in obj.state['jobs'] if j['input_id']==f['id'])
                j.update(status='partial',error='CALIBRATION_NO_SAFE_CONTROLS');return j
            obj.run_job=run
            with patch.object(F.C,'read_json',return_value={}):obj.stage_readiness()
            self.assertEqual(len(calls),2)
            self.assertNotIn('infrastructure_failure',obj.state)


if __name__=='__main__':unittest.main()
