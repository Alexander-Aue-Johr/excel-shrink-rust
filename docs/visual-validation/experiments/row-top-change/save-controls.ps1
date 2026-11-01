param(
    [string]$OutputDirectory = (Join-Path $env:TEMP 'excel-shrink-row-top-change')
)

$ErrorActionPreference = 'Stop'

function Get-LiveState {
    param($Workbook)

    $sheet = $Workbook.Worksheets.Item('Rows')
    return [pscustomobject]@{
        cached_formula_value = $sheet.Range('B2').Value2
        row_height_points = [double]$sheet.Rows.Item(500000).RowHeight
        last_top_points = [double]$sheet.Cells.Item(1000000, 1).Top
    }
}

function Test-SaveSequence {
    <#
    .SYNOPSIS
    Measures opening, SaveCopyAs, Save, and explicit Full Rebuild in one Excel session.
    .DESCRIPTION
    Works on a copy of the input. Writes a SaveCopyAs snapshot before Save,
    then a second snapshot after Full Rebuild, without reopening the workbook.
    A temporary blank workbook sets the application calculation mode before
    the test workbook is opened.
    .OUTPUTS
    The observed in-memory states and paths to all written workbooks.
    #>
    param([string]$InputPath, [string]$CaseDirectory, [int]$CalculationMode)

    New-Item -ItemType Directory -Force -Path $CaseDirectory | Out-Null
    $working = Join-Path $CaseDirectory 'saved.xlsx'
    $saveCopy = Join-Path $CaseDirectory 'save-copy-as.xlsx'
    $recalculated = Join-Path $CaseDirectory 'recalculated.xlsx'
    Copy-Item -LiteralPath $InputPath -Destination $working -Force

    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $control = $null
    $workbook = $null
    try {
        $control = $excel.Workbooks.Add()
        $excel.Calculation = $CalculationMode
        $workbook = $excel.Workbooks.Open($working, 0, $false)
        $opened = Get-LiveState $workbook
        $actualMode = $excel.Calculation

        $workbook.SaveCopyAs($saveCopy)
        $afterSaveCopy = Get-LiveState $workbook
        $workbook.Save()
        $afterSave = Get-LiveState $workbook

        $excel.CalculateFullRebuild()
        $afterRebuild = Get-LiveState $workbook
        $workbook.SaveCopyAs($recalculated)

        return [pscustomobject]@{
            input = $InputPath
            calculation_mode_on_open = $actualMode
            opened = $opened
            after_save_copy_as = $afterSaveCopy
            after_save = $afterSave
            after_full_rebuild = $afterRebuild
            saved = $working
            save_copy_as = $saveCopy
            recalculated = $recalculated
        }
    }
    finally {
        if ($null -ne $workbook) { $workbook.Close($false) }
        if ($null -ne $control) { $control.Close($false) }
        $excel.Quit()
    }
}

function Get-ReopenedState {
    <#
    .SYNOPSIS
    Reads Excel's layout after reopening one completed saved workbook.
    .DESCRIPTION
    Uses manual calculation to avoid an explicit rebuild during this check.
    The test also records the actual Excel calculation mode after opening.
    #>
    param([string]$WorkbookPath)

    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $control = $null
    $workbook = $null
    try {
        $control = $excel.Workbooks.Add()
        $excel.Calculation = -4135  # xlCalculationManual
        $workbook = $excel.Workbooks.Open($WorkbookPath, 0, $true)
        return [pscustomobject]@{
            path = $WorkbookPath
            calculation_mode = $excel.Calculation
            state = Get-LiveState $workbook
        }
    }
    finally {
        if ($null -ne $workbook) { $workbook.Close($false) }
        if ($null -ne $control) { $control.Close($false) }
        $excel.Quit()
    }
}

function Invoke-SaveControls {
    <#
    .SYNOPSIS
    Runs opening and save controls on stale-formula and empty-font-cell fixtures.
    .DESCRIPTION
    Generates two calculation-mode variants, then compares their live and
    reopened states. All outputs stay in the supplied temporary directory.
    #>
    param([string]$Directory)

    $source = Join-Path $Directory 'excel-generated.xlsx'
    & python.exe (Join-Path $PSScriptRoot 'make_opening_fixtures.py') $source $Directory
    if ($LASTEXITCODE -ne 0) {
        throw "Could not create opening fixtures (Python exit $LASTEXITCODE)"
    }

    $cases = @(
        @{ name = 'manual'; file = 'opening-manual.xlsx'; mode = -4135 },
        @{ name = 'auto'; file = 'opening-auto.xlsx'; mode = -4105 },
        @{ name = 'auto-full-on-load'; file = 'opening-auto-full-on-load.xlsx'; mode = -4105 },
        @{ name = 'removed-empty-cell'; file = 'removed-empty-cell.xlsx'; mode = -4135 }
    )
    $results = foreach ($case in $cases) {
        $inputPath = Join-Path $Directory $case.file
        $caseDirectory = Join-Path $Directory ('save-control-' + $case.name)
        $sequence = Test-SaveSequence $inputPath $caseDirectory $case.mode
        $reopened = @(
            Get-ReopenedState $sequence.saved
            Get-ReopenedState $sequence.save_copy_as
        )
        [pscustomobject]@{ case = $case.name; sequence = $sequence; reopened = $reopened }
    }
    $results | ConvertTo-Json -Depth 7 |
        Set-Content -Encoding UTF8 (Join-Path $Directory 'save-controls.json')
    $results | ConvertTo-Json -Depth 7 -Compress
}

Invoke-SaveControls -Directory $OutputDirectory
