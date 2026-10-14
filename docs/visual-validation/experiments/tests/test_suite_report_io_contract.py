"""Source-contract tests, not a Windows native IO execution test."""
import unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
class ReportIOContract(unittest.TestCase):
    def test_native_unicode_extended_paths(self):
        s=(ROOT/'_shared/report-io.cs').read_text(encoding='utf-8-sig')
        for token in ['GetFullPathNameW','FindFirstFileW','FindNextFileW','CreateFileW','GetFileAttributesW','SafeFileHandle',r'\\?\UNC']:
            self.assertIn(token,s)
        self.assertNotIn('WindowsBase',s)
    def test_no_legacy_recursive_enumeration_in_reports(self):
        for rel in ['_shared/reporting.ps1','_shared/report/inspector.ps1','_shared/suite/metrics.ps1']:
            self.assertNotIn('Get-ChildItem -LiteralPath',(ROOT/rel).read_text(encoding='utf-8-sig'),rel)
    def test_read_failures_not_silently_absent(self):
        s=(ROOT/'_shared/report-io.cs').read_text(encoding='utf-8-sig')
        self.assertIn('if(e==2 || e==3)return false',s)
        self.assertIn('throw Error("GetFileAttributesW"',s)
    def test_pixel_reader_uses_streams(self):
        s=(ROOT/'_shared/suite/metrics.ps1').read_text(encoding='utf-8-sig')
        self.assertIn('[VVSuitePixels]::ReadStream',s)
        self.assertNotIn('Test-Path -LiteralPath $full',s)
        self.assertIn('[VVReportIO]::OpenRead($full)',s)
    def test_archive_verifies_before_publishing(self):
        s=(ROOT/'_shared/reporting.ps1').read_text(encoding='utf-8-sig')
        i=s.index("$phase='verification';Test-VVMetadataZip")
        self.assertLess(i,s.index("$phase='publication'",i))
        self.assertIn("manifest.status -ne 'complete'",s)
        self.assertIn('stored_bytes',s)
        self.assertIn('$item.sha256',s)
        self.assertIn("cases/'+$job.id+'/case-metrics.json",s)
    def test_repair_preserves_timing_and_stays_offline(self):
        s=(ROOT/'repair-suite.ps1').read_text(encoding='utf-8-sig')
        self.assertIn("foreach($field in @('workload_ms','invocation_error'))",s)
        self.assertIn('Save-VVRepairOriginal',s)
        self.assertNotIn('Start-Process',s)
        self.assertNotIn('New-Object -ComObject',s)
if __name__=='__main__':unittest.main()
