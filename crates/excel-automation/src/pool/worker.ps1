param(
    [Parameter(Mandatory=$true)][string]$Session,
    [Parameter(Mandatory=$true)][int]$OwnerPid,
    [Parameter(Mandatory=$true)][long]$OwnerStartedMs,
    [string]$DesktopName
)
$ErrorActionPreference = 'Stop'
Add-Type -Path @((Join-Path $PSScriptRoot 'CaptureProcesses.cs'), (Join-Path $PSScriptRoot 'CaptureDesktop.cs'))
$performanceWorker = [IO.File]::Exists((Join-Path $PSScriptRoot 'performance-pool'))
if ($performanceWorker) { Add-Type -Path (Join-Path $PSScriptRoot 'ExcelProcessId.cs') }
Import-Module (Join-Path $PSScriptRoot 'CaptureSetup.psm1') -Force -DisableNameChecking
Import-Module (Join-Path $PSScriptRoot 'ExcelConversionErrors.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'WorkerPerformance.psm1') -Force

# PowerShell can create hidden windows before the script starts, preventing
# SetThreadDesktop. Start one persistent helper on the owned desktop instead.
if ($performanceWorker -and -not $DesktopName) {
    $bootstrapOwned = New-Object CaptureProcesses
    $bootstrapDesktop = [CaptureDesktop]::new()
    try {
        $arguments = '-NoProfile -ExecutionPolicy Bypass -File ' + (Quote-Argument $PSCommandPath) +
            ' -Session ' + (Quote-Argument $Session) + ' -OwnerPid ' + $OwnerPid +
            ' -OwnerStartedMs ' + $OwnerStartedMs + ' -DesktopName ' + (Quote-Argument $bootstrapDesktop.Name)
        $helper = $bootstrapDesktop.Start((Join-Path $PSHOME 'powershell.exe'), $arguments, $bootstrapOwned)
        $helper.WaitForExit()
        $helperExitCode = $helper.ExitCode
    } finally {
        $bootstrapOwned.Dispose()
        $bootstrapDesktop.Dispose()
    }
    exit $helperExitCode
}

function Write-Receipt([string]$Path, $Value) {
    $pending = $Path + '.pending'
    [IO.File]::WriteAllText($pending, ($Value | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $pending -Destination $Path -Force
}

function Set-Phase([string]$Directory, [string]$Phase) {
    Write-Receipt (Join-Path $Directory 'progress.json') @{phase=$Phase;updated_utc=[DateTime]::UtcNow.ToString('o')}
}

$owned = New-Object CaptureProcesses
$excel = $null
$settings = $null
$workbook = $null
$convertedCount = 0
$desktop = $null
if (-not $DesktopName) {
    $desktop = [CaptureDesktop]::new()
    $DesktopName = $desktop.Name
}
$checking = $null
$ownerProcess = [Diagnostics.Process]::GetProcessById($OwnerPid)
try {
    # Launch an owned native process on a private desktop, then bind COM to
    # that PID's native window. This avoids global COM activation serialization.
    $seed = Join-Path $Session 'seed.xlsx'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'seed.xlsx') -Destination $seed
    $process = [CaptureDesktop]::StartOnDesktop($DesktopName, (Find-ExcelExecutable), ('/x /automation ' + (Quote-Argument $seed)), $owned)
    $excelPid = $process.Id
    Write-Receipt (Join-Path $Session 'identity.json') @{
        pid=$excelPid;desktop=$DesktopName;started_ms=([DateTimeOffset]$process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
    }
    $startup = [Diagnostics.Stopwatch]::StartNew()
    while ($null -eq $checking -and $startup.Elapsed.TotalSeconds -lt 110) {
        $checking = [CaptureErrorChecking]::TryDisable($process, $DesktopName)
        if ($null -eq $checking) { Start-Sleep -Milliseconds 100 }
    }
    if ($null -eq $checking) { throw 'Excel native automation window did not become ready.' }
    $excel = $checking.GetOwnedApplication()
    $excel.Visible = $false
    $excel.Interactive = $false
    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $excel.AutomationSecurity = 3
    $excel.EnableEvents = $false
    $excel.AutoRecover.Enabled = $false
    # A retained blank workbook keeps manual calculation in force between inputs.
    $settings = $excel.ActiveWorkbook
    $settings.EnableAutoRecover = $false
    $excel.Calculation = -4135
    $monitorArguments = '-NoProfile -ExecutionPolicy Bypass -File ' + (Quote-Argument (Join-Path $PSScriptRoot 'watch-opening.ps1')) +
        ' -Session ' + (Quote-Argument $Session) + ' -HelperPid ' + $PID
    $dialogMonitor = [CaptureDesktop]::StartOnDesktop($DesktopName, (Join-Path $PSHOME 'powershell.exe'), $monitorArguments, $owned)
    $excelPath = Find-ExcelExecutable
    $excelFile = [Diagnostics.FileVersionInfo]::GetVersionInfo($excelPath)
    $excelStream = [IO.File]::OpenRead($excelPath)
    $excelHasher = [Security.Cryptography.SHA256]::Create()
    try {
        $excelHash = [BitConverter]::ToString($excelHasher.ComputeHash($excelStream)).Replace('-', '').ToLowerInvariant()
    } finally {
        $excelHasher.Dispose()
        $excelStream.Dispose()
    }
    $environment = @{
        machine=$env:COMPUTERNAME;user=$env:USERNAME
        excel_sha256=$excelHash
        excel_version=$excelFile.FileVersion
        culture=[Globalization.CultureInfo]::CurrentCulture.Name
        ui_culture=[Globalization.CultureInfo]::CurrentUICulture.Name
        calculation='manual';window_mode='owned-private-desktop'
    }
    Write-Receipt (Join-Path $Session 'ready.json') @{pid=$excelPid;helper_pid=$PID}
    while ($true) {
        if ($ownerProcess.HasExited -or ([DateTimeOffset]$ownerProcess.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds() -ne $OwnerStartedMs) { break }
        $requestPath = Join-Path $Session 'request.json'
        if (-not ([IO.File]::Exists($requestPath))) {
            if ([IO.File]::Exists((Join-Path $Session 'retire'))) { break }
            Start-Sleep -Milliseconds 50
            continue
        }
        try {
            $request = [IO.File]::ReadAllText($requestPath) | ConvertFrom-Json
        } catch [IO.IOException] {
            # Publication or a scanner may briefly hold the atomic request file.
            $code = $_.Exception.HResult -band 0xffff
            if ($code -notin @(2, 32, 33)) { throw }
            Start-Sleep -Milliseconds 50
            continue
        }
        Remove-Item -LiteralPath $requestPath -Force
        Write-Receipt (Join-Path $Session 'active.json') $request
        $phase = 'opening workbook'
        $response = $null
        $recycle = $false
        $phaseTimes = [ordered]@{}
        $phaseClock = [Diagnostics.Stopwatch]::StartNew()
        try {
            if ([bool]$request.performance -ne $performanceWorker) {
                throw 'The request operation does not match this dedicated worker pool.'
            }
            $excel.AutomationSecurity = 3
            $excel.EnableEvents = $false
            $excel.AskToUpdateLinks = $false
            $excel.Calculation = -4135
            Set-Phase $request.directory $phase
            $missing = [Type]::Missing
            $workbook = $excel.Workbooks.Open($request.input, 0, $true, $missing, '', '', $true)
            $openSeconds = $phaseClock.Elapsed.TotalSeconds
            $phaseTimes[$phase] = $openSeconds
            $workbook.EnableAutoRecover = $false
            $process.Refresh()
            $metrics = [ordered]@{
                open_seconds=$openSeconds;working_set_bytes=$process.WorkingSet64
                private_bytes=$process.PrivateMemorySize64;calculation='manual'
                macros_removed=[bool]$workbook.HasVBProject
                worker_session_id=[IO.Path]::GetFileName($Session)
                excel_pid=$excelPid;workbooks_converted=($convertedCount + 1)
            }
            if ([IO.File]::Exists((Join-Path $PSScriptRoot 'rebuild-pool'))) {
                $phase = 'full calculation rebuild'
                Set-Phase $request.directory $phase
                $phaseClock.Restart()
                Write-Receipt (Join-Path $request.directory 'calculation-state.json') @{
                    stage='calling_full_rebuild';updated_utc=[DateTime]::UtcNow.ToString('o')
                }
                $excel.CalculateFullRebuild()
                $calculationState = $excel.CalculationState
                Write-Receipt (Join-Path $request.directory 'calculation-state.json') @{
                    stage='full_rebuild_returned';state=$calculationState
                    updated_utc=[DateTime]::UtcNow.ToString('o')
                }
                while ($calculationState -ne 0) {
                    Start-Sleep -Milliseconds 100
                    $nextState = $excel.CalculationState
                    if ($nextState -ne $calculationState) {
                        Write-Receipt (Join-Path $request.directory 'calculation-state.json') @{
                            stage='waiting_for_calculation';state=$nextState
                            updated_utc=[DateTime]::UtcNow.ToString('o')
                        }
                    }
                    $calculationState = $nextState
                }
                $phaseTimes[$phase] = $phaseClock.Elapsed.TotalSeconds
                $excel.CalculateBeforeSave = $false
            }
            $phase = if ($request.performance) { 'optimizing workbook performance' } else { 'saving XLSX conversion' }
            Set-Phase $request.directory $phase
            $phaseClock.Restart()
            $nativeResult = $null
            if ($request.performance) {
                $nativeResult = Invoke-WorkerPerformance $excel $workbook $process $request $DesktopName
            } else {
                $workbook.CheckCompatibility = $false
                $pendingOutput = Join-Path $request.directory 'converted.pending.xlsx'
                $metrics['shared_editing_removed'] = [bool]$workbook.MultiUserEditing
                # Only the new conversion copy loses legacy sharing and revision history.
                $exclusiveAccess = 3
                $workbook.SaveAs($pendingOutput, 51, $missing, $missing, $false, $false, $exclusiveAccess)
            }
            $phaseTimes[$phase] = $phaseClock.Elapsed.TotalSeconds
            $phase = 'closing workbook'
            Set-Phase $request.directory $phase
            $phaseClock.Restart()
            $workbook.Close($false)
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($workbook)
            $workbook = $null
            $excel.CutCopyMode = 0
            if ($excel.Workbooks.Count -ne 1) { throw 'Unexpected workbook remained in the conversion session.' }
            $phaseTimes[$phase] = $phaseClock.Elapsed.TotalSeconds
            $metrics['phase_seconds'] = $phaseTimes
            if ($request.performance) {
                $metrics.Remove('workbooks_converted')
                $metrics['worker_jobs_completed'] = $convertedCount + 1
                $metrics['measurement_scope'] = 'current workbook in a fresh Check Performance Excel process'
                $metrics['excel_reuse_policy'] = 'fresh-excel-per-input'
                # Withdraw this session before publishing success to avoid stale native pane results.
                [IO.File]::WriteAllText((Join-Path $Session 'retire'), 'check-performance input complete')
                $recycle = $true
            }
            if (-not $request.performance) {
                Move-Item -LiteralPath $pendingOutput -Destination $request.output -Force
            }
            $response = @{success=$true;metrics=$metrics;native_result=$nativeResult;environment=$environment}
            $convertedCount++
        } catch {
            $response = @{
                success=$false
                kind=(Get-LegacyConversionFailureKind $_.Exception.Message $phase)
                phase=$phase
                error=$_.Exception.Message
                hresult=$_.Exception.HResult
                excel_pid=$excelPid
                error_position=[string]$_.InvocationInfo.PositionMessage
                script_stack_trace=$_.ScriptStackTrace
            }
            # A failed COM operation can leave hidden workbooks or modal state.
            $recycle = $true
        }
        Remove-Item -LiteralPath (Join-Path $Session 'active.json') -Force
        $response['id'] = $request.id
        Write-Receipt (Join-Path $request.directory 'response.json') $response
        if ($recycle) { break }
    }
} catch {
    Write-Receipt (Join-Path $Session 'startup-error.json') @{
        error=$_.Exception.Message
        error_position=[string]$_.InvocationInfo.PositionMessage
        script_stack_trace=$_.ScriptStackTrace
    }
} finally {
    try {
        if ($null -ne $workbook) {
            try { $workbook.Close($false) } catch {}
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($workbook)
        }
        if ($null -ne $settings) {
            try { $settings.Close($false) } catch {}
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($settings)
        }
        if ($null -ne $checking) { $checking.Dispose() }
        if ($null -ne $excel) {
            try { $excel.Quit() } catch {}
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel)
        }
    } finally {
        $owned.Dispose()
        if ($null -ne $desktop) { $desktop.Dispose() }
        $ownerProcess.Dispose()
    }
}
