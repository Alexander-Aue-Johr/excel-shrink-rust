param(
    [string]$Repository = (Join-Path $PSScriptRoot '..\..'),
    [switch]$LiveExcel
)
# Parser checks are safe without Excel. -LiveExcel creates a temporary test workbook
# and opens its OWN Excel instances. It never edits a user workbook. Use an interactive
# desktop, and do not change the clipboard while this test is running.
$ErrorActionPreference = 'Stop'
$Repository = (Resolve-Path -LiteralPath $Repository).Path
$source = Join-Path $Repository 'excel-shrink-cli\src\regression'
foreach ($name in @('excel_screenshots.ps1', 'excel_visual_bounds.ps1', 'excel_visual_clipboard.ps1', 'excel_recalculate.ps1', 'excel_render_environment.ps1')) {
    $tokens = $null; $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile((Join-Path $source $name), [ref]$tokens, [ref]$errors)
    if ($errors.Count -gt 0) { throw "$name parse failure: $($errors | Out-String)" }
    Write-Host "PASS parser: $name"
}
if (-not $LiveExcel) { Write-Host 'Live Excel tests skipped. Use -LiveExcel on an interactive Windows desktop.'; return }
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
$test = Join-Path ([IO.Path]::GetTempPath()) ('es-vtest-' + [guid]::NewGuid().ToString('N').Substring(0,10))
New-Item -ItemType Directory -Path $test | Out-Null
$fixture = Join-Path $test 'fixture.xlsx'
$excel = $null; $book = $null
$expectedShapeEnd = 0
try {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $true; $excel.DisplayAlerts = $false; $excel.AutomationSecurity = 3
    $book = $excel.Workbooks.Add()
    while ($book.Worksheets.Count -gt 1) { $book.Worksheets.Item($book.Worksheets.Count).Delete() }
    $sheet = $book.Worksheets.Item(1); $sheet.Name = 'OverflowAndChart'
    $sheet.Cells.Item(1,1).Value2 = 'A deliberately long overflow text that must remain visible beyond the first cell.'
    $sheet.Cells.Item(1,1).Font.Size = 24
    $sheet.Columns.Item(1).ColumnWidth = 5
    # A mixed-size cell reproduces the nullable Font.Size path without relying on
    # a damaged line. This must never be cast directly from DBNull to Single.
    $mixed = $sheet.Cells.Item(2,1)
    $mixed.Value2 = 'Small LARGE mixed-font overflow must remain visible.'
    $mixed.Font.Size = 11
    $mixed.Characters(7,5).Font.Size = 28
    # A mass-line fixture exercises the same drawing-primitives workload as 71717-49.
    for ($lineIndex=0; $lineIndex -lt 1920; $lineIndex++) {
        $x = 20 + ($lineIndex % 20) * 2; $y = 45 + [Math]::Floor($lineIndex / 20) * 0.5
        [void]$sheet.Shapes.AddLine($x,$y,$x+12,$y)
    }
    $values = New-Object 'object[,]' 4,2
    $values[0,0]='X'; $values[0,1]='Y'
    $values[1,0]=1.0; $values[1,1]=5.0
    $values[2,0]=2.0; $values[2,1]=8.0
    $values[3,0]=3.0; $values[3,1]=4.0
    $sheet.Range('A3:B6').Value2 = $values
    # Position beyond the cells with values: UsedRange must not be the only bound.
    $anchor = $sheet.Range('G10')
    $embedded = $sheet.ChartObjects().Add($anchor.Left, $anchor.Top, 320, 180)
    $embedded.Chart.SetSourceData($sheet.Range('A3:B6'))
    $embedded.Chart.ChartType = 51
    $embedded.Chart.HasTitle = $true; $embedded.Chart.ChartTitle.Text = 'CHART OUTSIDE USED RANGE'
    $expectedShapeEnd = [int]$embedded.BottomRightCell.Row
    # Bright solid sentinel proves objects are actually included in the range image.
    $shape = $sheet.Shapes.AddShape(1, $anchor.Left + 330, $anchor.Top, 60, 60)
    $shape.Fill.ForeColor.RGB = 16711935
    $shape.Line.Visible = 0
    $expectedShapeColumn = [int]$shape.BottomRightCell.Column
    $chartSheet = $book.Charts.Add()
    $chartSheet.Name = 'StandaloneChart'
    $chartSheet.SetSourceData($sheet.Range('A3:B6')); $chartSheet.ChartType = 51
    $book.SaveAs($fixture, 51)
} finally {
    if ($null -ne $book) { try { $book.Close($false) } catch { } }
    if ($null -ne $excel) { try { $excel.Quit() } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel) } }
    $book=$null; $excel=$null; [GC]::Collect(); [GC]::WaitForPendingFinalizers()
}
$limits = Join-Path $test 'limits.json'
ConvertTo-Json -InputObject @([pscustomobject]@{sheet_index=1;max_row=6;max_column=2;max_cell='B6'}) | Set-Content $limits -Encoding UTF8
$manifest = Join-Path $test 'captures.json'
$runner = Join-Path $source 'excel_screenshots.ps1'
try {
    # Separate PowerShell process avoids leaking script functions/variables into this test.
    & powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File $runner -OriginalPath $fixture -ShrunkPath $fixture -OutputDir $test -CaptureManifestPath $manifest -RangeLimitPath $limits -OriginalCacheDir (Join-Path $test 'original') -ShrunkCacheDir (Join-Path $test 'shrunk')
    if ($LASTEXITCODE -ne 0) { throw "Screenshot process failed with exit code $LASTEXITCODE" }
    $captures = @(Get-Content -LiteralPath $manifest -Raw -Encoding UTF8 | ConvertFrom-Json)
    if ($captures.Count -ne 2) { throw "Expected a worksheet and a chart sheet, got $($captures.Count)" }
    foreach ($capture in $captures) {
        if ($capture.screenshot_error) { throw "Capture failed: $($capture.screenshot_error)" }
        if (-not (Test-Path -LiteralPath $capture.original_image)) { throw "Missing image for $($capture.sheet_name)" }
    }
    if ($captures[0].capture_info.columns -lt $expectedShapeColumn -or $captures[0].capture_info.rows -lt $expectedShapeEnd) {
        throw 'The screenshot bounds exclude the embedded chart/shape.'
    }
    $bitmap = [Drawing.Bitmap]::FromFile([string]$captures[0].original_image)
    try {
        $magenta = 0
        for ($y=0; $y -lt $bitmap.Height; $y+=3) {
            for ($x=0; $x -lt $bitmap.Width; $x+=3) {
                $p=$bitmap.GetPixel($x,$y)
                if ($p.R -gt 220 -and $p.B -gt 220 -and $p.G -lt 50) { $magenta++ }
            }
        }
        if ($magenta -lt 30) {
            throw 'This Excel build did not include the visible sentinel shape in Range.CopyPicture. Do not accept chart/shape coverage without reviewing the saved screenshots.'
        }
    } finally { $bitmap.Dispose() }
    Write-Host 'PASS mixed-font/mass-line worksheet, bounds, floating-shape pixels, and standalone chart-sheet capture.'
    Write-Host 'Inspect the images for the complete sentence and embedded chart; text metrics are conservative, not pixel-perfect.'
    Write-Host "Artifacts retained: $test"
} catch {
    Write-Host "Failure artifacts retained: $test"
    throw
}
