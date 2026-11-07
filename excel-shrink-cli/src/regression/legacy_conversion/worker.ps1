param(
    [Parameter(Mandatory=$true)][string]$Session,
    [Parameter(Mandatory=$true)][int]$OwnerPid,
    [Parameter(Mandatory=$true)][long]$OwnerStartedMs
)
$ErrorActionPreference = 'Stop'
Add-Type -Path @((Join-Path $PSScriptRoot 'CaptureProcesses.cs'), (Join-Path $PSScriptRoot 'CaptureDesktop.cs'))
Import-Module (Join-Path $PSScriptRoot 'CaptureSetup.psm1') -Force -DisableNameChecking
Import-Module (Join-Path $PSScriptRoot 'ExcelConversionErrors.psm1') -Force

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
$desktop = [CaptureDesktop]::new()
$checking = $null
$ownerProcess = [Diagnostics.Process]::GetProcessById($OwnerPid)
try {
    # Launch an owned native process on a private desktop, then bind COM to
    # that PID's native window. This avoids global COM activation serialization.
    $seed = Join-Path $Session 'seed.xlsx'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'seed.xlsx') -Destination $seed
    $process = $desktop.Start((Find-ExcelExecutable), ('/x /automation ' + (Quote-Argument $seed)), $owned)
    $excelPid = $process.Id
    Write-Receipt (Join-Path $Session 'identity.json') @{
        pid=$excelPid;desktop=$desktop.Name;started_ms=([DateTimeOffset]$process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
    }
    $startup = [Diagnostics.Stopwatch]::StartNew()
    while ($null -eq $checking -and $startup.Elapsed.TotalSeconds -lt 110) {
        $checking = [CaptureErrorChecking]::TryDisable($process, $desktop.Name)
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
            $phase = 'saving XLSX conversion'
            Set-Phase $request.directory $phase
            $phaseClock.Restart()
            $workbook.CheckCompatibility = $false
            $pendingOutput = Join-Path $request.directory 'converted.pending.xlsx'
            $workbook.SaveAs($pendingOutput, 51)
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
            Move-Item -LiteralPath $pendingOutput -Destination $request.output -Force
            $response = @{success=$true;metrics=$metrics}
            $convertedCount++
        } catch {
            $response = @{success=$false;kind=(Get-LegacyConversionFailureKind $_.Exception.Message $phase);phase=$phase;error=$_.Exception.Message;hresult=$_.Exception.HResult;excel_pid=$excelPid}
            # A failed COM operation can leave hidden workbooks or modal state.
            $recycle = $true
        }
        Remove-Item -LiteralPath (Join-Path $Session 'active.json') -Force
        $response['id'] = $request.id
        Write-Receipt (Join-Path $request.directory 'response.json') $response
        if ($recycle) { break }
    }
} catch {
    Write-Receipt (Join-Path $Session 'startup-error.json') @{error=$_.Exception.Message}
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
        $desktop.Dispose()
        $ownerProcess.Dispose()
    }
}
