# Opening steps share the resident session's owned processes, metrics and controls.
# They preserve per-workbook staging, private desktops and add-in readiness semantics.

function Initialize-ResidentClientRequest {
    <# .SYNOPSIS
    Reserve capacity, validate a new client and publish its initial staging state.
    #>
    param($spec, $visibilityOverride)
    if ($null -eq $script:captureGroupSlot) {
        $script:captureGroupSlot = Enter-CaptureGroupSlot
        $specs = @(Get-ChildItem -LiteralPath (Join-Path $session 'clients') -File -Filter '*.json' | Sort-Object Name)
        $capacity = [IO.File]::ReadAllText((Join-Path $session 'capacity.json'), $utf8) | ConvertFrom-Json
        $script:reservedClientCount = [int]$capacity.reserved_clients
        if ($reservedClientCount -lt $specs.Count -or $reservedClientCount -gt 16) {
            throw 'Invalid resident client reservation.'
        }
    }
    $id = [string]$spec.client_id
    if ($id -notmatch '^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$') { throw 'Invalid resident client id.' }
    if ($clients.ContainsKey($id)) { return }
    if ($clients.Count -ge 16) { throw 'Resident session exceeded 16 clients.' }
    $specCount = @(Get-ChildItem -LiteralPath (Join-Path $session 'clients') -File -Filter '*.json').Count
    if ($specCount -gt $reservedClientCount) { throw 'Resident session exceeded reserved Excel slots.' }
    $source = [IO.Path]::GetFullPath([string]$spec.source_path)
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Resident workbook missing: $id" }
    if (-not @($spec.sheets).Count) { throw "Resident client has no planned sheets: $id" }
    Update-Resources
    $hideClient = Get-ResidentHideExcel
    if ($null -ne $visibilityOverride) { $hideClient = [bool]$visibilityOverride }
    $metrics.clients[$id] = [ordered]@{
        status='requested'
        stage='staging'
        requested_at=[DateTime]::UtcNow.ToString('o')
        pid=$null
        process_started_unix_ms=$null
        process_alive=$null
        heartbeat_count=0
        last_heartbeat_at=$null
        last_activity_at=$null
        hide_excel=[bool]$hideClient
        startup_seconds=$null
        opened_at=$null
        peak_private_bytes=0
        closed=$false
    }
    Write-ResourceMetrics
    return @{ id=$id; source=$source }
}

function Prepare-ResidentClientWorkbook {
    <# .SYNOPSIS
    Stage a separate workbook, inject its add-in and register its manifest.
    #>
    param($spec, $request)
    $id = $request.id
    $source = $request.source
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
        try { Set-InitialWorksheet $book ([int]$spec.sheets[0].index) $true ($env:EXCEL_SHRINK_CAPTURE_MANUAL_CALCULATION -eq '1'); break }
        catch { if ($attempt -eq 11) { throw }; Start-Sleep -Milliseconds 250 }
    }
    $registrations[$guid] = Register-AddIn $guid $manifestPath
    return @{ path=$book; registration=$guid }
}

function Start-ResidentClientExcel {
    <# .SYNOPSIS
    Start the owned Excel process and begin its launch-to-add-in-ready measurement.
    #>
    param([string]$id, $workbook, $visibilityOverride)
    # Each group starts independently. An add-in that never registers must
    # not prevent other groups from launching their own Excel processes.
    if (Test-Path -LiteralPath (Join-Path $session 'finish.json')) { throw 'Resident session finished before workbook startup.' }
    Add-SessionEvent 'opening_started' $id $null
    $arguments = '/x ' + (Quote-Argument $workbook.path)
    # Freeze visibility for this process. Study groups use their immutable
    # admission setting; standalone sessions use the next-launch preference.
    $hideClient = Get-ResidentHideExcel
    if ($null -ne $visibilityOverride) { $hideClient = [bool]$visibilityOverride }
    $desktopName = $null
    if ($hideClient) {
        if ($null -eq $script:captureDesktop) { $script:captureDesktop = New-Object CaptureDesktop }
        $desktopName = $captureDesktop.Name
    } elseif ($env:EXCEL_SHRINK_EXPECTED_DESKTOP) {
        # Targeted launchers own this desktop. Specify it explicitly: a helper
        # started through PowerShell must not redirect Excel to Default.
        if ($env:EXCEL_SHRINK_EXPECTED_DESKTOP -eq 'Default') { throw 'Expected a private capture desktop.' }
        $desktopName = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
    }
    # Browser processes may be shared within a desktop, but a visible reset
    # must not attach to a browser running on this group's private desktop.
    $profileRoot = Join-Path $stage 'webview2-profile'
    $metrics.clients[$id].webview_profile_policy = 'fresh-isolated'
    if ($env:EXCEL_SHRINK_RESIDENT_PROFILE_ROOT) {
        $profileRoot = [IO.Path]::GetFullPath($env:EXCEL_SHRINK_RESIDENT_PROFILE_ROOT)
        $metrics.clients[$id].webview_profile_policy = 'pooled-isolated'
    }
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $profileRoot $(if ($desktopName) { 'private' } else { 'interactive' })
    $metrics.clients[$id].webview2_user_data_folder = $env:WEBVIEW2_USER_DATA_FOLDER
    $openClock = [Diagnostics.Stopwatch]::StartNew()
    $process = $owned.Start($excelExecutable, $arguments, [bool]$hideClient, $desktopName)
    $clients[$id] = @{
        process=$process
        open_clock=$openClock
        seen=$false
        closed=$false
        registration=$workbook.registration
        error_checking_scope=$null
        desktop_name=$desktopName
        hide_excel=[bool]$hideClient
        private_desktop=($null -ne $desktopName)
    }
    $metrics.clients[$id].window_mode = if ($desktopName) { 'private_desktop' } elseif ($hideClient) { 'hidden_window' } else { 'interactive_desktop' }
    $metrics.clients[$id].desktop_name = $desktopName
    $metrics.clients[$id].pid = $process.Id
    $metrics.clients[$id].process_started_unix_ms = ([DateTimeOffset]$process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
    $metrics.clients[$id].process_alive = $true
    $metrics.clients[$id].hide_excel = [bool]$hideClient
    $metrics.clients[$id].status = 'opening'
    $metrics.clients[$id].stage = 'waiting_for_addin'
    $metrics.clients[$id].opened_at = [DateTime]::UtcNow.ToString('o')
    Write-ResourceMetrics
}

function Wait-ResidentClientAddIn {
    <# .SYNOPSIS
    Wait for the actual add-in registration and then publish the ready state.
    #>
    param([string]$id)
    while ($true) {
        Update-Resources
        $health = Read-HostHealth
        if ($clients[$id].seen) { break }
        if (Test-Path -LiteralPath (Join-Path $session 'finish.json')) { throw 'Resident session finished during workbook startup.' }
        if (Test-ResidentDeadline 'opening' $clients[$id].open_clock.Elapsed.TotalSeconds) { throw "Resident Office add-in startup timeout: $id" }
        Start-Sleep -Milliseconds 200
    }
    $metrics.clients[$id].seen_at = [DateTime]::UtcNow.ToString('o')
    $metrics.clients[$id].status = 'ready'
    $metrics.clients[$id].stage = 'waiting_for_job'
    Write-ResourceMetrics
}
