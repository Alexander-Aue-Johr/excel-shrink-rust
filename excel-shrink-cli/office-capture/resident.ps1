param(
    [Parameter(Mandatory=$true)][string]$SessionPath,
    [ValidateRange(10,600)][int]$StartupTimeoutSeconds = 300,
    [ValidateRange(10,7200)][int]$CaptureTimeoutSeconds = 1800,
    [switch]$Visible
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)
$session = [IO.Path]::GetFullPath($SessionPath)
if (-not (Test-Path -LiteralPath (Join-Path $session 'clients') -PathType Container)) {
    throw 'Resident session requires a clients directory.'
}

# Import the existing staging and registry helpers without running run.ps1.
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'run.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Cannot parse capture launcher helpers.' }
$runtimeLiteral = "'" + $PSScriptRoot.Replace("'", "''") + "'"
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    Invoke-Expression ($function.Extent.Text.Replace('$PSScriptRoot', $runtimeLiteral))
}
$archiveLauncher = Join-Path $PSScriptRoot '../src/regression/archive_processes.ps1'
$ast = [Management.Automation.Language.Parser]::ParseFile($archiveLauncher, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Cannot parse scoped archive cleanup helpers.' }
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    if ($function.Name -in @('Get-ArchiveStagePath', 'Remove-ArchiveStage')) {
        Invoke-Expression $function.Extent.Text
    }
}

function Write-SessionJson([string]$name, $value) {
    $target = Join-Path $session $name
    $pending = $target + '.' + [Guid]::NewGuid().ToString('N') + '.pending'
    [IO.File]::WriteAllText($pending, ($value | ConvertTo-Json -Depth 40), $utf8)
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try {
            [ResidentProcesses]::PublishFile($pending, $target)
            return
        } catch {
            if ($attempt -eq 19) { throw }
            Start-Sleep -Milliseconds 50
        }
    }
}

function Add-SessionEvent([string]$kind, [string]$client, $extra) {
    $event = [ordered]@{ event=$kind; client_id=$client; utc=[DateTime]::UtcNow.ToString('o'); elapsed_seconds=$clock.Elapsed.TotalSeconds }
    if ($null -ne $extra) { $event.detail = $extra }
    [IO.File]::AppendAllText((Join-Path $session 'launcher-events.jsonl'), (($event | ConvertTo-Json -Depth 20 -Compress) + "`n"), $utf8)
}

function Wait-BatchRelease {
    if ($null -eq $batchLifetime) { return 'released' }
    if ($batchReleased) { return $batchReleaseReason }
    $ready = [string]$batchLifetime.ready_path
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($ready))
    $pending = $ready + '.' + [Guid]::NewGuid().ToString('N') + '.pending'
    [IO.File]::WriteAllText($pending, (@{ launcher_pid=$PID; status=$metrics.status; ready_at=[DateTime]::UtcNow.ToString('o') } | ConvertTo-Json), $utf8)
    [ResidentProcesses]::PublishOnce($pending, $ready)
    Add-SessionEvent 'batch_wait_started' '' $null
    $wait = [Diagnostics.Stopwatch]::StartNew()
    $reason = 'released'
    while (-not (Test-Path -LiteralPath ([string]$batchLifetime.release_path))) {
        # A finished group's Excel/WebView shutdown can reload a peer's task
        # pane. Hold every owned process and registration until the whole batch
        # is finished; heartbeat silence is not a reason to dispose the group.
        if ($null -eq $batchOwner -or $batchOwner.HasExited) { $reason='coordinator_exited'; break }
        $available = [ResidentProcesses]::AvailablePhysicalBytes
        $commit = [ResidentProcesses]::AvailableCommitBytes
        $metrics.minimum_available_bytes = [Math]::Min([UInt64]$metrics.minimum_available_bytes, $available)
        $metrics.minimum_commit_bytes = [Math]::Min([UInt64]$metrics.minimum_commit_bytes, $commit)
        if ($available -lt 4GB -or $commit -lt 4GB) { $reason='memory_guard'; break }
        if ($wait.Elapsed.TotalSeconds -ge [double]$batchLifetime.timeout_seconds) { $reason='timeout'; break }
        if ($metricsClock.ElapsedMilliseconds -ge 1000) {
            $metrics.elapsed_seconds = $clock.Elapsed.TotalSeconds
            $metrics.batch_wait_seconds = $wait.Elapsed.TotalSeconds
            Write-SessionJson 'metrics.json' $metrics
            $metricsClock.Restart()
        }
        Start-Sleep -Milliseconds 200
    }
    $metrics.batch_wait_seconds = $wait.Elapsed.TotalSeconds
    $metrics.batch_release_reason = $reason
    $script:batchReleased = $true
    $script:batchReleaseReason = $reason
    Add-SessionEvent 'batch_released' '' @{ reason=$reason; wait_seconds=$wait.Elapsed.TotalSeconds }
    return $reason
}

