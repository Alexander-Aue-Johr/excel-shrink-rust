param(
    [Parameter(Mandatory=$true)][string]$Output,
    [ValidateSet('Accessibility','Messages')][string]$Mode
)
$ErrorActionPreference = 'Stop'
Add-Type -Path (Join-Path $PSScriptRoot 'DesktopIdentity.cs')
$env:EXCEL_SHRINK_EXPERIMENT_DESKTOP = [DesktopIdentity]::CurrentName()
[IO.File]::WriteAllText(($Output + '.child-desktop.txt'), $env:EXCEL_SHRINK_EXPERIMENT_DESKTOP)
$arguments = @((Join-Path $PSScriptRoot 'run.py'), '--output', $Output, '--native', '--optimize')
if ($Mode -eq 'Messages') { $arguments += '--message-optimize' }
$python = & python -c 'import sys; print(sys.executable)'
& $python @arguments > ($Output + '.stdout.log') 2> ($Output + '.stderr.log')
exit $LASTEXITCODE
