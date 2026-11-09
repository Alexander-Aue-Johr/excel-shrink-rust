param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$CaptureRoot,
    [Parameter(Mandatory=$true)][string]$Seed,
    [Parameter(Mandatory=$true)][string]$Original,
    [Parameter(Mandatory=$true)][string]$Candidate,
    [Parameter(Mandatory=$true)][string]$Output,
    [switch]$ResidentComparisons,
    [switch]$NativeChartControls,
    [switch]$DynamicScaling
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $Output) { throw 'Choose a new pool probe directory.' }
[void][IO.Directory]::CreateDirectory($Output)
$utf8 = [Text.UTF8Encoding]::new($false)
Add-Type -Path @((Join-Path $CaptureRoot 'CaptureProcesses.cs'), (Join-Path $CaptureRoot 'CaptureDesktop.cs'))
Add-Type -AssemblyName System.Net.Http
Import-Module (Join-Path $CaptureRoot 'CaptureSetup.psm1') -DisableNameChecking
$owned = New-Object CaptureProcesses
$desktop = New-Object CaptureDesktop
$http = New-Object Net.Http.HttpClient
$http.Timeout = [TimeSpan]::FromSeconds(120)
$excelProcesses = @()
$poolProcess = $null
$owner = [Diagnostics.Process]::GetCurrentProcess()
$token = [Guid]::NewGuid().ToString()
$headers = @{'X-Pool-Token'=$token}
$hashes = @((Get-FileHash -LiteralPath $Original).Hash, (Get-FileHash -LiteralPath $Candidate).Hash)

function Write-PoolProbeJson([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 12), $utf8)
}

