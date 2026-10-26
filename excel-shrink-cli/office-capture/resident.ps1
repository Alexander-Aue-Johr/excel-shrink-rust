param(
    [Parameter(Mandatory=$true)][string]$SessionPath,
    [ValidateRange(10,600)][int]$StartupTimeoutSeconds = 300,
    [ValidateRange(10,7200)][int]$CaptureTimeoutSeconds = 1800,
    [switch]$Visible,
    [switch]$HideExcel
)

$ErrorActionPreference = 'Stop'
if ($Visible -and $HideExcel) { throw 'Visible and HideExcel cannot be combined.' }
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
    Write-ResidentJsonFile (Join-Path $session $name) $value
}

function Confirm-ResidentClientExit($ownedClients, $clientMetrics, [int]$TimeoutMilliseconds = 3000) {
    # Closing a Job Object requests termination; it does not mean every process
    # has already signalled exit. Use one shared bound for the entire group.
    $remaining = New-Object 'Collections.Generic.HashSet[string]'
    foreach ($id in @($ownedClients.Keys)) {
        [void]$remaining.Add($id)
        $clientMetrics[$id].status = 'closing'
        $clientMetrics[$id].stage = 'closing'
    }
    $exitClock = [Diagnostics.Stopwatch]::StartNew()
    do {
        foreach ($id in @($remaining)) {
            try {
                if ($ownedClients[$id].process.WaitForExit(0)) {
                    $ownedClients[$id].closed = $true
                    $clientMetrics[$id].closed = $true
                    $clientMetrics[$id].status = 'closed'
                    $clientMetrics[$id].process_alive = $false
                    $clientMetrics[$id].stage = 'closed'
                    $clientMetrics[$id].exit_confirmed_unix_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                    [void]$remaining.Remove($id)
                }
            } catch {
                $clientMetrics[$id].exit_confirmation_error = $_.Exception.Message
            }
        }
        if (-not $remaining.Count -or $exitClock.ElapsedMilliseconds -ge $TimeoutMilliseconds) { break }
        Start-Sleep -Milliseconds 50
    } while ($true)
    foreach ($id in $remaining) {
        # A finished launcher cannot keep observing this process. Explicitly
        # report uncertainty rather than leaving an apparently active close.
        $clientMetrics[$id].status = 'exit_unconfirmed'
        $clientMetrics[$id].stage = 'exit_unconfirmed'
        $clientMetrics[$id].process_alive = $null
    }
    return @($remaining)
}

