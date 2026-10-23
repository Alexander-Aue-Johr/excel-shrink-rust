param(
    [Parameter(Mandatory=$true)][string]$RequestPath,
    [Parameter(Mandatory=$true)][string]$RuntimeRoot,
    [Parameter(Mandatory=$true)][string]$OutputDir,
    [ValidateRange(10,600)][int]$StartupTimeoutSeconds = 120,
    [ValidateRange(10,7200)][int]$CaptureTimeoutSeconds = 600,
    [ValidateRange(0,65536)][int]$MemoryMiB = 0,
    [ValidateRange(0,131072)][int]$ExpectedPeakMiB = 0,
    [switch]$IgnoreMemoryGuard,
    [switch]$Visible,
    [switch]$Minimized,
    [switch]$HiddenDesktop
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if ([int][bool]$Visible + [int][bool]$Minimized + [int][bool]$HiddenDesktop -gt 1) {
    throw 'Choose only one Excel window mode.'
}
$useMinimized = $Minimized -or (-not $Visible -and -not $HiddenDesktop)
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)
$RuntimeRoot = [IO.Path]::GetFullPath($RuntimeRoot)
$destination = [IO.Path]::GetFullPath($OutputDir)
[void][IO.Directory]::CreateDirectory($destination)
if (Test-Path -LiteralPath (Join-Path $destination 'result.json')) { throw 'Trial already has results.' }
$request = [IO.File]::ReadAllText([IO.Path]::GetFullPath($RequestPath), $utf8) | ConvertFrom-Json
if ($request.batch_size -notin @(1,2,4) -or
    $request.transport -notin @('images','hash') -or $request.max_batch_pixels -lt 32000000 -or
    $request.max_batch_pixels -gt 128000000) {
    throw 'Invalid transport or batch configuration.'
}

