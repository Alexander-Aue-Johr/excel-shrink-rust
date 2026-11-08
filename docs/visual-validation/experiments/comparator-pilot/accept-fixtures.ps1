# Run in a separate, externally time-bounded PowerShell process with TEMP/TMP on a working drive.
# Root contains the generated fixtures directory. Inputs are opened read-only and never saved.
param([Parameter(Mandatory = $true)][string]$Root)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PilotWindow {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
}
'@
function FileHash([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return [BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '')
    } finally {
        $stream.Dispose()
        $algorithm.Dispose()
    }
}
$excel = $null
$observations = @()
try {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $excel.AutomationSecurity = 3
    $excel.AskToUpdateLinks = $false
    $excel.AutoRecover.Enabled = $false
    [uint32]$excelPid = 0
    [void][PilotWindow]::GetWindowThreadProcessId([intptr]$excel.Hwnd, [ref]$excelPid)
    $owned = Get-Process -Id $excelPid
    @{
        pid = $excelPid
        started = $owned.StartTime.ToUniversalTime().ToString('o')
        executable = $owned.Path
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Root 'fixture-excel-owner.json') -Encoding utf8
    $manifest = Get-Content -LiteralPath (Join-Path $Root 'fixtures\fixtures.json') -Raw | ConvertFrom-Json
    foreach ($pair in $manifest.pairs) {
        foreach ($lane in @('original', 'candidate')) {
            $path = $pair.$lane
            $book = $null
            $watch = [Diagnostics.Stopwatch]::StartNew()
            try {
                $before = FileHash $path
                $expectedHash = $pair.sha256.([IO.Path]::GetFileName($path))
                if ($before -ne $expectedHash) {
                    throw "Frozen input hash changed for $($pair.id) $lane"
                }
                $book = $excel.Workbooks.Open($path, 0, $true)
                $sheet = $book.Worksheets.Item(1)
                $row = [ordered]@{
                    pair = $pair.id
                    lane = $lane
                    opened = $true
                    sheets = $book.Worksheets.Count
                    chart_count = $sheet.ChartObjects().Count
                    conditional_formats = $sheet.Cells.FormatConditions.Count
                    b1 = $sheet.Range('B1').Value2
                    open_seconds = $watch.Elapsed.TotalSeconds
                }
                if ($pair.id -eq 'offscreen-hidden') {
                    $row.b2000 = $sheet.Range('B2000').Value2
                    $row.b3000 = $sheet.Range('B3000').Value2
                    $row.row3000_hidden = $sheet.Rows.Item(3000).Hidden
                }
                if ($pair.id -eq 'direct-presentation') {
                    $row.row1_height = $sheet.Rows.Item(1).RowHeight
                    $row.column_a_width = $sheet.Columns.Item(1).ColumnWidth
                    $row.a1_color = $sheet.Range('A1').Interior.Color
                }
                $book.Close($false)
                [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($sheet)
                [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($book)
                $book = $null
                $row.source_unchanged = ((FileHash $path) -eq $before)
                $observations += [pscustomobject]$row
            } catch {
                $observations += [pscustomobject]@{
                    pair = $pair.id
                    lane = $lane
                    opened = $false
                    error = $_.Exception.Message
                }
                if ($null -ne $book) {
                    $book.Close($false)
                }
            }
            $observations | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $Root 'native-fixture-observations.json') -Encoding utf8
        }
    }
} finally {
    if ($null -ne $excel) {
        $excel.Quit()
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel)
    }
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}
if ($observations.Count -ne 10 -or @($observations | Where-Object {
    -not $_.opened -or -not $_.source_unchanged
}).Count -gt 0) {
    throw 'Native fixture acceptance failed. Inspect the retained observations.'
}