function Owner-Request($Process) {
    return @{pid=$Process.Id;started_ms=([DateTimeOffset]$Process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds();comparison_id=[Guid]::NewGuid().ToString()}
}

function Pool-Request([string]$Path, $Body, $Lease) {
    $requestHeaders = @{'X-Pool-Token'=$token}
    if ($Lease) { $requestHeaders['X-Lease-Token'] = $Lease.lease_token }
    $bytes = $utf8.GetBytes(($Body | ConvertTo-Json -Compress))
    return Invoke-RestMethod -Method Post ($url + $Path) -Headers $requestHeaders -ContentType 'application/json; charset=utf-8' -Body $bytes -TimeoutSec 120
}

function Pool-Command($Lease, [string]$Kind, [string]$Path) {
    $id = [Guid]::NewGuid().ToString()
    $reply = Pool-Request ('/command/' + $Lease.slot) @{id=$id;kind=$Kind;path=$Path} $Lease
    if (-not $reply.ok -or $reply.id -ne $id -or $reply.pid -ne $Lease.process.pid) { throw 'Pool command identity or status differs.' }
}

function Expect-PoolStatus([scriptblock]$Action, [int]$Status) {
    try { & $Action | Out-Null }
    catch [Net.WebException] {
        if ([int]$_.Exception.Response.StatusCode -ne $Status) { throw }
        return
    }
    throw 'Pool request unexpectedly succeeded.'
}

function Remember-Excel($Lease) {
    $process = [Diagnostics.Process]::GetProcessById([int]$Lease.process.pid)
    if ($process.StartTime.ToUniversalTime().Ticks -ne ([DateTimeOffset]::Parse($Lease.process.started_utc)).UtcTicks) {
        throw 'Pool Excel birth identity differs.'
    }
    $script:excelProcesses += $process
}

try {
    $settings = Join-Path $Output 'settings.json'
    $readyPath = Join-Path $Output 'ready.json'
    $poolSettings = @{
        worker=@{seed=$Seed;excel_executable=(Find-ExcelExecutable);capture_root=$CaptureRoot;
            profile_root=(Join-Path $Output 'workers');timeout_seconds=60}
        workers=2;token=$token;port=0;ready_path=$readyPath
    }
    $controlPath = Join-Path $Output 'control.json'
    if ($DynamicScaling) {
        Write-PoolProbeJson $controlPath @{target_groups=1}
        $poolSettings.control_path = $controlPath
    }
    Write-PoolProbeJson $settings $poolSettings
    $poolProcess = $desktop.Start($Binary, ('capture-host workbook-pool ' + (Quote-Argument $settings)), $owned)
    $startup = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $readyPath)) {
        if ($poolProcess.HasExited -or $startup.Elapsed.TotalSeconds -gt 120) { throw 'Pool startup failed or timed out.' }
        Start-Sleep -Milliseconds 100
    }
    $ready = [IO.File]::ReadAllText($readyPath) | ConvertFrom-Json
    $startupSeconds = $startup.Elapsed.TotalSeconds
    $url = $ready.url
    if ($ResidentComparisons) {
        $connection = Join-Path $Output 'connection.json'
        Write-PoolProbeJson $connection @{url=$url;token=$token}
        $previousConnection = $env:EXCEL_SHRINK_WORKBOOK_POOL_CONNECTION
        $previousAssets = $env:EXCEL_SHRINK_CAPTURE_DIR
        try {
            $env:EXCEL_SHRINK_WORKBOOK_POOL_CONNECTION = $connection
            $env:EXCEL_SHRINK_CAPTURE_DIR = $CaptureRoot
            $residentPids = @()
            for ($visit = 0; $visit -lt 2; $visit++) {
                $comparison = Join-Path $Output ('comparison-' + $visit)
                $group = Join-Path $Output ('group-' + $visit)
                [void][IO.Directory]::CreateDirectory($group)
                $arguments = @('-NoProfile','-ExecutionPolicy','Bypass','-File',
                    (Join-Path $CaptureRoot 'run-targeted-comparison.ps1'), '-Binary',$Binary,
                    '-Original',$Original,'-Default',$Candidate,'-Output',$comparison,
                    '-ObservatoryGroup',$group,'-SkipCheckPerformance',
                    '-OperationTimeoutSeconds','60','-CaptureIdleTimeoutSeconds','60')
                $commandLine = ($arguments | ForEach-Object { Quote-Argument ([string]$_) }) -join ' '
                $comparisonProcess = $desktop.Start((Join-Path $PSHOME 'powershell.exe'), $commandLine, $owned)
                if (-not $comparisonProcess.WaitForExit(180000)) { throw 'Pooled resident comparison timed out.' }
                if ($comparisonProcess.ExitCode -ne 0) { throw ('Pooled resident comparison failed: ' + $comparison) }
                $report = [IO.File]::ReadAllText((Join-Path $comparison 'report.json')) | ConvertFrom-Json
                if ($report.status -ne 'completed_with_limitations') { throw 'Pooled comparison did not complete.' }
                $metrics = @(Get-ChildItem -LiteralPath $comparison -Filter metrics.json -File -Recurse | ForEach-Object {
                    [IO.File]::ReadAllText($_.FullName) | ConvertFrom-Json
                } | Where-Object { $_.worker_pool })
                if ($metrics.Count -ne 2 -or @($metrics | Where-Object { $_.pool_lease_released -ne $true }).Count) {
                    throw 'Pooled residents did not confirm two released leases.'
                }
                $pids = @($metrics | ForEach-Object { [int]$_.worker_pool.process.pid } | Sort-Object)
                $residentPids += ,$pids
                foreach ($metric in $metrics) { Remember-Excel @{process=$metric.worker_pool.process} }
            }
            if (($residentPids[0] -join ',') -ne ($residentPids[1] -join ',')) { throw 'Resident comparisons did not reuse the same workers.' }
            Write-PoolProbeJson (Join-Path $Output 'resident-result.json') @{visits=2;worker_pids=$residentPids;warm_reuse=$true}
        } finally {
            $env:EXCEL_SHRINK_WORKBOOK_POOL_CONNECTION = $previousConnection
            $env:EXCEL_SHRINK_CAPTURE_DIR = $previousAssets
        }
    }
    $firstOwner = Owner-Request $owner
    $first = Pool-Request '/acquire' $firstOwner $null
    $repeated = Pool-Request '/acquire' $firstOwner $null
    if ($first.slot -ne $repeated.slot -or $first.lease_token -ne $repeated.lease_token) { throw 'Acquire retry allocated another worker.' }
    $second = Pool-Request '/acquire' (Owner-Request $owner) $null
    Remember-Excel $first
    Remember-Excel $second
    if ($first.slot -eq $second.slot -or $first.capture_port -eq $second.capture_port -or
        $first.process.pid -eq $second.process.pid) { throw 'Pool workers were not independent.' }
    if (-not $DynamicScaling) {
        Expect-PoolStatus { Pool-Request '/acquire' (Owner-Request $owner) $null } 409
    }

    # Submit both opens before waiting for either response.
    $pending = @()
    $workingCopy = Join-Path $Output 'working-copy.xlsx'
    Copy-Item -LiteralPath $Original -Destination $workingCopy
    foreach ($job in @(@{lease=$first;path=$workingCopy;kind='open_working_copy'}, @{lease=$second;path=$Candidate;kind='open'})) {
        $id = [Guid]::NewGuid().ToString()
        $request = New-Object Net.Http.HttpRequestMessage -ArgumentList @([Net.Http.HttpMethod]::Post, ($url + '/command/' + $job.lease.slot))
        [void]$request.Headers.TryAddWithoutValidation('X-Pool-Token', $token)
        [void]$request.Headers.TryAddWithoutValidation('X-Lease-Token', $job.lease.lease_token)
        $request.Content = New-Object Net.Http.StringContent -ArgumentList @((@{id=$id;kind=$job.kind;path=$job.path} | ConvertTo-Json -Compress), $utf8, 'application/json')
        $pending += @{task=$http.SendAsync($request);request=$request;id=$id;lease=$job.lease}
    }
    foreach ($job in $pending) {
        if (-not $job.task.Wait(120000)) { throw 'Concurrent workbook open exceeded its deadline.' }
        $response = $job.task.Result
        try {
            $reply = $response.Content.ReadAsStringAsync().Result | ConvertFrom-Json
            if (-not $response.IsSuccessStatusCode -or -not $reply.ok -or $reply.id -ne $job.id) { throw 'Concurrent open failed.' }
        } finally { $response.Dispose(); $job.request.Dispose() }
    }
    if ($NativeChartControls) {
        $embedded = @{sheet='Sheet1';chart='Chart 1'}
        if ($ResidentComparisons) {
            $embedded = @($report.comparisons | ForEach-Object { $_.charts } | Where-Object { $_.kind -eq 'embedded' })[0]
        }
        if (-not $embedded) { throw 'Native chart control requires an embedded chart.' }
        $previousHash = $null
        for ($export = 0; $export -lt 2; $export++) {
            $image = Join-Path $Output ('native-embedded-' + $export + '.png')
            $id = [Guid]::NewGuid().ToString()
            $reply = Pool-Request ('/command/' + $first.slot) @{
                id=$id;kind='export_chart';path=$image;chart_kind='embedded'
                sheet=$embedded.sheet;chart=$embedded.chart;chart_index=0
            } $first
            Write-PoolProbeJson (Join-Path $Output ('native-chart-response-' + $export + '.json')) $reply
            if (-not $reply.ok -or $reply.id -ne $id -or $reply.result.path -ne $image -or
                $reply.result.geometry.width -le 0 -or $reply.result.geometry.height -le 0) {
                throw 'Native embedded chart export returned invalid evidence.'
            }
            $hash = (Get-FileHash -LiteralPath $image).Hash
            if ($previousHash -and $hash -ne $previousHash) { throw 'Repeated native embedded chart export changed pixels.' }
            $previousHash = $hash
        }
    }
    Pool-Command $first 'save_copy' (Join-Path $Output 'first-before.xlsx')
    Pool-Command $first 'rebuild' ''
    Pool-Command $first 'save_copy' (Join-Path $Output 'first-after.xlsx')
    Pool-Command $first 'save_as' (Join-Path $Output 'first-save-as.xlsx')
    Pool-Command $first 'save_copy' (Join-Path $Output 'first-after-save-as.xlsx')
    Expect-PoolStatus { Pool-Request ('/release/' + $first.slot) @{} $first } 409
    Pool-Command $first 'close' ''
    $origin = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, [int]$first.capture_port)
    $origin.Start()
    try { Expect-PoolStatus { Pool-Request ('/release/' + $first.slot) @{} $first } 409 }
    finally { $origin.Stop() }
    Pool-Request ('/release/' + $first.slot) @{} $first | Out-Null
    Pool-Request ('/release/' + $first.slot) @{} $first | Out-Null
    $reused = Pool-Request '/acquire' (Owner-Request $owner) $null
    if ($reused.process.pid -ne $first.process.pid -or $reused.process.started_utc -ne $first.process.started_utc) { throw 'Warm process was not reused.' }
    Expect-PoolStatus { Pool-Command $first 'close' '' } 403
    Pool-Request ('/release/' + $first.slot) @{} $first | Out-Null
    if (-not $DynamicScaling) {
        Expect-PoolStatus { Pool-Request '/acquire' (Owner-Request $owner) $null } 409
    }
    Pool-Request ('/release/' + $reused.slot) @{} $reused | Out-Null

    $helper = $desktop.Start((Join-Path $PSHOME 'powershell.exe'), '-NoProfile -Command "Start-Sleep -Seconds 60"', $owned)
    $abandoned = Pool-Request '/acquire' (Owner-Request $helper) $null
    Pool-Command $abandoned 'open' $Original
    Remember-Excel $abandoned
    $abandonedProcess = $excelProcesses[-1]
    if (-not [CaptureProcesses]::Stop($helper)) { throw 'Probe owner exit unconfirmed.' }
    if (-not $abandonedProcess.WaitForExit(10000)) { throw 'Abandoned workbook worker remained alive.' }
    Pool-Command $second 'save_copy' (Join-Path $Output 'survivor.xlsx')
    $replacement = $null
    $replacementClock = [Diagnostics.Stopwatch]::StartNew()
    while (-not $replacement -and $replacementClock.Elapsed.TotalSeconds -lt 15) {
        try { $replacement = Pool-Request '/acquire' (Owner-Request $owner) $null }
        catch [Net.WebException] {
            if ([int]$_.Exception.Response.StatusCode -ne 409) { throw }
            Start-Sleep -Milliseconds 100
        }
    }
    if (-not $replacement) { throw 'Retired worker was not replaceable.' }
    Remember-Excel $replacement
    if ($replacement.process.pid -eq $first.process.pid -and $replacement.process.started_utc -eq $first.process.started_utc) {
        throw 'Abandoned worker identity was reused.'
    }
    Pool-Request ('/release/' + $replacement.slot) @{} $replacement | Out-Null
    Pool-Command $second 'close' ''
    Pool-Request ('/release/' + $second.slot) @{} $second | Out-Null
    if ($DynamicScaling) {
        $inflight = Pool-Request '/acquire' (Owner-Request $owner) $null
        Remember-Excel $inflight
        Pool-Command $inflight 'open' $Original
        Write-PoolProbeJson $controlPath @{target_groups=0}
        Start-Sleep -Seconds 2
        $peer = Pool-Request '/acquire' (Owner-Request $owner) $null
        Remember-Excel $peer
        Pool-Command $inflight 'rebuild' ''
        Pool-Command $inflight 'save_copy' (Join-Path $Output 'paused-inflight.xlsx')
        Pool-Command $inflight 'close' ''
        Pool-Request ('/release/' + $inflight.slot) @{} $inflight | Out-Null
        Pool-Request ('/release/' + $peer.slot) @{} $peer | Out-Null
        Write-PoolProbeJson $controlPath @{target_groups=2}
        Start-Sleep -Seconds 2
        $expanded = @()
        for ($index = 0; $index -lt 4; $index++) {
            $lease = Pool-Request '/acquire' (Owner-Request $owner) $null
            Remember-Excel $lease
            $expanded += $lease
            Pool-Command $lease 'open' $Original
            Pool-Command $lease 'rebuild' ''
            Pool-Command $lease 'save_copy' (Join-Path $Output ("scaled-$index.xlsx"))
        }
        if (@($expanded.process.pid | Select-Object -Unique).Count -ne 4) { throw 'Expansion did not provide four exclusive workers.' }
        Write-PoolProbeJson $controlPath @{target_groups=1}
        Start-Sleep -Seconds 2
        foreach ($lease in $expanded) {
            Pool-Command $lease 'save_copy' (Join-Path $Output ("draining-$($lease.slot).xlsx"))
            Pool-Command $lease 'close' ''
            Pool-Request ('/release/' + $lease.slot) @{} $lease | Out-Null
        }
        foreach ($lease in @($expanded | Where-Object { $_.slot -ge 2 })) {
            $process = $excelProcesses | Where-Object { $_.Id -eq $lease.process.pid } | Select-Object -Last 1
            if (-not $process.WaitForExit(10000)) { throw 'Excess worker did not retire after release.' }
        }
        foreach ($lease in @($expanded | Where-Object { $_.slot -lt 2 })) {
            $process = $excelProcesses | Where-Object { $_.Id -eq $lease.process.pid } | Select-Object -Last 1
            if ($process.HasExited) { throw 'Reduction stopped a retained worker.' }
        }
    }
    Write-PoolProbeJson (Join-Path $Output 'result.json') @{
        startup_seconds=$startupSeconds;probe_seconds=$startup.Elapsed.TotalSeconds
        initial_excel_pids=@($first.process.pid,$second.process.pid)
        replacement_pid=$replacement.process.pid;exclusive_leases=$true;stale_token_rejected=$true
        capture_origin_guard=$true;warm_reuse=$true;owner_death_retired=$true;peer_survived=$true
        acquire_retry_reused=$true;release_retry_safe=$true
        save_as_identity_verified=$true
        writable_staged_copy=$true
        native_chart_controls=[bool]$NativeChartControls
        dynamic_scaling=[bool]$DynamicScaling
    }
} finally {
    $http.Dispose()
    $owned.Dispose()
    if ($poolProcess -and -not $poolProcess.WaitForExit(10000)) { throw 'Pool process exit unconfirmed.' }
    foreach ($process in $excelProcesses) {
        if (-not $process.WaitForExit(10000)) { throw 'Pool Excel process exit unconfirmed.' }
    }
    $desktop.Dispose()
    $owner.Dispose()
}
if ($hashes[0] -ne (Get-FileHash -LiteralPath $Original).Hash -or $hashes[1] -ne (Get-FileHash -LiteralPath $Candidate).Hash) {
    throw 'Pool probe source changed.'
}
Write-PoolProbeJson (Join-Path $Output 'cleanup.json') @{pool_exit_confirmed=$true;excel_exits_confirmed=$true;source_hashes_unchanged=$true}
