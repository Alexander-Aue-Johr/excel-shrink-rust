param(
    [Parameter(Mandatory=$true)]
    [string]$FileListPath,

    [Parameter(Mandatory=$true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'


function Write-Utf8NoBom([string]$Path, [string]$Text) {
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($Path, $Text, $encoding)
}


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


function Quote-Json([string]$Value) {
    return ($Value | ConvertTo-Json -Compress)
}


function Invariant-Fixed([double]$Value) {
    return $Value.ToString(
        '0.000000',
        [Globalization.CultureInfo]::InvariantCulture
    )
}


function Measure-Worksheet($Worksheet) {
    $maxRight = 0.0
    $maxBottom = 0.0

    $shapes = $null

    try {
        $shapes = $Worksheet.Shapes
        $count = [int]$shapes.Count

        for ($index = 1; $index -le $count; $index++) {
            $shape = $null

            try {
                $shape = $shapes.Item($index)

                if ([int]$shape.Visible -eq 0) {
                    continue
                }

                $left = [double]$shape.Left
                $top = [double]$shape.Top
                $width = [double]$shape.Width
                $height = [double]$shape.Height

                $angle = [double]$shape.Rotation * [Math]::PI / 180.0

                $rotatedWidth =
                    [Math]::Abs($width * [Math]::Cos($angle)) +
                    [Math]::Abs($height * [Math]::Sin($angle))

                $rotatedHeight =
                    [Math]::Abs($width * [Math]::Sin($angle)) +
                    [Math]::Abs($height * [Math]::Cos($angle))

                $right =
                    $left +
                    $width / 2.0 +
                    $rotatedWidth / 2.0

                $bottom =
                    $top +
                    $height / 2.0 +
                    $rotatedHeight / 2.0

                if ($right -gt $maxRight) {
                    $maxRight = $right
                }

                if ($bottom -gt $maxBottom) {
                    $maxBottom = $bottom
                }
            }
            finally {
                Release-ComObject $shape
            }
        }
    }
    finally {
        Release-ComObject $shapes
    }

    return @($maxRight, $maxBottom)
}


$paths = @(
    Get-Content -LiteralPath $FileListPath |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_.Length -gt 0 }
)

if ($paths.Count -eq 0) {
    throw "File list is empty: $FileListPath"
}

$excel = $null
$workbooks = $null

try {
    $excel = New-Object -ComObject Excel.Application

    if(Get-Command Register-VVDecisionExcel -ErrorAction SilentlyContinue){Register-VVDecisionExcel $excel}
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $excel.EnableEvents = $false

    Write-Host ("Excel version: " + $excel.Version)

    $workbooks = $excel.Workbooks

    $json = New-Object Text.StringBuilder
    [void]$json.AppendLine('{')

    $firstWorkbook = $true
    $seen = @{}

    foreach ($path in $paths) {
        $workbook = $null
        $worksheets = $null

        try {
            $workbook = $workbooks.Open(
                $path,
                0,
                $true
            )

            $workbookName = [string]$workbook.Name
            $key = $workbookName.ToLowerInvariant()

            if ($seen.ContainsKey($key)) {
                throw "Duplicate workbook filename in corpus: $workbookName"
            }

            $seen[$key] = $true

            if (-not $firstWorkbook) {
                [void]$json.AppendLine(',')
            }

            $firstWorkbook = $false

            [void]$json.Append(
                '  ' + (Quote-Json $workbookName) + ': {' + "`n"
            )

            $worksheets = $workbook.Worksheets
            $sheetCount = [int]$worksheets.Count

            $firstSheet = $true

            for ($sheetIndex = 1; $sheetIndex -le $sheetCount; $sheetIndex++) {
                $worksheet = $null

                try {
                    $worksheet = $worksheets.Item($sheetIndex)
                    $sheetName = [string]$worksheet.Name

                    $stopwatch = [Diagnostics.Stopwatch]::StartNew()
                    $bounds = Measure-Worksheet $worksheet
                    $stopwatch.Stop()

                    $durationMs = $stopwatch.Elapsed.TotalMilliseconds

                    if (-not $firstSheet) {
                        [void]$json.AppendLine(',')
                    }

                    $firstSheet = $false

                    [void]$json.AppendLine(
                        '    ' + (Quote-Json $sheetName) + ': {'
                    )

                    [void]$json.AppendLine(
                        '      "right": ' +
                        (Invariant-Fixed $bounds[0]) +
                        ','
                    )

                    [void]$json.AppendLine(
                        '      "bottom": ' +
                        (Invariant-Fixed $bounds[1]) +
                        ','
                    )

                    [void]$json.Append(
                        '      "duration_ms": ' +
                        (Invariant-Fixed $durationMs) +
                        "`n" +
                        '    }'
                    )
                }
                finally {
                    Release-ComObject $worksheet
                }
            }

            [void]$json.Append("`n  }")

            $workbook.Close($false)
            Release-ComObject $workbook
            $workbook = $null
        }
        finally {
            if ($null -ne $workbook) {
                try { $workbook.Close($false) } catch {}
            }

            Release-ComObject $worksheets
            Release-ComObject $workbook
        }
    }

    [void]$json.AppendLine()
    [void]$json.AppendLine('}')

    Write-Utf8NoBom $OutputPath $json.ToString()
}
finally {
    Release-ComObject $workbooks

    if ($null -ne $excel) {
        try { $excel.Quit() } catch {}
    }

    Release-ComObject $excel

    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}
