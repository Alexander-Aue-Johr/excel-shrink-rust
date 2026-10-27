# Run with Windows PowerShell; no existing workspace or corpus run is changed.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Threading.Thread]::CurrentThread.CurrentUICulture = [Globalization.CultureInfo]::GetCultureInfo('en-US')
Import-Module (Join-Path $PSScriptRoot 'Core.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'Standalone.psm1') -Force
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.IO.Compression
$root = Join-Path ([IO.Path]::GetTempPath()) ('excel-shrink-standalone-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$script:checks = 0
function Assert($Condition, [string]$Message) { if (-not $Condition) { throw $Message }; $script:checks++ }
function Reject([scriptblock]$Action, [string]$Message) { $failed=$false; try { & $Action } catch { $failed=$true }; Assert $failed $Message }
function Write-TestArchive([string]$Path, [byte]$Value=42) {
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))
    $zip = [IO.Compression.ZipFile]::Open($Path, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($name in @('nested/book A.xlsx', 'nested/book B.xls')) {
            $stream = $zip.CreateEntry($name).Open()
            try { $stream.WriteByte($Value) } finally { $stream.Dispose() }
        }
    } finally { $zip.Dispose() }
}
try {
    $source = Join-Path $root 'source/archive.zip'; Write-TestArchive $source
    $expected = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    $archiveDirectory = Join-Path $root 'native archives'
    $id = 'a' * 64
    $destination = Join-Path $archiveDirectory ($id.Substring(0,20) + '.zip')
    $receipt = Copy-StandaloneArchive $source $destination $expected
    Assert ($receipt.path -eq $destination -and $receipt.sha256 -ceq $expected) 'The copy receipt lost its native path or source SHA-256.'
    Assert ($receipt.bytes -eq [IO.File]::ReadAllBytes($source).Length) 'The copy receipt has the wrong byte length.'
    Assert ((Get-FileHash -LiteralPath $destination).Hash.ToLowerInvariant() -ceq $expected) 'Archive bytes changed during copying.'
    Assert ((Test-LocalWorkbookArchive $destination) -eq 2) 'Copying changed the workbook members.'
    $written = [IO.File]::GetLastWriteTimeUtc($destination)
    $again = Copy-StandaloneArchive $source $destination $expected
    Assert ($again.sha256 -ceq $expected -and [IO.File]::GetLastWriteTimeUtc($destination) -eq $written) 'An idempotent copy rewrote the existing archive.'
    $different = Join-Path $root 'different.zip'; Write-TestArchive $different 7
    Reject { Copy-StandaloneArchive $different $destination } 'A conflicting existing archive was overwritten.'
    Assert ((Get-FileHash -LiteralPath $destination).Hash.ToLowerInvariant() -ceq $expected) 'A rejected copy changed the existing archive.'
    $wrongHashTarget = Join-Path $root 'wrong-hash/archive.zip'
    Reject { Copy-StandaloneArchive $source $wrongHashTarget ('0' * 64) } 'A source with a mismatched expected SHA-256 was copied.'
    Assert (-not [IO.File]::Exists($wrongHashTarget)) 'A rejected source hash published an archive.'
    $invalid = Join-Path $root 'invalid.zip'; [IO.File]::WriteAllText($invalid, 'not a ZIP central directory')
    $invalidTarget = Join-Path $root 'failed/archive.zip'
    Reject { Copy-StandaloneArchive $invalid $invalidTarget } 'An invalid ZIP was published.'
    Assert (-not [IO.File]::Exists($invalidTarget)) 'A failed archive validation published an archive.'
    Assert (@(Get-ChildItem -LiteralPath $root -Recurse -File -Filter '*.partial').Count -eq 0) 'A failed validation left partial copies behind.'
    Reject { Copy-StandaloneArchive $source '\\wsl.localhost\Offline\archive.zip' } 'A WSL deployment destination was accepted.'
    Reject { Copy-StandaloneArchive $source '\\server\share\archive.zip' } 'A network deployment destination was accepted.'
    Reject { Copy-StandaloneArchive $source '/home/user/archive.zip' } 'A Linux deployment destination was accepted.'

    $configuration = [pscustomobject]@{
        version=1; access_url='https://example.invalid/retained-access';
        migration=[pscustomobject]@{source_files=@('historical configuration');archives_moved=0};
        wsl_workspace=[pscustomobject]@{source_repo='\\wsl.localhost\Offline\repo'};
        settings=[pscustomobject]@{output_root='.tmp/existing study';download_root='\\wsl.localhost\Offline\downloads';
            corpus='Data.gov';partition=0;partitions=4;order='size-desc';build_release=$true;import_report=$null;reuse_prepared_from=@('b' * 64)};
        corpora=@(
            [pscustomobject]@{name='Destatis';id=$id;total_workbooks=2;local_archive=$source;url='https://example.invalid/destatis.zip'},
            [pscustomobject]@{name='Enron';id=('c' * 64);total_workbooks=20872;local_archive='\\wsl.localhost\Offline\enron.zip';url='https://example.invalid/enron.zip'},
            [pscustomobject]@{name='Data.gov';id=('d' * 64);total_workbooks=9117;local_archive='\\wsl.localhost\Offline\data.zip';url='https://example.invalid/data.zip'}
        )
    }
    $before = $configuration | ConvertTo-Json -Depth 40 -Compress
    $native = New-StandaloneStudyConfiguration $configuration $root @('Destatis') $archiveDirectory
    Assert (($configuration | ConvertTo-Json -Depth 40 -Compress) -ceq $before) 'Preparing the deployment modified the original configuration.'
    Assert ($native.standalone_windows -and @($native.standalone_corpora).Count -eq 1 -and $native.standalone_corpora[0] -ceq 'Destatis') 'The deployment marker does not identify its explicit selected corpus.'
    Assert ($null -eq $native.PSObject.Properties['wsl_workspace'] -and -not $native.settings.build_release) 'The native configuration retained an implicit WSL build.'
    Assert ($native.settings.corpus -ceq 'Destatis' -and $native.settings.output_root -eq (Join-Path $root '.tmp/existing study') -and $native.settings.download_root -eq $archiveDirectory) 'The native selection, output directory or download directory changed unexpectedly.'
    Assert ($native.access_url -ceq $configuration.access_url -and $native.settings.reuse_prepared_from[0] -ceq ('b' * 64)) 'Deployment changed access metadata or approved preparation builds.'
    Assert ($native.corpora.Count -eq 3 -and $native.corpora[1].id -ceq ('c' * 64) -and $native.corpora[1].total_workbooks -eq 20872) 'Deployment removed inactive corpus identities or totals.'
    foreach ($entry in $native.corpora) {
        Assert ($null -eq $entry.url) 'A deployment source retained an HTTP fallback.'
        if ($entry.name -ne 'Destatis') { Assert ($null -eq $entry.local_archive) 'An inactive corpus retained a WSL path.' }
    }
    $request = ConvertTo-StudyRequest $native $root 'Destatis'
    Assert ($request.corpora.Count -eq 3) 'Native launch planning lost report identities.'
    foreach ($entry in $request.corpora) { Assert ($entry.url.StartsWith($archiveDirectory + '\')) 'A planned archive depends on WSL or a remote URL.' }
    Reject { New-StandaloneStudyConfiguration $configuration $root @('Missing') $archiveDirectory } 'An unknown corpus was selected.'
    Reject { New-StandaloneStudyConfiguration $configuration $root @('Destatis','destatis') $archiveDirectory } 'A repeated corpus selection was accepted.'
    Reject { New-StandaloneStudyConfiguration $configuration $root @('Enron') $archiveDirectory } 'An uncopied archive was accepted.'
    $configuration.settings.import_report = '\\wsl.localhost\Offline\old-report.json'
    Reject { New-StandaloneStudyConfiguration $configuration $root @('Destatis') $archiveDirectory } 'A WSL import dependency was accepted.'
    $configuration.settings.import_report = $null
    $configuration.settings.output_root = '\\wsl.localhost\Offline\study'
    Reject { New-StandaloneStudyConfiguration $configuration $root @('Destatis') $archiveDirectory } 'A WSL result directory was accepted.'
    Write-Host "Standalone deployment: $script:checks checks passed."
} finally { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
