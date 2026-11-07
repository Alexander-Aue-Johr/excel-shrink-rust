param([string]$Session, [string]$BaseUrl)
$ErrorActionPreference = 'Stop'
$source = Join-Path $PSScriptRoot '../resident.ps1'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident script does not parse.' }
$names = @('Write-SessionJson', 'Write-ResidentJsonFile', 'Add-SessionEvent', 'Publish-ResidentDiagnostics', 'Write-PendingDiagnosticEvents')
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    if ($function.Name -in $names) { Invoke-Expression $function.Extent.Text }
}
$session = $Session
$controlUrl = $BaseUrl
$launcherToken = 'fixture-token'
$utf8 = New-Object Text.UTF8Encoding($false)
$clock = [Diagnostics.Stopwatch]::StartNew()
$ready = $true
$liveGroupDirectory = $null
$metrics = @{status='running'; sequence=101}
$script:sqliteDiagnosticsEnabled = $true
$script:diagnosticHostReady = $true
$script:diagnosticEvents = New-Object 'Collections.Generic.List[object]'
1..3 | ForEach-Object { Add-SessionEvent 'client-fixture' 'one' @{sequence=$_} }
if ($script:diagnosticEvents.Count -ne 3) { throw 'Events were not buffered.' }
$controlUrl = $BaseUrl + '/missing'
$failed = $false
try { Publish-ResidentDiagnostics $metrics } catch { $failed = $true }
if (-not $failed -or $script:diagnosticEvents.Count -ne 3) {
    throw 'Unacknowledged events must remain queued.'
}
$controlUrl = $BaseUrl
Write-SessionJson 'metrics.json' $metrics
if ($script:diagnosticEvents.Count -ne 0) { throw 'Acknowledged events were not released.' }
Publish-ResidentDiagnostics $metrics -Export
'PowerShell diagnostic batching and retry retention passed.'
