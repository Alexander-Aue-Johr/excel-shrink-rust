# Observe opening dialogs independently of a blocked COM Workbooks.Open call.
param(
    [Parameter(Mandatory=$true)][string]$Session,
    [Parameter(Mandatory=$true)][int]$HelperPid
)
$ErrorActionPreference = 'Stop'
try {
    Add-Type -Path (Join-Path $PSScriptRoot 'TargetedOpeningDialogs.cs')
    Import-Module (Join-Path $PSScriptRoot 'OpeningDialogKinds.psm1') -Force
    $identity = [IO.File]::ReadAllText((Join-Path $Session 'identity.json')) | ConvertFrom-Json
    $helper = [Diagnostics.Process]::GetProcessById($HelperPid)
    while (-not $helper.HasExited) {
        $activePath = Join-Path $Session 'active.json'
        if ([IO.File]::Exists($activePath)) {
            try { $active = [IO.File]::ReadAllText($activePath) | ConvertFrom-Json }
            catch [IO.FileNotFoundException] { continue }
            $progressPath = Join-Path $active.directory 'progress.json'
            if ([IO.File]::Exists($progressPath)) {
                $progress = [IO.File]::ReadAllText($progressPath) | ConvertFrom-Json
                if ($progress.phase -eq 'opening workbook') {
                    $excel = [Diagnostics.Process]::GetProcessById($identity.pid)
                    try { $sameProcess = ([DateTimeOffset]$excel.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds() -eq $identity.started_ms }
                    finally { $excel.Dispose() }
                    if (-not $sameProcess) { throw 'Opening monitor Excel identity changed.' }
                    foreach ($dialog in [TargetedOpeningDialogs]::Inspect($identity.pid, $identity.desktop)) {
                        if ($dialog.HasProgressBar) { continue }
                        $kind = Get-TargetedOpeningDialogKind $dialog.Names $dialog.HasPasswordInput
                        if (-not $kind) { continue }
                        $receipt = @{
                            id=$active.id; kind=$kind; phase=$progress.phase
                            excel_pid=$identity.pid; started_ms=$identity.started_ms
                            desktop=$identity.desktop; handle=$dialog.Handle
                            names=@($dialog.Names); action='skip_without_accepting_prompt'
                            observed_at=[DateTime]::UtcNow.ToString('o')
                        }
                        $path = Join-Path $active.directory 'blocking-dialog.json'
                        [IO.File]::WriteAllText(($path + '.pending'), ($receipt | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
                        Move-Item -LiteralPath ($path + '.pending') -Destination $path
                        exit 0
                    }
                }
            }
        }
        Start-Sleep -Milliseconds 1000
    }
} catch {
    [IO.File]::WriteAllText((Join-Path $Session 'dialog-monitor-error.json'), (@{error=$_.Exception.Message} | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    exit 1
} finally {
    if ($null -ne $helper) { $helper.Dispose() }
}
