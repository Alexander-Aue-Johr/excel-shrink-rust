param(
    [Parameter(Mandatory=$true)]
    [string]$FileListPath,

    [Parameter(Mandatory=$true)]
    [string]$PlanPath,

    [ValidateRange(1, 20)]
    [int]$SheetsPerWorkbook = 3,

    [switch]$AllVisibleSheets,

    [ValidateRange(1, 1048576)]
    [int]$MaxCaptureRows = 500,

    [ValidateRange(1, 16384)]
    [int]$MaxCaptureColumns = 64
)

$ErrorActionPreference = 'Stop'
$StrictUtf8 = New-Object System.Text.UTF8Encoding($false, $true)
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Release-ComObject($Object) {
    if (
        $null -ne $Object -and
        [Runtime.InteropServices.Marshal]::IsComObject($Object)
    ) {
        try {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Object)
        }
        catch {
        }
    }
}

function Assert-TsvField([string]$Value, [string]$Label) {
    if ($Value.Contains("`t") -or $Value.Contains("`r") -or $Value.Contains("`n")) {
        throw "$Label contains a tab or newline and cannot be written to the TSV plan: $Value"
    }
}

function Convert-ExcelColumnNumberToName([int]$ColumnNumber) {
    if ($ColumnNumber -lt 1 -or $ColumnNumber -gt 16384) {
        throw "Excel column number is out of range: $ColumnNumber"
    }

    $value = [int]$ColumnNumber
    $name = ''

    while ($value -gt 0) {
        $value--

        # Windows PowerShell 5.1 promotes Math.Floor results to Double.
        # Keep the Excel base-26 conversion explicitly integral so [char]
        # never receives values such as 65.0.
        $remainder = [int]($value % 26)
        $characterCode = [int](65 + $remainder)
        $name = ([char]$characterCode).ToString() + $name
        $value = [int]([Math]::Floor([double]$value / 26.0))
    }

    return $name
}

$paths = @(
    [IO.File]::ReadAllLines($FileListPath, $StrictUtf8) |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_.Length -gt 0 }
)

if ($paths.Count -eq 0) {
    throw "File list is empty: $FileListPath"
}

$rows = New-Object 'System.Collections.Generic.List[string]'
$rows.Add(
    "id`tworkbook_path`tworkbook_name`tsheet_name`taddress`tused_rows`tused_columns`tshape_count`trelative_png`treason"
)

$excel = $null
$workbooks = $null
$workbook = $null
$captureIndex = 0

