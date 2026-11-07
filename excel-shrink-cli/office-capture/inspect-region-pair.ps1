<#
.SYNOPSIS
Reuses two owned, visible Excel applications for read-only region inspection.
.DESCRIPTION
Only staged copies are opened. Switching files closes the previous inspection
workbook without saving. A closed workbook or application is recreated on demand.
#>
$ErrorActionPreference = 'Stop'
Add-Type -Path @((Join-Path $PSScriptRoot 'CaptureProcesses.cs'), (Join-Path $PSScriptRoot 'InspectionWindows.cs'))
$owned = New-Object CaptureProcesses
$sessions = @{}

function Release-InspectionBook($Entry) {
    if ($null -eq $Entry.book) { return }
    try { $Entry.book.Close($false) } catch [Runtime.InteropServices.COMException] {
        # Closing an inspection window manually invalidates the retained COM proxy.
    }
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Entry.book)
    $Entry.book = $null
}

function Get-InspectionSession([string]$Side) {
    $entry = $sessions[$Side]
    if ($null -ne $entry -and $entry.process.HasExited) {
        Release-InspectionBook $entry
        if ($null -ne $entry.seed) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($entry.seed) }
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($entry.app)
        $entry.process.Dispose()
        $sessions.Remove($Side)
        $entry = $null
    }
    if ($null -eq $entry) {
        $app = New-Object -ComObject Excel.Application
        $excelPid = [uint32]0
        [void][InspectionWindows]::GetWindowThreadProcessId([IntPtr]$app.Hwnd, [ref]$excelPid)
        $process = [Diagnostics.Process]::GetProcessById($excelPid)
        $owned.Add($process)
        $app.DisplayAlerts = $false
        $app.AutomationSecurity = 3
        $app.EnableEvents = $false
        $app.AskToUpdateLinks = $false
        $app.AutoRecover.Enabled = $false
        $seed = $app.Workbooks.Add()
        $seed.EnableAutoRecover = $false
        $app.Calculation = -4135
        $entry = @{app=$app; process=$process; seed=$seed; book=$null; path=$null}
        $sessions[$Side] = $entry
    }
    return $entry
}

function Select-InspectionRegion([string]$Side, [string]$Path, [string]$Sheet, [string]$Range) {
    $entry = Get-InspectionSession $Side
    $open = $false
    if ($null -ne $entry.book) {
        try { $open = $entry.book.FullName -eq $Path } catch [Runtime.InteropServices.COMException] {
            # The process can remain alive after the user closes just this workbook.
        }
    }
    if (-not $open) {
        Release-InspectionBook $entry
        $missing = [Type]::Missing
        $entry.book = $entry.app.Workbooks.Open($Path, 0, $true, $missing, '', '', $true)
        $entry.book.EnableAutoRecover = $false
        $entry.path = $Path
        if ($null -ne $entry.seed) {
            # The seed permits setting calculation mode before opening the first file.
            # Once a real workbook exists it must not become an extra visible window.
            $entry.seed.Close($false)
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($entry.seed)
            $entry.seed = $null
        }
    }
    $sheetObject = $null
    $selection = $null
    try {
        $sheetObject = $entry.book.Worksheets.Item($Sheet)
        $selection = $sheetObject.Range($Range)
        $entry.app.Visible = $true
        [void]$entry.book.Activate()
        [void]$sheetObject.Activate()
        [void]$entry.app.Goto($selection, $true)
        return @{pid=$entry.process.Id; process_started_unix_ms=([DateTimeOffset]$entry.process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds(); hwnd=[long]$entry.app.Hwnd; path=$entry.path; sheet=$Sheet; range=$selection.Address(); read_only=[bool]$entry.book.ReadOnly}
    } finally {
        if ($null -ne $selection) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($selection) }
        if ($null -ne $sheetObject) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($sheetObject) }
    }
}

try {
    while ($null -ne ($line = [Console]::ReadLine())) {
        $request = $null
        try {
            $request = $line | ConvertFrom-Json
            $left = Select-InspectionRegion 'left' $request.original $request.sheet $request.range
            $right = Select-InspectionRegion 'right' $request.default $request.sheet $request.range
            [InspectionWindows]::Arrange([IntPtr]$left.hwnd, [IntPtr]$right.hwnd)
            [Console]::WriteLine((@{id=$request.id; ok=$true; original=$left; default=$right} | ConvertTo-Json -Depth 5 -Compress))
        } catch {
            [Console]::WriteLine((@{id=$request.id; ok=$false; error=$_.Exception.Message} | ConvertTo-Json -Compress))
        }
    }
} finally {
    try {
        foreach ($entry in $sessions.Values) {
            Release-InspectionBook $entry
            if ($null -ne $entry.seed) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($entry.seed) }
            try { $entry.app.Quit() } catch [Runtime.InteropServices.COMException] {}
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($entry.app)
            $entry.process.Dispose()
        }
    } finally {
        $owned.Dispose()
    }
}
