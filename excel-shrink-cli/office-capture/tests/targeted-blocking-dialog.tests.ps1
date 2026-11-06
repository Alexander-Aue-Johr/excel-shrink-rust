param([switch]$DialogWorker, [string]$Text, [switch]$BrowserProbe, [int]$ExcelProcessId, [long]$DialogHandle, [string]$Session)
$ErrorActionPreference='Stop'
if ($DialogWorker) {
    Add-Type -AssemblyName System.Windows.Forms
    $owner=New-Object Windows.Forms.Form
    $owner.Show()
    [void][Windows.Forms.MessageBox]::Show($owner,$Text,'Microsoft Excel test',[Windows.Forms.MessageBoxButtons]::YesNo)
    return
}
if ($BrowserProbe) {
    Add-Type -Path @((Join-Path $PSScriptRoot '../CaptureProcesses.cs'), (Join-Path $PSScriptRoot '../CaptureDesktop.cs'))
    Import-Module (Join-Path $PSScriptRoot '../TargetedDialogText.psm1') -Force
    foreach ($className in @('BasicEmbeddedBrowser', 'NUIDialog')) {
        $entry = @{process=Get-Process -Id $ExcelProcessId; session=$Session}
        $dialog = [pscustomobject]@{Handle=$DialogHandle; IsModal=$true; ClassName=$className}
        $labels = @(Read-TargetedEmbeddedDialog $entry $dialog)
        if (($labels -join ' ') -notmatch 'AADSTS50020') { throw 'Private-desktop dispatch missed dialog labels.' }
        if (@(Read-TargetedEmbeddedDialog $entry $dialog).Count -ne 0) { throw 'Repeated dialog probes must be throttled.' }
    }
    [IO.File]::WriteAllText((Join-Path $Session 'browser-probe-passed.txt'), 'passed')
    return
}
$capture=Split-Path -Parent $PSScriptRoot
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'),(Join-Path $capture 'CaptureDesktop.cs'))
Add-Type -Path (Join-Path $capture 'TargetedOpeningDialogs.cs')
Import-Module (Join-Path $capture 'TargetedResident.psm1') -Force -DisableNameChecking
$originalDesktop=$env:EXCEL_SHRINK_EXPECTED_DESKTOP
$root=Join-Path ([IO.Path]::GetTempPath()) ('targeted-dialog-test-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
try {
    foreach($case in @(
        @{text='Password required to open this file';kind='password_required'},
        @{text='AADSTS50020: User account does not exist in tenant EPA';kind='tenant_access_denied'},
        @{text='Sign in';kind='authentication_required'},
        @{text='We found a problem with some content in one.xlsx.';kind='repair_required'},
        @{text='Microsoft Excel - Compatibility Checker';kind='compatibility_confirmation_required'},
        @{text='The author would like you to open this workbook as read-only. Open as read-only?';kind='read_only_prompt'},
        @{text='Choose whether to continue';kind='modal_confirmation_required'}
    )) {
        $owned=New-Object CaptureProcesses
        $desktop=New-Object CaptureDesktop
        try {
            $env:EXCEL_SHRINK_EXPECTED_DESKTOP=$desktop.Name
            $arguments='-STA -NoProfile -ExecutionPolicy Bypass -File "{0}" -DialogWorker -Text "{1}"' -f $PSCommandPath,$case.text
            $worker=$desktop.Start((Join-Path $PSHOME 'powershell.exe'),$arguments,$owned)
            $clock=[Diagnostics.Stopwatch]::StartNew()
            $dialogs=@()
            while($dialogs.Count -eq 0 -and $clock.Elapsed.TotalSeconds -lt 10) {
                $dialogs=@([TargetedOpeningDialogs]::Inspect($worker.Id,$desktop.Name))
                Start-Sleep -Milliseconds 50
            }
            if($dialogs.Count -eq 0){throw 'No owned dialog found on the explicit private desktop.'}
            if([TargetedOpeningDialogs]::Inspect($PID,$desktop.Name).Length -ne 0){throw 'Foreign PID matched an owned dialog.'}
            if([TargetedOpeningDialogs]::Find($worker.Id).Length -eq 0){throw 'Compatibility lookup missed the private desktop.'}
            $session=Join-Path $root $case.kind
            [void][IO.Directory]::CreateDirectory($session)
            $entry=@{process=$worker;launcher=$worker;session=$session}
            if ($case.kind -eq 'tenant_access_denied') {
                $labels = Join-Path $session 'labels.json'
                $arguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -ExcelProcessId {1} -Handle {2} -Output "{3}"' -f `
                    (Join-Path $capture 'read-targeted-dialog.ps1'), $worker.Id, $dialogs[0].Handle, $labels
                $reader = $desktop.Start((Join-Path $PSHOME 'powershell.exe'), $arguments, $owned)
                if (-not $reader.WaitForExit(5000)) { throw 'Dialog label reader exceeded its deadline.' }
                if ($reader.ExitCode -ne 0) { throw 'Dialog label reader failed.' }
                # The production command host and its probe both execute on the workbook's private desktop.
                $arguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -BrowserProbe -ExcelProcessId {1} -DialogHandle {2} -Session "{3}"' -f `
                    $PSCommandPath, $worker.Id, $dialogs[0].Handle, $session
                $probeProcess = $desktop.Start((Join-Path $PSHOME 'powershell.exe'), $arguments, $owned)
                if (-not $probeProcess.WaitForExit(7000) -or $probeProcess.ExitCode -ne 0 -or
                    -not (Test-Path (Join-Path $session 'browser-probe-passed.txt'))) { throw 'Private-desktop browser dispatch failed.' }
                $observed = Get-Content $labels -Raw | ConvertFrom-Json
                if ($observed.pid -ne $worker.Id -or ($observed.names -join ' ') -notmatch 'AADSTS50020') {
                    throw 'Accessible labels did not preserve the owned dialog error.'
                }
            }
            $caught=$null
            try { & (Get-Module TargetedResident) {param($value) Test-TargetedWorkbookBlocker $value 'opening'} $entry }
            catch {$caught=$_.Exception.Message}
            if(-not $caught -or -not $caught.Contains('['+$case.kind+']')){throw "Wrong early failure: $caught"}
            $evidence=Get-Content (Join-Path $session 'blocking-dialog.json') -Raw | ConvertFrom-Json
            if($evidence.kind -ne $case.kind -or $evidence.action -ne 'skip_without_accepting_prompt'){throw 'Missing evidence of unaccepted prompt.'}
            if(-not $worker.WaitForExit(3000)){throw 'Blocked owned process survived the early skip.'}
        } finally {$owned.Dispose();$desktop.Dispose()}
    }
} finally {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP=$originalDesktop
    $resolved=[IO.Path]::GetFullPath($root)
    if(-not $resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()),[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notlike 'targeted-dialog-test-*'){throw 'Unsafe test cleanup path'}
    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
}
'Private-desktop blocking-dialog tests passed.'
