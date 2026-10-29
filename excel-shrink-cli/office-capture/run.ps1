param(
    [Parameter(Mandatory=$true)][string]$WorkbookPath,
    [Parameter(Mandatory=$true)][string]$ConfigPath,
    [Parameter(Mandatory=$true)][string]$OutputDir,
    [string]$PublishedOutputDir,
    [string]$ComparisonWorkbookPath,
    [string]$StreamPipe,
    [string]$HostExecutable,
    [string]$TelemetryPath,
    [string]$AttemptId,
    [string]$ComparisonProfile = 'shrunk',
    [ValidateRange(10, 600)][int]$StartupTimeoutSeconds = 180,
    [ValidateRange(10, 7200)][int]$CaptureTimeoutSeconds = 1800
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ExcelProcessSlots.ps1')
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)

Import-Module (Join-Path $PSScriptRoot 'CaptureSetup.psm1') -DisableNameChecking

$sourceWorkbook = [IO.Path]::GetFullPath($WorkbookPath)
$sourceConfig = [IO.Path]::GetFullPath($ConfigPath)
$destination = [IO.Path]::GetFullPath($OutputDir)
if (-not $PublishedOutputDir) { $PublishedOutputDir = $destination }
if (-not (Test-Path -LiteralPath $sourceWorkbook -PathType Leaf)) { throw "Workbook missing: $sourceWorkbook" }
if (-not (Test-Path -LiteralPath $sourceConfig -PathType Leaf)) { throw "Capture config missing: $sourceConfig" }
New-Item -ItemType Directory -Path $destination -Force | Out-Null
if (Test-Path -LiteralPath (Join-Path $destination 'result.json')) { throw 'Output directory already has a result.json.' }

$stage = Join-Path $env:TEMP ('es-office-capture-' + [Guid]::NewGuid().ToString('N'))
& (Join-Path $PSScriptRoot '../src/regression/archive_processes.ps1') -StagePath $stage
$runId = [Guid]::NewGuid().ToString('N')
$listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
$listener.Start()
try { $port = $listener.LocalEndpoint.Port } finally { $listener.Stop() }
$baseUrl = 'http://localhost:' + $port
$controlUrl = 'http://127.0.0.1:' + $port
$hostProcess = $null
$excelProcess = $null
$comparisonProcess = $null
$originalSlot = $null
$comparisonSlot = $null
$comparisonAddInId = $null
$comparisonRegistration = $false
$relay = $null
$ownedProcesses = $null
$captureDesktop = $null
$addInId = $null
$registrationDone = $false
$previousRegistration = $null
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$classicTelemetryAvailable = $true
$errorCheckingScopes = @{}
$captureReadyPath = $null

