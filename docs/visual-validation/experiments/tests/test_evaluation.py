"""Portable planner, controller and publication tests. Excel is not invoked."""
import copy,io,json,sys,tempfile,unittest,zipfile,hashlib,importlib.util,re
from pathlib import Path
from unittest.mock import patch
import xml.etree.ElementTree as ET
EXP=Path(__file__).resolve().parents[1];ROOT=EXP.parents[2];sys.path.insert(0,str(EXP/'_shared/suite'))
import common as C,plan as P,engine as N,analysis as A,report as V,archive as Z,publication as U

def fixture(path,names,hidden=()):
    path.parent.mkdir(parents=True,exist_ok=True)
    root=ET.Element('workbook',xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main');ss=ET.SubElement(root,'sheets')
    for i,n in enumerate(names):ET.SubElement(ss,'sheet',name=n,sheetId=str(i+1),state='hidden' if n in hidden else 'visible')
    with zipfile.ZipFile(path,'w') as z:z.writestr('xl/workbook.xml',ET.tostring(root,encoding='utf-8'))

def make_case(j):
    names=j['candidate']['sheet_names'] or [];w=Path(j['workbook']).name
    canonical=[dict(sheet=n,sheet_index=i+1,workbook=w,status='complete',width=5,height=8,algorithm='test-pixel-v1',sha256=C.obj_digest(n),tiles=1,overlap_rows_checked=0) for i,n in enumerate(names)]
    sheets=[dict(sheet=n,sheet_index=i+1,address='A1:B4',status='complete') for i,n in enumerate(names)]
    env={'office':{'version':'test','platform':'PC'},'device_pixel_ratio':1,'screen':{'width':1600,'height':900},'browser':'test'}
    return dict(schema='vv-suite-case-metrics-v1',status='complete',workload_ms=1000,canonical_sheets=canonical,workbooks=[dict(workbook=w,kind='capture',status='complete',sheets=sheets,environment=env,context_policy='visible-cell-halo-v1',capture_count=len(names))],report_publication={'status':'ready'},pixel_verification_status='complete',captures=[])

class Planner(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name);self.cfg=json.loads((EXP/'reproduce.json').read_text());catalog={}
        for g in self.cfg['groups']:
            for t in g['targets']:catalog.setdefault(t['workbook'],[]).extend(t.get('sheets') or ['Sheet 1'])
        for pin in self.cfg['inputs']:
            p=self.root/'corpus-local'/pin['path'];fixture(p,list(dict.fromkeys(catalog[pin['path']])));pin['bytes']=p.stat().st_size;pin['sha256']=C.digest(p)
    def tearDown(self):self.tmp.cleanup()
    def req(self,**kw):return dict(schema='vv-experiment-request-v1',mode='single',workbook=self.cfg['inputs'][0]['path'],**kw)
    def test_suite_has_62_pinned_cases(self):
        p=P.resolve(self.root,self.cfg);self.assertEqual(len(p['jobs']),62);self.assertEqual(len(p['files']),8)
        self.assertEqual({g:sum(j['stage']==g for j in p['jobs']) for g in ('coverage','host-overflow','tile-budget')},{'coverage':8,'host-overflow':36,'tile-budget':18})
    def test_group_only_budget(self):
        p=P.resolve(self.root,self.cfg,{'groups':['tile-budget']});self.assertEqual(len(p['jobs']),18);self.assertEqual(len(p['files']),2)
    def test_every_budget_occupies_each_position(self):
        p=P.resolve(self.root,self.cfg,{'groups':['tile-budget']})
        for target in {'wide-table','long-table'}:
            for mp in (16,32,64):
                positions=[]
                for r in (1,2,3):
                    js=[j for j in p['jobs'] if j['candidate']['block_id']==target and j['round']==r]
                    positions.append(next(i for i,j in enumerate(js) if j['candidate']['tile_megapixels']==mp))
                self.assertEqual(set(positions),{0,1,2})
    def test_exact_sheets_and_case_limits(self):
        f=self.cfg['inputs'][0]['path'];fixture(self.root/'corpus-local'/f,[' lead','tail ','H'],['H'])
        p=P.resolve(self.root,self.cfg,self.req(sheets=['tail ',' lead'],limits={'capture_timeout_seconds':1800}))
        self.assertEqual(p['jobs'][0]['candidate']['sheet_names'],[' lead','tail ']);self.assertEqual(p['jobs'][0]['candidate']['phase_limits']['capture_timeout_seconds'],1800)
    def test_no_trim_or_hidden_mutation(self):
        f=self.cfg['inputs'][0]['path'];fixture(self.root/'corpus-local'/f,[' lead','H'],['H'])
        for names in (['lead'],['H']):
            with self.assertRaises(ValueError):P.resolve(self.root,self.cfg,self.req(sheets=names))
    def test_native_rejects_sheet_subset(self):
        with self.assertRaises(ValueError):P.resolve(self.root,self.cfg,self.req(experiment='shape-bounds',sheets=['Sheet 1']))
    def test_native_has_reference_and_setup(self):
        p=P.resolve(self.root,self.cfg,self.req(experiment='native-bitmap'))
        self.assertEqual([j['candidate']['kind'] for j in p['jobs']],['lab-setup','capture','lab-capture'])
        self.assertEqual(p['jobs'][-1]['candidate']['native_id'],'lab-bitmap')
    def test_fixture_without_corpus(self):
        p=P.resolve(self.root,self.cfg,dict(mode='single',experiment='units',corpus_root='does-not-exist'))
        self.assertEqual(len(p['files']),0);self.assertEqual(p['jobs'][0]['candidate']['suite'],'units')
    def test_bad_input_hash_blocks_suite(self):
        self.cfg['inputs'][0]['sha256']='0'*64
        with self.assertRaises(ValueError):P.resolve(self.root,self.cfg)
    def test_suite_parameter_factor_override_rejected(self):
        with self.assertRaises(ValueError):P.resolve(self.root,self.cfg,{'settings':{'tile_megapixels':64}})
    def test_unknown_keys_and_undersized_session(self):
        for r in ({'something':1},{'limits':{'session_timeout_seconds':60}}):
            with self.assertRaises(ValueError):P.resolve(self.root,self.cfg,r)
    def test_full_and_selected_scope_cannot_mix(self):
        with self.assertRaises(ValueError):P.resolve(self.root,self.cfg,dict(mode='single',workbook='all',sheets=['Sheet 1']))
    def test_dtd_catalogue_rejected(self):
        p=self.root/'bad.xlsx'
        with zipfile.ZipFile(p,'w') as z:z.writestr('xl/workbook.xml','<!DOCTYPE workbook><workbook/>')
        with self.assertRaises(ValueError):P.sheet_catalogue(p)
    def test_sheets_index_order(self):
        f=self.cfg['inputs'][0]['path'];fixture(self.root/'corpus-local'/f,['a','b','c'])
        p=P.resolve(self.root,self.cfg,self.req(sheet_indexes=[3,1]));self.assertEqual(p['jobs'][0]['candidate']['sheet_names'],['a','c'])

class Execution(Planner):
    # Inherited planner assertions also run against this fixture type.
    def one(self):return P.resolve(self.root,self.cfg,self.req(sheets=['Sheet 1']))
    def runner(self,p):
        r=self.root/'docs/visual-validation/results/runs/testrun';r.mkdir(parents=True);C.atomic_json(r/'run.json',dict(schema='vv-report-v1',experiment='visual-evaluation',status='running'))
        return N.Runner(self.root,r,p)
    def test_successful_mock_execution_and_verified_archive(self):
        obj=self.runner(self.one())
        def run(stage,c,f=None,repeat=1,reference_job=None):
            j=obj.ensure_job(stage,c,f,repeat);j.update(status='complete',metrics=make_case(j),case_seconds=1);return j
        with patch.object(obj,'run_job',side_effect=run),patch.object(N.time,'sleep'):self.assertEqual(obj.run(),0)
        with zipfile.ZipFile(obj.root/'review-metadata.zip') as z:
            self.assertIn('evaluation-state.json',z.namelist());self.assertIsNone(z.testzip())
            m=json.loads(z.read('bundle-manifest.json'))
            for row in m['included']:self.assertEqual(hashlib.sha256(z.read(row['path'])).hexdigest(),row['sha256'])
    def test_snapshot_io_failure_not_capture_failure(self):
        d=A.case_diagnostic(self.root,{'id':'one','status':'failed','error':'VV_SNAPSHOT_IO: read','supervisor':{}})
        self.assertEqual(d['category'],'infrastructure / report I/O')
    def test_same_scope_fallback_and_real_cost(self):
        p=self.one();obj=self.runner(p);c=copy.deepcopy(p['jobs'][0]['candidate']);c['tile_megapixels']=64;c['settings']['tile_megapixels']=64;calls=[]
        def run(stage,c,f=None,repeat=1,reference_job=None):
            calls.append(copy.deepcopy(c));j=obj.ensure_job(stage,c,f,repeat)
            if len(calls)==1:j.update(status='timeout',case_seconds=12,metrics={'status':'failed','workload_ms':9000})
            else:j.update(status='complete',case_seconds=4,metrics=make_case(j))
            return j
        f=obj.files[p['files'][0]['id']]
        with patch.object(obj,'run_job',side_effect=run):j=obj.run_policy('individual',c,f)
        self.assertEqual(len(calls),2);self.assertEqual(calls[0]['sheet_names'],calls[1]['sheet_names']);self.assertEqual(calls[1]['settings']['tile_megapixels'],16)
        self.assertEqual(j['recovery_policy']['total_workload_ms'],13000)
    def test_wrong_scope_is_not_qualified(self):
        j=self.one()['jobs'][0];j.update(status='complete',metrics=make_case(j));self.assertTrue(A.is_qualified(j));j['candidate']['sheet_names']=['missing'];self.assertFalse(A.is_qualified(j))
    def test_first_seam_requires_local_reference(self):
        j=self.one()['jobs'][0];j.update(status='complete',metrics=make_case(j));j['candidate']['context_validation']='first-seam';j['metrics']['canonical_sheets'][0]['tiles']=2
        self.assertFalse(A.is_qualified(j))
    def test_environment_difference_is_not_equal(self):
        a=self.one()['jobs'][0];a.update(status='complete',metrics=make_case(a));b=copy.deepcopy(a);b['metrics']['workbooks'][0]['environment']['device_pixel_ratio']=2
        self.assertEqual(A.comparable(a,b),'environment-changed')
    def test_pixel_difference_is_not_equal(self):
        a=self.one()['jobs'][0];a.update(status='complete',metrics=make_case(a));b=copy.deepcopy(a);b['metrics']['canonical_sheets'][0]['sha256']='f'*64
        self.assertEqual(A.comparable(a,b),'different-pixels')
    def test_completed_cases_are_not_reexecuted_on_resume(self):
        p=self.one();obj=self.runner(p);j=obj.state['jobs'][0];j.update(status='complete',metrics=make_case(j),case_seconds=1);obj.save()
        again=N.Runner(self.root,obj.root,p)
        with patch.object(again,'stage_input',side_effect=AssertionError('must not execute')):
            got=again.run_job(j['stage'],j['candidate'],again.files[j['input_id']],j['round'])
        self.assertEqual(got['status'],'complete')
    def test_missing_archive_cannot_publish(self):
        obj=self.runner(self.one());obj.state['status']='complete';obj.save()
        with self.assertRaises(FileNotFoundError):U.publish(self.root,obj.root)
    def test_publish_no_machine_paths_and_immutable_id(self):
        obj=self.runner(self.one());j=obj.state['jobs'][0];j.update(status='complete',metrics=make_case(j),case_seconds=1);j['error']='C:\\Users\\secret\\token';obj.state['status']='complete';obj.save();Z.write_bundle(obj.root,obj.state,V.render)
        dest=U.publish(self.root,obj.root);self.assertTrue(U.verify_record(dest));text=(dest/'results.json').read_text();self.assertNotIn('C:\\\\Users',text);self.assertNotIn('secret',text)
        with self.assertRaises(ValueError):U.publish(self.root,obj.root)
    def test_archive_mutation_blocks_publication(self):
        obj=self.runner(self.one());obj.state['status']='complete';obj.save();Z.write_bundle(obj.root,obj.state,V.render)
        with zipfile.ZipFile(obj.root/'review-metadata.zip','a') as z:z.writestr('extra.json','{}')
        with self.assertRaises(ValueError):U.publish(self.root,obj.root)
    def test_source_changed_rejects_resume(self):
        p=self.one();obj=self.runner(p);src=self.root/'docs/visual-validation/experiments/new.py';src.parent.mkdir(parents=True,exist_ok=True);src.write_text('x=1')
        with self.assertRaises(ValueError):N.Runner(self.root,obj.root,p)

class Repository(unittest.TestCase):
    def test_gui_names_are_real(self):
        s=(EXP/'gui.ps1').read_text(encoding='utf-8-sig');xml=s.split("$xaml=@'",1)[1].split("'@",1)[0];root=ET.fromstring(xml)
        names={x.attrib.get('{http://schemas.microsoft.com/winfx/2006/xaml}Name') for x in root.iter()}
        refs=set(re.findall(r'\$ui\.([A-Za-z0-9_]+)',s));self.assertFalse(refs-names)
    def test_powershell_files_use_utf8_bom_for_nonascii(self):
        for p in [EXP/'gui.ps1',EXP/'reproduce.ps1']:
            self.assertTrue(p.read_bytes().startswith(b'\xef\xbb\xbf'))
    def test_tasks_and_launch_ids(self):
        t=json.loads((ROOT/'.vscode/tasks.json').read_text());launch=json.loads((ROOT/'.vscode/launch.json').read_text());labels={x['label'] for x in t['tasks']}
        self.assertEqual(len(labels),len(t['tasks']))
        for c in launch['configurations']:
            if c.get('preLaunchTask'):self.assertIn(c['preLaunchTask'],labels)
        for name in ('Reproduce capture decision','Validate experiment code','Publish evidence'):self.assertIn('Visual validation: '+name,labels)
    def test_snapshot_helper_used_and_no_deleted_routes(self):
        s=(EXP/'_shared/suite/supervise.ps1').read_text(encoding='utf-8-sig');self.assertIn('Get-VVPhaseDeadlineFailure',s);self.assertNotIn('..\\decision',s)
        s=(EXP/'_shared/suite/worker.ps1').read_text(encoding='utf-8-sig');self.assertNotIn('..\\decision',s)
    def test_curated_manifest(self):
        U.verify_record(EXP.parent/'evidence/20261017-reviewed-captures')
    def test_declared_source_deletions_have_no_root_entrypoints(self):
        for name in ('decision-campaign.ps1','capture-followup.json','suite.ps1','repair-suite.ps1'):self.assertFalse((EXP/name).exists())
    def test_controller_preserves_scalar_strings(self):
        s=(EXP/'_shared/suite/controller.ps1').read_text(encoding='utf-8-sig');self.assertLess(s.index('$Object -is [string]'),s.index('$Object -is [pscustomobject]'))

if __name__=='__main__':unittest.main()
