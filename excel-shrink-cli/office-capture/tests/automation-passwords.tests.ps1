param([switch]$Run)

# Opt-in integration test: Windows PowerShell 5.1 and desktop Excel are required.
if (-not $Run) {
    Write-Host 'Run automation-passwords.tests.ps1 -Run to test unattended workbook opening with Excel.'
    return
}
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$regressionRoot = Join-Path $repoRoot 'excel-shrink-cli/src/regression'
$root = Join-Path $repoRoot ('.tmp/automation-passwords-' + [Guid]::NewGuid().ToString('N'))
$code = Join-Path $root 'code'
New-Item -ItemType Directory -Path $code -Force | Out-Null
foreach ($name in @('excel_open.ps1', 'excel_recalculate.ps1', 'archive_processes.ps1', 'ArchiveProcesses.cs')) {
    Copy-Item -LiteralPath (Join-Path $regressionRoot $name) -Destination $code
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../CaptureProcesses.cs') -Destination $code
Add-Type -Path (Join-Path $code 'CaptureProcesses.cs')

# Generate small, reproducible fixtures; no corpus files or real passwords are used.
$fixtureScript = Join-Path $code 'fixtures.ps1'
@'
param([string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'archive_processes.ps1')
Add-Type -Path (Join-Path $PSScriptRoot 'CaptureProcesses.cs')
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class FixtureWindow {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
}
"@
$existing = @(Get-Process EXCEL -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id)
$owned = New-Object CaptureProcesses
$excel = $null
$workbook = $null
$ownsExcel = $false
try {
    $excel = New-Object -ComObject Excel.Application
    $excelId = [uint32]0
    [void][FixtureWindow]::GetWindowThreadProcessId([IntPtr]$excel.Hwnd, [ref]$excelId)
    if ($existing -contains $excelId) { throw 'Excel reused an existing process; refusing to use it.' }
    $owned.Add((Get-Process -Id $excelId))
    $ownsExcel = $true
    $excel.Visible = $false
    $excel.Interactive = $false
    $excel.DisplayAlerts = $false
    $excel.EnableEvents = $false
    $excel.AutomationSecurity = 3
    foreach ($extension in @('xls', 'xlsx')) {
        foreach ($kind in @('normal', 'recommended', 'write-reserved', 'encrypted')) {
            $workbook = $excel.Workbooks.Add()
            $sheet = $workbook.Worksheets.Item(1)
            $cell = $sheet.Range('A1'); $cell.Value2 = 21.0
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($cell)
            $cell = $sheet.Range('B1'); $cell.Formula = '=A1*2'
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($cell)
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($sheet)
            $workbook.CheckCompatibility = $false
            $format = if ($extension -eq 'xls') { 56 } else { 51 }
            $password = if ($kind -eq 'encrypted') { 'open-test' } else { '' }
            $writePassword = if ($kind -eq 'write-reserved') { 'write-test' } else { '' }
            $recommended = $kind -eq 'recommended'
            $workbook.SaveAs((Join-Path $OutputDirectory "$kind.$extension"), $format,
                $password, $writePassword, $recommended, $false)
            $workbook.Close($false)
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($workbook)
            $workbook = $null
        }
    }
} finally {
    if ($ownsExcel) {
        if ($null -ne $workbook) {
            try { $workbook.Close($false) } catch {}
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($workbook)
        }
        try { $excel.Quit() } catch {}
    }
    if ($null -ne $excel) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel) }
    $owned.Dispose()
}
'@ | Set-Content -LiteralPath $fixtureScript -Encoding UTF8

