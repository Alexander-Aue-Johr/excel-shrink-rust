$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
Add-Type -Path (Join-Path $captureRoot 'ResidentProcesses.cs')
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $captureRoot 'resident.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident launcher has syntax errors.' }
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    if ($function.Name -in @('Read-OtherGroupPauseMillis', 'Update-OtherGroupPause', 'Update-SystemReserve', 'Wait-BatchRelease')) {
        Invoke-Expression $function.Extent.Text
    }
}
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
function Add-SessionEvent { }
function Write-ResourceMetrics { $metricsClock.Restart() }
function Start-Sleep {
    param([int]$Milliseconds)
    [Threading.Thread]::Sleep(20)
    if ($script:scenario -eq 'advancing') {
        $counter = 3000 + [long]$scenarioClock.Elapsed.TotalMilliseconds
        [IO.File]::WriteAllText($batchLifetime.pause_path, ('{"elapsed_millis":' + $counter + '}'))
        if ($scenarioClock.Elapsed.TotalMilliseconds -ge 700) {
            [IO.File]::WriteAllText($batchLifetime.release_path, 'released')
        }
    } elseif ($script:scenario -eq 'frozen') {
        [IO.File]::WriteAllText($batchLifetime.pause_path, '{"elapsed_millis":18446744073709551615}')
    } elseif ($script:scenario -eq 'live' -and $scenarioClock.Elapsed.TotalMilliseconds -ge 700) {
        [IO.File]::WriteAllText($batchLifetime.release_path, 'released')
    }
}
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)
$root = Join-Path ([IO.Path]::GetTempPath()) ('es-resident-cleanup-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$owner = [Diagnostics.Process]::GetCurrentProcess()
try {
    foreach ($script:scenario in @('advancing', 'frozen', 'legacy', 'dead-parent', 'live')) {
        $directory = Join-Path $root $scenario
        [void][IO.Directory]::CreateDirectory($directory)
        $batchLifetime = @{
            ready_path=(Join-Path $directory 'ready.json')
            release_path=(Join-Path $directory 'release')
            pause_path=(Join-Path $directory 'pause.json')
            timeout_seconds=0.3
        }
        $batchOwner = $owner
        if ($scenario -eq 'dead-parent') { $batchOwner = @{HasExited=$true} }
        $liveControlPath = $null
        if ($scenario -eq 'live') { $liveControlPath = Join-Path $directory 'control.json' }
        $script:batchReleased = $false
        $script:batchReleaseReason = $null
        $script:pauseCounter = $null
        $script:pauseBaselineSeconds = 0.0
        $script:otherOpeningSeconds = 0.0
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $metricsClock = [Diagnostics.Stopwatch]::StartNew()
        $metrics = @{status='running'; minimum_available_bytes=[UInt64]::MaxValue; minimum_commit_bytes=[UInt64]::MaxValue}
        $systemReserveBytes = [UInt64]0
        [IO.File]::WriteAllText($batchLifetime.pause_path, '{"elapsed_millis":0}')
        if ($scenario -eq 'advancing' -or $scenario -eq 'legacy') {
            $marker = '{"opening_pause_millis":1000}'
            if ($scenario -eq 'legacy') { $marker = 'ready' }
            [IO.File]::WriteAllText($batchLifetime.ready_path, $marker)
            [IO.File]::SetLastWriteTimeUtc($batchLifetime.ready_path, [DateTime]::UtcNow.AddSeconds(-2))
            [IO.File]::WriteAllText($batchLifetime.pause_path, '{"elapsed_millis":3000}')
        }
        $scenarioClock = [Diagnostics.Stopwatch]::StartNew()
        $reason = Wait-BatchRelease
        if ($scenario -eq 'advancing') {
            Assert ($reason -eq 'released') 'A ready group timed out while its already admitted peer was opening.'
            Assert ($scenarioClock.Elapsed.TotalMilliseconds -ge 700) 'The test did not exceed the uncredited wall deadline.'
            Assert ($metrics.batch_wait_opening_pause_seconds -ge 2.5) 'A late observer lost the original ready marker pause baseline.'
            Assert (([IO.File]::ReadAllText($batchLifetime.ready_path)) -eq $marker) 'The first ready marker was replaced.'
        } elseif ($scenario -eq 'dead-parent') {
            Assert ($reason -eq 'coordinator_exited') 'Opening pause hid a dead coordinator.'
        } elseif ($scenario -eq 'live') {
            Assert ($reason -eq 'released') 'Live cleanup expired while the coordinator still owned the comparison batch.'
            Assert ($scenarioClock.Elapsed.TotalMilliseconds -ge 700) 'Live cleanup did not outlast the fixed legacy deadline.'
        } else {
            Assert ($reason -eq 'timeout') 'A frozen counter or legacy marker disabled the cleanup deadline.'
            Assert ($scenarioClock.Elapsed.TotalSeconds -lt 2.0) 'Discarded excess pause credit extended a later deadline.'
        }
    }
    Write-Host 'PASS: ready-wait startup credit, late observer baseline, frozen-counter expiry, legacy markers, coordinator death, live batch lifetime.'
} finally {
    $owner.Dispose()
    $resolved = [IO.Path]::GetFullPath($root)
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notlike 'es-resident-cleanup-*') {
        throw 'Unexpected cleanup-test directory.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
