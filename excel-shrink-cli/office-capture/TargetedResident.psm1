# Transport to the existing native launcher and Office.js command protocol.
$ErrorActionPreference = 'Stop'
$sessions = @{}
$script:responseWatcher = $null
$utf8 = [Text.UTF8Encoding]::new($false)
Import-Module (Join-Path $PSScriptRoot 'TargetedChartSheets.psm1') -Force -DisableNameChecking

function Write-TargetedJson {
    param([string]$Path, $Value)
    $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))
    $pending = $Path + '.pending'
    [IO.File]::WriteAllText($pending, ($Value | ConvertTo-Json -Depth 12 -Compress), $utf8)
    Move-Item -LiteralPath $pending -Destination $Path -Force
}

function Read-TargetedJson {
    param([string]$Path)
    if ([IO.File]::Exists($Path)) {
        try { return [IO.File]::ReadAllText($Path) | ConvertFrom-Json }
        catch [IO.IOException] { return $null } # Publisher may briefly hold or replace the snapshot.
    }
    return $null
}

function Open-TargetedWorkbook {
    <# .SYNOPSIS
    Starts one native Excel/add-in client; all measurements use this same workbook.
    #>
    param($Command, $Owned)
    if ($sessions.ContainsKey($Command.lane)) { throw 'Targeted lane is already open.' }
    if (-not $env:EXCEL_SHRINK_EXPECTED_DESKTOP -or $env:EXCEL_SHRINK_EXPECTED_DESKTOP -eq 'Default') {
        throw 'Targeted comparisons require their private-desktop launcher.'
    }
    $root = Join-Path ([IO.Path]::GetDirectoryName($Command.path)) ('resident-' + $Command.lane)
    $session = Join-Path $root 'session'
    $control = Join-Path $root 'control'
    Write-TargetedJson (Join-Path $control 'control.json') @{paused=$false; hide_excel=$false}
    Write-TargetedJson (Join-Path $session 'capacity.json') @{reserved_clients=1}
    Write-TargetedJson (Join-Path $session 'clients/one.json') @{
        client_id='one'; source_path=$Command.path; workbook='one.xlsx'; sheets=$Command.sheets
    }
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -SessionPath "{1}" -HostExecutable "{2}" -StartupTimeoutSeconds {3} -CaptureTimeoutSeconds {3} -Visible' -f `
        (Join-Path $PSScriptRoot 'resident.ps1'), $session, $Command.host_executable, $Command.timeout_seconds
    # A manual recheck must not inherit a paused corpus or its visibility settings.
    $environment = @{
        EXCEL_SHRINK_CONTROL_ROOM_DIR=$control
        EXCEL_SHRINK_CONTROL_ROOM_GROUP_ID=('targeted-' + $Command.lane)
        EXCEL_SHRINK_CONTROL_ROOM_ATTEMPT_ID=[Guid]::NewGuid().ToString('N')
        EXCEL_SHRINK_CONTROL_ROOM_OWNER_PID=[string]$PID
        # This launcher already owns a private desktop. An inherited hide flag
        # would create a second desktop, outside the native adapter's scope.
        EXCEL_SHRINK_HIDE_EXCEL='0'
    }
    $previous = @{}
    try {
        foreach ($key in $environment.Keys) {
            $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
            [Environment]::SetEnvironmentVariable($key, $environment[$key], 'Process')
        }
        $launcher = Start-Process powershell.exe -ArgumentList $arguments -WindowStyle Hidden -PassThru `
            -RedirectStandardOutput (Join-Path $root 'stdout.log') -RedirectStandardError (Join-Path $root 'stderr.log')
    } finally {
        foreach ($key in $previous.Keys) { [Environment]::SetEnvironmentVariable($key, $previous[$key], 'Process') }
    }
    $Owned.Add($launcher)
    $entry = @{launcher=$launcher; session=$session; path=$Command.path; process=$null; sequence=0}
    $sessions[$Command.lane] = $entry
    while (-not $launcher.HasExited) {
        $metrics = Read-TargetedJson (Join-Path $session 'metrics.json')
        if ($null -ne $metrics -and $metrics.clients.one.pid -and $null -eq $entry.process) {
            $entry.process = [Diagnostics.Process]::GetProcessById($metrics.clients.one.pid)
            $started = ([DateTimeOffset]$entry.process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
            if ([Math]::Abs($started - $metrics.clients.one.process_started_unix_ms) -gt 2) { throw 'Excel PID identity changed.' }
            [Console]::WriteLine((@{event='process_started'; lane=$Command.lane; pid=$entry.process.Id; started_at=$started} | ConvertTo-Json -Compress))
        }
        if ($null -ne $metrics -and $metrics.primary_clients_ready) {
            $configuration = Read-TargetedJson (Join-Path $session 'launcher.json')
            $entry.staged = Join-Path $configuration.stage 'one.xlsx'
            return @{pid=$entry.process.Id}
        }
        Start-Sleep -Milliseconds 100
    }
    throw "Targeted resident exited during opening; inspect $root/stderr.log"
}

function Start-TargetedJob {
    <# .SYNOPSIS
    Publishes one command without waiting for its client, retaining its response identity.
    #>
    param($Command, [int]$Index)
    $entry = $sessions[$Command.lane]
    if ($null -eq $entry) { throw 'Targeted workbook is not open.' }
    $entry.sequence++
    $id = 'targeted-' + $entry.sequence
    Write-TargetedJson (Join-Path $entry.session "commands/one/$id.json") @{
        id=$id; kind='diagnosis'; operation='targeted'; request=$Command
    }
    return @{command=$Command; entry=$entry; index=$Index; response=(Join-Path $entry.session "responses/one/$id.json")}
}

function Resolve-TargetedResponse {
    <# .SYNOPSIS
    Validates a completed response and adopts only the image owned by this session.
    #>
    param($Job, $Response)
    $command = $Job.command
    $entry = $Job.entry
    if ($Response.ok -ne $true -or $Response.diagnostic_error) {
        throw ($Response | ConvertTo-Json -Depth 6 -Compress)
    }
    if ($command.kind -eq 'charts') {
        $Response.charts = @($Response.charts) + @(Get-TargetedChartSheets $entry)
    }
    if ($command.kind -in @('save','recalculate')) {
        Copy-Item -LiteralPath $entry.staged -Destination $entry.path -Force
        Remove-TargetedTaskpane $entry
    }
    if ($command.kind -in @('capture','capture_chart')) {
        if ($Response.non_rendering -eq $true) {
            return @{non_rendering=$true; reason=$Response.reason; geometry=$Response.geometry}
        }
        $image = [IO.Path]::GetFullPath([string]$Response.image_path)
        $ownedImages = [IO.Path]::GetFullPath((Join-Path $entry.session 'diagnosis-images/one'))
        if (-not $image.StartsWith($ownedImages + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Image receipt is outside this resident session.'
        }
        Move-Item -LiteralPath $image -Destination $command.output -ErrorAction Stop
        return @{path=$command.output; geometry=$Response.geometry}
    }
    return $Response
}

function Invoke-TargetedJobs {
    <# .SYNOPSIS
    Dispatches distinct lanes together and collects responses in request order.
    File notifications wake the waiter; bounded rescans also cover missed notifications.
    #>
    param([object[]]$Commands)
    if ($Commands.Count -lt 1 -or $Commands.Count -gt 6) { throw 'Invalid targeted batch size.' }
    $lanes = New-Object 'Collections.Generic.HashSet[string]'
    foreach ($command in $Commands) {
        if (-not $lanes.Add([string]$command.lane)) { throw 'A batch may contain only one job per Excel lane.' }
        if (-not $sessions.ContainsKey($command.lane)) { throw 'Targeted workbook is not open.' }
    }
    $root = [IO.Path]::GetDirectoryName($sessions[$Commands[0].lane].path)
    if ($null -eq $script:responseWatcher) {
        $script:responseWatcher = New-Object IO.FileSystemWatcher $root, 'targeted-*.json'
        $script:responseWatcher.IncludeSubdirectories = $true
        $script:responseWatcher.EnableRaisingEvents = $true
    } elseif ($script:responseWatcher.Path -ne $root) {
        throw 'A batch cannot span different comparison directories.'
    }
    $pending = New-Object 'Collections.Generic.List[object]'
    $results = New-Object object[] $Commands.Count
    for ($index = 0; $index -lt $Commands.Count; $index++) {
        $pending.Add((Start-TargetedJob $Commands[$index] $index))
    }
    while ($pending.Count -gt 0) {
        for ($index = $pending.Count - 1; $index -ge 0; $index--) {
            $job = $pending[$index]
            if ($job.entry.launcher.HasExited) { throw 'Targeted resident exited without a response.' }
            $response = Read-TargetedJson $job.response
            if ($null -eq $response) { continue }
            $results[$job.index] = Resolve-TargetedResponse $job $response
            $pending.RemoveAt($index)
        }
        if ($pending.Count -gt 0) {
            $null = $script:responseWatcher.WaitForChanged([IO.WatcherChangeTypes]::All, 100)
        }
    }
    return $results
}

function Invoke-TargetedJob {
    <# .SYNOPSIS
    Uses the same response protocol for single commands and native chart-sheet exports.
    #>
    param($Command)
    $entry = $sessions[$Command.lane]
    if ($null -eq $entry) { throw 'Targeted workbook is not open.' }
    if ($Command.kind -eq 'capture_chart' -and $Command.chart_kind -eq 'chart_sheet') {
        return Export-TargetedChartSheet $entry $Command
    }
    $results = @(Invoke-TargetedJobs @($Command))
    return $results[0]
}

function Invoke-TargetedCaptureBatch {
    <# .SYNOPSIS
    Allows only cell captures in a parallel batch; mutations remain explicit ordered phases.
    #>
    param($Command)
    foreach ($job in $Command.jobs) {
        if ($job.kind -ne 'capture') { throw 'A capture batch may not mutate workbooks.' }
    }
    return @{images=@(Invoke-TargetedJobs $Command.jobs)}
}

function Remove-TargetedTaskpane {
    <# .SYNOPSIS
    Removes only this resident's injected add-in from an exported working copy.
    .DESCRIPTION
    The live workbook keeps its add-in. Rebuild copies must be clean inputs to
    the launcher, which deliberately refuses pre-existing taskpanes.
    #>
    param($Entry)
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::Open($Entry.path, [IO.Compression.ZipArchiveMode]::Update)
    try {
        $extension = $archive.GetEntry('xl/webextensions/webextension1.xml')
        if ($null -eq $extension) { throw 'Expected injected taskpane is missing.' }
        $reader = [IO.StreamReader]::new($extension.Open())
        try { [xml]$definition = $reader.ReadToEnd() } finally { $reader.Dispose() }
        [xml]$manifest = [IO.File]::ReadAllText((Join-Path ([IO.Path]::GetDirectoryName($Entry.staged)) 'one-manifest.xml'))
        $expected = $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Id']").InnerText
        $actual = $definition.SelectSingleNode("/*[local-name()='webextension']/*[local-name()='reference']").GetAttribute('id')
        if ($actual -ne $expected) { throw 'Refusing to remove an unowned workbook add-in.' }
        $ownedParts = @('xl/webextensions/taskpanes.xml', 'xl/webextensions/webextension1.xml', 'xl/webextensions/_rels/taskpanes.xml.rels')
        foreach ($part in @($archive.Entries | Where-Object { $_.FullName.StartsWith('xl/webextensions/') })) {
            if ($part.FullName -notin $ownedParts) { throw 'Refusing to remove an unexpected workbook web extension.' }
        }
        foreach ($name in @('_rels/.rels','[Content_Types].xml')) {
            $part = $archive.GetEntry($name)
            $reader = [IO.StreamReader]::new($part.Open())
            try { [xml]$document = $reader.ReadToEnd() } finally { $reader.Dispose() }
            foreach ($node in @($document.DocumentElement.ChildNodes)) {
                if ($node.NodeType -ne [Xml.XmlNodeType]::Element) { continue }
                if ($name -eq '_rels/.rels' -and $node.GetAttribute('Type').EndsWith('/webextensiontaskpanes') -and
                    $node.GetAttribute('Target').TrimStart('/') -ne 'xl/webextensions/taskpanes.xml') {
                    throw 'Refusing to remove an unowned taskpane relationship.'
                }
                if (($name -eq '_rels/.rels' -and $node.GetAttribute('Type').EndsWith('/webextensiontaskpanes')) -or
                    ($name -eq '[Content_Types].xml' -and $node.GetAttribute('PartName').TrimStart('/') -in $ownedParts)) {
                    $null = $document.DocumentElement.RemoveChild($node)
                }
            }
            $part.Delete()
            $writer = [IO.StreamWriter]::new($archive.CreateEntry($name).Open(), $utf8)
            try { $writer.Write($document.OuterXml) } finally { $writer.Dispose() }
        }
        foreach ($part in @($archive.Entries | Where-Object { $_.FullName.StartsWith('xl/webextensions/') })) {
            $part.Delete()
        }
    } finally { $archive.Dispose() }
}

function Optimize-TargetedWorkbook {
    <# .SYNOPSIS
    Uses native UI only for Check Performance, borrowing the exact resident-owned Excel.
    #>
    param($Command)
    $entry = $sessions[$Command.lane]
    $scope = [CaptureErrorChecking]::TryDisable($entry.process, $env:EXCEL_SHRINK_EXPECTED_DESKTOP)
    if ($null -eq $scope) { throw 'Owned Excel has no accessible workbook window.' }
    $excel = $null
    $book = $null
    try {
        $excel = $scope.GetOwnedApplication()
        $book = $excel.ActiveWorkbook
        if ($book.FullName -ne $entry.staged) { throw 'Unexpected native Check Performance workbook.' }
        return Invoke-WorkbookPerformanceOptimization @{excel=$excel; workbook=$book; process=$entry.process} $Command.timeout_seconds
    } finally {
        foreach ($reference in @($book, $excel)) {
            if ($null -ne $reference -and [Runtime.InteropServices.Marshal]::IsComObject($reference)) {
                [void][Runtime.InteropServices.Marshal]::ReleaseComObject($reference)
            }
        }
        $scope.Dispose()
    }
}

function Close-TargetedWorkbook {
    <# .SYNOPSIS
    Requests resident cleanup; the Rust deadline and owned job bound a stalled close.
    #>
    param([string]$Lane)
    $entry = $sessions[$Lane]
    if ($null -eq $entry) { return @{closed=$true} }
    Write-TargetedJson (Join-Path $entry.session 'finish.json') @{complete=$true}
    $entry.launcher.WaitForExit()
    if ($null -ne $entry.process -and -not $entry.process.WaitForExit(1000)) { throw 'Targeted Excel exit unconfirmed.' }
    $null = $sessions.Remove($Lane)
    if ($sessions.Count -eq 0 -and $null -ne $script:responseWatcher) {
        $script:responseWatcher.Dispose()
        $script:responseWatcher = $null
    }
    return @{closed=$true}
}

Export-ModuleMember -Function Open-TargetedWorkbook, Invoke-TargetedJob, Invoke-TargetedCaptureBatch, Optimize-TargetedWorkbook, Close-TargetedWorkbook
