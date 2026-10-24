$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $captureRoot 'resident.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident launcher has syntax errors.' }
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    if ($function.Name -in @('Get-ResidentControlRoot', 'Get-ResidentOpenTimeoutSeconds', 'Test-ResidentDeadline', 'Update-Resources')) {
        Invoke-Expression $function.Extent.Text
    }
}
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
function Update-OtherGroupPause { }
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)
$root = Join-Path ([IO.Path]::GetTempPath()) ('es-resident-control-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
try {
    Assert ((Get-ResidentControlRoot 'V:\study\control-room') -eq 'V:\study\control-room') 'Ordinary Windows root changed.'
    Assert ((Get-ResidentControlRoot '\\?\V:\study\control-room') -eq 'V:\study\control-room') 'Extended drive path is not usable by PowerShell providers.'
    Assert ((Get-ResidentControlRoot '\\?\UNC\tower\share\control-room') -eq '\\tower\share\control-room') 'Extended UNC path was not normalized correctly.'
    $relativeRejected = $false
    try { Get-ResidentControlRoot 'relative\study' } catch { $relativeRejected = $true }
    Assert $relativeRejected 'Relative control room paths were accepted.'
    $StartupTimeoutSeconds = 300
    $CaptureTimeoutSeconds = 1800
    $liveControlPath = $null
    $script:liveOpenTimeoutSeconds = $StartupTimeoutSeconds
    Assert ((Get-ResidentOpenTimeoutSeconds) -eq 300) 'Legacy startup timeout changed.'
    Assert (Test-ResidentDeadline 'opening' 301) 'Legacy opening deadline stopped expiring.'
    Assert (-not (Test-ResidentDeadline 'opening' 299)) 'Legacy opening expired early.'
    Assert (Test-ResidentDeadline 'opening_queue' 1321) 'Legacy opening queue lost its deadline.'
    Assert (Test-ResidentDeadline 'session' 1801) 'Legacy session deadline stopped expiring.'

    $liveControlPath = Join-Path $root 'control.json'
    [IO.File]::WriteAllText($liveControlPath, '{"open_timeout_seconds":900,"paused":true}')
    Assert ((Get-ResidentOpenTimeoutSeconds) -eq 900) 'Live startup timeout did not replace the CLI default.'
    Assert (-not (Test-ResidentDeadline 'opening' 850)) 'Live opening expired against the obsolete deadline.'
    Assert (Test-ResidentDeadline 'opening' 901) 'Pausing captures also disabled the active opening deadline.'
    Assert (-not (Test-ResidentDeadline 'opening_queue' 1000000)) 'Time waiting in the Excel opening queue caused a live failure.'
    Assert (-not (Test-ResidentDeadline 'session' 1000000)) 'A fixed session deadline would kill a user-paused live capture.'

    # Changing the dashboard timeout affects the already-running opening clock.
    [IO.File]::WriteAllText($liveControlPath, '{"open_timeout_seconds":3600}')
    Assert (-not (Test-ResidentDeadline 'opening' 1100)) 'An in-flight opening ignored an extended timeout.'
    [IO.File]::WriteAllText($liveControlPath, '{"open_timeout_seconds":100}')
    Assert (Test-ResidentDeadline 'opening' 1100) 'An in-flight opening ignored a shortened timeout.'
    [IO.File]::WriteAllText($liveControlPath, '{"open_timeout_seconds":9000}')
    Assert ((Get-ResidentOpenTimeoutSeconds) -eq 9000) 'The live timeout retained an old CLI upper limit.'
    foreach ($invalid in @('{', '{}', '{"open_timeout_seconds":-1}', '{"open_timeout_seconds":1.5}', '{"open_timeout_seconds":"10"}', '{"open_timeout_seconds":true}')) {
        [IO.File]::WriteAllText($liveControlPath, $invalid)
        Assert ((Get-ResidentOpenTimeoutSeconds) -eq 9000) 'Invalid control data shortened the last valid opening deadline.'
    }
    Remove-Item -LiteralPath $liveControlPath
    Assert ((Get-ResidentOpenTimeoutSeconds) -eq 9000) 'A transiently missing snapshot shortened the opening deadline.'

    $liveOwner = @{HasExited=$true}
    $caught = $null
    try { Update-Resources } catch { $caught = $_.Exception.Message }
    Assert ($caught -eq 'Control room coordinator exited.') 'Live waiting masked coordinator death.'
    Write-Host 'PASS: legacy deadlines, live opening overrides, queued opens, pause-safe session lifetime, retained valid controls, coordinator death.'
} finally {
    $resolved = [IO.Path]::GetFullPath($root)
    $temporary = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($temporary, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notlike 'es-resident-control-*') {
        throw 'Unexpected control-test directory.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
