"""Pure source/integration tests. Native lock/rename tests live in the sibling PS1."""
from pathlib import Path
from unittest import TestCase, main
from unittest.mock import patch
import sys,json,re,tempfile
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'_shared/decision'))
import focus

def text(rel):return (ROOT/rel).read_text(encoding='utf-8-sig')

class SnapshotPublicationContract(TestCase):
    def test_no_in_place_copy_for_live_json(self):
        s=text('_shared/reporting.ps1').split('function Write-VVJson(',1)[1].split('function Get-VVRepo(',1)[0]
        self.assertNotIn('[IO.File]::Copy',s)
        self.assertNotIn('[IO.File]::Replace',s)
        self.assertIn('[VVSnapshotFile]::WriteText',s)
        self.assertIn('[VVSnapshotFile]::ReadTextOrNull',s)
        self.assertIn('ConvertFrom-Json -ErrorAction Stop',s)

    def test_publisher_stages_closes_and_renames(self):
        s=text('_shared/office-addin/hosts/dotnet/SnapshotPublication.cs')
        self.assertIn('Guid.NewGuid().ToString("N")',s)
        self.assertIn('stream.Flush(true)',s)
        self.assertIn('0x1U|0x8U',s)
        self.assertIn('code!=5 || !TryWslRename(pending,full)',s)
        self.assertIn('process.WaitForExit(15000)',s)
        self.assertIn('/usr/bin/mv -f --',s)
        self.assertNotIn('File.Copy(',s)
        self.assertNotIn('File.Delete(',s)
        self.assertNotIn('0x2U',s) # no cross-volume copy fallback
        self.assertIn('PendingComplete',s)
        self.assertIn('"publish",full,pending,true',s)

    def test_bounded_retry_not_generic_error_suppression(self):
        s=text('_shared/office-addin/hosts/dotnet/SnapshotPublication.cs')
        self.assertIn('int[] Delays = {20, 40, 80, 160, 320, 500}',s)
        self.assertIn('attempt==Delays.Length',s)
        self.assertIn('code==32 || code==33 || (publication && code==5)',s)
        self.assertIn('if(code==2 || code==3)return null',s)
        self.assertIn('throw new VVSnapshotIOException("read"',s)

    def test_shared_writer_has_no_office_dependency(self):
        s=text('_shared/office-addin/hosts/dotnet/SnapshotPublication.cs')
        for token in ('System.Drawing','WindowsBase','PresentationCore','Excel','SixLabors'):
            self.assertNotIn(token,s)
        init=text('_shared/report-io.ps1')
        self.assertIn("office-addin\\hosts\\dotnet\\SnapshotPublication.cs",init)
        self.assertIn('Add-Type -Path $script:VVSnapshotSource',init)

    def test_hosts_use_closed_snapshot_publication(self):
        p=text('_shared/office-addin/hosts/dotnet/Program.cs')
        self.assertNotIn('File.Move(outputPath + ".new"',p)
        self.assertNotIn('File.Move(path+".new"',p)
        self.assertIn('VVSnapshotFile.WriteText(outputPath, text)',p)
        self.assertIn('VVSnapshotFile.WriteText(path, text)',p)
        self.assertIn('VVSnapshotFile.WriteText(destination, text)',text('_shared/office-addin/hosts/dotnet/DiagnosticEndpoints.cs'))

    def test_status_requires_published_not_just_received_result(self):
        s=text('_shared/office-addin/hosts/dotnet/Program.cs')
        status=s.split('app.MapGet("/status"',1)[1].split('app.MapPost(',1)[0]
        self.assertIn('publishedResults.ContainsKey(workbook)',status)
        self.assertNotIn('results.ContainsKey(workbook)',status)
        write=s.split('async Task WriteOutputAsync()',1)[1].split('var builder',1)[0]
        self.assertLess(write.index('VVSnapshotFile.WriteText(outputPath, text)'),write.index('publishedResults[name] = 1'))

    def test_permanent_result_read_error_is_not_swallowed(self):
        self.assertIn('Experiment host reported: durable result read failed:',text('_shared/office-addin/run.ps1'))

    def test_retries_are_reported_not_used_to_change_deadlines(self):
        s=text('_shared/decision/supervise.ps1')
        self.assertIn("$receipt.failure_category='report-io'",s)
        self.assertIn('reader_retries=[VVSnapshotFile]::ReadRetries',s)
        self.assertIn('Get-VVFocusDeadlineFailure',s)
        self.assertIn('total_remaining_seconds',s)

    def test_focus_report_separates_infrastructure(self):
        j={'id':'x','status':'failed','metrics':{},'supervisor':{'failure_category':'report-io'},'error':'VV_SNAPSHOT_IO: read'}
        with tempfile.TemporaryDirectory() as d:
            value=focus.case_diagnostic(Path(d),j)
        self.assertEqual(value['category'],'infrastructure / report I/O')

    def test_csharp_native_error_checks_have_specific_code_paths(self):
        s=text('_shared/office-addin/hosts/dotnet/SnapshotPublication.cs')
        self.assertIn('Win32Exception',s)
        self.assertIn('hr & 0xffff0000U',s)
        self.assertIn('code==32 || code==33',s)
        self.assertIn('create?0U:7U',s)

    def test_native_test_runs_before_excel_on_report_filesystem(self):
        p=text('_shared/decision/focus-preflight.ps1')
        self.assertIn("$snapshotRoot=if($RunRoot){$RunRoot}",p)
        self.assertIn('snapshot-publication.tests.ps1',p)
        self.assertIn('snapshot-preflight.json',p)
        s=text('tests/snapshot-publication.tests.ps1')
        self.assertIn('VVSnapshotTestRelease',s)
        self.assertIn('[IO.FileShare]::None',s)
        self.assertIn('capture',s) # identity tests, not Excel invocation
        self.assertNotIn('Start-Excel',s)
        self.assertNotIn('New-Object -ComObject',s)
        self.assertIn('permanent conflict preserves old snapshot',s)
        self.assertIn('separate Windows PowerShell writer and polling reader',s)

    def test_shared_launch_identity_does_not_change_diagnostic_write_token(self):
        ps=text('_shared/office-addin/powershell/asset-preflight.ps1')
        self.assertIn('$Config.host_run_id -cne $LaunchId',ps)
        self.assertIn("$Experiment -eq 'range-image'",ps)
        self.assertIn('$Config.diagnostic_run_id -cnotmatch',ps)
        cs=text('_shared/office-addin/hosts/dotnet/Program.cs')
        self.assertIn('host_run_id = hostRunId',cs)
        self.assertIn('diagnostic_run_id = diagnosticRunId',cs)
        self.assertIn('diagnosticRunId, StringComparison.Ordinal',cs)
        self.assertIn('"host_run_id": state.config.capture_run_id',text('_shared/office-addin/hosts/rust/src/main.rs'))

    def test_no_mode_cannot_start_a_campaign(self):
        s=text('decision-campaign.ps1')
        guard=s.index('if(-not $Focused -and -not $Finalize -and -not $Campaign)')
        load=s.index(". (Join-Path $PSScriptRoot '_shared\\reporting.ps1')")
        self.assertLess(guard,load)
        self.assertIn('return',s[guard:load])
        self.assertIn('[switch]$Campaign',s)

    def test_legacy_campaign_task_is_explicit_other_tasks_unchanged(self):
        root=ROOT.parents[2]
        task=json.loads((root/'.vscode/tasks.json').read_text('utf-8-sig'))['tasks']
        t=next(x for x in task if x['label']=='Visual validation: Decide strategy (all workbooks)')
        self.assertIn('-Campaign',t['windows']['args'])
        self.assertIn('-Campaign',t['args'][1])
        f=next(x for x in task if x['label']=='Visual validation: Finalize capture strategy')
        self.assertIn('-Finalize',f['windows']['args'])

    def test_remaining_selection_does_not_rerun_long_sheet(self):
        s=json.loads(text('capture-selections/kernhaushalt-remaining.json'))
        sheets=s['sheets']
        self.assertEqual(sheets,['csv-71717-'+str(i) for i in range(23,49)])
        self.assertNotIn('csv-71717-22',sheets)
        self.assertEqual(s['capture_timeout_seconds'],1800)
        self.assertEqual(s['host_kind'],'dotnet')

if __name__=='__main__':main()