# Reuse the pinned runtime helpers without executing its capture launcher.
$tokens = $null; $parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $RuntimeRoot 'run.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Cannot parse pinned launcher helpers.' }
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    $source = $function.Extent.Text.Replace('$PSScriptRoot', ("'" + $PSScriptRoot.Replace("'", "''") + "'"))
    Invoke-Expression $source
}
$stage = Join-Path $env:TEMP ('es-parallel-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($stage)
$owned = $null; $desktop = $null
$registrations = @{}
$clock = [Diagnostics.Stopwatch]::StartNew()
$result = [ordered]@{ status='failed'; workbook=$request.workbook; transport=$request.transport;
    batch_size=[int]$request.batch_size; max_batch_pixels=[int]$request.max_batch_pixels;
    memory_guard_ignored=[bool]$IgnoreMemoryGuard; visible=[bool]$Visible;
    minimized=[bool]$useMinimized; hidden_desktop=[bool]$HiddenDesktop }
$baseUrl = $null
try {
    Add-Type -Path @((Join-Path $RuntimeRoot 'CaptureProcesses.cs'), (Join-Path $RuntimeRoot 'CaptureDesktop.cs'), (Join-Path $PSScriptRoot 'Memory.cs'))
    Add-Type -Path (Join-Path $RuntimeRoot 'CaptureWorkbook.cs') -ReferencedAssemblies @('System.IO.Compression','System.IO.Compression.FileSystem','System.Xml')
    if ($useMinimized) {
        Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;

public static class CaptureWindowMinimizer
{
    [DllImport("user32.dll")]
    private static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")]
    private static extern bool ShowWindowAsync(IntPtr window, int command);

    public static void Minimize(Process excel)
    {
        try
        {
            excel.Refresh();
            if (excel.HasExited) return;
            // Let Excel create its Office.js task pane before minimizing it.
            if ((DateTime.Now - excel.StartTime).TotalSeconds < 3) return;
            IntPtr window = excel.MainWindowHandle;
            if (window != IntPtr.Zero && !IsIconic(window))
                ShowWindowAsync(window, 6); // SW_MINIMIZE
        }
        catch (InvalidOperationException) { } // Excel may exit during cleanup.
    }
}
'@
    }
    $available = [CaptureMemory]::Available
    if (-not $IgnoreMemoryGuard -and $available -lt 2GB) { throw 'Less than 2 GiB physical RAM available before trial.' }
    if ($IgnoreMemoryGuard) { $MemoryMiB = 65536 }
    elseif ($MemoryMiB -eq 0) { $MemoryMiB = [int][Math]::Min(32768, [Math]::Floor($available / 1MB * 0.65)) }
    $env:EXCEL_SHRINK_EXCEL_MEMORY_MIB = [string]$MemoryMiB
    $result.memory_limit_mib = $MemoryMiB
    $result.minimum_available_bytes = $available
    $result.expected_peak_mib = $ExpectedPeakMiB
    if (-not $IgnoreMemoryGuard -and ($ExpectedPeakMiB -gt $MemoryMiB -or ($ExpectedPeakMiB + 1024) * 1MB -gt $available)) {
        throw 'RAM budget guard: predicted peak exceeds the job budget or leaves less than 1 GiB free.'
    }
    $owned = New-Object CaptureProcesses
    $desktop = New-Object CaptureDesktop
    $dotnet = Get-DotNet8
    $env:DOTNET_ROOT = Split-Path -Parent $dotnet
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $dll = Build-Host $dotnet $stage $destination
    $portProbe = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
    $portProbe.Start()
    try { $port = $portProbe.LocalEndpoint.Port } finally { $portProbe.Stop() }
    $baseUrl = 'http://localhost:' + $port
    $control = 'http://127.0.0.1:' + $port
    $web = Join-Path $stage 'web'
    Copy-Item -LiteralPath (Join-Path $RuntimeRoot 'web') -Destination $web -Recurse
    $hostOutput = Join-Path $stage 'output'
    [void][IO.Directory]::CreateDirectory($hostOutput)
    $hostConfig = [ordered]@{
        port=$port; output=$hostOutput; web=$web;
        transport=$request.transport; batch_size=[int]$request.batch_size;
        max_batch_pixels=[int]$request.max_batch_pixels; capture=$request.capture
    }
    $configPath = Join-Path $stage 'config.json'
    [IO.File]::WriteAllText($configPath, ($hostConfig | ConvertTo-Json -Depth 30), $utf8)
    $arguments = (Quote-Argument $dll) + ' ' + (Quote-Argument $configPath)
    $hostProcess = Start-Process -FilePath $dotnet -ArgumentList $arguments -WorkingDirectory $stage -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $stage 'host.stdout.log') -RedirectStandardError (Join-Path $stage 'host.stderr.log')
    $owned.Add($hostProcess)
    $ready = $false
    for ($attempt=0; $attempt -lt 100; $attempt++) {
        try { $health = Invoke-RestMethod ($control + '/health') -TimeoutSec 2; $ready=$true; break } catch { Start-Sleep -Milliseconds 200 }
    }
    if (-not $ready) { throw 'Central capture host did not start.' }
    $exe = Find-ExcelExecutable
    $processes = @()
    $minimizeClock = [Diagnostics.Stopwatch]::StartNew()
    function Minimize-OwnedExcel {
        if (-not $useMinimized -or $minimizeClock.ElapsedMilliseconds -lt 250) { return }
        $minimizeClock.Restart()
        foreach ($excel in $processes) { [CaptureWindowMinimizer]::Minimize($excel) }
    }
    $launchSpecs = @()
    foreach ($side in @('original','default','maximum','too-aggressive')) {
            $id = $side
            $guid = [Guid]::NewGuid().ToString()
            [xml]$manifest = [IO.File]::ReadAllText((Join-Path $web 'manifest.xml'), $utf8).Replace('http://localhost:3000', $baseUrl)
            $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Id']").InnerText = $guid
            foreach ($node in $manifest.SelectNodes('//*[@DefaultValue]')) {
                $url=$node.GetAttribute('DefaultValue')
                if ($url.StartsWith($baseUrl + '/')) { $node.SetAttribute('DefaultValue', ($url + '?run=' + $id)) }
            }
            $manifestPath = Join-Path $stage ($id + '.xml')
            [IO.File]::WriteAllText($manifestPath, $manifest.OuterXml, $utf8)
            $book = Join-Path $stage ($id + '.xlsx')
            Copy-Item -LiteralPath ([string]$request.$side) -Destination $book
            Add-AutoOpenTaskpane $book $guid '1.0.0.0'
            Set-InitialWorksheet $book ([int]$request.capture.sheets[0].index)
            # Large staged XLSX files can briefly retain a Windows file handle
            # after the preceding ZIP rewrite. Retry only this reversible step.
            for ($patchAttempt = 0; $patchAttempt -lt 12; $patchAttempt++) {
                try {
                    [CaptureWorkbook]::HideErrorIndicators($book)
                    break
                } catch {
                    if ($patchAttempt -eq 11) { throw }
                    Start-Sleep -Milliseconds 500
                }
            }
            $registrations[$guid] = Register-AddIn $guid $manifestPath
            $launchSpecs += @{ id=$id; book=$book }
    }
    [IO.File]::WriteAllText([string]$request.stage_ready_signal, 'ready', $utf8)
    $stageWait = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath ([string]$request.launch_signal))) {
        Minimize-OwnedExcel
        if ($stageWait.Elapsed.TotalSeconds -gt 4 * $StartupTimeoutSeconds + 120) {
            throw 'Global Excel launch barrier timeout.'
        }
        if (-not $IgnoreMemoryGuard -and [CaptureMemory]::Available -lt 1GB) { throw 'Physical RAM guard at launch barrier.' }
        Start-Sleep -Milliseconds 200
    }
    if (([IO.File]::ReadAllText([string]$request.launch_signal, $utf8)).Trim() -ne 'go') { throw 'Global launch aborted.' }
    $result.stage_wait_seconds = $stageWait.Elapsed.TotalSeconds
    $phaseIndex = 0
    foreach ($spec in $launchSpecs) {
        $id = [string]$spec.id
        $phaseLaunch = Join-Path (Split-Path -Parent ([string]$request.ready_signal)) ("phase-$phaseIndex.launch")
        $tokenWait = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-Path -LiteralPath $phaseLaunch)) {
            Minimize-OwnedExcel
            if ($tokenWait.Elapsed.TotalSeconds -gt 4 * $StartupTimeoutSeconds + 300) {
                throw "Excel phase $phaseIndex launch token timeout."
            }
            Start-Sleep -Milliseconds 100
        }
        if (([IO.File]::ReadAllText($phaseLaunch, $utf8)).Trim() -ne 'go') { throw "Excel phase $phaseIndex launch aborted." }
        if ($Visible) {
            $process = $desktop.StartVisible($exe, ('/x ' + (Quote-Argument ([string]$spec.book))), $owned)
        } elseif ($useMinimized) {
            $process = $desktop.StartMinimized($exe, ('/x ' + (Quote-Argument ([string]$spec.book))), $owned)
        } else {
            $process = $desktop.Start($exe, ('/x ' + (Quote-Argument ([string]$spec.book))), $owned)
        }
        $processes += $process
        $startWait = [Diagnostics.Stopwatch]::StartNew()
        while ($true) {
            Minimize-OwnedExcel
            $health = Invoke-RestMethod ($control + '/health') -TimeoutSec 3
            if ($health.errors.Count) { throw ($health.errors -join '; ') }
            if ($health.seen -contains $id) { break }
            $process.Refresh()
            if ($process.HasExited) { throw "Excel exited before add-in startup: $id" }
            if ($startWait.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds) { throw "Add-in startup timeout: $id" }
            if (-not $IgnoreMemoryGuard -and [CaptureMemory]::Available -lt 1GB) { throw 'Physical RAM guard during startup.' }
            Start-Sleep -Milliseconds 250
        }
        Write-Host ("Ready " + $id)
        $phaseReady = Join-Path (Split-Path -Parent ([string]$request.ready_signal)) ("phase-$phaseIndex.ready")
        [IO.File]::WriteAllText($phaseReady, 'ready', $utf8)
        if ($phaseIndex -lt 3) {
            $phaseSignal = Join-Path (Split-Path -Parent ([string]$request.launch_signal)) ("phase-$phaseIndex.signal")
            $phaseWait = [Diagnostics.Stopwatch]::StartNew()
            while (-not (Test-Path -LiteralPath $phaseSignal)) {
                Minimize-OwnedExcel
                if ($phaseWait.Elapsed.TotalSeconds -gt 2 * $StartupTimeoutSeconds + 120) {
                    throw "Excel phase $phaseIndex barrier timeout."
                }
                if (-not $IgnoreMemoryGuard -and [CaptureMemory]::Available -lt 1GB) { throw 'Physical RAM guard at Excel phase barrier.' }
                Start-Sleep -Milliseconds 200
            }
            if (([IO.File]::ReadAllText($phaseSignal, $utf8)).Trim() -ne 'go') { throw "Excel phase $phaseIndex aborted." }
        }
        $phaseIndex++
    }
    # Office can replace a bootstrap task pane shortly after its first request.
    # Wait for a quiet origin before releasing any capture work.
    $quietWait = [Diagnostics.Stopwatch]::StartNew()
    do {
        Minimize-OwnedExcel
        Start-Sleep -Milliseconds 250
        $health = Invoke-RestMethod ($control + '/health') -TimeoutSec 3
        if ($health.errors.Count) { throw ($health.errors -join '; ') }
        if ($quietWait.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds) { throw 'Add-in startup never became quiet.' }
    } until ($health.config_quiet_ms -ge 2000)
    Invoke-RestMethod ($control + '/control/preflight') -Method Post -TimeoutSec 3 | Out-Null
    $readyWait = [Diagnostics.Stopwatch]::StartNew()
    do {
        Minimize-OwnedExcel
        $health = Invoke-RestMethod ($control + '/health') -TimeoutSec 3
        if ($health.errors.Count) { throw ($health.errors -join '; ') }
        if ($readyWait.Elapsed.TotalSeconds -gt 4 * $StartupTimeoutSeconds + 120) {
            throw 'Four workers did not confirm workbook readiness.'
        }
        if (@($health.ready).Count -ne 4) { Start-Sleep -Milliseconds 250 }
    } until (@($health.ready).Count -eq 4)
    $result.setup_seconds = $clock.Elapsed.TotalSeconds
    [IO.File]::WriteAllText([string]$request.ready_signal, 'ready', $utf8)
    $measurementWait = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath ([string]$request.measurement_signal))) {
        Minimize-OwnedExcel
        if ($measurementWait.Elapsed.TotalSeconds -gt 4 * $StartupTimeoutSeconds + 120) {
            throw 'Common measurement start timeout.'
        }
        Start-Sleep -Milliseconds 100
    }
    if (([IO.File]::ReadAllText([string]$request.measurement_signal, $utf8)).Trim() -ne 'go') { throw 'Common measurement aborted.' }
    $trialCpuBefore = $owned.CpuSeconds
    $result.minimum_available_bytes = [CaptureMemory]::Available
    $barrierClock = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath ([string]$request.start_signal))) {
        Minimize-OwnedExcel
        if ($barrierClock.Elapsed.TotalSeconds -gt 4 * $CaptureTimeoutSeconds + 120) {
            throw 'Global capture barrier timeout.'
        }
        if (-not $IgnoreMemoryGuard -and [CaptureMemory]::Available -lt 1GB) { throw 'Physical RAM guard at capture barrier.' }
        $result.minimum_available_bytes = [Math]::Min([UInt64]$result.minimum_available_bytes, [CaptureMemory]::Available)
        foreach ($process in $processes) {
            $process.Refresh()
            if ($process.HasExited) { throw ('Excel exited at capture barrier: ' + $process.Id) }
        }
        Start-Sleep -Milliseconds 200
    }
    if (([IO.File]::ReadAllText([string]$request.start_signal, $utf8)).Trim() -ne 'go') { throw 'Capture start aborted.' }
    $result.barrier_wait_seconds = $barrierClock.Elapsed.TotalSeconds
    $cpuBefore = $owned.CpuSeconds
    $result.excel_pids = @($processes | ForEach-Object { $_.Id })
    [IO.File]::WriteAllText((Join-Path $destination 'owned-processes.json'), (@{ excel=$result.excel_pids; host=$hostProcess.Id; launcher=$PID } | ConvertTo-Json), $utf8)
    Invoke-RestMethod ($control + '/control/start') -Method Post -TimeoutSec 3 | Out-Null
    $captureClock = [Diagnostics.Stopwatch]::StartNew()
    do {
        Minimize-OwnedExcel
        Start-Sleep -Milliseconds 500
        $health = Invoke-RestMethod ($control + '/health') -TimeoutSec 5
        $result.minimum_available_bytes = [Math]::Min([UInt64]$result.minimum_available_bytes, [CaptureMemory]::Available)
        if ($health.errors.Count) { throw ($health.errors -join '; ') }
        if (-not $IgnoreMemoryGuard -and [CaptureMemory]::Available -lt 1GB) { throw 'Physical RAM guard during capture.' }
        if ($captureClock.Elapsed.TotalSeconds -gt $CaptureTimeoutSeconds) { throw 'Concurrent capture timeout.' }
        foreach ($process in $processes) {
            $process.Refresh()
            if ($process.HasExited) { throw ('Excel exited during capture: ' + $process.Id) }
        }
    } until ($health.complete)
    $result.status = 'complete'
    $result.capture_seconds = $health.capture_seconds
    $result.comparisons_per_minute = 3.0 * 60.0 / $health.capture_seconds
    $result.capture = $health
    $result.capture_cpu_seconds = $owned.CpuSeconds - $cpuBefore
    $result.logical_processors = [Environment]::ProcessorCount
    # The Python scheduler treats the final filename as a completion signal.
    # Publish it only after the JSON has been fully written.
    $captureResult = Join-Path $destination 'capture-result.json'
    $captureResultPending = Join-Path $destination 'capture-result.json.pending'
    [IO.File]::WriteAllText($captureResultPending, ($result | ConvertTo-Json -Depth 60), $utf8)
    [IO.File]::Move($captureResultPending, $captureResult)
    $release = [string]$request.release_signal
    $releaseWait = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $release)) {
        Minimize-OwnedExcel
        if ($releaseWait.Elapsed.TotalSeconds -gt 4 * $CaptureTimeoutSeconds + 120) {
            throw 'Completed group was not released for cleanup.'
        }
        if (-not $IgnoreMemoryGuard -and [CaptureMemory]::Available -lt 1GB) { throw 'Physical RAM guard while holding Excel open.' }
        $result.minimum_available_bytes = [Math]::Min([UInt64]$result.minimum_available_bytes, [CaptureMemory]::Available)
        foreach ($process in $processes) {
            $process.Refresh()
            if ($process.HasExited) { throw ('Excel exited before common trial completion: ' + $process.Id) }
        }
        Start-Sleep -Milliseconds 200
    }
    $result.measured_cpu_seconds = $owned.CpuSeconds - $trialCpuBefore
    # Copy report evidence after the common measurement has ended. The host
    # already received every PNG before it reported capture completion.
    foreach ($item in @(Get-ChildItem -LiteralPath $hostOutput)) {
        Copy-Item -LiteralPath $item.FullName -Destination $destination -Recurse -Force
    }
} catch {
    $result.status = 'failed'
    if ($_.Exception.Message.StartsWith('RAM budget guard:')) { $result.status = 'resource-limited' }
    $result.error = $_.Exception.Message
    $result.error_position = $_.InvocationInfo.PositionMessage
    Write-Warning $result.error
    if ($baseUrl) {
        try { $result.partial = Invoke-RestMethod ($control + '/health') -TimeoutSec 3 } catch {}
    }
} finally {
    if ($null -ne $owned) {
        try { $result.peak_job_bytes = $owned.PeakBytes } catch { $result.metrics_error = $_.Exception.Message }
        finally { $owned.Dispose() }
    }
    if ($null -ne $desktop) { $desktop.Dispose() }
    foreach ($id in @($registrations.Keys)) { Restore-AddIn $id $registrations[$id] }
    foreach ($name in @('host.stdout.log','host.stderr.log')) {
        $path=Join-Path $stage $name
        if (Test-Path -LiteralPath $path) { Copy-Item -LiteralPath $path -Destination $destination -Force }
    }
    # Job termination is asynchronous; Excel can briefly retain workbook handles.
    for ($attempt=0; $attempt -lt 20; $attempt++) {
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
        if (-not (Test-Path -LiteralPath $stage)) { break }
        Start-Sleep -Milliseconds 250
    }
    if (Test-Path -LiteralPath $stage) { $result.cleanup_warning = "Temporary files remain: $stage" }
    $result.total_seconds = $clock.Elapsed.TotalSeconds
    [IO.File]::WriteAllText((Join-Path $destination 'result.json'), ($result | ConvertTo-Json -Depth 40), $utf8)
}
if ($result.status -ne 'complete') { exit 1 }