function Write-ResidentJsonFile([string]$target, $value) {
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

function Get-ResidentOpenTimeoutSeconds {
    if (-not $liveControlPath) { return $StartupTimeoutSeconds }
    try {
        $file = Get-Item -LiteralPath $liveControlPath -ErrorAction Stop
        if ($file.Length -le 16384) {
            $value = ([IO.File]::ReadAllText($file.FullName, $utf8) | ConvertFrom-Json).open_timeout_seconds
            $seconds = 0
            if (($value -is [int] -or $value -is [long]) -and [int]::TryParse([string]$value, [ref]$seconds) -and $seconds -gt 0) {
                $script:liveOpenTimeoutSeconds = $seconds
            }
        }
    } catch { } # A transient snapshot problem must not silently shorten a deadline.
    return $liveOpenTimeoutSeconds
}

function Get-ResidentHideExcel {
    # Study admission records one immutable configuration for the entire group.
    # Standalone launchers without an attempt may still follow live controls.
    if ($env:EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID) {
        if ($env:EXCEL_SHRINK_HIDE_EXCEL -in @('0', '1')) { return $env:EXCEL_SHRINK_HIDE_EXCEL -eq '1' }
        return [bool]$HideExcel
    }
    if (-not $liveControlPath) { return [bool]$HideExcel }
    try {
        $file = Get-Item -LiteralPath $liveControlPath -ErrorAction Stop
        if ($file.Length -le 16384) {
            $value = ([IO.File]::ReadAllText($file.FullName, $utf8) | ConvertFrom-Json).hide_excel
            if ($value -is [bool]) { $script:liveHideExcel = $value }
        }
    } catch { } # Retain the last valid choice during an atomic replacement.
    return $liveHideExcel
}

function Update-ClientActivity($health) {
    if ($null -eq $health.clients) { return }
    foreach ($property in $health.clients.PSObject.Properties) {
        $id = $property.Name
        if (-not $metrics.clients.ContainsKey($id)) { continue }
        $sample = $property.Value
        $item = $metrics.clients[$id]
        foreach ($name in @('heartbeat_count', 'last_heartbeat_at', 'last_activity_at', 'active_command', 'command_count', 'data_started', 'error', 'monitor', 'runtime')) {
            $field = $sample.PSObject.Properties[$name]
            if ($null -ne $field) { $item[$name] = $field.Value }
        }
        if ($sample.error) {
            $item.stage = 'client_error'
            if ($sample.error.code -eq 'InvalidOperationInCellEditMode') { $item.stage = 'blocked_edit_mode' }
        } elseif ($sample.monitor -and $sample.monitor.state) { $item.stage = [string]$sample.monitor.state }
        elseif ($sample.active_command) { $item.stage = 'executing_job' }
        elseif ($item.status -eq 'ready') { $item.stage = 'waiting_for_job' }
    }
}

function Update-ClientReadySamples($health) {
    foreach ($property in $health.clients.PSObject.Properties) {
        $id = $property.Name
        $sample = $property.Value
        if (-not $clients.ContainsKey($id) -or -not $sample.ready_at -or $sample.startup_sampled) { continue }
        $entry = $clients[$id]
        $item = $metrics.clients[$id]
        # The host withholds all jobs until this exact ready observation is
        # sampled. Retain the Process object so a reused PID cannot be sampled.
        $entry.process.Refresh()
        if ($entry.process.HasExited) { throw "Excel exited before its startup memory sample: $id" }
        $readyAt = [DateTimeOffset]::Parse([string]$sample.ready_at)
        $observedAt = [DateTimeOffset]::UtcNow
        $seconds = $entry.open_clock.Elapsed.TotalSeconds - ($observedAt - $readyAt).TotalSeconds
        if ($seconds -lt 0) { throw "Ready observation predates this Excel launch: $id" }
        $item.startup_seconds = $seconds
        $item.startup_memory_private_bytes = $entry.process.PrivateMemorySize64
        $item.startup_memory_working_set_bytes = $entry.process.WorkingSet64
        $item.startup_sampled_unix_ms = $observedAt.ToUnixTimeMilliseconds()
        $item.ready_at = [string]$sample.ready_at
        $item.startup_measurement_method = 'launch-to-addin-ready-v1'
        $item.status = 'ready'
        $item.stage = 'waiting_for_job'
        $entry.seen = $true
        # Persist provenance before permitting capture allocations.
        Write-ResourceMetrics
        $body = @{ ready_at=[string]$sample.ready_at } | ConvertTo-Json -Compress
        try {
            $null = Invoke-RestMethod -Uri ($controlUrl + '/launcher/startup-sampled/' + $id) -Method Post `
                -Headers @{ 'X-Launcher-Token'=$launcherToken } -Body $body -ContentType 'application/json' -TimeoutSec 5
        } catch {
            if ($_.Exception.Response -and [int]$_.Exception.Response.StatusCode -eq 409) {
                # A replacement document can become ready before its first job.
                # Its next ready observation needs a fresh, separately bound sample.
                $entry.seen = $false
                $item.startup_measurement_method = $null
                continue
            }
            throw
        }
        Add-SessionEvent 'addin_ready_sampled' $id @{ startup_seconds=$seconds; ready_at=$item.ready_at; pid=$entry.process.Id; private_bytes=$item.startup_memory_private_bytes; working_set_bytes=$item.startup_memory_working_set_bytes }
    }
}

function Update-ClientStateTimes {
    if (-not $script:clientStageTimes) { $script:clientStageTimes = @{} }
    foreach ($id in @($metrics.clients.Keys)) {
        $item = $metrics.clients[$id]
        $identity = '{0}:{1}:{2}' -f $id,$item.pid,$item.stage
        if ($script:clientStageTimes[$id] -ne $identity) {
            $script:clientStageTimes[$id] = $identity
            $item.stage_since = [DateTime]::UtcNow.ToString('o')
            $item.last_launcher_event = [ordered]@{ at=$item.stage_since; kind='launcher_state_changed'; message=[string]$item.stage }
            if ($item.stage -in @('waiting_for_open_slot','waiting_for_addin','closed')) {
                $item.last_launcher_success = $item.last_launcher_event
            }
        }
        # A state sampled repeatedly retains its actual transition timestamp.
        if ($item.monitor -and $item.monitor.state -eq $item.stage -and $item.monitor.state_since) {
            $item.stage_since = [string]$item.monitor.state_since
        }
    }
}

function Update-ClientProcessMetrics {
    foreach ($id in @($clients.Keys)) {
        $entry = $clients[$id]
        $item = $metrics.clients[$id]
        try {
            $entry.process.Refresh()
            $item.process_alive = -not $entry.process.HasExited
            if ($item.process_alive) {
                $item.working_set_bytes = $entry.process.WorkingSet64
                $item.private_bytes = $entry.process.PrivateMemorySize64
                $item.peak_working_set_bytes = $entry.process.PeakWorkingSet64
                $item.peak_private_bytes = [Math]::Max([Int64]$item.peak_private_bytes, $entry.process.PrivateMemorySize64)
            } else { $item.stage = 'process_exited' }
        } catch {
            $item.process_alive = $null
            $item.process_sample_error = $_.Exception.Message
        }
    }
}

function Get-ResidentControlRoot([string]$path) {
    if (-not [IO.Path]::IsPathRooted($path)) { throw 'Control room directory must be absolute.' }
    $resolved = [IO.Path]::GetFullPath($path)
    # Rust's canonical paths use the extended Windows prefix. PowerShell 5.1
    # Join-Path interprets that prefix as an invalid provider drive.
    if ($resolved.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) { return '\\' + $resolved.Substring(8) }
    if ($resolved.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) { return $resolved.Substring(4) }
    return $resolved
}

function Test-ResidentDeadline([string]$kind, [double]$elapsed) {
    switch ($kind) {
        'opening' { return $elapsed -gt (Get-ResidentOpenTimeoutSeconds) }
        'session' { return (-not $liveControlPath) -and $elapsed -gt $CaptureTimeoutSeconds }
        default { throw 'Unknown resident deadline kind.' }
    }
}

function Add-SessionEvent([string]$kind, [string]$client, $extra) {
    $event = [ordered]@{ event=$kind; client_id=$client; utc=[DateTime]::UtcNow.ToString('o'); elapsed_seconds=$clock.Elapsed.TotalSeconds }
    if ($null -ne $extra) { $event.detail = $extra }
    [IO.File]::AppendAllText((Join-Path $session 'launcher-events.jsonl'), (($event | ConvertTo-Json -Depth 20 -Compress) + "`n"), $utf8)
}

function Get-SystemReserveBytes($configured) {
    $mib = 1024
    if ($null -ne $configured) {
        if (-not [int]::TryParse($configured, [ref]$mib) -or $mib -lt 1 -or $mib -gt 65536) {
            throw 'EXCEL_SHRINK_SYSTEM_RESERVE_MIB must be between 1 and 65536.'
        }
    }
    return [UInt64]$mib * 1MB
}

function Update-SystemReserve {
    if ($liveControlPath) {
        try {
            $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
            $stream = [IO.File]::Open($liveControlPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
            try {
                if ($stream.Length -le 16384) {
                    $reader = [IO.StreamReader]::new($stream, $utf8)
                    try { $controls = $reader.ReadToEnd() | ConvertFrom-Json }
                    finally { $reader.Dispose() }
                    $value = $controls.system_reserve_mib
                    if (($value -is [int] -or $value -is [long]) -and $value -ge 1 -and $value -le 65536) {
                        $script:systemReserveBytes = [UInt64]$value * 1MB
                    }
                }
            } finally { $stream.Dispose() }
        } catch { } # Keep the last valid reserve during missing, malformed or replaced snapshots.
    }
    $metrics.system_reserve_mib = $systemReserveBytes / 1MB
    $metrics.system_reserve_bytes = $systemReserveBytes
}

function Get-PrimaryClientIds([string]$directory) {
    $ids = @{}
    foreach ($file in @(Get-ChildItem -LiteralPath $directory -File -Filter '*.json')) {
        $spec = [IO.File]::ReadAllText($file.FullName, $utf8) | ConvertFrom-Json
        $id = [string]$spec.client_id
        if ($id -notmatch '^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$' -or $ids.ContainsKey($id)) {
            throw 'Invalid or duplicate primary resident client.'
        }
        $ids[$id] = $true
    }
    if (-not $ids.Count) { throw 'Resident session requires primary clients.' }
    return @($ids.Keys)
}

function Test-PrimaryClientsReady($ids, $entries) {
    if (-not @($ids).Count) { return $false }
    foreach ($id in $ids) {
        if (-not $entries.ContainsKey($id) -or -not $entries[$id].seen -or $entries[$id].closed) { return $false }
        $entries[$id].process.Refresh()
        if ($entries[$id].process.HasExited) { return $false }
    }
    return $true
}

function Read-OtherGroupPauseMillis {
    if ($null -eq $batchLifetime -or -not $batchLifetime.pause_path) { return $null }
    try {
        $file = Get-Item -LiteralPath ([string]$batchLifetime.pause_path) -ErrorAction Stop
        if ($file.Length -gt 4096) { return $null }
        $value = ([IO.File]::ReadAllText($file.FullName, $utf8) | ConvertFrom-Json).elapsed_millis
        if ($value -isnot [int] -and $value -isnot [long] -and $value -isnot [uint64]) { return $null }
        $counter = [UInt64]0
        if (-not [UInt64]::TryParse([string]$value, [ref]$counter)) { return $null }
        return $counter
    } catch { return $null }
}

function Update-OtherGroupPause {
    $counter = Read-OtherGroupPauseMillis
    if ($null -eq $counter) { return }
    $now = $clock.Elapsed.TotalSeconds
    if ($null -eq $script:pauseCounter) {
        $script:pauseCounter = $counter
        $script:pauseBaselineSeconds = $now
        return
    }
    if ($counter -le $script:pauseCounter) { return }
    # Cap cumulative credit, not individual polling intervals: the coordinator
    # and launcher sample at different cadences. Discard excess counter jumps,
    # and keep the high-water mark so reset/replay cannot credit an interval twice.
    $remaining = [Math]::Max(0.0, $now - $script:pauseBaselineSeconds - $script:otherOpeningSeconds)
    $delta = [Math]::Min(($counter - $script:pauseCounter) / 1000.0, $remaining)
    $script:otherOpeningSeconds += $delta
    $script:pauseCounter = $counter
}

function Get-OwnOpeningSeconds([double]$elapsed, [double]$otherBefore, [double]$otherAfter) {
    # Whitespace controls can open while another group warms up. Count that
    # overlap only once when discounting global Excel startup stalls.
    return [Math]::Max(0.0, $elapsed - [Math]::Max(0.0, $otherAfter - $otherBefore))
}

function Write-ResourceMetrics {
    Update-ClientProcessMetrics
    Update-ClientStateTimes
    $metrics.elapsed_seconds = $clock.Elapsed.TotalSeconds
    $metrics.sampled_unix_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $metrics.primary_clients_ready = Test-PrimaryClientsReady $primaryClientIds $clients
    $metrics.peak_job_bytes = $owned.PeakBytes
    try {
        $sample = $owned.SampleMemory()
        $metrics.current_job_bytes = $sample.PrivateBytes
        $metrics.current_job_private_working_set_bytes = $sample.PrivateWorkingSetBytes
        # One replaceable snapshot, including browser/render processes. This
        # distinguishes pane memory from Excel without collecting heap dumps.
        $metrics.owned_process_resources = @($sample.Processes | ForEach-Object {
            @{ pid=$_.Pid; name=$_.Name; process_started_unix_ms=$_.StartedUnixMilliseconds;
                private_bytes=$_.PrivateBytes; working_set_bytes=$_.WorkingSetBytes }
        })
        [void]$metrics.Remove('memory_sample_error')
    }
    catch {
        # An unavailable sample denies additional admission, without discarding
        # this group's already running capture or claiming a zero-memory group.
        $metrics.current_job_bytes = $null
        $metrics.current_job_private_working_set_bytes = $null
        $metrics.primary_clients_ready = $false
        $metrics.memory_sample_error = $_.Exception.Message
    }
    $metrics.other_group_opening_seconds = $otherOpeningSeconds
    Write-SessionJson 'metrics.json' $metrics
    if ($liveGroupDirectory) {
        # This separate snapshot has one writer. The Rust comparison owns
        # state.json; the HTTP service combines both without write races.
        $metrics.open_timeout_seconds = Get-ResidentOpenTimeoutSeconds
        Write-ResidentJsonFile (Join-Path $liveGroupDirectory 'launcher.json') $metrics
    }
    $metricsClock.Restart()
}

function Wait-BatchRelease {
    if ($null -eq $batchLifetime) { return 'released' }
    if ($batchReleased) { return $batchReleaseReason }
    $ready = [string]$batchLifetime.ready_path
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($ready))
    $pending = $ready + '.' + [Guid]::NewGuid().ToString('N') + '.pending'
    [IO.File]::WriteAllText($pending, (@{ launcher_pid=$PID; status=$metrics.status; ready_at=[DateTime]::UtcNow.ToString('o'); opening_pause_millis=(Read-OtherGroupPauseMillis) } | ConvertTo-Json), $utf8)
    [ResidentProcesses]::PublishOnce($pending, $ready)
    Add-SessionEvent 'batch_wait_started' '' $null
    # Rust or another observer may already have published readiness. Preserve
    # its first timestamp and pause baseline; repeated waits cannot restart it.
    $readyAge = [Math]::Max(0.0, ([DateTime]::UtcNow - [IO.File]::GetLastWriteTimeUtc($ready)).TotalSeconds)
    $waitCounter = $null
    try {
        $marker = Get-Item -LiteralPath $ready -ErrorAction Stop
        if ($marker.Length -le 4096) {
            $baseline = ([IO.File]::ReadAllText($ready, $utf8) | ConvertFrom-Json).opening_pause_millis
            $parsed = [UInt64]0
            if (($baseline -is [int] -or $baseline -is [long] -or $baseline -is [uint64]) -and
                [UInt64]::TryParse([string]$baseline, [ref]$parsed)) { $waitCounter = $parsed }
        }
    } catch { } # Old or malformed markers grant no historical pause credit.
    $waitPauseSeconds = 0.0
    $waitActiveSeconds = $readyAge
    $wait = [Diagnostics.Stopwatch]::StartNew()
    $reason = 'released'
    while (-not (Test-Path -LiteralPath ([string]$batchLifetime.release_path))) {
        # A finished group's Excel/WebView shutdown can reload a peer's task
        # pane. Hold every owned process and registration until the whole batch
        # is finished; heartbeat silence is not a reason to dispose the group.
        if ($null -eq $batchOwner -or $batchOwner.HasExited) { $reason='coordinator_exited'; break }
        Update-SystemReserve
        $available = [ResidentProcesses]::AvailablePhysicalBytes
        $commit = [ResidentProcesses]::AvailableCommitBytes
        $metrics.minimum_available_bytes = [Math]::Min([UInt64]$metrics.minimum_available_bytes, $available)
        $metrics.minimum_commit_bytes = [Math]::Min([UInt64]$metrics.minimum_commit_bytes, $commit)
        if ($available -lt $systemReserveBytes -or $commit -lt $systemReserveBytes) { $reason='memory_guard'; break }
        Update-OtherGroupPause
        $wall = $readyAge + $wait.Elapsed.TotalSeconds
        $counter = Read-OtherGroupPauseMillis
        if ($null -ne $counter) {
            if ($null -eq $waitCounter) { $waitCounter = $counter }
            elseif ($counter -gt $waitCounter) {
                $waitPauseSeconds = [Math]::Min($wall, $waitPauseSeconds + ($counter - $waitCounter) / 1000.0)
                $waitCounter = $counter
            }
        }
        $waitActiveSeconds = [Math]::Max(0.0, $wall - $waitPauseSeconds)
        if (-not $liveControlPath -and $waitActiveSeconds -ge [double]$batchLifetime.timeout_seconds) { $reason='timeout'; break }
        if ($metricsClock.ElapsedMilliseconds -ge 1000) {
            # Failed/finished clients remain owned until all peer groups finish.
            # Their heartbeats still describe the real add-ins during this wait.
            try { $null = Read-HostHealth } catch { }
            $metrics.batch_wait_seconds = $wait.Elapsed.TotalSeconds
            $metrics.batch_wait_active_seconds = $waitActiveSeconds
            $metrics.batch_wait_opening_pause_seconds = $waitPauseSeconds
            Write-ResourceMetrics
        }
        Start-Sleep -Milliseconds 200
    }
    $metrics.batch_wait_seconds = $wait.Elapsed.TotalSeconds
    $metrics.batch_wait_active_seconds = $waitActiveSeconds
    $metrics.batch_wait_opening_pause_seconds = $waitPauseSeconds
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
    Update-OtherGroupPause
    if ($null -ne $hostProcess -and -not $script:processingClientReset) { Invoke-ClientResets }
    if ($null -ne $liveOwner -and $liveOwner.HasExited) { throw 'Control room coordinator exited.' }
    if ($null -ne $batchLifetime -and ($null -eq $batchOwner -or $batchOwner.HasExited)) {
        throw 'Resident batch coordinator exited.'
    }
    Update-SystemReserve
    $available = [ResidentProcesses]::AvailablePhysicalBytes
    $commit = [ResidentProcesses]::AvailableCommitBytes
    $metrics.minimum_available_bytes = [Math]::Min([UInt64]$metrics.minimum_available_bytes, $available)
    $metrics.minimum_commit_bytes = [Math]::Min([UInt64]$metrics.minimum_commit_bytes, $commit)
    if ($available -lt $systemReserveBytes -or $commit -lt $systemReserveBytes) {
        throw "Resident RAM guard: less than $($metrics.system_reserve_mib) MiB physical RAM or commit headroom remains."
    }
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
        if ($entry.hide_excel) {
            if (-not $entry.private_desktop) { [ResidentProcesses]::Hide($entry.process) }
        }
        elseif (-not $Visible -and $entry.seen) { [ResidentProcesses]::Minimize($entry.process) }
    }
    if ($metricsClock.ElapsedMilliseconds -ge 1000) {
        Write-ResourceMetrics
    }
}

function Read-HostHealth {
    $health = Invoke-RestMethod -Uri ($controlUrl + '/health') -TimeoutSec 5
    Update-ClientActivity $health
    if ($health.errors.Count) { throw ('Resident client error: ' + ($health.errors | ConvertTo-Json -Depth 12 -Compress)) }
    Update-ClientReadySamples $health
    if ($healthSampleClock.ElapsedMilliseconds -ge 1000) {
        # Diagnostic only: a concurrent Workbooks.Open can pause Office.js.
        # These timestamps let us distinguish that pause from a dead process.
        Add-SessionEvent 'client_activity' '' $health.clients
        $healthSampleClock.Restart()
    }
    return $health
}

function Invoke-ClientResets {
    if (-not $liveGroupDirectory -or $clientResetClock.ElapsedMilliseconds -lt 500) { return }
    $clientResetClock.Restart()
    $directory = Join-Path $liveGroupDirectory 'client-resets'
    if (-not (Test-Path -LiteralPath $directory)) { return }
    foreach ($file in @(Get-ChildItem -LiteralPath $directory -Filter '*.json' -File | Sort-Object Name)) {
        $request = [IO.File]::ReadAllText($file.FullName, $utf8) | ConvertFrom-Json
        if ($request.status -ne 'pending') { continue }
        $id = [string]$request.client_id
        if ($request.attempt_id -ne $env:EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID -or
            $id -notmatch '^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$' -or -not $clients.ContainsKey($id) -or
            $clients[$id].process.Id -ne $request.expected_pid -or
            ([DateTimeOffset]$clients[$id].process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds() -ne $request.expected_started_unix_ms) {
            $request.status = 'rejected'
            $request | Add-Member -NotePropertyName message -NotePropertyValue 'Excel process or attempt has already changed.' -Force
            Write-ResidentJsonFile $file.FullName $request
            continue
        }
        $script:processingClientReset = $true
        try {
            $request.status = 'running'
            Write-ResidentJsonFile $file.FullName $request
            $headers = @{ 'X-Launcher-Token'=$launcherToken }
            $groupRestart = $false
            try {
                Invoke-RestMethod -Method Post -Uri ($controlUrl + '/launcher/reset/' + $id) -Headers $headers -ContentType 'application/json' -Body '{"stage":"prepare"}' -TimeoutSec 10 | Out-Null
            } catch {
                if ($null -ne $_.Exception.Response -and [int]$_.Exception.Response.StatusCode -eq 409) { $groupRestart = $true }
                else { throw }
            }
            if ($groupRestart) {
                # Publish the retry before killing a process whose evidence may
                # already be in use. The scheduler archives and invalidates it.
                $retry = [ordered]@{ id=[Guid]::NewGuid().ToString('N'); type='restart_group'; group_id=$request.group_id;
                    run_id=$request.run_id; attempt_id=$request.attempt_id; client_id=$id; reason=$request.reason;
                    status='pending'; requested_unix_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();
                    message='Hard reset requested after comparison started; requeueing the group.' }
                Write-ResidentJsonFile (Join-Path (Join-Path $liveRoot 'commands') ($retry.id + '.json')) $retry
            }
            [ResidentProcesses]::Stop($clients[$id].process)
            $clients[$id].closed = $true
            $metrics.clients[$id].closed = $true
            $metrics.clients[$id].status = 'closed'
            $metrics.clients[$id].stage = 'hard_reset'
            $metrics.clients[$id].process_alive = $false
            Add-SessionEvent 'client_hard_reset' $id @{ hide_excel=[bool]$request.hide_excel; group_restart=$groupRestart }
            Write-ResourceMetrics
            if ($groupRestart) {
                $request.status = 'group_restart'
                $request | Add-Member -NotePropertyName message -NotePropertyValue 'Excel stopped; group comparison will restart with current controls.' -Force
            } else {
                $guid = $clients[$id].registration
                Restore-AddIn $guid $registrations[$guid]
                $registrations.Remove($guid)
                $clients.Remove($id)
                Invoke-RestMethod -Method Post -Uri ($controlUrl + '/launcher/reset/' + $id) -Headers $headers -ContentType 'application/json' -Body '{"stage":"ready"}' -TimeoutSec 10 | Out-Null
                $spec = [IO.File]::ReadAllText((Join-Path (Join-Path $session 'clients') ($id + '.json')), $utf8) | ConvertFrom-Json
                Open-ResidentClient $spec ([bool]$request.hide_excel)
                $metrics.clients[$id].reset_from_pid = $request.expected_pid
                $request.status = 'complete'
                $request | Add-Member -NotePropertyName message -NotePropertyValue 'Excel restarted from the source file with the requested visibility.' -Force
            }
            Write-ResidentJsonFile $file.FullName $request
        } catch {
            $request.status = 'failed'
            $request | Add-Member -NotePropertyName message -NotePropertyValue $_.Exception.Message -Force
            Write-ResidentJsonFile $file.FullName $request
            throw
        } finally { $script:processingClientReset = $false }
    }
}

function Open-ResidentClient($spec, $visibilityOverride = $null) {
    $id = [string]$spec.client_id
    if ($id -notmatch '^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$') { throw 'Invalid resident client id.' }
    if ($clients.ContainsKey($id)) { return }
    if ($clients.Count -ge 16) { throw 'Resident session exceeded 16 clients.' }
    $source = [IO.Path]::GetFullPath([string]$spec.source_path)
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Resident workbook missing: $id" }
    if (-not @($spec.sheets).Count) { throw "Resident client has no planned sheets: $id" }
    Update-Resources
    $hideClient = Get-ResidentHideExcel
    if ($null -ne $visibilityOverride) { $hideClient = [bool]$visibilityOverride }
    $metrics.clients[$id] = [ordered]@{ status='requested'; stage='staging'; requested_at=[DateTime]::UtcNow.ToString('o'); pid=$null; process_started_unix_ms=$null; process_alive=$null; heartbeat_count=0; last_heartbeat_at=$null; last_activity_at=$null; hide_excel=[bool]$hideClient; startup_seconds=$null; opened_at=$null; peak_private_bytes=0; closed=$false }
    Write-ResourceMetrics
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
    $wait = [Diagnostics.Stopwatch]::StartNew()
    $otherBefore = $otherOpeningSeconds
    try {
        # Each group starts independently. An add-in that never registers must
        # not prevent other groups from launching their own Excel processes.
        if (Test-Path -LiteralPath (Join-Path $session 'finish.json')) { throw 'Resident session finished before workbook startup.' }
        Add-SessionEvent 'opening_started' $id $null
        $arguments = '/x ' + (Quote-Argument $book)
        # Freeze visibility for this process. Study groups use their immutable
        # admission setting; standalone sessions use the next-launch preference.
        $hideClient = Get-ResidentHideExcel
        if ($null -ne $visibilityOverride) { $hideClient = [bool]$visibilityOverride }
        $desktopName = $null
        if ($hideClient) {
            if ($null -eq $script:captureDesktop) { $script:captureDesktop = New-Object CaptureDesktop }
            $desktopName = $captureDesktop.Name
        }
        # Browser processes may be shared within a desktop, but a visible reset
        # must not attach to a browser running on this group's private desktop.
        $profileRoot = Join-Path $stage 'webview2-profile'
        $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $profileRoot $(if ($desktopName) { 'private' } else { 'interactive' })
        $metrics.clients[$id].webview2_user_data_folder = $env:WEBVIEW2_USER_DATA_FOLDER
        $openClock = [Diagnostics.Stopwatch]::StartNew()
        $process = $owned.Start($excelExecutable, $arguments, [bool]$hideClient, $desktopName)
        $clients[$id] = @{ process=$process; open_clock=$openClock; seen=$false; closed=$false; registration=$guid; hide_excel=[bool]$hideClient; private_desktop=($null -ne $desktopName) }
        $metrics.clients[$id].window_mode = if ($desktopName) { 'private_desktop' } elseif ($hideClient) { 'hidden_window' } else { 'interactive_desktop' }
        $metrics.clients[$id].pid = $process.Id
        $metrics.clients[$id].process_started_unix_ms = ([DateTimeOffset]$process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
        $metrics.clients[$id].process_alive = $true
        $metrics.clients[$id].hide_excel = [bool]$hideClient
        $metrics.clients[$id].status = 'opening'
        $metrics.clients[$id].stage = 'waiting_for_addin'
        $metrics.clients[$id].opened_at = [DateTime]::UtcNow.ToString('o')
        Write-ResourceMetrics
        while ($true) {
            Update-Resources
            $health = Read-HostHealth
            if ($clients[$id].seen) { break }
            if (Test-Path -LiteralPath (Join-Path $session 'finish.json')) { throw 'Resident session finished during workbook startup.' }
            if (Test-ResidentDeadline 'opening' $openClock.Elapsed.TotalSeconds) { throw "Resident Office add-in startup timeout: $id" }
            Start-Sleep -Milliseconds 200
        }
        $metrics.clients[$id].seen_at = [DateTime]::UtcNow.ToString('o')
        $metrics.clients[$id].status = 'ready'
        $metrics.clients[$id].stage = 'waiting_for_job'
        Write-ResourceMetrics
    } finally {
        Update-OtherGroupPause
        $script:openingSeconds += Get-OwnOpeningSeconds $wait.Elapsed.TotalSeconds $otherBefore $otherOpeningSeconds
    }
}

$stage = Join-Path ([IO.Path]::GetTempPath()) ('es-office-capture-' + [Guid]::NewGuid().ToString('N'))
& $archiveLauncher -StagePath $stage
[void][IO.Directory]::CreateDirectory($stage)
$clock = [Diagnostics.Stopwatch]::StartNew()
$metricsClock = [Diagnostics.Stopwatch]::StartNew()
$healthSampleClock = [Diagnostics.Stopwatch]::StartNew()
$openingSeconds = 0.0
$otherOpeningSeconds = 0.0
$pauseCounter = $null
$pauseBaselineSeconds = 0.0
$owned = $null
$captureDesktop = $null
$hostProcess = $null
$registrations = @{}
$clients = @{}
$batchLifetime = $null
$batchOwner = $null
$batchReleased = $false
$batchReleaseReason = $null
$finishWait = $null
$liveControlPath = $null
$liveGroupDirectory = $null
$liveOwner = $null
$liveOpenTimeoutSeconds = $StartupTimeoutSeconds
$liveHideExcel = [bool]$HideExcel
$launcherToken = [Guid]::NewGuid().ToString('N')
$processingClientReset = $false
$clientResetClock = [Diagnostics.Stopwatch]::StartNew()
$systemReserveBytes = [UInt64]1GB
$primaryClientIds = @()
$metrics = [ordered]@{ status='starting'; client_reset_supported=$true; attempt_id=$env:EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID; visible=[bool]$Visible; hide_excel=[bool]$HideExcel; memory_limit_mib=16384; system_reserve_mib=1024; system_reserve_bytes=$systemReserveBytes; minimum_available_bytes=[UInt64]::MaxValue; minimum_commit_bytes=[UInt64]::MaxValue; clients=@{}; expected_primary_clients=0; primary_clients_ready=$false }
try {
    Add-Type -Path @((Join-Path $PSScriptRoot 'ResidentProcesses.cs'), (Join-Path $PSScriptRoot 'CaptureProcesses.cs'), (Join-Path $PSScriptRoot 'CaptureDesktop.cs'))
    if ($env:EXCEL_SHRINK_CONTROL_ROOM_DIR) {
        $liveRoot = Get-ResidentControlRoot $env:EXCEL_SHRINK_CONTROL_ROOM_DIR
        $liveControlPath = Join-Path $liveRoot 'control.json'
        if ($env:EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID) {
            if ($env:EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID -notmatch '^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$') { throw 'Invalid control room group id.' }
            $liveGroupDirectory = Join-Path (Join-Path $liveRoot 'groups') $env:EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID
            [void][IO.Directory]::CreateDirectory($liveGroupDirectory)
        }
        if ($env:EXCEL_SHRINK_CONTROL_ROOM_OWNER_PID) {
            $liveOwner = [Diagnostics.Process]::GetProcessById([int]$env:EXCEL_SHRINK_CONTROL_ROOM_OWNER_PID)
            $null = $liveOwner.Handle
        }
    }
    $primaryClientIds = @(Get-PrimaryClientIds (Join-Path $session 'clients'))
    $metrics.expected_primary_clients = $primaryClientIds.Count
    Add-Type -Path (Join-Path $PSScriptRoot 'CaptureWorkbook.cs') -ReferencedAssemblies @('System.IO.Compression', 'System.IO.Compression.FileSystem', 'System.Xml')
    $systemReserveBytes = Get-SystemReserveBytes ([Environment]::GetEnvironmentVariable('EXCEL_SHRINK_SYSTEM_RESERVE_MIB'))
    $metrics.system_reserve_mib = $systemReserveBytes / 1MB
    $metrics.system_reserve_bytes = $systemReserveBytes
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
        if ($configured.pause_path -and -not [IO.Path]::IsPathRooted([string]$configured.pause_path)) {
            throw 'Resident batch pause path must be absolute.'
        }
        $batchLifetime = $configured
        Update-OtherGroupPause
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
    # Separate browser profiles prevent Office add-ins in independent jobs from
    # attaching to a browser process owned (and terminated) by another group.
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $stage 'webview2-profile'
    $metrics.webview2_user_data_folder = $env:WEBVIEW2_USER_DATA_FOLDER
    $owned = New-Object ResidentProcesses([int]$metrics.memory_limit_mib)
    Update-Resources
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
    [IO.File]::WriteAllText($configPath, (@{port=$port; web=$webStage; output=$hostOutput; session=$session; launcher_token=$launcherToken} | ConvertTo-Json), $utf8)
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
                $metrics.clients[$id].status = 'closed'
                $metrics.clients[$id].process_alive = $false
                $metrics.clients[$id].stage = 'closed'
                $guid = $clients[$id].registration
                Restore-AddIn $guid $registrations[$guid]
                $registrations.Remove($guid)
                Add-SessionEvent 'client_closed' $id $null
            }
        }
        # The live coordinator owns per-command timeouts and user pauses. A
        # fixed session budget would incorrectly expire a deliberately paused run.
        if ($null -ne $batchLifetime -and (Test-Path -LiteralPath ([string]$batchLifetime.ready_path))) {
            $reason = Wait-BatchRelease
            if ($reason -ne 'released') { throw "Resident batch cleanup released early: $reason" }
            if ($null -eq $finishWait) { $finishWait = [Diagnostics.Stopwatch]::StartNew() }
            if ($finishWait.Elapsed.TotalSeconds -gt 60) { throw 'Resident worker did not finish after its batch was released.' }
        } elseif (Test-ResidentDeadline 'session' ($clock.Elapsed.TotalSeconds - $openingSeconds - $otherOpeningSeconds)) {
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
        $metrics.cleanup_started_unix_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        try { $metrics.cleanup_processes = @($owned.ProcessIdentities()) }
        catch { $metrics.cleanup_snapshot_warning = $_.Exception.Message }
        Add-SessionEvent 'owned_cleanup_started' '' @{ processes=$metrics.cleanup_processes }
        $owned.Dispose()
        $metrics.cleanup_job_released_unix_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $unconfirmedClients = @(Confirm-ResidentClientExit $clients $metrics.clients)
        if ($unconfirmedClients.Count -eq 0) {
            $metrics.cleanup_completed_unix_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            Add-SessionEvent 'owned_cleanup_completed' '' $null
        } else {
            $metrics.cleanup_warning = 'Excel exit unconfirmed after job release: ' + ($unconfirmedClients -join ', ')
            Add-SessionEvent 'owned_cleanup_unconfirmed' '' @{ clients=$unconfirmedClients }
        }
        if ($liveGroupDirectory) {
            try {
                Write-ResidentJsonFile (Join-Path $liveGroupDirectory ('cleanup-' + $metrics.attempt_id + '.json')) @{
                    attempt_id=$metrics.attempt_id; launcher_pid=$PID
                    started_unix_ms=$metrics.cleanup_started_unix_ms
                    job_released_unix_ms=$metrics.cleanup_job_released_unix_ms
                    completed_unix_ms=$metrics.cleanup_completed_unix_ms
                    unconfirmed_clients=$unconfirmedClients
                    process_identities=$metrics.cleanup_processes
                    identity_format='pid|process name|UTC process start'
                }
            } catch { $metrics.cleanup_diagnostic_warning = $_.Exception.Message }
        }
        $metrics.primary_clients_ready = $false
        $metrics.current_job_bytes = $null
        $metrics.current_job_private_working_set_bytes = $null
    }
    foreach ($guid in @($registrations.Keys)) {
        try { Restore-AddIn $guid $registrations[$guid] } catch { $metrics.registry_cleanup_warning = $_.Exception.Message }
    }
    if ($null -ne $captureDesktop) { $captureDesktop.Dispose() }
    if ($null -ne $batchOwner) { $batchOwner.Dispose() }
    if ($null -ne $liveOwner) { $liveOwner.Dispose() }
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try { Remove-ArchiveStage $stage; break }
        catch { if ($attempt -eq 19) { $metrics.cleanup_warning = $_.Exception.Message } else { Start-Sleep -Milliseconds 250 } }
    }
    $metrics.elapsed_seconds = $clock.Elapsed.TotalSeconds
    $metrics.opening_seconds = $openingSeconds
    $metrics.other_group_opening_seconds = $otherOpeningSeconds
    Update-ClientStateTimes
    Write-SessionJson 'metrics.json' $metrics
    if ($liveGroupDirectory) { Write-ResidentJsonFile (Join-Path $liveGroupDirectory 'launcher.json') $metrics }
    Write-SessionJson 'launcher-result.json' @{ status=$metrics.status; error=$metrics.error; elapsed_seconds=$metrics.elapsed_seconds; peak_job_bytes=$metrics.peak_job_bytes }
}
if ($metrics.status -ne 'complete') { exit 1 }