try {
    $excel = New-Object -ComObject Excel.Application
    if(Get-Command Register-VVDecisionExcel -ErrorAction SilentlyContinue){Register-VVDecisionExcel $excel}
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $excel.EnableEvents = $false
    $excel.AskToUpdateLinks = $false
    $workbooks = $excel.Workbooks

    foreach ($path in $paths) {
        $workbook = $workbooks.Open(
            $path,
            0,
            $true
        )

        $workbookName = [string]$workbook.Name
        $worksheetCollection = $workbook.Worksheets
        $candidates = New-Object 'System.Collections.Generic.List[object]'

        try {
            for ($index = 1; $index -le $worksheetCollection.Count; $index++) {
                $worksheet = $null
                $usedRange = $null
                $usedRows = $null
                $usedColumns = $null
                $shapes = $null

                try {
                    $worksheet = $worksheetCollection.Item($index)

                    # xlSheetVisible = -1
                    if ([int]$worksheet.Visible -ne -1) {
                        continue
                    }

                    $usedRange = $worksheet.UsedRange
                    $usedRows = $usedRange.Rows
                    $usedColumns = $usedRange.Columns
                    $shapes = $worksheet.Shapes

                    $rowCount = [int]$usedRows.Count
                    $columnCount = [int]$usedColumns.Count
                    $shapeCount = [int]$shapes.Count
                    $area = [double]$rowCount * [double]$columnCount

                    # UsedRange can be pathological (whole rows/columns or nearly an
                    # entire worksheet) because of stale formatting. Keep the sheet
                    # selection based on the real UsedRange, but benchmark a bounded
                    # top-left slice so both PNG APIs receive a renderable range.
                    $startRow = [int]$usedRange.Row
                    $startColumn = [int]$usedRange.Column
                    $endRow = [Math]::Min(
                        1048576,
                        $startRow + [Math]::Min($rowCount, $MaxCaptureRows) - 1
                    )
                    $endColumn = [Math]::Min(
                        16384,
                        $startColumn + [Math]::Min($columnCount, $MaxCaptureColumns) - 1
                    )

                    $address = (
                        (Convert-ExcelColumnNumberToName $startColumn) +
                        [string]$startRow +
                        ':' +
                        (Convert-ExcelColumnNumberToName $endColumn) +
                        [string]$endRow
                    )

                    $candidates.Add(
                        [pscustomobject]@{
                            Index = $index
                            Name = [string]$worksheet.Name
                            Address = $address
                            UsedRows = $rowCount
                            UsedColumns = $columnCount
                            ShapeCount = $shapeCount
                            Area = $area
                        }
                    )
                }
                finally {
                    Release-ComObject $shapes
                    Release-ComObject $usedColumns
                    Release-ComObject $usedRows
                    Release-ComObject $usedRange
                    Release-ComObject $worksheet
                }
            }
        }
        finally {
            Release-ComObject $worksheetCollection
        }

        $selected = New-Object 'System.Collections.Generic.List[object]'
        $selectedNames = @{}

        function Add-Candidate($Candidate, [string]$Reason) {
            if ($null -eq $Candidate) {
                return
            }

            $key = ([string]$Candidate.Name).ToLowerInvariant()

            if (-not $selectedNames.ContainsKey($key)) {
                $selectedNames[$key] = $true
                $selected.Add(
                    [pscustomobject]@{
                        Candidate = $Candidate
                        Reason = $Reason
                    }
                )
            }
        }

        if ($AllVisibleSheets) {
            foreach ($candidate in ($candidates | Sort-Object Index)) {
                Add-Candidate $candidate 'all-visible'
            }
        }
        elseif ($candidates.Count -gt 0) {
            Add-Candidate `
                ($candidates | Sort-Object Index | Select-Object -First 1) `
                'first-visible'

            if ($selected.Count -lt $SheetsPerWorkbook) {
                Add-Candidate `
                    ($candidates |
                        Sort-Object `
                            @{ Expression = 'ShapeCount'; Descending = $true },
                            @{ Expression = 'Index'; Descending = $false } |
                        Select-Object -First 1) `
                    'most-shapes'
            }

            if ($selected.Count -lt $SheetsPerWorkbook) {
                Add-Candidate `
                    ($candidates |
                        Sort-Object `
                            @{ Expression = 'Area'; Descending = $true },
                            @{ Expression = 'Index'; Descending = $false } |
                        Select-Object -First 1) `
                    'largest-used-range'
            }

            foreach ($candidate in ($candidates | Sort-Object Index)) {
                if ($selected.Count -ge $SheetsPerWorkbook) {
                    break
                }

                Add-Candidate $candidate 'fill'
            }
        }

        foreach ($entry in $selected) {
            $candidate = $entry.Candidate
            $captureIndex++
            $captureId = 'c{0:D4}' -f $captureIndex
            $relativePng = "$captureId.png"

            Assert-TsvField $path 'Workbook path'
            Assert-TsvField $workbookName 'Workbook name'
            Assert-TsvField $candidate.Name 'Worksheet name'
            Assert-TsvField $candidate.Address 'Range address'

            $rows.Add(
                @(
                    $captureId,
                    $path,
                    $workbookName,
                    $candidate.Name,
                    $candidate.Address,
                    [string]$candidate.UsedRows,
                    [string]$candidate.UsedColumns,
                    [string]$candidate.ShapeCount,
                    $relativePng,
                    $entry.Reason
                ) -join "`t"
            )
        }

        $workbook.Close($false)
        Release-ComObject $workbook
        $workbook = $null
    }
}
finally {
    if ($null -ne $workbook) {
        try { $workbook.Close($false) } catch {}
    }

    Release-ComObject $workbook
    Release-ComObject $workbooks

    if ($null -ne $excel) {
        try { $excel.Quit() } catch {}
    }

    Release-ComObject $excel

    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}

$parent = Split-Path -Parent $PlanPath
if (-not [string]::IsNullOrWhiteSpace($parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
}

[IO.File]::WriteAllText(
    $PlanPath,
    (($rows -join "`n") + "`n"),
    $Utf8NoBom
)

Write-Host 'Capture plan:'
Write-Host "  $PlanPath"
Write-Host "  Captures: $captureIndex"
Write-Host "  Capture cap: $MaxCaptureRows rows x $MaxCaptureColumns columns"
