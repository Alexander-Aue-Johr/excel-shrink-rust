param(
    [string]$OutputDirectory = (Join-Path $env:TEMP 'excel-shrink-row-top-change')
)

$ErrorActionPreference = 'Stop'

function New-Fixture {
    <#
    .SYNOPSIS
    Creates an Excel workbook and two XML variants with one empty-cell difference.
    .DESCRIPTION
    Excel establishes the large-font style and its row-height cache. The XML
    helper populates all 16,384 columns with empty cells, then removes only
    the uniquely styled cell from the changed copy.
    #>
    param($Excel, [string]$SourcePath, [string]$BaselinePath,
        [string]$ChangedPath, [int]$SentinelRow, [int]$LastRow)

    $workbook = $Excel.Workbooks.Add()
    try {
        $sheet = $workbook.Worksheets.Item(1)
        $sheet.Name = 'Rows'
        $sheet.Cells.Item(1, 1).Value2 = 'Start'
        $sheet.Cells.Item($LastRow, 1).Value2 = 'Last content row'
        $sheet.Range("XFD$SentinelRow").Font.Size = 40
        $workbook.SaveAs($SourcePath, 51)
    }
    finally {
        $workbook.Close($false)
    }

    & python.exe (Join-Path $PSScriptRoot 'make_variants.py') `
        $SourcePath $BaselinePath $ChangedPath
    if ($LASTEXITCODE -ne 0) {
        throw "Could not create the XLSX variants (Python exit $LASTEXITCODE)"
    }
}

function Get-RowTop {
    param($Sheet, [int]$Row)
    return [double]$Sheet.Cells.Item($Row, 1).Top
}

function Find-FirstDisplacedRow {
    <#
    .SYNOPSIS
    Locates the first row whose Excel-calculated Top differs between sheets.
    .DESCRIPTION
    Binary search works for this fixture because one removed cell changes one
    row height, leaving a constant position offset in every later row. Other
    height changes could cancel and invalidate this search condition.
    .OUTPUTS
    A record containing the row, number of comparisons, and elapsed time.
    #>
    param($OriginalSheet, $ChangedSheet, [int]$LastRow)

    $timer = [Diagnostics.Stopwatch]::StartNew()
    $low = 1
    $high = $LastRow
    $queries = 0
    while ($low -lt $high) {
        $middle = [int][Math]::Floor(($low + $high) / 2)
        $delta = (Get-RowTop $ChangedSheet $middle) -
            (Get-RowTop $OriginalSheet $middle)
        $queries++
        if ([Math]::Abs($delta) -gt 0.01) {
            $high = $middle
        }
        else {
            $low = $middle + 1
        }
    }
    $timer.Stop()
    return [pscustomobject]@{
        Row = $low
        Queries = $queries
        Milliseconds = $timer.Elapsed.TotalMilliseconds
    }
}

function Measure-Comparison {
    <#
    .SYNOPSIS
    Compares Excel's effective row geometry in two read-only workbooks.
    .DESCRIPTION
    Opens both variants, measures the changed row and its neighbors, then
    verifies that the first displaced row follows the removed font cell.
    Closes both workbooks even when a measurement fails.
    .OUTPUTS
    A result record with geometry, search timing, and workbook paths.
    #>
    param($Excel, [string]$BaselinePath, [string]$ChangedPath,
        [int]$SentinelRow, [int]$LastRow)

    $baseline = $null
    $changed = $null
    try {
        $baseline = $Excel.Workbooks.Open($BaselinePath, 0, $true)
        $changed = $Excel.Workbooks.Open($ChangedPath, 0, $true)
        $originalRows = $baseline.Worksheets.Item('Rows')
        $changedRows = $changed.Worksheets.Item('Rows')
        $originalHeight = [double]$originalRows.Rows.Item($SentinelRow).RowHeight
        $changedHeight = [double]$changedRows.Rows.Item($SentinelRow).RowHeight
        $sentinelOriginalTop = Get-RowTop $originalRows $SentinelRow
        $sentinelChangedTop = Get-RowTop $changedRows $SentinelRow
        $nextOriginalTop = Get-RowTop $originalRows ($SentinelRow + 1)
        $nextChangedTop = Get-RowTop $changedRows ($SentinelRow + 1)
        $lastOriginalTop = Get-RowTop $originalRows $LastRow
        $lastChangedTop = Get-RowTop $changedRows $LastRow
        $search = Find-FirstDisplacedRow $originalRows $changedRows $LastRow
        if ($search.Row -ne $SentinelRow + 1 -or $changedHeight -ge $originalHeight) {
            throw "Unexpected row height or first displaced row: $changedHeight, $($search.Row)"
        }

        return [pscustomobject]@{
            last_content_row = $LastRow
            removed_empty_cell = "XFD$SentinelRow"
            empty_cells_before = 16384
            empty_cells_after = 16383
            original_height_points = $originalHeight
            changed_height_points = $changedHeight
            changed_row_top_before_points = $sentinelOriginalTop
            changed_row_top_after_points = $sentinelChangedTop
            next_row_top_before_points = $nextOriginalTop
            next_row_top_after_points = $nextChangedTop
            last_top_before_points = $lastOriginalTop
            last_top_after_points = $lastChangedTop
            first_displaced_row = $search.Row
            binary_search_queries = $search.Queries
            binary_search_ms = $search.Milliseconds
            baseline = $BaselinePath
            changed = $ChangedPath
        }
    }
    finally {
        if ($null -ne $changed) { $changed.Close($false) }
        if ($null -ne $baseline) { $baseline.Close($false) }
    }
}

function Invoke-Experiment {
    <#
    .SYNOPSIS
    Creates the paired XLSX files, measures their layout, and records the result.
    .DESCRIPTION
    Owns the private Excel process. Generated workbooks and JSON stay in the
    requested output directory; Excel is closed even when the test fails.
    #>
    param([string]$Directory)

    $lastRow = 1000000
    $sentinelRow = 500000
    $sourcePath = Join-Path $Directory 'excel-generated.xlsx'
    $baselinePath = Join-Path $Directory 'baseline.xlsx'
    $changedPath = Join-Path $Directory 'removed-empty-cell.xlsx'
    New-Item -ItemType Directory -Force -Path $Directory | Out-Null

    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    try {
        New-Fixture $excel $sourcePath $baselinePath $changedPath $sentinelRow $lastRow
        $result = Measure-Comparison $excel $baselinePath $changedPath $sentinelRow $lastRow
        $result | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $Directory 'result.json')
        $result | ConvertTo-Json -Compress
    }
    finally {
        $excel.Quit()
    }
}

Invoke-Experiment -Directory $OutputDirectory