function Update-ClassicErrorChecking([string]$Side, $Process) {
    if (-not $hideErrorIndicators -or $null -eq $Process -or $errorCheckingScopes.ContainsKey($Side)) { return }
    $scope = [CaptureErrorChecking]::TryDisable($Process, $captureDesktop.Name)
    if ($null -ne $scope) { $errorCheckingScopes[$Side] = $scope }
}
function Write-ClassicObservation([string]$Side, [string]$State, $Process = $null) {
    if (-not $TelemetryPath -or -not $hostReady -or -not $classicTelemetryAvailable) { return }
    $body = @{ run_id = $runId; side = $Side; state = $State; message = ('Classic Excel ' + $State) }
    try {
        if ($null -ne $Process) {
            $body.pid = $Process.Id
            $body.process_started_unix_ms = ([DateTimeOffset]($Process.StartTime.ToUniversalTime())).ToUnixTimeMilliseconds()
        }
        $receipt = Invoke-RestMethod -Uri ($controlUrl + '/telemetry') -Method Post -ContentType 'application/json' `
            -Body ($body | ConvertTo-Json -Compress) -TimeoutSec 2
        if (-not $receipt.telemetry_saved) { throw 'The classic host could not save its observations.' }
    } catch {
        $script:classicTelemetryAvailable = $false
        # Stdout carries binary image frames in paired mode.
        [Console]::Error.WriteLine('Classic telemetry update failed: ' + $_.Exception.Message)
    }
}

try {
    Add-Type -Path @((Join-Path $PSScriptRoot 'CaptureProcesses.cs'), (Join-Path $PSScriptRoot 'CaptureDesktop.cs'))
    $ownedProcesses = New-Object CaptureProcesses
    $captureDesktop = New-Object CaptureDesktop
    $hostExecutable = Get-CaptureHostExecutable $HostExecutable

    $webSource = Join-Path $PSScriptRoot 'web'
    $webStage = Join-Path $stage 'web'
    $hostOutput = Join-Path $stage 'output'
    $documentDirectory = Join-Path $stage 'document'
    foreach ($path in @($webStage, $hostOutput, $documentDirectory)) { New-Item -ItemType Directory -Path $path -Force | Out-Null }
    Copy-Item -Path (Join-Path $webSource '*') -Destination $webStage -Recurse -Force
    foreach ($asset in @('manifest.xml', 'taskpane.html', 'commands.html', 'app.js', 'capture-geometry.js', 'capture-overflow.js', 'taskpane.css', 'icon-32.png')) {
        if (-not (Test-Path -LiteralPath (Join-Path $webStage $asset) -PathType Leaf)) {
            throw "Office add-in asset is missing: $asset"
        }
    }
    $taskpanePath = Join-Path $webStage 'taskpane.html'
    $taskpane = [IO.File]::ReadAllText($taskpanePath, $utf8).Replace('__CAPTURE_RUN_ID__', $runId)
    [IO.File]::WriteAllText($taskpanePath, $taskpane, $utf8)
    $manifestSource = Join-Path $webStage 'manifest.xml'
    [xml]$manifest = [IO.File]::ReadAllText($manifestSource, $utf8).Replace('http://localhost:3000', $baseUrl)
    foreach ($node in $manifest.SelectNodes('//*[@DefaultValue]')) {
        $url = $node.GetAttribute('DefaultValue')
        if ($url.StartsWith($baseUrl + '/')) {
            $node.SetAttribute('DefaultValue', ($url + '?run=' + $runId))
        }
    }
    $idNode = $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Id']")
    $versionNode = $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Version']")
    if ($null -eq $idNode -or $null -eq $versionNode) { throw 'Office add-in manifest is incomplete.' }
    $addInId = [Guid]::NewGuid().ToString()
    $idNode.InnerText = $addInId
    $manifestPath = Join-Path $stage 'manifest.xml'
    [IO.File]::WriteAllText($manifestPath, $manifest.OuterXml, $utf8)

    $workbookCopy = Join-Path $documentDirectory ([IO.Path]::GetFileName($sourceWorkbook))
    Copy-Item -LiteralPath $sourceWorkbook -Destination $workbookCopy
    Add-AutoOpenTaskpane $workbookCopy $addInId $versionNode.InnerText.Trim()
    if ($ComparisonWorkbookPath) {
        if (-not $StreamPipe) { throw 'Paired capture requires a stream pipe.' }
        # Each workbook owns a separate task pane and a distinct manifest URL.
        $comparisonAddInId = [Guid]::NewGuid().ToString()
        $comparisonManifest = $manifest.OuterXml.Replace($addInId, $comparisonAddInId).Replace('?run=' + $runId, '?run=' + $runId + '&amp;side=shrunk')
        $comparisonManifestPath = Join-Path $stage 'comparison-manifest.xml'
        [IO.File]::WriteAllText($comparisonManifestPath, $comparisonManifest, $utf8)
        [IO.File]::WriteAllText($manifestPath, $manifest.OuterXml.Replace('?run=' + $runId, '?run=' + $runId + '&amp;side=original'), $utf8)
        $comparisonCopy = Join-Path $documentDirectory 'comparison.xlsx'
        Copy-Item -LiteralPath $ComparisonWorkbookPath -Destination $comparisonCopy
        Add-AutoOpenTaskpane $comparisonCopy $comparisonAddInId $versionNode.InnerText.Trim()
    }
    $configCopy = Join-Path $stage 'config.json'
    Copy-Item -LiteralPath $sourceConfig -Destination $configCopy
    $captureConfig = [IO.File]::ReadAllText($configCopy, $utf8) | ConvertFrom-Json
    Add-Type -Path (Join-Path $PSScriptRoot 'CaptureWorkbook.cs') `
        -ReferencedAssemblies @('System.IO.Compression', 'System.IO.Compression.FileSystem', 'System.Xml')
    $hideErrorIndicators = $captureConfig.mode -in @('capture', 'paired')
    if ($hideErrorIndicators) {
        # The host holds configuration responses until this launcher verifies
        # the setting on every owned Excel instance. No image job can race it.
        $captureReadyPath = Join-Path $stage 'capture-ready.json'
        $captureConfig | Add-Member -NotePropertyName _capture_ready_path -NotePropertyValue $captureReadyPath -Force
        $captureConfig | Add-Member -NotePropertyName _capture_ready_timeout_seconds -NotePropertyValue $StartupTimeoutSeconds -Force
        [IO.File]::WriteAllText($configCopy, ($captureConfig | ConvertTo-Json -Depth 30), $utf8)
    }
    Set-InitialWorksheet $workbookCopy ([int]$captureConfig.sheets[0].index) $hideErrorIndicators
    if ($ComparisonWorkbookPath) { Set-InitialWorksheet $comparisonCopy ([int]$captureConfig.sheets[0].index) $hideErrorIndicators }

    $hostStdout = Join-Path $stage 'host.stdout.log'
    $hostStderr = Join-Path $stage 'host.stderr.log'
    $hostArguments = 'capture-host classic --config {0} --output {1} --published-output {2} --web {3} --run-id {4}' -f `
        (Quote-Argument $configCopy), (Quote-Argument $hostOutput), `
        (Quote-Argument $PublishedOutputDir), (Quote-Argument $webStage), (Quote-Argument $runId)
    $hostArguments += ' --port ' + $port
    if ($StreamPipe) { $hostArguments += ' --stream-pipe ' + (Quote-Argument $StreamPipe) }
    if ($TelemetryPath) {
        if (-not $AttemptId) { throw 'Classic telemetry requires an attempt id.' }
        $hostArguments += ' --telemetry-path {0} --attempt-id {1} --comparison-profile {2}' -f `
            (Quote-Argument $TelemetryPath), (Quote-Argument $AttemptId), (Quote-Argument $ComparisonProfile)
    }
    $hostProcess = Start-Process -FilePath $hostExecutable -ArgumentList $hostArguments -WorkingDirectory $stage `
        -RedirectStandardOutput $hostStdout -RedirectStandardError $hostStderr -WindowStyle Hidden -PassThru
    $ownedProcesses.Add($hostProcess)
    $hostReady = $false
    $startup = [Diagnostics.Stopwatch]::StartNew()
    while ($startup.Elapsed.TotalSeconds -lt 30) {
        Start-Sleep -Milliseconds 200
        $hostProcess.Refresh()
        if ($hostProcess.HasExited) { break }
        try {
            $health = Invoke-RestMethod -Uri ($controlUrl + '/health') -TimeoutSec 2
            if ($health.ok -and $health.run_id -eq $runId) { $hostReady = $true; break }
        } catch {}
    }
    if (-not $hostReady) { throw 'Worksheet capture host did not become ready within 30 seconds.' }
    $taskpaneResponse = Invoke-WebRequest -UseBasicParsing -Uri ($baseUrl + '/taskpane.html') -TimeoutSec 10
    if ($taskpaneResponse.StatusCode -ne 200) { throw 'Office add-in taskpane is unavailable over HTTP.' }

    if ($ComparisonWorkbookPath) {
        Add-Type -Path (Join-Path $PSScriptRoot 'PairRelay.cs')
        $relay = [PairRelay]::Start($StreamPipe)
        $comparisonPrevious = Register-AddIn $comparisonAddInId $comparisonManifestPath
        $comparisonRegistration = $true
    }
    $previousRegistration = Register-AddIn $addInId $manifestPath
    $registrationDone = $true
    $excelExecutable = Find-ExcelExecutable
    Write-ClassicObservation 'original' 'opening'
    $captureSlots = @(Enter-ExcelProcessSlots 2)
    if ($captureSlots.Count -gt 0) {
        $originalSlot = $captureSlots[0]
        $comparisonSlot = $captureSlots[1]
    }
    $excelProcess = $captureDesktop.Start($excelExecutable, ('/x ' + (Quote-Argument $workbookCopy)), $ownedProcesses)
    Write-ClassicObservation 'original' 'waiting_for_addin' $excelProcess

    if ($ComparisonWorkbookPath) {
        # Starting both Office runtimes together can leave one waiting for startup.
        $startup.Restart()
        while ($true) {
            $excelProcess.Refresh()
            if ($excelProcess.HasExited) { throw 'Original Excel process exited before its add-in started.' }
            $health = Invoke-RestMethod -Uri ($controlUrl + '/health') -TimeoutSec 2
            Update-ClassicErrorChecking 'original' $excelProcess
            if ($health.run_id -eq $runId -and $health.original_seen -and
                (-not $hideErrorIndicators -or $errorCheckingScopes.ContainsKey('original'))) { break }
            if ($startup.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds) { throw 'Original Office add-in did not start before the startup timeout.' }
            Start-Sleep -Milliseconds 250
        }
        Write-ClassicObservation 'shrunk' 'opening'
        $comparisonProcess = $captureDesktop.Start($excelExecutable, ('/x ' + (Quote-Argument $comparisonCopy)), $ownedProcesses)
        Write-ClassicObservation 'shrunk' 'waiting_for_addin' $comparisonProcess
    }

    $startup.Restart()
    $captureStarted = $false
    $result = Join-Path $hostOutput 'result.json'
    $captureTimer = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $result -PathType Leaf)) {
        if ($null -ne $relay -and $relay.IsFaulted) { $relay.GetAwaiter().GetResult() }
        $hostProcess.Refresh()
        if ($hostProcess.HasExited) { throw 'Worksheet capture host exited before writing a result.' }
        if (-not $captureStarted) {
            Update-ClassicErrorChecking 'original' $excelProcess
            Update-ClassicErrorChecking 'shrunk' $comparisonProcess
            $expected = if ($ComparisonWorkbookPath) { 2 } else { 1 }
            if ($captureReadyPath -and $errorCheckingScopes.Count -eq $expected -and
                -not (Test-Path -LiteralPath $captureReadyPath)) {
                $pending = $captureReadyPath + '.pending'
                [IO.File]::WriteAllText($pending, '{"background_error_checking_disabled":true}', $utf8)
                [IO.File]::Move($pending, $captureReadyPath)
            }
            try {
                $health = Invoke-RestMethod -Uri ($controlUrl + '/health') -TimeoutSec 2
                $ready = if ($ComparisonWorkbookPath) { $health.original_seen -and $health.shrunk_seen } else { $health.config_seen }
                if ($health.run_id -eq $runId -and $ready -and (-not $hideErrorIndicators -or $errorCheckingScopes.Count -eq $expected)) {
                    $captureStarted = $true
                    $captureTimer.Restart()
                }
            } catch {}
            if ($startup.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds) { throw 'Office add-in did not start before the startup timeout.' }
        } elseif ($captureTimer.Elapsed.TotalSeconds -gt $CaptureTimeoutSeconds) {
            throw 'Worksheet capture exceeded the capture timeout.'
        }
        Start-Sleep -Milliseconds 250
    }

    if ($null -ne $relay) { $relay.GetAwaiter().GetResult() }
    Copy-CompletedOutput $hostOutput $destination
    $captureResult = [IO.File]::ReadAllText((Join-Path $destination 'result.json'), $utf8) | ConvertFrom-Json
    if ($null -ne $captureResult.PSObject.Properties['error'] -and $null -ne $captureResult.error) {
        throw "Worksheet capture failed: $($captureResult.error | ConvertTo-Json -Compress -Depth 5)"
    }
    if (-not $StreamPipe) { Write-Host ('RESULT: ' + (Join-Path $destination 'result.json')) }
} catch {
    foreach ($name in @('host.stdout.log', 'host.stderr.log')) {
        $log = Join-Path $stage $name
        if (Test-Path -LiteralPath $log -PathType Leaf) {
            Copy-Item -LiteralPath $log -Destination (Join-Path $destination $name) -Force -ErrorAction SilentlyContinue
        }
    }
    throw
} finally {
    try {
        if ($null -ne $excelProcess) { Write-ClassicObservation 'original' 'closing' }
        if ($null -ne $comparisonProcess) { Write-ClassicObservation 'shrunk' 'closing' }
        foreach ($scope in $errorCheckingScopes.Values) {
            try { $scope.Dispose() }
            catch { [Console]::Error.WriteLine('Could not restore owned Excel error checking: ' + $_.Exception.Message) }
        }
        Stop-OwnedProcess $comparisonProcess
        Stop-OwnedProcess $excelProcess
        foreach ($entry in @(@('original', $excelProcess), @('shrunk', $comparisonProcess))) {
            if ($null -eq $entry[1]) { continue }
            try {
                $entry[1].Refresh()
                if ($entry[1].HasExited) { Write-ClassicObservation $entry[0] 'closed' }
            } catch {
                [Console]::Error.WriteLine('Could not confirm classic Excel exit: ' + $_.Exception.Message)
            }
        }
    } finally {
        # Keep the host alive through the exit observations. The ownership job
        # still catches every remaining child if individual shutdown fails.
        if ($null -ne $ownedProcesses) { $ownedProcesses.Dispose() }
        if ($null -eq $excelProcess -or $excelProcess.HasExited) {
            Exit-ExcelProcessSlot $originalSlot
        }
        if ($null -eq $comparisonProcess -or $comparisonProcess.HasExited) {
            Exit-ExcelProcessSlot $comparisonSlot
        }
        Stop-OwnedProcess $hostProcess
        if ($null -ne $captureDesktop) { $captureDesktop.Dispose() }
        if ($comparisonRegistration) { Restore-AddIn $comparisonAddInId $comparisonPrevious }
        if ($registrationDone) { Restore-AddIn $addInId $previousRegistration }
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    }
}