function Build-ResidentHost([string]$dotnet, [string]$stage) {
    $source = Join-Path $PSScriptRoot 'resident-host'
    $key = Get-SourceKey $source
    $cache = Join-Path $env:LOCALAPPDATA ('excel-shrink-rust\office-capture\resident-host\' + $key)
    $dll = Join-Path $cache 'ResidentCaptureHost.dll'
    $buildMutex = New-Object Threading.Mutex($false, ('Local\ExcelShrinkResidentHost-' + $key))
    $locked = $false
    try {
        try { $locked = $buildMutex.WaitOne(180000) } catch [Threading.AbandonedMutexException] { $locked = $true }
        if (-not $locked) { throw 'Resident host build lock timed out.' }
        if ((Test-Path -LiteralPath $dll -PathType Leaf) -and
            (Test-Path -LiteralPath (Join-Path $cache 'ResidentCaptureHost.runtimeconfig.json') -PathType Leaf) -and
            (Test-Path -LiteralPath (Join-Path $cache 'ResidentCaptureHost.deps.json') -PathType Leaf)) { return $dll }
        $localSource = Join-Path $stage 'host-source'
        $localBuild = Join-Path $stage 'host-build'
        [void][IO.Directory]::CreateDirectory($localSource)
        foreach ($file in @(Get-ChildItem -LiteralPath $source -File | Where-Object { $_.Extension -in @('.cs', '.csproj') })) {
            Copy-Item -LiteralPath $file.FullName -Destination $localSource
        }
        $project = Join-Path $localSource 'ResidentCaptureHost.csproj'
        $arguments = 'build {0} -c Release -o {1} --nologo' -f (Quote-Argument $project), (Quote-Argument $localBuild)
        $build = Start-Process -FilePath $dotnet -ArgumentList $arguments -WorkingDirectory $localSource -WindowStyle Hidden -PassThru `
            -RedirectStandardOutput (Join-Path $session 'host-build.stdout.log') -RedirectStandardError (Join-Path $session 'host-build.stderr.log')
        $owned.Add($build)
        if (-not $build.WaitForExit(180000)) { throw 'Resident host build exceeded 180 seconds.' }
        $build.Refresh()
        if (($null -ne $build.ExitCode -and $build.ExitCode -ne 0) -or
            -not (Test-Path -LiteralPath (Join-Path $localBuild 'ResidentCaptureHost.dll') -PathType Leaf)) {
            throw 'Resident host build failed; see host-build logs in the session.'
        }
        [void][IO.Directory]::CreateDirectory($cache)
        foreach ($file in @(Get-ChildItem -LiteralPath $localBuild -File)) {
            Copy-Item -LiteralPath $file.FullName -Destination $cache -Force
        }
        return $dll
    } finally {
        if ($locked) { $buildMutex.ReleaseMutex() }
        $buildMutex.Dispose()
    }
}

function Update-Resources {
    if ($null -ne $batchLifetime -and ($null -eq $batchOwner -or $batchOwner.HasExited)) {
        throw 'Resident batch coordinator exited.'
    }
    $available = [ResidentProcesses]::AvailablePhysicalBytes
    $commit = [ResidentProcesses]::AvailableCommitBytes
    $metrics.minimum_available_bytes = [Math]::Min([UInt64]$metrics.minimum_available_bytes, $available)
    $metrics.minimum_commit_bytes = [Math]::Min([UInt64]$metrics.minimum_commit_bytes, $commit)
    if ($available -lt 4GB -or $commit -lt 4GB) { throw 'Resident RAM guard: less than 4 GiB physical RAM or commit headroom remains.' }
    if ($null -ne $hostProcess) {
        $hostProcess.Refresh()
        if ($hostProcess.HasExited) { throw 'Resident capture host exited unexpectedly.' }
    }
    foreach ($id in @($clients.Keys)) {
        $entry = $clients[$id]
        if ($entry.closed) { continue }
        $entry.process.Refresh()
        if ($entry.process.HasExited) { throw "Resident Excel process exited unexpectedly: $id" }
        $item = $metrics.clients[$id]
        $item.working_set_bytes = $entry.process.WorkingSet64
        $item.private_bytes = $entry.process.PrivateMemorySize64
        $item.peak_working_set_bytes = $entry.process.PeakWorkingSet64
        $item.peak_private_bytes = [Math]::Max([Int64]$item.peak_private_bytes, $entry.process.PrivateMemorySize64)
        if (-not $Visible -and $entry.seen) { [ResidentProcesses]::Minimize($entry.process) }
    }
    $metrics.peak_job_bytes = $owned.PeakBytes
    if ($metricsClock.ElapsedMilliseconds -ge 1000) {
        $metrics.elapsed_seconds = $clock.Elapsed.TotalSeconds
        Write-SessionJson 'metrics.json' $metrics
        $metricsClock.Restart()
    }
}

function Read-HostHealth {
    $health = Invoke-RestMethod -Uri ($controlUrl + '/health') -TimeoutSec 5
    if ($health.errors.Count) { throw ('Resident client error: ' + ($health.errors | ConvertTo-Json -Depth 12 -Compress)) }
    if ($healthSampleClock.ElapsedMilliseconds -ge 1000) {
        # Diagnostic only: a concurrent Workbooks.Open can pause Office.js.
        # These timestamps let us distinguish that pause from a dead process.
        Add-SessionEvent 'client_activity' '' $health.clients
        $healthSampleClock.Restart()
    }
    return $health
}

function Open-ResidentClient($spec) {
    $id = [string]$spec.client_id
    if ($id -notmatch '^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$') { throw 'Invalid resident client id.' }
    if ($clients.ContainsKey($id)) { return }
    if ($clients.Count -ge 16) { throw 'Resident session exceeded 16 clients.' }
    $source = [IO.Path]::GetFullPath([string]$spec.source_path)
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Resident workbook missing: $id" }
    if (-not @($spec.sheets).Count) { throw "Resident client has no planned sheets: $id" }
    Update-Resources
    $guid = [Guid]::NewGuid().ToString()
    [xml]$manifest = [IO.File]::ReadAllText((Join-Path $webStage 'manifest.xml'), $utf8).Replace('http://localhost:3000', $baseUrl)
    $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Id']").InnerText = $guid
    $version = $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Version']").InnerText.Trim()
    foreach ($node in $manifest.SelectNodes('//*[@DefaultValue]')) {
        $url = $node.GetAttribute('DefaultValue')
        if ($url.StartsWith($baseUrl + '/')) { $node.SetAttribute('DefaultValue', ($url + '?client=' + $id)) }
    }
    $manifestPath = Join-Path $stage ($id + '-manifest.xml')
    [IO.File]::WriteAllText($manifestPath, $manifest.OuterXml, $utf8)
    $book = Join-Path $stage ($id + [IO.Path]::GetExtension($source))
    Copy-Item -LiteralPath $source -Destination $book
    Add-AutoOpenTaskpane $book $guid $version
    for ($attempt = 0; $attempt -lt 12; $attempt++) {
        try { Set-InitialWorksheet $book ([int]$spec.sheets[0].index) $true; break }
        catch { if ($attempt -eq 11) { throw }; Start-Sleep -Milliseconds 250 }
    }
    $registrations[$guid] = Register-AddIn $guid $manifestPath
    $locked = $false
    $wait = [Diagnostics.Stopwatch]::StartNew()
    try {
        Add-SessionEvent 'opening_wait' $id $null
        while (-not $locked) {
            try { $locked = $openMutex.WaitOne(200) } catch [Threading.AbandonedMutexException] { $locked = $true }
            Update-Resources
            if (Test-Path -LiteralPath (Join-Path $session 'finish.json')) { throw 'Resident session finished while waiting to open a workbook.' }
            if ($wait.Elapsed.TotalSeconds -gt 4 * $StartupTimeoutSeconds + 120) { throw 'Timed out waiting for another resident Excel launch.' }
        }
        Add-SessionEvent 'opening_started' $id $null
        $openClock = [Diagnostics.Stopwatch]::StartNew()
        $process = $owned.Start($excelExecutable, ('/x ' + (Quote-Argument $book)))
        $clients[$id] = @{ process=$process; seen=$false; closed=$false; registration=$guid }
        $metrics.clients[$id] = [ordered]@{ pid=$process.Id; startup_seconds=$null; opened_at=[DateTime]::UtcNow.ToString('o'); peak_private_bytes=0; closed=$false }
        while ($true) {
            Update-Resources
            $health = Read-HostHealth
            if (@($health.seen) -contains $id) { break }
            if (Test-Path -LiteralPath (Join-Path $session 'finish.json')) { throw 'Resident session finished during workbook startup.' }
            if ($openClock.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds) { throw "Resident Office add-in startup timeout: $id" }
            Start-Sleep -Milliseconds 200
        }
        $clients[$id].seen = $true
        $metrics.clients[$id].startup_seconds = $openClock.Elapsed.TotalSeconds
        $metrics.clients[$id].seen_at = [DateTime]::UtcNow.ToString('o')
        Add-SessionEvent 'config_seen' $id @{ startup_seconds=$openClock.Elapsed.TotalSeconds; pid=$process.Id }
        Write-SessionJson 'metrics.json' $metrics
    } finally {
        $script:openingSeconds += $wait.Elapsed.TotalSeconds
        if ($locked) { $openMutex.ReleaseMutex() }
    }
}

$stage = Join-Path ([IO.Path]::GetTempPath()) ('es-office-capture-' + [Guid]::NewGuid().ToString('N'))
& $archiveLauncher -StagePath $stage
[void][IO.Directory]::CreateDirectory($stage)
$clock = [Diagnostics.Stopwatch]::StartNew()
$metricsClock = [Diagnostics.Stopwatch]::StartNew()
$healthSampleClock = [Diagnostics.Stopwatch]::StartNew()
$openingSeconds = 0.0
$owned = $null
$hostProcess = $null
$openMutex = $null
$registrations = @{}
$clients = @{}
$batchLifetime = $null
$batchOwner = $null
$batchReleased = $false
$batchReleaseReason = $null
$finishWait = $null
$metrics = [ordered]@{ status='starting'; visible=[bool]$Visible; memory_limit_mib=16384; minimum_available_bytes=[UInt64]::MaxValue; minimum_commit_bytes=[UInt64]::MaxValue; clients=@{} }
try {
    Add-Type -Path (Join-Path $PSScriptRoot 'ResidentProcesses.cs')
    Add-Type -Path (Join-Path $PSScriptRoot 'CaptureWorkbook.cs') -ReferencedAssemblies @('System.IO.Compression', 'System.IO.Compression.FileSystem', 'System.Xml')
    $batchPath = Join-Path $session 'batch-lifetime.json'
    if (Test-Path -LiteralPath $batchPath -PathType Leaf) {
        $configured = [IO.File]::ReadAllText($batchPath, $utf8) | ConvertFrom-Json
        if (-not [IO.Path]::IsPathRooted([string]$configured.ready_path) -or
            -not [IO.Path]::IsPathRooted([string]$configured.release_path) -or
            [int]$configured.owner_pid -le 0 -or
            [double]$configured.timeout_seconds -lt 1 -or
            [double]::IsInfinity([double]$configured.timeout_seconds) -or [double]::IsNaN([double]$configured.timeout_seconds) -or
            [IO.Path]::GetFullPath([string]$configured.ready_path) -eq [IO.Path]::GetFullPath([string]$configured.release_path)) {
            throw 'Invalid resident batch lifetime configuration.'
        }
        $batchLifetime = $configured
        $batchOwner = [Diagnostics.Process]::GetProcessById([int]$configured.owner_pid)
        # Acquire and retain the handle before any Excel starts. Subsequent
        # HasExited checks refer to this process even if Windows reuses its PID.
        $null = $batchOwner.Handle
    }
    if ($env:EXCEL_SHRINK_RESIDENT_MEMORY_MIB) {
        $budget = 0
        if (-not [int]::TryParse($env:EXCEL_SHRINK_RESIDENT_MEMORY_MIB, [ref]$budget) -or $budget -lt 1024 -or $budget -gt 65536) {
            throw 'EXCEL_SHRINK_RESIDENT_MEMORY_MIB must be between 1024 and 65536.'
        }
        $metrics.memory_limit_mib = $budget
    }
    $owned = New-Object ResidentProcesses([int]$metrics.memory_limit_mib)
    Update-Resources
    $openMutex = New-Object Threading.Mutex($false, 'Local\ExcelShrinkResidentOpen')
    $dotnet = Get-DotNet8
    $env:DOTNET_ROOT = Split-Path -Parent $dotnet
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $dll = Build-ResidentHost $dotnet $stage
    $portProbe = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
    $portProbe.Start()
    try { $port = $portProbe.LocalEndpoint.Port } finally { $portProbe.Stop() }
    $baseUrl = 'http://localhost:' + $port
    $controlUrl = 'http://127.0.0.1:' + $port
    $webStage = Join-Path $stage 'web'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'web') -Destination $webStage -Recurse
    $taskpanePath = Join-Path $webStage 'taskpane.html'
    [IO.File]::WriteAllText($taskpanePath, ([IO.File]::ReadAllText($taskpanePath, $utf8).Replace('__CAPTURE_RUN_ID__', [Guid]::NewGuid().ToString('N'))), $utf8)
    $hostOutput = Join-Path $session 'host-output'
    [void][IO.Directory]::CreateDirectory($hostOutput)
    $configPath = Join-Path $stage 'host.json'
    [IO.File]::WriteAllText($configPath, (@{port=$port; web=$webStage; output=$hostOutput; session=$session} | ConvertTo-Json), $utf8)
    $arguments = (Quote-Argument $dll) + ' ' + (Quote-Argument $configPath)
    $hostProcess = Start-Process -FilePath $dotnet -ArgumentList $arguments -WorkingDirectory $stage -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $session 'host.stdout.log') -RedirectStandardError (Join-Path $session 'host.stderr.log')
    $owned.Add($hostProcess)
    $ready = $false
    $hostWait = [Diagnostics.Stopwatch]::StartNew()
    while ($hostWait.Elapsed.TotalSeconds -lt 30) {
        Update-Resources
        try { $health = Read-HostHealth; if ($health.ok) { $ready=$true; break } } catch { Start-Sleep -Milliseconds 200 }
    }
    if (-not $ready) { throw 'Resident capture host did not become ready within 30 seconds.' }
    $excelExecutable = Find-ExcelExecutable
    Write-SessionJson 'launcher.json' @{ launcher_pid=$PID; host_pid=$hostProcess.Id; port=$port; stage=$stage; started_at=[DateTime]::UtcNow.ToString('o') }
    $metrics.status = 'running'
    Add-SessionEvent 'host_ready' '' @{ port=$port }
    while (-not (Test-Path -LiteralPath (Join-Path $session 'finish.json'))) {
        Update-Resources
        $health = Read-HostHealth
        foreach ($file in @(Get-ChildItem -LiteralPath (Join-Path $session 'clients') -File -Filter '*.json' | Sort-Object Name)) {
            $spec = [IO.File]::ReadAllText($file.FullName, $utf8) | ConvertFrom-Json
            if (-not $clients.ContainsKey([string]$spec.client_id)) { Open-ResidentClient $spec }
        }
        foreach ($id in @($clients.Keys)) {
            if ($null -eq $batchLifetime -and -not $clients[$id].closed -and (Test-Path -LiteralPath (Join-Path $session ('close\' + $id + '.json')))) {
                [ResidentProcesses]::Stop($clients[$id].process)
                $clients[$id].closed = $true
                $metrics.clients[$id].closed = $true
                $guid = $clients[$id].registration
                Restore-AddIn $guid $registrations[$guid]
                $registrations.Remove($guid)
                Add-SessionEvent 'client_closed' $id $null
            }
        }
        # Excel opening can pause other Office.js runtimes. Only an absolute
        # session budget (excluding serialized opens), liveness and RAM apply.
        if ($null -ne $batchLifetime -and (Test-Path -LiteralPath ([string]$batchLifetime.ready_path))) {
            $reason = Wait-BatchRelease
            if ($reason -ne 'released') { throw "Resident batch cleanup released early: $reason" }
            if ($null -eq $finishWait) { $finishWait = [Diagnostics.Stopwatch]::StartNew() }
            if ($finishWait.Elapsed.TotalSeconds -gt 60) { throw 'Resident worker did not finish after its batch was released.' }
        } elseif ($clock.Elapsed.TotalSeconds - $openingSeconds -gt $CaptureTimeoutSeconds) {
            throw 'Resident capture session exceeded its total capture timeout.'
        }
        Start-Sleep -Milliseconds 200
    }
    $finish = [IO.File]::ReadAllText((Join-Path $session 'finish.json'), $utf8) | ConvertFrom-Json
    if ($finish.complete -is [bool] -and $finish.complete) {
        $metrics.status = 'complete'
        Add-SessionEvent 'session_finished' '' $null
    } else {
        $metrics.status = 'cancelled'
        $metrics.error = 'Resident comparison ended before successful completion.'
        Add-SessionEvent 'session_cancelled' '' $null
    }
} catch {
    $metrics.status = 'failed'
    $metrics.error = $_.Exception.Message
    $metrics.error_position = $_.InvocationInfo.PositionMessage
    Write-SessionJson 'launcher-error.json' @{ error=$_.Exception.Message; position=$_.InvocationInfo.PositionMessage }
    Add-SessionEvent 'session_failed' '' $_.Exception.Message
    Write-Warning $_.Exception.Message
} finally {
    if ($null -ne $batchLifetime) {
        try {
            $reason = Wait-BatchRelease
            if ($reason -ne 'released') {
                $metrics.status = 'failed'
                if (-not $metrics.error) { $metrics.error = "Resident batch cleanup released early: $reason" }
            }
        } catch {
            $metrics.status = 'failed'
            $metrics.batch_cleanup_warning = $_.Exception.Message
        }
    }
    if ($null -ne $owned) {
        try { $metrics.peak_job_bytes = $owned.PeakBytes } catch {}
        $owned.Dispose()
    }
    foreach ($guid in @($registrations.Keys)) {
        try { Restore-AddIn $guid $registrations[$guid] } catch { $metrics.registry_cleanup_warning = $_.Exception.Message }
    }
    if ($null -ne $openMutex) { $openMutex.Dispose() }
    if ($null -ne $batchOwner) { $batchOwner.Dispose() }
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try { Remove-ArchiveStage $stage; break }
        catch { if ($attempt -eq 19) { $metrics.cleanup_warning = $_.Exception.Message } else { Start-Sleep -Milliseconds 250 } }
    }
    $metrics.elapsed_seconds = $clock.Elapsed.TotalSeconds
    $metrics.opening_seconds = $openingSeconds
    Write-SessionJson 'metrics.json' $metrics
    Write-SessionJson 'launcher-result.json' @{ status=$metrics.status; error=$metrics.error; elapsed_seconds=$metrics.elapsed_seconds; peak_job_bytes=$metrics.peak_job_bytes }
}
if ($metrics.status -ne 'complete') { exit 1 }
