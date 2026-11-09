param([switch]$DialogWorker)
$ErrorActionPreference = 'Stop'
if ($DialogWorker) {
    Add-Type -AssemblyName System.Windows.Forms
    $owner = New-Object Windows.Forms.Form
    $owner.Show()
    [void][Windows.Forms.MessageBox]::Show($owner, 'Choose a name.', 'Name Conflict', [Windows.Forms.MessageBoxButtons]::YesNo)
    return
}

$capture = Split-Path -Parent $PSScriptRoot
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
$root = Join-Path ([IO.Path]::GetTempPath()) ('conversion-dialog-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$owned = New-Object CaptureProcesses
$desktop = New-Object CaptureDesktop
try {
    foreach ($name in @('TargetedOpeningDialogs.cs', 'OpeningDialogKinds.psm1')) {
        Copy-Item -LiteralPath (Join-Path $capture $name) -Destination $root
    }
    Copy-Item -LiteralPath (Join-Path $capture '../../crates/excel-automation/src/pool/watch-opening.ps1') -Destination $root
    $shell = Join-Path $PSHOME 'powershell.exe'
    $arguments = '-STA -NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -DialogWorker'
    $worker = $desktop.Start($shell, $arguments, $owned)
    $identity = @{pid=$worker.Id; started_ms=([DateTimeOffset]$worker.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds(); desktop=$desktop.Name}
    $identity | ConvertTo-Json | Set-Content (Join-Path $root 'identity.json')
    $request = @{id='fixture-request'; directory=$root}
    $request | ConvertTo-Json | Set-Content (Join-Path $root 'active.json')
    @{phase='optimizing workbook performance'} | ConvertTo-Json | Set-Content (Join-Path $root 'progress.json')
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $root 'watch-opening.ps1') + '" -Session "' + $root + '" -HelperPid ' + $PID
    $monitor = $desktop.Start($shell, $arguments, $owned)
    Start-Sleep -Seconds 3
    if ($monitor.HasExited) { throw 'Monitor exited before an opening request.' }
    if (Test-Path (Join-Path $root 'blocking-dialog.json')) { throw 'Non-opening operation was classified.' }
    @{phase='opening workbook'} | ConvertTo-Json | Set-Content (Join-Path $root 'progress.json')
    if (-not $monitor.WaitForExit(15000) -or $monitor.ExitCode -ne 0) { throw 'Opening monitor failed or exceeded its deadline.' }
    $receipt = Get-Content (Join-Path $root 'blocking-dialog.json') -Raw | ConvertFrom-Json
    if ($receipt.id -ne $request.id -or $receipt.kind -ne 'name_conflict' -or $receipt.excel_pid -ne $worker.Id) {
        throw 'Opening evidence did not match the exact request and process.'
    }
    if ($receipt.action -ne 'skip_without_accepting_prompt' -or $worker.HasExited) {
        throw 'Read-only monitoring changed the dialog process.'
    }
    'Conversion opening monitor tests passed.'
} finally {
    $owned.Dispose()
    $desktop.Dispose()
    $resolved = [IO.Path]::GetFullPath($root)
    if (-not $resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolved) -notlike 'conversion-dialog-test-*') { throw 'Unsafe test cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
