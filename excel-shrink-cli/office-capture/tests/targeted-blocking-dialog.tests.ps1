param([switch]$DialogWorker, [string]$Text)
$ErrorActionPreference='Stop'
if ($DialogWorker) {
    Add-Type -AssemblyName System.Windows.Forms
    $owner=New-Object Windows.Forms.Form
    $owner.Show()
    [void][Windows.Forms.MessageBox]::Show($owner,$Text,'Microsoft Excel test',[Windows.Forms.MessageBoxButtons]::YesNo)
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
        @{text='We found a problem with some content in one.xlsx.';kind='repair_required'},
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
