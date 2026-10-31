param(
    [Parameter(Mandatory=$true)][string]$Output,
    [ValidateSet('Accessibility','Messages')][string]$Mode = 'Accessibility'
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$capture = Join-Path $repository 'excel-shrink-cli/office-capture'
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
Add-Type -Path (Join-Path $PSScriptRoot 'DesktopIdentity.cs')
$outputPath = [IO.Path]::GetFullPath($Output)
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$process = $null
$before = [DesktopIdentity]::InputName()
try {
    $script = Join-Path $PSScriptRoot 'private-desktop-child.ps1'
    $arguments = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -Output "{1}" -Mode {2}' -f $script, $outputPath, $Mode
    $process = $desktop.Start((Join-Path $PSHOME 'powershell.exe'), $arguments, $owned)
    if (-not $process.WaitForExit(240000)) { throw 'Private desktop experiment exceeded 240 seconds.' }
    if ($process.ExitCode -ne 0) { throw "Private desktop experiment exited with $($process.ExitCode)." }
} finally {
    try {
        $after = [DesktopIdentity]::InputName()
        $evidence = @{private_desktop=$desktop.Name; input_desktop_before=$before;
            input_desktop_after=$after; input_desktop_unchanged=($before -eq $after)}
        [IO.File]::WriteAllText(($outputPath + '.desktop.json'),
            ($evidence | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    } finally {
        try { $owned.Dispose() }
        finally {
            if ($null -ne $process) { $process.Dispose() }
            $desktop.Dispose()
        }
    }
}
Write-Host "Private desktop experiment complete: $outputPath"
