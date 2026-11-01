# Persistent Excel sessions for the targeted comparison CLI. One process per lane.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -Path (Join-Path $PSScriptRoot 'CaptureProcesses.cs')
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ExcelProcessId {
    [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] public static extern IntPtr GetClipboardOwner();
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr handle, out uint processId);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint threadId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool GetUserObjectInformation(IntPtr handle, int index, System.Text.StringBuilder name, uint length, out uint needed);
    public static string DesktopName(IntPtr window) {
        uint pid, needed;
        var desktop = GetThreadDesktop(GetWindowThreadProcessId(window, out pid));
        var name = new System.Text.StringBuilder(256);
        if (!GetUserObjectInformation(desktop, 2, name, 512, out needed)) throw new System.ComponentModel.Win32Exception();
        return name.ToString();
    }
}
'@

Import-Module (Join-Path $PSScriptRoot 'CheckPerformance.psm1') -Force

$sessions = @{}
$ownedExcel = New-Object CaptureProcesses

function Open-Workbook {
    <#
    .SYNOPSIS
    Opens one private Excel instance and keeps it alive across commands.
    .DESCRIPTION
    Sets manual calculation before opening the requested workbook so an
    explicit Full Rebuild remains distinguishable from ordinary opening.
    Excel may still recalculate when workbook settings require it; callers
    must inspect the observed state instead of assuming an untouched cache.
    #>
    param([string]$Lane, [string]$Path)

    if ($sessions.ContainsKey($Lane)) { throw "Excel lane already open: $Lane" }
    $excel = New-Object -ComObject Excel.Application
    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $control = $null
    $workbook = $null
    try {
        [uint32]$processId = 0
        $null = [ExcelProcessId]::GetWindowThreadProcessId([IntPtr]$excel.Hwnd, [ref]$processId)
        $process = [Diagnostics.Process]::GetProcessById($processId)
        $ownedExcel.Add($process)
        if ($env:EXCEL_SHRINK_EXPECTED_DESKTOP -and [ExcelProcessId]::DesktopName([IntPtr]$excel.Hwnd) -ne $env:EXCEL_SHRINK_EXPECTED_DESKTOP) {
            throw 'Excel did not start on the expected private desktop; refusing to show it.'
        }
        $excel.Visible = $true
        $startedAt = ([DateTimeOffset]$process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
        [Console]::WriteLine((@{event='process_started'; lane=$Lane; pid=$processId; started_at=$startedAt} | ConvertTo-Json -Compress))
        $control = $excel.Workbooks.Add()
        $excel.Calculation = -4135  # xlCalculationManual
        $workbook = $excel.Workbooks.Open($Path, 0, $false)
        [uint32]$processId = 0
        $null = [ExcelProcessId]::GetWindowThreadProcessId(
            [IntPtr]$excel.Hwnd, [ref]$processId)
        $sessions[$Lane] = @{excel=$excel; control=$control; workbook=$workbook; process=$process}
        return @{pid=$processId; calculation_mode=$excel.Calculation}
    }
    catch {
        if ($null -ne $workbook) { $workbook.Close($false) }
        if ($null -ne $control) { $control.Close($false) }
        $excel.Quit()
        throw
    }
}

function Save-Workbook {
    param([string]$Lane)
    $session = $sessions[$Lane]
    if ($null -eq $session) { throw "Excel lane is not open: $Lane" }
    $session.workbook.Save()
    return @{path=$session.workbook.FullName}
}

function Recalculate-Workbook {
    param([string]$Lane)
    $session = $sessions[$Lane]
    if ($null -eq $session) { throw "Excel lane is not open: $Lane" }
    $session.excel.CalculateFullRebuild()
    $session.workbook.Save()
    return @{path=$session.workbook.FullName}
}

function Capture-Region {
    <#
    .SYNOPSIS
    Captures a bounded Excel range and writes its PNG before replying.
    .DESCRIPTION
    The clipboard is shared within the Windows window station, including other
    desktops. Capture commands here are sequential; a fresh sequence and the
    owning Excel PID are verified before consuming an image. External clipboard
    activity fails explicitly instead of silently comparing unrelated pixels.
    #>
    param([string]$Lane, [string]$Sheet, [string]$Range, [string]$Output)

    $session = $sessions[$Lane]
    if ($null -eq $session) { throw "Excel lane is not open: $Lane" }
    $worksheet = $session.workbook.Worksheets.Item($Sheet)
    $session.workbook.Activate()
    $worksheet.Activate()
    $session.excel.ActiveWindow.Activate()
    $target = $worksheet.Range($Range)
    $session.excel.Goto($target, $true)
    for ($attempt = 0; $attempt -lt 3; $attempt++) {
        try {
            $sequence = [ExcelProcessId]::GetClipboardSequenceNumber()
            $null = $target.CopyPicture(1, 2)
            break
        } catch [Runtime.InteropServices.COMException] {
            if ($attempt -eq 2) { throw }
            Start-Sleep -Milliseconds 150
        }
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(2)
    while ([ExcelProcessId]::GetClipboardSequenceNumber() -eq $sequence -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 25
    }
    if ([ExcelProcessId]::GetClipboardSequenceNumber() -eq $sequence) { throw 'Excel did not publish a new capture.' }
    [uint32]$clipboardPid = 0
    $null = [ExcelProcessId]::GetWindowThreadProcessId([ExcelProcessId]::GetClipboardOwner(), [ref]$clipboardPid)
    if ($clipboardPid -ne $session.process.Id) { throw 'Clipboard ownership changed; refusing unrelated image data.' }
    $bitmap = [System.Windows.Forms.Clipboard]::GetImage()
    if ($null -eq $bitmap) { throw "Excel returned no image for $Sheet!$Range" }
    try {
        $bitmap.Save($Output, [System.Drawing.Imaging.ImageFormat]::Png)
        return @{path=$Output; width=$bitmap.Width; height=$bitmap.Height}
    }
    finally {
        $bitmap.Dispose()
    }
}

function Measure-Region {
    <#
    .SYNOPSIS
    Reads rendered row positions without taking a screenshot.
    #>
    param([string]$Lane, [string]$Sheet, [string]$Range)
    $session = $sessions[$Lane]
    if ($null -eq $session) { throw "Excel lane is not open: $Lane" }
    $cell = $session.workbook.Worksheets.Item($Sheet).Range($Range)
    return @{top=$cell.Top; height=$cell.Height; left=$cell.Left; width=$cell.Width}
}

function Close-Workbook {
    <#
    .SYNOPSIS
    Closes a disposable workbook and confirms exit of its owned Excel process.
    .DESCRIPTION
    The Rust caller bounds this operation. If COM stalls, terminating the command
    host closes its job handle and the OS releases the owned Excel instances.
    #>
    param([string]$Lane)
    $session = $sessions[$Lane]
    if ($null -eq $session) { return @{} }
    $session.workbook.Close($false)
    $session.control.Close($false)
    $session.excel.Quit()
    if (-not $session.process.WaitForExit(3000)) {
        if (-not [CaptureProcesses]::Stop($session.process)) { throw "Excel exit unconfirmed: $Lane" }
    }
    $null = $sessions.Remove($Lane)
    return @{closed=$true}
}

function Invoke-Command {
    param($Command)
    switch ($Command.kind) {
        'open' { return Open-Workbook $Command.lane $Command.path }
        'save' { return Save-Workbook $Command.lane }
        'optimize' { return Invoke-WorkbookPerformanceOptimization $sessions[$Command.lane] $Command.timeout_seconds }
        'recalculate' { return Recalculate-Workbook $Command.lane }
        'capture' { return Capture-Region $Command.lane $Command.sheet $Command.range $Command.output }
        'measure' { return Measure-Region $Command.lane $Command.sheet $Command.range }
        'close' { return Close-Workbook $Command.lane }
        default { throw "Unknown comparison command: $($Command.kind)" }
    }
}

try {
    while ($null -ne ($line = [Console]::ReadLine())) {
        try {
            $command = $line | ConvertFrom-Json
            $result = Invoke-Command $command
            [Console]::WriteLine((@{ok=$true; result=$result} | ConvertTo-Json -Depth 5 -Compress))
        }
        catch {
            [Console]::WriteLine((@{ok=$false; error=$_.Exception.Message} | ConvertTo-Json -Compress))
        }
    }
}
finally {
    try {
        foreach ($lane in @($sessions.Keys)) { Close-Workbook $lane | Out-Null }
    } finally {
        $ownedExcel.Dispose()
    }
}
