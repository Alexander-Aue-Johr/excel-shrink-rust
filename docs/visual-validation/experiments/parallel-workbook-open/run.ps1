param(
    [Parameter(Mandatory=$true)][string]$Output,
    [ValidateRange(10,120)][int]$TargetSeconds = 45,
    [ValidateRange(1,4)][int]$Repetitions = 2,
    [string]$Fixture,
    [ValidateSet(2,4)][int]$Workbooks = 2
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$capture = Join-Path $repository 'excel-shrink-cli/office-capture'
$identity = Join-Path $PSScriptRoot '../check-performance-trigger/DesktopIdentity.cs'
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
Add-Type -Path $identity
$outputPath = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $outputPath) { throw 'Use a new experiment output directory.' }
$python = & python -c 'import sys; print(sys.executable)'
$first = New-Object CaptureDesktop
$second = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$process = $null
$before = [DesktopIdentity]::InputName()
try {
    $arguments = '"{0}" --output "{1}" --desktop-one {2} --desktop-two {3} --target-seconds {4} --repetitions {5}' -f `
        (Join-Path $PSScriptRoot 'run.py'), $outputPath, $first.Name, $second.Name, $TargetSeconds, $Repetitions
    if ($Fixture) { $arguments += ' --fixture "{0}"' -f [IO.Path]::GetFullPath($Fixture) }
    $arguments += ' --workbooks {0}' -f $Workbooks
    $process = $first.Start($python, $arguments, $owned)
    $deadline = [DateTime]::UtcNow.AddMinutes(30)
    $previous = ''
    while (-not $process.WaitForExit(500)) {
        $progressPath = Join-Path $outputPath 'progress.json'
        if (Test-Path -LiteralPath $progressPath) {
            $current = [IO.File]::ReadAllText($progressPath)
            if ($current -ne $previous) { Write-Host $current; $previous = $current }
        }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Experiment exceeded its 30-minute deadline.' }
    }
    if ($process.ExitCode -ne 0) { throw "Experiment failed; see $outputPath/progress.json and error.log." }
} finally {
    try {
        $after = [DesktopIdentity]::InputName()
        $evidence = @{first_desktop=$first.Name; second_desktop=$second.Name;
            input_desktop_before=$before; input_desktop_after=$after; input_desktop_unchanged=($before -eq $after)}
        [IO.File]::WriteAllText(($outputPath + '.desktops.json'),
            ($evidence | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    } finally {
        try { $owned.Dispose() }
        finally {
            if ($null -ne $process) { $process.Dispose() }
            $first.Dispose()
            $second.Dispose()
        }
    }
}
Write-Host "Complete: $outputPath/results.json"
