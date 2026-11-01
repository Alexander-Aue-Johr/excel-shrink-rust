param(
    [string]$OutputDirectory = (Join-Path $env:TEMP 'excel-shrink-row-top-change')
)

$ErrorActionPreference = 'Stop'

function Save-RecalculatedCopy {
    <#
    .SYNOPSIS
    Recalculates a writable workbook and saves Excel's resulting XML.
    .DESCRIPTION
    Measures the effective height before and after a full rebuild, saves the
    workbook, and closes it even if recalculation or saving fails.
    .OUTPUTS
    A record containing the output path and both measured heights.
    #>
    param($Excel, [string]$WorkbookPath)

    $workbook = $null
    try {
        $workbook = $Excel.Workbooks.Open($WorkbookPath, 0, $false)
        $before = [double]$workbook.Worksheets.Item('Rows').Rows.Item(500000).RowHeight
        $Excel.CalculateFullRebuild()
        $after = [double]$workbook.Worksheets.Item('Rows').Rows.Item(500000).RowHeight
        $workbook.Save()
        return [pscustomobject]@{ path = $WorkbookPath; before = $before; after = $after }
    }
    finally {
        if ($null -ne $workbook) { $workbook.Close($false) }
    }
}

function Invoke-RecalcCheck {
    <#
    .SYNOPSIS
    Copies the changed workbook, recalculates it, and reports its row height.
    .DESCRIPTION
    Leaves the original comparison workbooks untouched and closes its private
    Excel process after the saved copy has been measured.
    #>
    param([string]$Directory)

    $source = Join-Path $Directory 'removed-empty-cell.xlsx'
    $destination = Join-Path $Directory 'removed-empty-cell-recalculated.xlsx'
    Copy-Item -LiteralPath $source -Destination $destination -Force

    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    try {
        Save-RecalculatedCopy $excel $destination | ConvertTo-Json -Compress
    }
    finally {
        $excel.Quit()
    }
}

Invoke-RecalcCheck -Directory $OutputDirectory
