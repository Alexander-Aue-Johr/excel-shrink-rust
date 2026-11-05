# Isolate UI Automation providers so a blocked accessibility call cannot stall the host.
$ErrorActionPreference = 'Stop'

function Read-TargetedEmbeddedDialog {
    <# .SYNOPSIS Read bounded accessible labels from a modal browser owned by this Excel session. #>
    param($Entry, $Dialog)
    if (-not $Dialog.IsModal -or $Dialog.ClassName -ne 'BasicEmbeddedBrowser') { return @() }
    if (-not $Entry.ContainsKey('dialog_probe_clock')) {
        $Entry.dialog_probe_clock = [Diagnostics.Stopwatch]::StartNew()
        $Entry.dialog_probe_handle = 0L
    }
    if ($Entry.dialog_probe_handle -eq $Dialog.Handle -and $Entry.dialog_probe_clock.Elapsed.TotalSeconds -lt 5) { return @() }
    $Entry.dialog_probe_handle = $Dialog.Handle
    $Entry.dialog_probe_clock.Restart()
    $output = Join-Path $Entry.session 'embedded-dialog-text.json'
    if ([IO.File]::Exists($output)) { [IO.File]::Delete($output) }
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -ExcelProcessId {1} -Handle {2} -Output "{3}"' -f `
        (Join-Path $PSScriptRoot 'read-targeted-dialog.ps1'), $Entry.process.Id, $Dialog.Handle, $output
    $scope = New-Object CaptureProcesses
    $probe = $null
    try {
        $shell = Join-Path ([Environment]::SystemDirectory) 'WindowsPowerShell/v1.0/powershell.exe'
        $probe = [CaptureDesktop]::StartOnDesktop($env:EXCEL_SHRINK_EXPECTED_DESKTOP, $shell, $arguments, $scope)
        if (-not $probe.WaitForExit(4000)) {
            $probe.Kill()
            return @()
        }
        if ($probe.ExitCode -ne 0 -or -not [IO.File]::Exists($output)) { return @() }
        $result = [IO.File]::ReadAllText($output) | ConvertFrom-Json
        if ($result.pid -ne $Entry.process.Id -or $result.handle -ne $Dialog.Handle) { throw 'Unexpected embedded dialog observation.' }
        return @($result.names)
    } finally {
        $scope.Dispose()
        if ($null -ne $probe) { $probe.Dispose() }
    }
}

Export-ModuleMember -Function Read-TargetedEmbeddedDialog
