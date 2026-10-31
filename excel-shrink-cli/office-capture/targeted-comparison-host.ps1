# Persistent Excel sessions for the targeted comparison CLI. One process per lane.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ExcelProcessId {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr handle, out uint processId);
}
'@

$sessions = @{}

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
    $excel.Visible = $true
    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $control = $null
    $workbook = $null
    try {
        $control = $excel.Workbooks.Add()
        $excel.Calculation = -4135  # xlCalculationManual
        $workbook = $excel.Workbooks.Open($Path, 0, $false)
        [uint32]$processId = 0
        $null = [ExcelProcessId]::GetWindowThreadProcessId(
            [IntPtr]$excel.Hwnd, [ref]$processId)
        $sessions[$Lane] = @{excel=$excel; control=$control; workbook=$workbook}
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
    The clipboard belongs to the active Windows desktop. This host processes
    commands sequentially so concurrent lanes cannot overwrite each other's
    picture while it is being copied. The caller owns the PNG afterward.
    #>
    param([string]$Lane, [string]$Sheet, [string]$Range, [string]$Output)

    $session = $sessions[$Lane]
    if ($null -eq $session) { throw "Excel lane is not open: $Lane" }
    $worksheet = $session.workbook.Worksheets.Item($Sheet)
    $worksheet.Activate()
    $null = $worksheet.Range($Range).CopyPicture(1, 2)
    Start-Sleep -Milliseconds 100
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
    param([string]$Lane)
    $session = $sessions[$Lane]
    if ($null -eq $session) { return @{} }
    $session.workbook.Close($false)
    $session.control.Close($false)
    $session.excel.Quit()
    $null = $sessions.Remove($Lane)
    return @{}
}

function Invoke-Command {
    param($Command)
    switch ($Command.kind) {
        'open' { return Open-Workbook $Command.lane $Command.path }
        'save' { return Save-Workbook $Command.lane }
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
    foreach ($lane in @($sessions.Keys)) { Close-Workbook $lane | Out-Null }
}