function Invoke-ExcelTest([string]$name, [string]$script, [hashtable]$parameters, [bool]$succeeds) {
    $directory = Join-Path $root $name
    New-Item -ItemType Directory -Path $directory | Out-Null
    $registry = Join-Path $directory 'owners.jsonl'
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $script)
    foreach ($key in $parameters.Keys) {
        $arguments += @(('-' + $key), ([string]$parameters[$key]).Replace('{case}', $directory))
    }
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = Join-Path $PSHOME 'powershell.exe'
    $info.Arguments = ($arguments | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.EnvironmentVariables['EXCEL_SHRINK_ARCHIVE_OWNERS'] = $registry
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $info
    $owned = New-Object CaptureProcesses
    $stdout = $null
    $stderr = $null
    $timer = [Diagnostics.Stopwatch]::StartNew()
    try {
        [void]$process.Start()
        $owned.Add($process)
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(45000)) { throw "$name exceeded 45 seconds; possible blocked dialog." }
        if (($process.ExitCode -eq 0) -ne $succeeds) {
            throw "$name returned exit code $($process.ExitCode); expected success=$succeeds. See $directory."
        }
        Write-Host ('PASS: {0} ({1:N1}s)' -f $name, $timer.Elapsed.TotalSeconds)
        return $directory
    } finally {
        # Closing this job kills only its child; that closes the child's own Excel job.
        $owned.Dispose()
        if (Test-Path -LiteralPath $registry) {
            & (Join-Path $code 'archive_processes.ps1') -RegistryPath $registry -Stop
        }
        if ($null -ne $stdout) { [IO.File]::WriteAllText((Join-Path $directory 'stdout.log'), $stdout.Result) }
        if ($null -ne $stderr) { [IO.File]::WriteAllText((Join-Path $directory 'stderr.log'), $stderr.Result) }
        $process.Dispose()
    }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
function Assert-Workbook([string]$path) {
    $archive = [IO.Compression.ZipFile]::OpenRead($path)
    try {
        $reader = New-Object IO.StreamReader($archive.GetEntry('xl/worksheets/sheet1.xml').Open())
        try { [xml]$sheet = $reader.ReadToEnd() } finally { $reader.Dispose() }
        $ns = New-Object Xml.XmlNamespaceManager($sheet.NameTable)
        $ns.AddNamespace('s', 'http://schemas.openxmlformats.org/spreadsheetml/2006/main')
        if ($sheet.SelectSingleNode('//s:c[@r="A1"]/s:v', $ns).InnerText -ne '21' -or
            $sheet.SelectSingleNode('//s:c[@r="B1"]/s:f', $ns).InnerText -ne 'A1*2') {
            throw "Workbook cells were not preserved: $path"
        }
    } finally { $archive.Dispose() }
}

Write-Host "Test artifacts: $root"
$fixtures = Invoke-ExcelTest 'fixtures' $fixtureScript @{ OutputDirectory = '{case}' } $true
foreach ($operation in @('open', 'recalculate')) {
    $extension = if ($operation -eq 'open') { 'xls' } else { 'xlsx' }
    $script = Join-Path $code "excel_$operation.ps1"
    foreach ($kind in @('normal', 'recommended', 'write-reserved', 'encrypted')) {
        $source = Join-Path $fixtures "$kind.$extension"
        $before = (Get-FileHash -LiteralPath $source).Hash
        $parameters = @{ InputPath = $source; OutputPath = '{case}/output.xlsx'; ProgressPath = '{case}/progress.log' }
        if ($operation -eq 'open') { $parameters.MetricsPath = '{case}/metrics.json' }
        $encrypted = $kind -eq 'encrypted'
        $directory = Invoke-ExcelTest "$operation-$kind" $script $parameters (-not $encrypted)
        if ((Get-FileHash -LiteralPath $source).Hash -ne $before) { throw "Source fixture changed: $source" }
        $progress = Get-Content -LiteralPath (Join-Path $directory 'progress.log') -Raw
        if ($progress -notmatch 'opening workbook') { throw "$operation-$kind failed before reaching Excel.Open." }
        $output = Join-Path $directory 'output.xlsx'
        if ($encrypted) {
            if (Test-Path -LiteralPath $output) { throw "$operation-$kind unexpectedly produced an output workbook." }
        } else {
            Assert-Workbook $output
            if ($operation -eq 'open') {
                $metrics = Get-Content -LiteralPath (Join-Path $directory 'metrics.json') -Raw | ConvertFrom-Json
                if ($null -eq $metrics.open_seconds -or $metrics.open_seconds -lt 0) { throw 'Missing open timing.' }
            }
        }
    }
}
Write-Host 'PASS: conversion and recalculation open readable workbooks without prompts and reject encrypted workbooks promptly.'
