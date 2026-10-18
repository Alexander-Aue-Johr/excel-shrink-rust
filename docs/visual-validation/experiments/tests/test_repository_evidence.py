"""Review projections, immutable evidence and read-only Git audit. No Excel calls."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile
from unittest.mock import patch

EXP = Path(__file__).resolve().parents[1]
ROOT = EXP.parents[2]
sys.path.insert(0, str(EXP/'_shared/suite'))
sys.path.insert(0, str(ROOT/'tools'))
import common as C
import archive as Z
import analysis as A
import publication as U
import report as V
import repository_policy as G
from test_evaluation import make_case


def example_state():
    c = dict(kind='capture', host_kind='dotnet', overflow_mode='selective', tile_megapixels=32,
             sheet_names=['tail '], block_id='test-block', comparison_variant='dotnet-selective-32',
             label='Test', phase_limits={'capture_timeout_seconds':600}, settings={'strategy':'F'})
    j = dict(id='000001-test', key='test', sequence=1, candidate=c, input_id='one', round=1,
             stage='individual', workbook='input.xlsx', status='complete', case_seconds=2,
             report='cases/000001-test/report.html', error=None)
    j['metrics'] = make_case(j)
    plan = dict(protocol_id='test-v1', mode='single', jobs=[copy.deepcopy(j)],
                selected_policy={}, decision_policy={'prefer_smaller_budget_within_fraction':.05},
                groups=[dict(id='individual', description='Test', question='capture')], limits={})
    return dict(schema='vv-evaluation-run-v1', run_id='test-run', source_version='test-source',
                status='complete', plan=plan, jobs=[j], files=[], errors=[], ended_utc='2026-10-18T00:00:00Z')


class ReviewProjection(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        self.root = self.repo/'docs/visual-validation/results/runs/test-run'
        self.root.mkdir(parents=True)
        self.state = example_state()
        for name, value in [('run.json', {'experiment':'visual-evaluation'}),
                            ('evaluation-state.json', self.state),
                            ('resolved-plan.json', self.state['plan']), ('source-manifest.json', {})]:
            C.atomic_json(self.root/name, value)
        self.case = self.root/'cases/000001-test'
        self.case.mkdir(parents=True)
        self.local_html = b'<html><body>Full local image inspector</body></html>'
        (self.case/'report.html').write_bytes(self.local_html)
        C.atomic_json(self.case/'case-metrics.json', self.state['jobs'][0]['metrics'])
        C.atomic_json(self.case/'case-spec.json', {'job':self.state['jobs'][0]})
        C.atomic_json(self.case/'supervisor-result.json', {'status':'complete'})
        (self.case/'inputs/plans').mkdir(parents=True)
        (self.case/'inputs/plans/visual-plan.json').write_text('{"repeated":true}')
        (self.case/'artifact.png').write_bytes(b'not-an-image-test-placeholder')
    def tearDown(self):
        self.tmp.cleanup()
    def bundle(self, detail='compact'):
        Z.write_bundle(self.root, self.state, V.render, detail=detail)
        return self.root/'review-metadata.zip'
    def test_complete_metrics_and_local_inspector_preserved(self):
        path = self.bundle()
        manifest = Z.verify_bundle(path)
        self.assertEqual(manifest['profile'], 'compact-metadata-v1')
        with zipfile.ZipFile(path) as z:
            self.assertEqual(z.read('cases/000001-test/case-metrics.json'), (self.case/'case-metrics.json').read_bytes())
            self.assertIn(b'Metadata review view', z.read('cases/000001-test/report.html'))
        self.assertEqual((self.case/'report.html').read_bytes(), self.local_html)
    def test_omitted_plan_has_hash_and_stays_local(self):
        path = self.bundle(); manifest = Z.verify_bundle(path)
        rows = [x for x in manifest['omitted'] if x['path'].endswith('visual-plan.json')]
        self.assertEqual(rows[0]['sha256'], C.digest(self.case/'inputs/plans/visual-plan.json'))
        self.assertTrue((self.case/'inputs/plans/visual-plan.json').is_file())
    def test_long_log_excerpt_is_explicit_not_original_filename(self):
        raw = b'FIRST\n' + b'x'*(Z.LOG_LIMIT+1000) + b'\nLAST'
        (self.case/'worker.stderr.log').write_bytes(raw)
        path = self.bundle()
        with zipfile.ZipFile(path) as z:
            self.assertNotIn('cases/000001-test/worker.stderr.log', z.namelist())
            text = z.read('cases/000001-test/worker.stderr.log.excerpt.txt')
            self.assertIn(b'NOT the complete log', text)
            self.assertIn(b'FIRST',text); self.assertIn(b'LAST',text)
            self.assertIn(hashlib.sha256(raw).hexdigest().encode(),text)
        self.assertEqual((self.case/'worker.stderr.log').read_bytes(), raw)
    def test_full_detail_retains_original_plan_and_log(self):
        raw=b'x'*(Z.LOG_LIMIT+1);(self.case/'worker.stderr.log').write_bytes(raw)
        path=self.bundle('full')
        with zipfile.ZipFile(path) as z:
            self.assertEqual(z.read('cases/000001-test/worker.stderr.log'),raw)
            self.assertIn('cases/000001-test/inputs/plans/visual-plan.json',z.namelist())
            self.assertNotIn('cases/000001-test/artifact.png',z.namelist())
        self.assertEqual(Z.verify_bundle(path)['profile'],'full-metadata-v1')
    def test_non_capture_measurement_not_omitted(self):
        self.state['jobs'][0]['candidate']['kind']='diagnostic'
        path=self.bundle()
        with zipfile.ZipFile(path) as z:
            self.assertIn('cases/000001-test/inputs/plans/visual-plan.json',z.namelist())
    def test_unlisted_entry_rejected(self):
        path=self.bundle()
        with zipfile.ZipFile(path,'a') as z:z.writestr('extra.json','{}')
        with self.assertRaises(ValueError):Z.verify_bundle(path)
    def test_archive_hash_mismatch_rejected(self):
        path=self.bundle();other=self.root/'mutated.zip'
        with zipfile.ZipFile(path) as src,zipfile.ZipFile(other,'w') as dst:
            for name in src.namelist():dst.writestr(name,b'{}' if name=='summary.json' else src.read(name))
        with self.assertRaises(ValueError):Z.verify_bundle(other)
    def test_failed_verification_preserves_previous_archive(self):
        path=self.bundle();old=path.read_bytes()
        with patch.object(Z,'verify_bundle',side_effect=ValueError('test corruption')):
            with self.assertRaises(ValueError):self.bundle()
        self.assertEqual(path.read_bytes(),old)
        self.assertEqual(C.read_json(self.root/'bundle-status.json')['status'],'failed')
    def test_compact_bundle_publishes_even_with_original_local_inspector(self):
        self.bundle();dest=U.publish(self.repo,self.root)
        record=C.read_json(dest/'results.json')
        self.assertEqual(record['observations'][0]['requested_sheets'],['tail '])
        self.assertEqual(record['observations'][0]['phase_limits']['capture_timeout_seconds'],600)
        self.assertEqual((self.case/'report.html').read_bytes(),self.local_html)
        self.assertTrue(U.verify_record(dest))
    def test_partly_failed_repeats_are_not_reported_equal(self):
        obs=U.observation(self.state['jobs'][0]);obs['source_run']='test-run'
        failed=copy.deepcopy(obs);failed.update(status='failed',metrics_status='failed',qualified=False)
        record={'id':'partial','scope':'Test','sources':[dict(run_id='test-run')],
                'observations':[obs,copy.deepcopy(obs),failed]}
        out=self.repo/'partial';U.write_record(out,record)
        text=(out/'index.html').read_text()
        self.assertIn('2/3',text);self.assertIn('not fully established',text)
        self.assertNotIn('<td>equal</td>',text)
    def test_publisher_environment_change_blocks_repeat_claim(self):
        a=U.observation(self.state['jobs'][0]);a['source_run']='test-run'
        b=copy.deepcopy(a);b['environment'][0]['device_pixel_ratio']=2
        out=self.repo/'different';U.write_record(out,{'id':'different','scope':'Test',
            'sources':[dict(run_id='test-run')],'observations':[a,b]})
        self.assertNotIn('<td>equal</td>',(out/'index.html').read_text())
    def test_failures_grouped_without_stack_or_machine_paths(self):
        a=copy.deepcopy(self.state['jobs'][0]);a['status']='failed'
        s=a['metrics']['workbooks'][0]['sheets'][0];s['status']='failed'
        s['error']=dict(code='DRAWING_BOUND_UNRESOLVED',stack='C:\\Users\\private',
            detail={'axis':'y','target_points':45,'final_checks':[{'edge_points':40}]})
        b=copy.deepcopy(a);b['id']='000002-test'
        issues=A.worksheet_issues([a,b])
        self.assertEqual(len(issues),1);self.assertEqual(len(issues[0]['cases']),2)
        self.assertEqual(issues[0]['sheet'],'tail ')
        self.assertNotIn('private',json.dumps(issues))
    def test_full_option_is_report_only_and_forwarded(self):
        source=(EXP/'reproduce.ps1').read_text(encoding='utf-8-sig')
        self.assertIn("if($FullMetadata){$extra+='--full-metadata'}",source)
        self.assertIn('$FullMetadata -and -not $ReportOnly',source)
    def test_supervised_child_requests_text_output(self):
        s=(EXP/'_shared/suite/supervise.ps1').read_text(encoding='utf-8-sig')
        self.assertIn("'-OutputFormat','Text'",s)


class RepositoryMembership(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        subprocess.run(['git','init','-q',str(self.root)],check=True)
    def tearDown(self):self.tmp.cleanup()
    def git(self,*args):return subprocess.check_output(['git','-C',str(self.root),*args])
    def put(self,name,text='input'):
        p=self.root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(text);return p
    def assets(self):
        for p in G.FIXTURES|G.HELPERS:self.put(p)
    def test_no_git_is_unavailable_not_clean(self):
        with tempfile.TemporaryDirectory() as t:self.assertEqual(G.audit(Path(t))['status'],'unavailable')
    def test_required_inputs_can_be_tracked_without_generated_warning(self):
        self.assets();self.git('add','--','.')
        self.assertEqual(G.audit(self.root)['status'],'passed')
    def test_generated_file_detection_and_index_unchanged(self):
        self.assets();self.put('.tmp/retained.json','{}');self.git('add','--','.')
        before=self.git('ls-files','--stage','-z');index=(self.root/'.git/index').read_bytes()
        result=G.audit(self.root)
        self.assertIn('tracked-generated',{x['code'] for x in result['findings']})
        self.assertEqual(before,self.git('ls-files','--stage','-z'))
        self.assertEqual(index,(self.root/'.git/index').read_bytes())
        self.assertTrue((self.root/'.tmp/retained.json').exists())
    def test_missing_standalone_lock_is_not_generated(self):
        self.assets();self.put('Cargo.toml','[workspace]\nmembers=[]\n');self.git('add','--','.')
        result=G.audit(self.root)
        self.assertIn(('Cargo.lock','missing-input'),{(r['path'],r['code']) for r in result['findings']})
        self.assertIsNone(G.generated_reason('Cargo.lock'))
    def test_local_ignored_lock_reported_untracked(self):
        self.assets();self.put('Cargo.toml','[workspace]\n');self.put('.gitignore','/Cargo.lock\n')
        self.put('Cargo.lock','exact local dependencies');self.git('add','--','.')
        result=G.audit(self.root)
        self.assertIn(('Cargo.lock','input-not-tracked'),{(r['path'],r['code']) for r in result['findings']})
    def test_ignored_tracked_filename_with_space_preserved(self):
        self.assets();self.put('name with space.txt');self.git('add','--','.')
        self.put('.gitignore','*.txt\n');self.git('add','--','.gitignore')
        result=G.audit(self.root)
        self.assertIn(('name with space.txt','tracked-but-ignored'),{(r['path'],r['code']) for r in result['findings']})
    def test_curated_file_change_is_detected(self):
        self.assets();base='docs/visual-validation/evidence/test/'
        self.put(base+'results.json','{}')
        self.put(base+'manifest.json',json.dumps({'files':[{'path':'results.json','bytes':2,'sha256':'0'*64}]}))
        self.git('add','--','.')
        self.assertIn('evidence-integrity',{r['code'] for r in G.audit(self.root)['findings']})
    def test_pinned_fixture_allowlist_excludes_arbitrary_outputs(self):
        self.assertIsNone(G.generated_reason(next(iter(G.FIXTURES))))
        self.assertTrue(G.generated_reason('docs/visual-validation/results/run.png'))
        self.assertTrue(G.generated_reason('docs/visual-validation/experiments/a/results/output.json'))
    def test_planner_lock_not_ignored_by_patch(self):
        text=(EXP/'range-image/rust-ooxml-plan/.gitignore').read_text()
        self.assertNotIn('Cargo.lock',text)

if __name__=='__main__':unittest.main()
