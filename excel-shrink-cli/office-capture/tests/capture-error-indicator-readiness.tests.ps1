$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$tokens = $null
$errors = $null
$path = Join-Path $PSScriptRoot '../resident.ps1'
$ast = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident launcher syntax errors.' }
$definition = $ast.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Update-ClientReadySamples'
}, $false)
. ([scriptblock]::Create($definition.Extent.Text))
Add-Type @"
using System.Diagnostics;
public static class CaptureErrorChecking {
    public static object Next;
    public static object TryDisable(Process process, string desktop) { return Next; }
}
"@
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
function Add-SessionEvent { }
$script:acks = 0
$script:persisted = $false
function Write-ResourceMetrics { $script:persisted = $metrics.clients.original.error_indicators_disabled }
function Invoke-RestMethod {
    Assert $script:persisted 'The setting must be verified and persisted before the host releases capture jobs.'
    $script:acks++
}
$controlUrl = 'http://127.0.0.1:1'
$launcherToken = 'fixture'
$clock = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Milliseconds 10
$entry = @{ process=[Diagnostics.Process]::GetCurrentProcess(); open_clock=$clock; seen=$false; desktop_name='test-owned'; error_checking_scope=$null }
$clients = @{ original=$entry }
$metrics = @{ clients=@{ original=@{} } }
$health = [pscustomobject]@{clients=[pscustomobject]@{original=[pscustomobject]@{ready_at=[DateTimeOffset]::UtcNow.ToString('o');startup_sampled=$false}}}

[CaptureErrorChecking]::Next = $null
Update-ClientReadySamples $health
Assert ($script:acks -eq 0 -and -not $entry.seen) 'An unavailable native workbook window must not admit any image job.'

[CaptureErrorChecking]::Next = [pscustomobject]@{IsDisabled=$false;PreviousBackgroundChecking=$true}
$failed = $false
try { Update-ClientReadySamples $health } catch { $failed = $_.Exception.Message -like '*still enabled*' }
Assert ($failed -and $script:acks -eq 0 -and -not $entry.seen) 'An unverified setting must fail before capture admission.'

$entry.error_checking_scope = $null
[CaptureErrorChecking]::Next = [pscustomobject]@{IsDisabled=$true;PreviousBackgroundChecking=$true}
Update-ClientReadySamples $health
Assert ($script:acks -eq 1 -and $entry.seen -and $metrics.clients.original.error_indicators_disabled) 'A verified owned instance must be admitted.'
Assert $metrics.clients.original.previous_background_error_checking 'The previous instance setting must be retained for restoration.'
$health.clients.original.startup_sampled = $true
Update-ClientReadySamples $health
Assert ($script:acks -eq 1) 'An already sampled document must not be acknowledged twice.'
'PASS: capture remains blocked until owned Excel error suppression is verified and persisted.'
