"""Pure selection/scheduling/evidence tests. No Windows processes or real Excel."""
from __future__ import annotations
import copy
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET
import zipfile

EXP=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(EXP/'_shared/decision'))
import campaign as C
import focus as F
import focus_selection as S
import focus_profiles as Q


def workbook(path,names):
    path.parent.mkdir(parents=True,exist_ok=True)
    root=ET.Element('workbook',xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main')
    sheets=ET.SubElement(root,'sheets')
    for i,n in enumerate(names,1):
        ET.SubElement(sheets,'sheet',name=n,sheetId=str(i),state='hidden' if n=='Hidden' else 'visible')
    with zipfile.ZipFile(path,'w') as z:z.writestr('xl/workbook.xml',ET.tostring(root))


class Questions(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.repo=Path(self.temp.name);self.exp=self.repo/'docs/visual-validation/experiments';self.exp.mkdir(parents=True)
        for name in ('capture-focus-cases.json','capture-question-profiles.json'):
            shutil.copyfile(EXP/name,self.exp/name)
        self.cfg=C.read_json(EXP/'capture-followup.json');self.profiles=C.read_json(EXP/'capture-question-profiles.json')['profiles']
        names={
            'wanderungen':['Titelseite','Inhalt','Vorbemerkung','Anhang','Qualitätsbericht'],
            'krankenhaus':['Titelblatt','Inhalt',' KHV_2022','DSB_KHV_2022','DSB_RHV_2022'],
            'statistischer':['Before','csv-71717-22','After','Hidden','After2'],
            'verkehrs':['Titel','1.1','1.2','Hidden','1.3'],
            'seeschiff':['Titel','Other']}
        paths={t['workbook'] for p in self.profiles for t in p['targets']}
        for rel in paths:
            key=next(k for k in names if Path(rel).name.startswith(k));workbook(self.repo/'corpus-local'/rel,names[key])
    def resolve(self,profile,**kw):return S.resolve(self.repo,self.cfg,dict(profile=profile,**kw))
    def state(self,cfg):
        root=self.repo/'docs/visual-validation/results/runs/mock';root.mkdir(parents=True,exist_ok=True)
        C.atomic_json(root/'run.json',{'id':'mock','status':'running'})
        obj=F.FocusCampaign(self.repo,root,cfg);return root,obj
    def finish(self,obj,c,t,r,variant_time=None):
        f=next(x for x in obj.state['files'] if x['relative_path']==t['workbook'])
        j=obj.ensure_job('focus',c,f,r);ss=c['sheet_names']
        cats=t['sheet_catalog'];indices={x['name']:x['index'] for x in cats};wn=Path(t['workbook']).name
        m={'status':'complete','workload_ms':variant_time or 1000,'canonical_sheets':[{'sheet':n,'sheet_index':indices[n],'workbook':wn,'width':20,'height':30,'algorithm':'test','sha256':C.obj_digest([t['id'],n]),'status':'complete'} for n in ss],
           'workbooks':[{'kind':'capture','workbook':wn,'status':'complete','context_policy':'visible-cell-halo-v1','environment':{'device_pixel_ratio':1,'office':{'version':'mock','platform':'PC'}},
              'sheets':[{'sheet':n,'sheet_index':indices[n],'address':'A1:A10','status':'complete'} for n in ss]}]}
        j.update(status='complete',metrics=m,case_seconds=2,report='cases/'+j['id']+'/report.html')
        return j
    def complete(self,profile,**kw):
        cfg=self.resolve(profile,**kw);root,obj=self.state(cfg)
        for c,t,r in Q.planned_jobs(cfg):self.finish(obj,c,t,r,100000/c['tile_megapixels'])
        return root,obj
    def test_all_profiles_load(self):
        for p in self.profiles:self.assertEqual(Q.load_profile(self.repo,p['id']),p)
    def test_unknown_profile(self):
        with self.assertRaisesRegex(ValueError,'Unknown'):self.resolve('not-a-profile')
    def test_no_selector_profile_mixing(self):
        with self.assertRaises(ValueError):self.resolve('fixed-width',case_ids=['kernhaushalt-long'])
    def test_cannot_override_varied_dimension(self):
        for name,kw in [('tile-budget',{'tile_megapixels':32}),('hosts',{'host_kind':'rust'}),('overflow',{'overflow_mode':'selective'})]:
            with self.subTest(name=name),self.assertRaisesRegex(ValueError,'varies'):self.resolve(name,**kw)
    def test_fixed_profile_runs_only_three_sheets_twice(self):
        cfg=self.resolve('fixed-width');self.assertEqual(S.plan(cfg)['case_count'],2)
        self.assertEqual(cfg['focused_targets'][0]['sheets'],['Vorbemerkung','Anhang','Qualitätsbericht'])
        self.assertEqual(cfg['focus_parameters']['host_kind'],'dotnet')
    def test_budget_scope_is_twelve_cases_not_workbook_cartesian(self):
        cfg=self.resolve('tile-budget');jobs=Q.planned_jobs(cfg)
        self.assertEqual(len(jobs),12);self.assertEqual({c['tile_megapixels'] for c,t,r in jobs},{16,32,64})
        self.assertEqual({c['host_kind'] for c,t,r in jobs},{'rust'})
        self.assertTrue(all(c['sheet_names'] in [[' KHV_2022'],['csv-71717-22']] for c,t,r in jobs))
    def test_profile_case_counts(self):
        for p,n in [('hosts',12),('overflow',8),('startup',12),('coverage-gaps',2)]:
            with self.subTest(p=p):self.assertEqual(S.plan(self.resolve(p))['case_count'],n)
    def test_arm_order_rotates(self):
        jobs=Q.planned_jobs(self.resolve('tile-budget'));first=[c['tile_megapixels'] for c,t,r in jobs if t['id']=='wide-table' and r==1]
        second=[c['tile_megapixels'] for c,t,r in jobs if t['id']=='wide-table' and r==2]
        self.assertNotEqual(first,second);self.assertEqual(set(first),set(second))
    def test_coverage_after_anchor_excludes_hidden(self):
        cfg=self.resolve('coverage-gaps');self.assertEqual([t['sheets'] for t in cfg['focused_targets']],[['After','After2'],['1.2','1.3']])
    def test_missing_anchor_rejected(self):
        path=self.repo/'corpus-local'/self.profiles[-1]['targets'][0]['workbook'];workbook(path,['No anchor'])
        with self.assertRaisesRegex(ValueError,'anchor'):self.resolve('coverage-gaps')
    def test_deadlines_and_chosen_budget_are_preserved(self):
        cfg=self.resolve('hosts',tile_megapixels=32,capture_timeout_seconds=1800)
        self.assertEqual({c['tile_megapixels'] for c,t,r in Q.planned_jobs(cfg)},{32})
        self.assertEqual(cfg['focus_parameters']['capture_timeout_seconds'],1800)
        self.assertEqual(cfg['limits']['session_seconds'],10800)
    def test_inadequate_session_not_silently_clamped(self):
        with self.assertRaisesRegex(ValueError,'shorter'):self.resolve('fixed-width',session_timeout_seconds=100)
    def test_fixed_repeat_evidence(self):
        root,obj=self.complete('fixed-width');e=Q.evidence(obj.state)
        self.assertEqual(e['evidence_status'],'complete-compatible-repeats');self.assertIsNone(e['fastest_on_tested_scopes'])
    def test_exact_pixels_required_for_ranking(self):
        root,obj=self.complete('tile-budget');self.assertEqual(Q.evidence(obj.state)['fastest_on_tested_scopes'],'f-rust-selective-64')
        obj.state['jobs'][0]['metrics']['canonical_sheets'][0]['sha256']='different'
        e=Q.evidence(obj.state);self.assertEqual(e['evidence_status'],'open');self.assertIsNone(e['fastest_on_tested_scopes'])
    def test_environment_change_prevents_ranking(self):
        root,obj=self.complete('tile-budget');obj.state['jobs'][0]['metrics']['workbooks'][0]['environment']['device_pixel_ratio']=1.5
        self.assertIsNone(Q.evidence(obj.state)['fastest_on_tested_scopes'])
    def test_failed_missing_timeout_never_fast_successes(self):
        root,obj=self.complete('tile-budget');obj.state['jobs'][0].update(status='timeout',metrics=None,case_seconds=600)
        e=Q.evidence(obj.state);self.assertFalse(e['runs'][0]['policy_complete']);self.assertIsNone(e['fastest_on_tested_scopes'])
    def test_single_repeat_is_not_stability_evidence(self):
        root,obj=self.complete('hosts',repetitions=1);self.assertIsNone(Q.evidence(obj.state)['fastest_on_tested_scopes'])
        self.assertEqual(Q.evidence(obj.state)['rows'][0]['repeat_pixels'],'not-tested')
    def test_replay_root_preserves_profile_case_freezes_arm(self):
        root,obj=self.complete('tile-budget');j=obj.state['jobs'][2];case=root/'cases'/j['id'];case.mkdir(parents=True)
        F.publish_replay_files(root,obj.state)
        request=C.read_json(root/'focus-request.json');self.assertEqual(request['profile'],'tile-budget');self.assertNotIn('tile_megapixels',request)
        again=S.resolve(self.repo,self.cfg,request);self.assertEqual(S.plan(again)['case_count'],12)
        one=C.read_json(case/'focus-request.json');self.assertNotIn('profile',one);self.assertEqual(one['tile_megapixels'],64)
        resolved=S.resolve(self.repo,self.cfg,one);self.assertEqual(S.plan(resolved)['case_count'],1)
        self.assertEqual(resolved['focused_targets'][0]['sheets'],j['candidate']['sheet_names'])
    def test_actual_fallback_preserves_scope_and_counts_failed_cost(self):
        cfg=self.resolve('tile-budget');root,obj=self.state(cfg);c,t,r=next(x for x in Q.planned_jobs(cfg) if x[0]['tile_megapixels']==64)
        f=next(x for x in obj.state['files'] if x['relative_path']==t['workbook']);seen=[]
        def run(stage,cc,ff,rr):
            seen.append(copy.deepcopy(cc));j=obj.ensure_job(stage,cc,ff,rr)
            if cc['tile_megapixels']==64:j.update(status='timeout',metrics=None,case_seconds=7)
            else:self.finish(obj,cc,t,rr,1000)
            return j
        with patch.object(obj,'run_job',run):j=obj.run_policy('focus',c,f,r)
        self.assertEqual(len(seen),2);self.assertEqual(seen[0]['sheet_names'],seen[1]['sheet_names']);self.assertEqual(seen[1]['target_id'],c['target_id'])
        self.assertEqual(j['recovery_policy']['total_workload_ms'],8000)
        self.assertEqual(j['status'],'timeout')
    def test_reports_have_question_links_and_bundle_entries(self):
        root,obj=self.complete('fixed-width');obj.save();C.write_bundle(root,obj.state,renderer=F.render_focus)
        self.assertIn('question-summary.json',(root/'report.html').read_text())
        with zipfile.ZipFile(root/'review-metadata.zip') as z:
            self.assertIn('question-summary.json',z.namelist());self.assertIn('question-cases.csv',z.namelist());self.assertFalse(any(n.endswith('.png') for n in z.namelist()))
    def test_profile_inputs_are_never_modified(self):
        before={str(p):p.read_bytes() for p in (self.repo/'corpus-local').rglob('*.xlsx')};self.resolve('hosts')
        self.assertTrue(all(Path(p).read_bytes()==b for p,b in before.items()))
    def test_no_new_web_module_routes_required(self):
        self.assertFalse((EXP/'_shared/office-addin/web/fixed-width.js').exists())
    def test_wpf_profile_controls_and_request_exclusion(self):
        txt=(EXP/'_shared/decision/focus-gui.ps1').read_text(encoding='utf-8-sig')
        self.assertIn('x:Name="ProfileBox"',txt);self.assertIn("if(-not $ui[$d[0]].IsEnabled){continue}",txt)
        ps=(EXP/'decision-campaign.ps1').read_text(encoding='utf-8-sig');self.assertIn("QuestionProfile='profile'",ps)
    def test_observer_detaches_before_disposal(self):
        ps=(EXP/'_shared/office-addin/run.ps1').read_text(encoding='utf-8-sig')
        self.assertLess(ps.index('Detach-VVFocusProcess'),ps.index('$launchProcess.Dispose()'))
        self.assertIn('Complete-VVFocusResult $focusObserver $resultState',ps)
        text=(EXP/'_shared/office-addin/powershell/focus-observer.ps1').read_text(encoding='utf-8-sig')
        self.assertIn("$Observer.State.capture_status=$CaptureStatus",text);self.assertIn("'process-exited'",text)

if __name__=='__main__':unittest.main()
