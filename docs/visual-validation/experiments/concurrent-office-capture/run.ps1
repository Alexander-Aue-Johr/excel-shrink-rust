param(
    [Parameter(Mandatory=$true)][string]$RequestPath,
    [Parameter(Mandatory=$true)][string]$RuntimeRoot,
    [Parameter(Mandatory=$true)][string]$OutputDir,
    [ValidateRange(10,600)][int]$StartupTimeoutSeconds = 120,
    [ValidateRange(10,7200)][int]$CaptureTimeoutSeconds = 600,
    [ValidateRange(0,65536)][int]$MemoryMiB = 0,
    [ValidateRange(0,131072)][int]$ExpectedPeakMiB = 0
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)
$RuntimeRoot = [IO.Path]::GetFullPath($RuntimeRoot)
$destination = [IO.Path]::GetFullPath($OutputDir)
[void][IO.Directory]::CreateDirectory($destination)
if (Test-Path -LiteralPath (Join-Path $destination 'result.json')) { throw 'Trial already has results.' }
$request = [IO.File]::ReadAllText([IO.Path]::GetFullPath($RequestPath), $utf8) | ConvertFrom-Json
if ($request.pairs -lt 1 -or $request.pairs -gt 8 -or $request.batch_size -notin @(1,2,4) -or
    $request.transport -notin @('images','hash') -or $request.max_batch_pixels -lt 32000000 -or
    $request.max_batch_pixels -gt 128000000) {
    throw 'Invalid pair, transport or batch configuration.'
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
$result = [ordered]@{ status='failed'; pairs=[int]$request.pairs; transport=$request.transport;
    batch_size=[int]$request.batch_size; max_batch_pixels=[int]$request.max_batch_pixels }
$baseUrl = $null
try {
    Add-Type -Path @((Join-Path $RuntimeRoot 'CaptureProcesses.cs'), (Join-Path $RuntimeRoot 'CaptureDesktop.cs'), (Join-Path $PSScriptRoot 'Memory.cs'))
    Add-Type -Path (Join-Path $RuntimeRoot 'CaptureWorkbook.cs') -ReferencedAssemblies @('System.IO.Compression','System.IO.Compression.FileSystem','System.Xml')
    $available = [CaptureMemory]::Available
    if ($available -lt 2GB) { throw 'Less than 2 GiB physical RAM available before trial.' }
    if ($MemoryMiB -eq 0) { $MemoryMiB = [int][Math]::Min(32768, [Math]::Floor($available / 1MB * 0.65)) }
    $env:EXCEL_SHRINK_EXCEL_MEMORY_MIB = [string]$MemoryMiB
    $result.memory_limit_mib = $MemoryMiB
    $result.minimum_available_bytes = $available
    $result.expected_peak_mib = $ExpectedPeakMiB
    if ($ExpectedPeakMiB -gt $MemoryMiB -or ($ExpectedPeakMiB + 1024) * 1MB -gt $available) {
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
        port=$port; output=$hostOutput; web=$web; pairs=[int]$request.pairs;
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
    for ($pair=0; $pair -lt $request.pairs; $pair++) {
        foreach ($side in @('original','shrunk')) {
            $id = "$pair-$side"
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
            [CaptureWorkbook]::HideErrorIndicators($book)
            $registrations[$guid] = Register-AddIn $guid $manifestPath
            $process = $desktop.Start($exe, ('/x ' + (Quote-Argument $book)), $owned)
            $processes += $process
            $startWait = [Diagnostics.Stopwatch]::StartNew()
            while ($true) {
                $health = Invoke-RestMethod ($control + '/health') -TimeoutSec 3
                if ($health.errors.Count) { throw ($health.errors -join '; ') }
                if ($health.seen -contains $id) { break }
                $process.Refresh()
                if ($process.HasExited) { throw "Excel exited before add-in startup: $id" }
                if ($startWait.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds) { throw "Add-in startup timeout: $id" }
                if ([CaptureMemory]::Available -lt 1GB) { throw 'Physical RAM guard during startup.' }
                Start-Sleep -Milliseconds 250
            }
            Write-Host ("Ready " + $id)
        }
    }
    # Office can replace a bootstrap task pane shortly after its first request.
    # Wait for a quiet origin before releasing any capture work.
    $quietWait = [Diagnostics.Stopwatch]::StartNew()
    do {
        Start-Sleep -Milliseconds 250
        $health = Invoke-RestMethod ($control + '/health') -TimeoutSec 3
        if ($health.errors.Count) { throw ($health.errors -join '; ') }
        if ($quietWait.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds) { throw 'Add-in startup never became quiet.' }
    } until ($health.config_quiet_ms -ge 2000)
    $result.setup_seconds = $clock.Elapsed.TotalSeconds
    $cpuBefore = $owned.CpuSeconds
    $result.excel_pids = @($processes | ForEach-Object { $_.Id })
    [IO.File]::WriteAllText((Join-Path $destination 'owned-processes.json'), (@{ excel=$result.excel_pids; host=$hostProcess.Id; launcher=$PID } | ConvertTo-Json), $utf8)
    Invoke-RestMethod ($control + '/control/start') -Method Post -TimeoutSec 3 | Out-Null
    $captureClock = [Diagnostics.Stopwatch]::StartNew()
    do {
        Start-Sleep -Milliseconds 500
        $health = Invoke-RestMethod ($control + '/health') -TimeoutSec 5
        $result.minimum_available_bytes = [Math]::Min([UInt64]$result.minimum_available_bytes, [CaptureMemory]::Available)
        if ($health.errors.Count) { throw ($health.errors -join '; ') }
        if ([CaptureMemory]::Available -lt 1GB) { throw 'Physical RAM guard during capture.' }
        if ($captureClock.Elapsed.TotalSeconds -gt $CaptureTimeoutSeconds) { throw 'Concurrent capture timeout.' }
        foreach ($process in $processes) {
            $process.Refresh()
            if ($process.HasExited) { throw ('Excel exited during capture: ' + $process.Id) }
        }
    } until ($health.complete)
    $result.status = 'complete'
    $result.capture_seconds = $health.capture_seconds
    $result.comparisons_per_minute = $request.pairs * 60.0 / $health.capture_seconds
    $result.capture = $health
    $result.capture_cpu_seconds = $owned.CpuSeconds - $cpuBefore
    $result.logical_processors = [Environment]::ProcessorCount
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
    $result.total_seconds = $clock.Elapsed.TotalSeconds
    [IO.File]::WriteAllText((Join-Path $destination 'result.json'), ($result | ConvertTo-Json -Depth 40), $utf8)
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
if ($result.status -ne 'complete') { exit 1 }
