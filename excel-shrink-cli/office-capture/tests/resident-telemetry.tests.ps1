$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
Add-Type -Path (Join-Path $captureRoot 'ResidentProcesses.cs')
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $captureRoot 'resident.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident launcher has syntax errors.' }
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    if ($function.Name -in @('Get-PrimaryClientIds', 'Test-PrimaryClientsReady', 'Read-OtherGroupPauseMillis', 'Update-OtherGroupPause', 'Get-OwnOpeningSeconds')) {
        Invoke-Expression $function.Extent.Text
    }
}
function Assert($condition, [string]$message) { if (-not $condition) { throw $message } }
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)
$root = Join-Path ([IO.Path]::GetTempPath()) ('es-resident-telemetry-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$owned = New-Object ResidentProcesses(1024)
$child = $null
$self = [Diagnostics.Process]::GetCurrentProcess()
try {
    $specs = Join-Path $root 'clients'
    [void][IO.Directory]::CreateDirectory($specs)
    [IO.File]::WriteAllText((Join-Path $specs 'original.json'), '{"client_id":"original"}')
    [IO.File]::WriteAllText((Join-Path $specs 'default.json'), '{"client_id":"default"}')
    $primary = @(Get-PrimaryClientIds $specs)
    $entries = @{original=@{seen=$true;closed=$false;process=$self}}
    Assert (-not (Test-PrimaryClientsReady $primary $entries)) 'A partial startup was admitted.'
    $entries.default = @{seen=$false;closed=$false;process=$self}
    Assert (-not (Test-PrimaryClientsReady $primary $entries)) 'A not-yet-connected client was admitted.'
    $entries.default.seen = $true
    Assert (Test-PrimaryClientsReady $primary $entries) 'All connected primary clients should be ready.'
    [IO.File]::WriteAllText((Join-Path $specs 'control.json'), '{"client_id":"control"}')
    Assert ($primary.Count -eq 2) 'A later whitespace control changed the primary set.'
    $entries.default.closed = $true
    Assert (-not (Test-PrimaryClientsReady $primary $entries)) 'A closed primary client was admitted.'

    $pausePath = Join-Path $root 'pause.json'
    $batchLifetime = @{pause_path=$pausePath}
    $clock = @{Elapsed=@{TotalSeconds=10.0}}
    $script:pauseCounter = $null
    $script:pauseBaselineSeconds = 0.0
    $script:otherOpeningSeconds = 0.0
    [IO.File]::WriteAllText($pausePath, '{"elapsed_millis":5000}')
    Update-OtherGroupPause
    Assert ($otherOpeningSeconds -eq 0) 'Historical startup time was credited at launch.'
    $clock.Elapsed.TotalSeconds = 12.0
    [IO.File]::WriteAllText($pausePath, '{"elapsed_millis":6500}')
    Update-OtherGroupPause
    Assert ($otherOpeningSeconds -eq 1.5) 'Peer startup time was not discounted.'
    $clock.Elapsed.TotalSeconds = 13.0
    [IO.File]::WriteAllText($pausePath, '{"elapsed_millis":999999999}')
    Update-OtherGroupPause
    Assert ($otherOpeningSeconds -eq 3.0) "Pause credit exceeded elapsed wall time since the baseline: $otherOpeningSeconds (baseline $pauseBaselineSeconds)."
    foreach ($invalid in @('{', '{"elapsed_millis":-1}', '{"elapsed_millis":"300"}', '{"elapsed_millis":true}', '{"elapsed_millis":2.5}')) {
        [IO.File]::WriteAllText($pausePath, $invalid)
        Assert ($null -eq (Read-OtherGroupPauseMillis)) 'An invalid pause counter was accepted.'
        Update-OtherGroupPause
        Assert ($otherOpeningSeconds -eq 3.0) 'Invalid pause data added timeout credit.'
    }
    [IO.File]::WriteAllText($pausePath, '{"elapsed_millis":1}')
    Update-OtherGroupPause
    Assert ($otherOpeningSeconds -eq 3.0) 'Counter rollback added timeout credit.'
    $clock.Elapsed.TotalSeconds = 15.0
    [IO.File]::WriteAllText($pausePath, '{"elapsed_millis":999999999}')
    Update-OtherGroupPause
    Assert ($otherOpeningSeconds -eq 3.0) 'A replayed counter or discarded excess gained future credit.'
    $script:pauseCounter = $null
    $script:otherOpeningSeconds = 0.0
    foreach ($sample in @(@(0.0,0), @(0.2,0), @(0.4,250), @(0.6,250), @(0.8,500), @(1.0,750))) {
        $clock.Elapsed.TotalSeconds = $sample[0]
        [IO.File]::WriteAllText($pausePath, ('{"elapsed_millis":' + $sample[1] + '}'))
        Update-OtherGroupPause
    }
    Assert ($otherOpeningSeconds -eq 0.75) 'Different polling cadences lost legitimate startup-pause credit.'
    Assert ((Get-OwnOpeningSeconds 10 2 6) -eq 6) 'Own and peer startup overlap was counted twice.'
    Assert ((Get-OwnOpeningSeconds 2 2 6) -eq 0) 'Own startup credit became negative.'
    Assert ((Get-OwnOpeningSeconds 2.5 0.5 1.0) -eq 2.0) 'Fractional startup overlap was rounded.'

    # Native ownership test: the only explicitly assigned process starts its own
    # hidden child after assignment. That child's committed memory must count.
    $helper = Join-Path $root 'memory-child.ps1'
    @'
param([string]$Root, [switch]$Grandchild)
$ErrorActionPreference = 'Stop'
if ($Grandchild) {
    $allocation = New-Object byte[] (128MB)
    for ($offset=0; $offset -lt $allocation.Length; $offset+=4096) { $allocation[$offset]=1 }
    [IO.File]::WriteAllText((Join-Path $Root 'allocated'), [string]$PID)
    while (-not (Test-Path -LiteralPath (Join-Path $Root 'release'))) { Start-Sleep -Milliseconds 100 }
    exit
}
while (-not (Test-Path -LiteralPath (Join-Path $Root 'assigned'))) { Start-Sleep -Milliseconds 100 }
$arguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -Root "{1}" -Grandchild' -f $PSCommandPath, $Root
$descendant = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList $arguments -WindowStyle Hidden -PassThru
$descendant.WaitForExit()
[IO.File]::WriteAllText((Join-Path $Root 'released'), 'done')
while (-not (Test-Path -LiteralPath (Join-Path $Root 'finish'))) { Start-Sleep -Milliseconds 100 }
'@ | Set-Content -LiteralPath $helper -Encoding UTF8
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -Root "{1}"' -f $helper, $root
    $child = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList $arguments -WindowStyle Hidden -PassThru
    $owned.Add($child)
    [IO.File]::WriteAllText((Join-Path $root 'assigned'), 'go')
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not (Test-Path -LiteralPath (Join-Path $root 'allocated'))) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Descendant memory allocation timed out.' }
        Start-Sleep -Milliseconds 100
    }
    $child.Refresh()
    $measured = $owned.SampleMemory()
    $current = $measured.PrivateBytes
    $grandchildPid = [int][IO.File]::ReadAllText((Join-Path $root 'allocated'))
    Assert (($measured.Processes.Pid -contains $child.Id) -and ($measured.Processes.Pid -contains $grandchildPid)) 'Resource snapshot omitted an owned process.'
    Assert (($measured.Processes | Where-Object { $_.PrivateBytes -gt 100MB -and $_.StartedUnixMilliseconds -gt 0 }).Count -ge 1) 'Process samples lack private memory or identity.'
    Assert ($current -gt $child.PrivateMemorySize64 + 100MB) 'Current job memory omitted the descendant.'
    if ($null -ne $measured.PrivateWorkingSetBytes) {
        Assert ($measured.PrivateWorkingSetBytes -gt 100MB) 'Private working set omitted the resident descendant allocation.'
        Assert ($measured.PrivateWorkingSetBytes -le $current) 'Private working set exceeds committed memory.'
    }
    $peak = $owned.PeakBytes
    Assert ($peak -gt 100MB) 'The job did not retain its memory peak.'
    [IO.File]::WriteAllText((Join-Path $root 'release'), 'go')
    while (-not (Test-Path -LiteralPath (Join-Path $root 'released'))) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Descendant did not exit.' }
        Start-Sleep -Milliseconds 100
    }
    $released = $owned.SampleMemory()
    $after = $released.PrivateBytes
    Assert ($after + 100MB -lt $current) 'Current job memory retained an exited child or reported the peak.'
    Assert ($owned.PeakBytes -ge $peak) 'The job peak disappeared after the descendant exited.'
    if ($null -ne $measured.PrivateWorkingSetBytes -and $null -ne $released.PrivateWorkingSetBytes) {
        Assert ($released.PrivateWorkingSetBytes + 100MB -lt $measured.PrivateWorkingSetBytes) 'Private working set retained an exited descendant.'
    }
    Write-Host ('Native private working set available: ' + ($null -ne $measured.PrivateWorkingSetBytes))
    $owned.Dispose()
    Assert ($child.WaitForExit(5000)) 'Owned process survived job cleanup.'
    Write-Host 'PASS: primary readiness, bounded peer-startup pause, descendant current memory, retained peak, scoped cleanup.'
} finally {
    $owned.Dispose()
    if ($null -ne $child) { $child.Dispose() }
    $self.Dispose()
    $resolved = [IO.Path]::GetFullPath($root)
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notlike 'es-resident-telemetry-*') {
        throw 'Unexpected telemetry-test cleanup path.'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
