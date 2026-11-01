param(
    [Parameter(Mandatory=$true)][string]$Output,
    [Parameter(Mandatory=$true)][string]$Plan
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$capture = Join-Path $repository 'excel-shrink-cli/office-capture'
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$process = $null
$outputPath = [IO.Path]::GetFullPath($Output)
try {
    $python = & python -c 'import sys; print(sys.executable)'
    $arguments = '"{0}" --output "{1}" --plan "{2}" --desktop {3}' -f `
        (Join-Path $PSScriptRoot 'run_probe.py'), $outputPath, ([IO.Path]::GetFullPath($Plan)), $desktop.Name
    $process = $desktop.Start($python, $arguments, $owned)
    $deadline = [DateTime]::UtcNow.AddMinutes(20)
    $previous = ''
    while (-not $process.WaitForExit(500)) {
        $progress = Join-Path $outputPath 'progress.json'
        if (Test-Path -LiteralPath $progress) {
            $current = [IO.File]::ReadAllText($progress)
            if ($current -ne $previous) { Write-Host $current; $previous = $current }
        }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Font-tail experiment exceeded 20 minutes.' }
    }
    if ($process.ExitCode -ne 0) { throw "Probe failed; inspect $outputPath." }
} finally {
    try { $owned.Dispose() }
    finally {
        if ($null -ne $process) { $process.Dispose() }
        $desktop.Dispose()
    }
}
