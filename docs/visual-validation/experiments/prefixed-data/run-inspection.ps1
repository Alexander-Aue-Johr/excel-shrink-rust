param(
    [Parameter(Mandatory=$true)]
    [ValidatePattern('^\d{5}$')]
    [string]$Case,
    [string]$Inventory = '.tmp/prefix-inventory-datagov-20261102',
    [string]$Trials = '.tmp/prefix-shrink-trials-20261102',
    [string]$Output,
    [string]$Binary = 'target/debug/excel-shrink-cli.exe'
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
if (-not $Output) { $Output = ".tmp/prefix-visual-$Case" }
$capture = Join-Path $repository 'excel-shrink-cli/office-capture'
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$previous = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
try {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $desktop.Name
    $source = Join-Path (Join-Path $repository $Inventory) "sources/$Case.xlsx"
    $trial = Join-Path (Join-Path $repository $Trials) $Case
    $output = Join-Path $repository $Output
    $arguments = 'regression prefix-inspect --source "{0}" --normalized "{1}" --shrunk "{2}" --output "{3}"' -f $source, (Join-Path $trial "normalized/$Case.xlsx"), (Join-Path $trial "shrunk/$Case.xlsx"), $output
    $process = $desktop.Start((Join-Path $repository $Binary), $arguments, $owned)
    $deadline = [DateTime]::UtcNow.AddMinutes(15)
    while (-not $process.WaitForExit(500)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Inspection exceeded 15 minutes.' }
    }
    if ($process.ExitCode -ne 0) { throw "Inspection failed: $($process.ExitCode). Inspect $output" }
} finally {
    $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $previous
    $owned.Dispose()
    $desktop.Dispose()
}
