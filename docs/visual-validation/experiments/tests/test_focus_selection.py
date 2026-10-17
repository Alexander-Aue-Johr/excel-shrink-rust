"""Focused request planning, replay and report contracts; no Excel required."""
from __future__ import annotations
import contextlib
import copy
import io
import json
import re
import shutil
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path
from unittest.mock import patch

EXPERIMENTS=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(EXPERIMENTS/'_shared/decision'))
import campaign as C
import focus_selection as S
import focus as F


def make_catalogue(path, sheets):
    path.parent.mkdir(parents=True,exist_ok=True)
    root=ET.Element('workbook',xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main')
    collection=ET.SubElement(root,'sheets')
    for i,item in enumerate(sheets,1):
        name,state=item if isinstance(item,tuple) else (item,'visible')
        ET.SubElement(collection,'sheet',name=name,sheetId=str(i),state=state)
    # Deliberately metadata only. Resolver must never access a worksheet XML file.
    with zipfile.ZipFile(path,'w') as z:z.writestr('xl/workbook.xml',ET.tostring(root))


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.repo=Path(self.temp.name)/'repo';self.exp=self.repo/'docs/visual-validation/experiments';self.exp.mkdir(parents=True)
        shutil.copyfile(EXPERIMENTS/'capture-focus-cases.json',self.exp/'capture-focus-cases.json')
        self.cfg=C.read_json(EXPERIMENTS/'capture-followup.json')
        self.cat=S.catalogue(self.repo);self.kernel=self.cat['cases'][0]['workbook']
        names={
            self.kernel:['71717-50','csv-71717-22','After'],
            self.cat['cases'][1]['workbook']:['Titelseite','Vorbemerkung','Anhang','Qualitätsbericht'],
            self.cat['cases'][3]['workbook']:['Titel','1.1','1.2'],
            self.cat['cases'][4]['workbook']:[' Cover ',('Hidden','hidden'),'Data'],
        }
        for name,sheets in names.items():make_catalogue(self.repo/'corpus-local'/name,sheets)
    def resolve(self,request):return S.resolve(self.repo,self.cfg,request)
    def test_default_runs_only_long_sheet(self):
        cfg=self.resolve({})
        self.assertEqual(cfg['workbooks'],[self.kernel]);self.assertEqual(cfg['focused_targets'][0]['sheets'],['csv-71717-22'])
        self.assertEqual(cfg['focus_parameters']['capture_timeout_seconds'],1800)
        self.assertEqual(cfg['focus_parameters']['workload_timeout_seconds'],2730)
        self.assertEqual(cfg['focus_parameters']['context_validation'],'off')
        self.assertFalse(cfg['auto_resume'])
    def test_basename_and_exact_sheet(self):
        cfg=self.resolve({'workbook':Path(self.kernel).name,'sheets':['csv-71717-22']})
        self.assertEqual(cfg['workbooks'],[self.kernel]);self.assertEqual(cfg['focused_targets'][0]['sheets'],['csv-71717-22'])
    def test_absolute_local_path(self):
        cfg=self.resolve({'workbook':str(self.repo/'corpus-local'/self.kernel),'sheet_indexes':[2]})
        self.assertEqual(cfg['focused_targets'][0]['sheets'],['csv-71717-22'])
    def test_only_metadata_is_needed(self):
        with zipfile.ZipFile(self.repo/'corpus-local'/self.kernel) as z:self.assertEqual(z.namelist(),['xl/workbook.xml'])
        self.assertEqual(len(self.resolve({})['focused_targets']),1)
    def test_exact_whitespace_and_workbook_order(self):
        workbook=self.cat['cases'][4]['workbook']
        cfg=self.resolve({'workbook':workbook,'sheets':['Data',' Cover ']})
        self.assertEqual(cfg['focused_targets'][0]['sheets'],[' Cover ','Data'])
        with self.assertRaisesRegex(ValueError,'Unknown exact'):self.resolve({'workbook':workbook,'sheets':['Cover']})
    def test_original_indexes_include_hidden_sheets(self):
        cfg=self.resolve({'workbook':self.cat['cases'][4]['workbook'],'sheet_indexes':[3]})
        self.assertEqual(cfg['focused_targets'][0]['sheets'],['Data'])
    def test_hidden_sheet_rejected(self):
        with self.assertRaisesRegex(ValueError,'Hidden worksheets'):self.resolve({'workbook':self.cat['cases'][4]['workbook'],'sheet_indexes':[2]})
    def test_zero_index_rejected(self):
        with self.assertRaisesRegex(ValueError,'outside'):self.resolve({'workbook':self.kernel,'sheet_indexes':[0]})
    def test_missing_workbook(self):
        with self.assertRaisesRegex(ValueError,'exactly one'):self.resolve({'workbook':'missing.xlsx'})
    def test_ambiguous_filename(self):
        make_catalogue(self.repo/'corpus-local'/'other'/Path(self.kernel).name,['x'])
        with self.assertRaisesRegex(ValueError,'matched 2'):self.resolve({'workbook':Path(self.kernel).name})
    def test_no_implicit_all(self):
        for name in ['*','all','kern*']:
            with self.subTest(name=name),self.assertRaises(ValueError):self.resolve({'workbook':name})
    def test_unknown_case(self):
        with self.assertRaisesRegex(ValueError,'Unknown case'):self.resolve({'case_ids':['missing']})
    def test_two_cases_same_workbook_are_distinct(self):
        cfg=self.resolve({'case_ids':['wanderungen-drawings','wanderungen-single-column']})
        self.assertEqual(len(cfg['workbooks']),1);self.assertEqual(len(cfg['focused_targets']),2)
        F.validate_targets(cfg)
    def test_exact_case_override(self):
        cfg=self.resolve({'case_ids':['wanderungen-drawings'],'sheets':['Anhang']})
        self.assertEqual(cfg['focused_targets'][0]['sheets'],['Anhang'])
    def test_ambiguous_override_rejected(self):
        with self.assertRaises(ValueError):self.resolve({'case_ids':['wanderungen-drawings','wanderungen-single-column'],'sheets':['Anhang']})
    def test_json_null_means_all_visible(self):
        cfg=self.resolve({'targets':[{'workbook':self.kernel,'sheets':None}]})
        self.assertIsNone(cfg['focused_targets'][0]['sheets'])
    def test_duplicate_target_id_rejected(self):
        with self.assertRaisesRegex(ValueError,'unique lowercase'):self.resolve({'targets':[{'id':'same','workbook':self.kernel,'sheets':['After']},{'id':'same','workbook':self.kernel,'sheets':['csv-71717-22']}]})
    def test_bad_request_types(self):
        for request in [None,[],{'targets':[]},{'repetitions':True},{'extra':1},{'sheets':[]},{'sheet_indexes':[True]},{'capture_timeout_seconds':float('inf')},{'tile_megapixels':128},{'wait_for_start':1}]:
            with self.subTest(request=request),self.assertRaises(ValueError):S.validate_request(request)
    def test_session_not_silently_shortened(self):
        with self.assertRaisesRegex(ValueError,'shorter than one'):self.resolve({'session_timeout_seconds':360})
    def test_native_timeout_relations(self):
        for params in [{'native_call_timeout_seconds':200},{'capture_timeout_seconds':30}]:
            with self.subTest(params=params),self.assertRaises(ValueError):self.resolve(params)
    def test_extended_deadlines_are_preserved(self):
        cfg=self.resolve({'capture_timeout_seconds':3600,'startup_timeout_seconds':600,'host_kind':'rust','tile_megapixels':64,'repetitions':2})
        self.assertEqual(cfg['focus_parameters']['workload_timeout_seconds'],4830)
        self.assertEqual(cfg['limits']['case_default_seconds'],4830)
        self.assertEqual(cfg['limits']['case_max_seconds'],4830)
        self.assertEqual(S.plan(cfg)['case_count'],2)
    def test_input_bytes_unchanged(self):
        path=self.repo/'corpus-local'/self.kernel;before=path.read_bytes();self.resolve({});self.assertEqual(path.read_bytes(),before)
    def test_xml_entity_rejected(self):
        path=self.repo/'corpus-local'/self.kernel
        with zipfile.ZipFile(path,'w') as z:z.writestr('xl/workbook.xml','<!DOCTYPE a [<!ENTITY x "abc">]><workbook/>')
        with self.assertRaisesRegex(ValueError,'DTD'):S.sheet_catalogue(path)
    def test_source_config_not_mutated(self):
        before=copy.deepcopy(self.cfg);self.resolve({});self.assertEqual(self.cfg,before)
    def test_plan_does_not_start_workers(self):
        config=self.exp/'capture-followup.json';C.atomic_json(config,self.cfg)
        request=self.repo/'request.json';C.atomic_json(request,{'case_ids':['kernhaushalt-long']})
        dest=self.repo/'resolved.json'
        output=io.StringIO()
        with patch.object(C.subprocess,'Popen',side_effect=AssertionError('No process allowed')),contextlib.redirect_stdout(output):
            code=C.main(['--repo-root',str(self.repo),'--focused','--plan','--focus-request',str(request),'--write-resolved-config',str(dest)])
        self.assertEqual(code,0);self.assertEqual(json.loads(output.getvalue())['case_count'],1);self.assertTrue(dest.exists())
    def test_list_sheets_ignores_named_filter(self):
        config=self.exp/'capture-followup.json';C.atomic_json(config,self.cfg)
        # Catalogue still works when a named case points to a formerly existing sheet.
        self.cat['cases'][0]['sheets']=['Obsolete'];C.atomic_json(self.exp/'capture-focus-cases.json',self.cat)
        request=self.repo/'request.json';C.atomic_json(request,{'case_ids':['kernhaushalt-long']})
        output=io.StringIO()
        with contextlib.redirect_stdout(output):code=C.main(['--repo-root',str(self.repo),'--focused','--focus-request',str(request),'--list-focus-sheets'])
        self.assertEqual(code,0);self.assertEqual(len(json.loads(output.getvalue())[0]['sheets']),3)
    def test_replay_is_resolvable(self):
        cfg=self.resolve({'targets':[{'workbook':self.kernel,'sheets':None}]})
        root=self.repo/'run';root.mkdir();F.publish_replay_files(root,{'config':cfg,'jobs':[]})
        replay=C.read_json(root/'focus-request.json');again=self.resolve(replay)
        self.assertEqual(again['focused_targets'],cfg['focused_targets'])
    def test_error_classification_not_crash_claim(self):
        root=self.repo/'run';root.mkdir();j={'id':'000001-x','status':'timeout','supervisor':{'timeout_kind':'capture','phase':'capture'},'metrics':None}
        d=F.case_diagnostic(root,j);self.assertEqual(d['category'],'timeout / capture')
    def test_mock_focused_run_no_matrix_and_distinct_repetitions(self):
        cfg=self.resolve({'case_ids':['wanderungen-drawings','wanderungen-single-column'],'repetitions':2})
        root=self.repo/'docs/visual-validation/results/runs/test';root.mkdir(parents=True)
        C.atomic_json(root/'run.json',{'id':'test','status':'running'})
        seen=[]
        def mock_run(campaign,stage,c,f=None,repeat=1,reference_job=None):
            job=campaign.ensure_job(stage,c,f,repeat);seen.append((c['target_id'],c['sheet_names'],repeat))
            job.update(status='failed',error='Simulated calibration failure',metrics={'status':'failed','workbooks':[]},case_seconds=1)
            case=root/'cases'/job['id'];case.mkdir(parents=True);C.atomic_json(case/'capture-lifecycle.json',{'schema':'vv-focus-lifecycle-v1','phase':'failed','last_sheet':c['sheet_names'][0]})
            return job
        with patch.object(F.FocusCampaign,'run_job',mock_run):
            campaign=F.FocusCampaign(self.repo,root,cfg);code=campaign.run()
        self.assertEqual(code,2);self.assertEqual(len(seen),4);self.assertEqual({x[0] for x in seen},{'wanderungen-drawings','wanderungen-single-column'})
        self.assertEqual({j['stage'] for j in campaign.state['jobs']},{'focus'})
        self.assertIsNone(C.read_json(root/'decision-summary.json')['deployment_selection'])
        with zipfile.ZipFile(root/'review-metadata.zip') as z:
            self.assertIn('reproduce.ps1',z.namelist());self.assertFalse(any(n.endswith('.png') for n in z.namelist()))
            self.assertEqual(len([n for n in z.namelist() if n.endswith('/focus-request.json')]),4)


class SourceContracts(unittest.TestCase):
    def test_wpf_names_and_handlers(self):
        text=(EXPERIMENTS/'_shared/decision/focus-gui.ps1').read_text()
        markup=text.split("[xml]$markup=@'\n",1)[1].split("\n'@",1)[0]
        root=ET.fromstring(markup);names={n.attrib['{http://schemas.microsoft.com/winfx/2006/xaml}Name'] for n in root.iter() if '{http://schemas.microsoft.com/winfx/2006/xaml}Name' in n.attrib}
        refs=set(re.findall(r'\$ui\.([A-Za-z][A-Za-z0-9]+)',text))
        self.assertFalse(refs-names,refs-names)
    def test_timeout_parameters_end_to_end(self):
        worker=(EXPERIMENTS/'_shared/suite/worker.ps1').read_text();runner=(EXPERIMENTS/'range-image/run.ps1').read_text()
        for parameter in ('StartupTimeoutSeconds','CaptureTimeoutSeconds','InactivityTimeoutSeconds','AbsoluteWorkbookTimeoutSeconds','NativeCallTimeoutSeconds','DiagnosticLevel'):
            self.assertIn('$params.'+parameter,worker);self.assertIn('$'+parameter,runner)
        self.assertIn('native_call_timeout_ms',runner)
    def test_existing_focused_task_opens_the_gui(self):
        task=json.loads((EXPERIMENTS.parents[2]/'.vscode/tasks.json').read_text())
        item=next(t for t in task['tasks'] if t['label']=='Visual validation: Check capture fixes (focused)')
        self.assertIn('-InterfaceMode gui',item['args'][1]);self.assertEqual(item['windows']['args'][-2:],['-InterfaceMode','gui'])
    def test_preflight_parameter_names_exist(self):
        pre=(EXPERIMENTS/'_shared/decision/focus-preflight.ps1').read_text();runner=(EXPERIMENTS/'range-image/run.ps1').read_text()
        self.assertNotIn('SheetNamesJson',pre);self.assertIn('$SheetNames',runner)
        self.assertIn(".kind -ne 'capture'",pre);self.assertIn(".kind -ne 'workload'",pre)
    def test_no_forced_excel_or_clipboard_mutation_in_observer(self):
        text=(EXPERIMENTS/'_shared/office-addin/powershell/focus-observer.ps1').read_text()
        for forbidden in ('ComObject','GetActiveObject','SendKeys','SetForegroundWindow','CopyFromScreen','Stop-Process'):self.assertNotIn(forbidden,text)
    def test_parser_preflight_runs_before_pixel_preflight(self):
        text=(EXPERIMENTS/'decision-campaign.ps1').read_text()
        self.assertLess(text.index("_shared\\decision\\focus-preflight.ps1"),text.index("_shared\\decision\\preflight.ps1"))
    def test_selection_conversion_preserves_scalar_sheet_names(self):
        text=(EXPERIMENTS/'decision-campaign.ps1').read_text()
        scalar=text.index('if($Object -is [string] -or $Object -is [ValueType])')
        object_conversion=text.index('if($Object -is [pscustomobject])',scalar)
        self.assertLess(scalar,object_conversion)

if __name__=='__main__':unittest.main()
