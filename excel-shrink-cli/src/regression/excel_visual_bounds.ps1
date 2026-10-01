# Read-only screenshot geometry. OOXML plans avoid per-shape/per-row COM scans.
$script:VisualPlanners = @{}
function Test-VisualNull($value) {
    return ($null -eq $value -or $value -is [DBNull] -or $value -is [System.Reflection.Missing])
}
function Convert-VisualFiniteNumber($value) {
    if ((Test-VisualNull $value) -or ($value -is [string] -and [string]::IsNullOrWhiteSpace($value)) -or $value -is [bool]) { return $null }
    try {
        $number = [double]$value
        if ([double]::IsNaN($number) -or [double]::IsInfinity($number)) { return $null }
        return $number
    } catch { return $null }
}
function Get-VisualNumber($object, [string]$property, [string]$label) {
    $raw = $object.$property
    $value = Convert-VisualFiniteNumber $raw
    if ($null -eq $value) {
        $typeName = if ($null -eq $raw) { 'null' } else { $raw.GetType().FullName }
        throw "Invalid numeric property '$property' on $label (type=$typeName)"
    }
    return $value
}
function Initialize-VisualPlanner([string]$path) {
    try {
        $script:VisualPlanners[$path] = [ExcelShrink.Visual.WorkbookPlan]::new($path)
        Write-ProgressLine "bounds OOXML planner ready path='$path'"
    } catch { Write-ProgressLine "bounds OOXML planner unavailable path='$path' error='$($_.Exception.Message)'" }
}
function Close-VisualPlanners {
    foreach ($planner in $script:VisualPlanners.Values) { try { $planner.Dispose() } catch { } }
    $script:VisualPlanners.Clear()
}
function Get-WorksheetVisualPlan($worksheet, $baseRange) {
    $planner = $script:VisualPlanners[[string]$worksheet.Parent.FullName]
    if ($null -eq $planner) { return $null }
    try {
        [Action[string]]$progress = { param([string]$message) Write-ProgressLine $message }
        return ,$planner.Get([string]$worksheet.Name, ([int]$baseRange.Row + [int]$baseRange.Rows.Count - 1), ([int]$baseRange.Column + [int]$baseRange.Columns.Count - 1), $progress)
    } catch {
        Write-ProgressLine "bounds OOXML plan fallback worksheet='$($worksheet.Name)' error='$($_.Exception.Message)'"
        return $null
    }
}
function Find-VisualEndIndex($worksheet, [double]$point, [bool]$columns, [int]$minimum) {
    $count = if ($columns) { 16384 } else { 1048576 }
    $low = [Math]::Max(1, [Math]::Min($minimum, $count)); $high = $count
    $band = if ($columns) { $worksheet.Columns.Item($low) } else { $worksheet.Rows.Item($low) }
    $edge = if ($columns) { [double]$band.Left + [double]$band.Width } else { [double]$band.Top + [double]$band.Height }
    if ($edge -ge $point) { return $low }
    while ($low -lt $high) {
        $mid = [int][Math]::Floor(($low + $high) / 2.0)
        $band = if ($columns) { $worksheet.Columns.Item($mid) } else { $worksheet.Rows.Item($mid) }
        $edge = if ($columns) { [double]$band.Left + [double]$band.Width } else { [double]$band.Top + [double]$band.Height }
        if ($edge -ge $point) { $high = $mid } else { $low = $mid + 1 }
    }
    $band = if ($columns) { $worksheet.Columns.Item($low) } else { $worksheet.Rows.Item($low) }
    $edge = if ($columns) { [double]$band.Left + [double]$band.Width } else { [double]$band.Top + [double]$band.Height }
    if ($edge + 0.5 -lt $point) { throw "Visible content exceeds worksheet geometry (target=$point, edge=$edge)." }
    return $low
}
function Get-VisualFont($excelFont, $fonts) {
    $sizeValue = Convert-VisualFiniteNumber $excelFont.Size
    $rawName = $excelFont.Name
    if ($null -eq $sizeValue -or $sizeValue -le 0 -or $sizeValue -gt 4096 -or (Test-VisualNull $rawName) -or
        [string]::IsNullOrWhiteSpace([string]$rawName) -or (Test-VisualNull $excelFont.Bold) -or (Test-VisualNull $excelFont.Italic)) {
        return $null # A mixed font is not a default 11pt font. Measure its characters.
    }
    $name = [string]$rawName; $size = [single]$sizeValue
    $style = [System.Drawing.FontStyle]::Regular
    if ([bool]$excelFont.Bold) { $style = $style -bor [System.Drawing.FontStyle]::Bold }
    if ([bool]$excelFont.Italic) { $style = $style -bor [System.Drawing.FontStyle]::Italic }
    $key = "$name|$size|$style"
    if (-not $fonts.ContainsKey($key)) { $fonts[$key] = [System.Drawing.Font]::new($name, $size, $style, [System.Drawing.GraphicsUnit]::Point) }
    return $fonts[$key]
}
function Measure-VisualCellText($cell, [string]$text, $graphics, $fonts) {
    try { $excelFont = $cell.DisplayFormat.Font } catch { $excelFont = $cell.Font }
    $font = Get-VisualFont $excelFont $fonts
    $width = 0.0
    if ($null -eq $font) {
        Write-ProgressLine "bounds rich-text worksheet='$($cell.Worksheet.Name)' cell='$(Range-Address $cell)' characters=$($text.Length)"
        $lineWidth = 0.0; $timer = [Diagnostics.Stopwatch]::StartNew()
        for ($j = 0; $j -lt $text.Length; $j++) {
            if ($text[$j] -eq "`n") { $width = [Math]::Max($width, $lineWidth); $lineWidth = 0.0; continue }
            $characters = $cell.Characters($j + 1, 1)
            $characterFont = Get-VisualFont $characters.Font $fonts
            if ($null -eq $characterFont) { throw "Cannot resolve rich-text font at $(Range-Address $cell), character $($j + 1)." }
            $lineWidth += [double]$graphics.MeasureString([string]$text[$j], $characterFont).Width
            if ($timer.Elapsed.TotalSeconds -ge 2) { Write-ProgressLine "bounds rich-text progress character=$j total=$($text.Length)"; $timer.Restart() }
        }
        $width = [Math]::Max($width, $lineWidth)
    } else { $width = [double]$graphics.MeasureString($text, $font).Width }
    $indent = Convert-VisualFiniteNumber $cell.IndentLevel
    if ($null -eq $indent) { $indent = 0 }
    return ($width * 72.0 / [double]$graphics.DpiX * 1.20 + 12.0 + [Math]::Max(0, $indent) * 12.0)
}
function Get-VisualStyle($cell, $fonts) {
    $wrap = $cell.WrapText; $shrink = $cell.ShrinkToFit
    if (-not (Test-VisualNull $wrap) -and [bool]$wrap) { return [pscustomobject]@{ skip=$true } }
    if (-not (Test-VisualNull $shrink) -and [bool]$shrink) { return [pscustomobject]@{ skip=$true } }
    try { $font = Get-VisualFont $cell.DisplayFormat.Font $fonts } catch { $font = Get-VisualFont $cell.Font $fonts }
    if ($null -eq $font) { return $null }
    $alignment = Convert-VisualFiniteNumber $cell.HorizontalAlignment
    $orientation = Convert-VisualFiniteNumber $cell.Orientation
    $indent = Convert-VisualFiniteNumber $cell.IndentLevel
    if ($null -eq $alignment -or $null -eq $orientation -or $null -eq $indent -or $orientation -ne 0 -or
        $alignment -notin @(1, -4131, -4152, -4108)) { return $null }
    return [pscustomobject]@{ skip=$false; font=$font; alignment=[int]$alignment; indent=[double]$indent }
}
function Get-VisualColumnGeometry($worksheet, [int]$column, $columns) {
    if (-not $columns.ContainsKey($column)) {
        $band = $worksheet.Columns.Item($column)
        $columns[$column] = [pscustomobject]@{ left=[double]$band.Left; width=[double]$band.Width }
    }
    return $columns[$column]
}
# Legacy entry points deliberately fail instead of reintroducing COM Shapes.
function Add-ComVisualShape { throw 'COM drawing geometry is disabled; use Rust bounds.' }
function Add-VisualShapes { throw 'COM drawing geometry is disabled; use Rust bounds.' }
function Add-LiveVisualTextCell($worksheet, $cell, [string]$text, $state, $graphics, $fonts) {
    if ([bool]$cell.EntireRow.Hidden) { return }
    $area = $cell.MergeArea
    $state.maxRow = [Math]::Max($state.maxRow, [int]$area.Row + [int]$area.Rows.Count - 1)
    $state.maxColumn = [Math]::Max($state.maxColumn, [int]$area.Column + [int]$area.Columns.Count - 1)
    $wrap = $cell.WrapText; $shrink = $cell.ShrinkToFit
    if ((-not (Test-VisualNull $wrap) -and [bool]$wrap) -or (-not (Test-VisualNull $shrink) -and [bool]$shrink)) { return }
    $display = $cell.Text
    if (-not (Test-VisualNull $display) -and [string]$display -ne '') { $text = [string]$display }
    $width = Measure-VisualCellText $cell $text $graphics $fonts
    $orientation = Convert-VisualFiniteNumber $cell.Orientation
    if ($null -ne $orientation -and $orientation -ne 0) {
        # Rare rotated/vertical text: retain a conservative envelope in both axes.
        $state.right = [Math]::Max($state.right, [double]$area.Left + [double]$area.Width + $width)
        $state.bottom = [Math]::Max($state.bottom, [double]$area.Top + [double]$area.Height + $width)
    } else { $state.right = [Math]::Max($state.right, [double]$area.Left + $width) }
    $state.liveText++
}
function Add-VisualTextFromPlan($worksheet, $plan, $state, $graphics, $fonts) {
    $styles = @{}; $columnGeometry = @{}; $measurements = @{}
    $timer = [Diagnostics.Stopwatch]::StartNew(); $done = 0
    foreach ($candidate in $plan.Text) {
        $done++
        $state.maxRow = [Math]::Max($state.maxRow, [int]$candidate.MergeEndRow)
        $state.maxColumn = [Math]::Max($state.maxColumn, [int]$candidate.MergeEndColumn)
        $cell = $null; $style = $null
        if (-not $candidate.NeedsLiveFormatting -and -not $candidate.RichText) {
            $key = [string]$candidate.StyleKey
            if (-not $styles.ContainsKey($key)) {
                $cell = $worksheet.Cells.Item([int]$candidate.Row, [int]$candidate.Column)
                $styles[$key] = Get-VisualStyle $cell $fonts
            }
            $style = $styles[$key]
        }
        if ($null -ne $style) {
            if (-not $style.skip) {
                $geometry = Get-VisualColumnGeometry $worksheet ([int]$candidate.Column) $columnGeometry
                $measureKey = [string]$candidate.StyleKey + '|' + [string]$candidate.Text
                if (-not $measurements.ContainsKey($measureKey)) {
                    $measurements[$measureKey] = [double]$graphics.MeasureString([string]$candidate.Text, $style.font).Width * 72.0 / [double]$graphics.DpiX * 1.20 + 12.0 + $style.indent * 12.0
                }
                $width = $measurements[$measureKey]
                $right = if ($style.alignment -eq -4152) { $geometry.left + $geometry.width }
                    elseif ($style.alignment -eq -4108) { $geometry.left + $geometry.width / 2 + $width / 2 }
                    else { $geometry.left + $width }
                $state.right = [Math]::Max($state.right, $right)
                $state.fastText++
            }
        } else {
            if ($null -eq $cell) { $cell = $worksheet.Cells.Item([int]$candidate.Row, [int]$candidate.Column) }
            Add-LiveVisualTextCell $worksheet $cell ([string]$candidate.Text) $state $graphics $fonts
        }
        if ($timer.Elapsed.TotalSeconds -ge 2) {
            Write-ProgressLine "bounds text worksheet='$($worksheet.Name)' done=$done candidates=$($plan.Text.Count) fast=$($state.fastText) live=$($state.liveText)"
            $timer.Restart()
        }
    }
}
function Add-VisualTextFallback($worksheet, $baseRange, $state, $graphics, $fonts) {
    # Used only if OOXML planning is unavailable. Preserve the original bulk Value2
    # scan, with mixed-font handling fixed and no arbitrary candidate cut-off.
    $scanRows = [int]$baseRange.Row + [int]$baseRange.Rows.Count - 1
    $scanColumns = [int]$baseRange.Column + [int]$baseRange.Columns.Count - 1
    $visible = New-Object 'bool[]' ($scanColumns + 1)
    for ($c=1; $c -le $scanColumns; $c++) { $visible[$c] = -not [bool]$worksheet.Columns.Item($c).Hidden }
    $blockRows = [Math]::Max(1, [Math]::Min(256, [int][Math]::Floor(16384.0 / [Math]::Max(1, $scanColumns))))
    $timer = [Diagnostics.Stopwatch]::StartNew()
    for ($r=1; $r -le $scanRows; $r += $blockRows) {
        $last = [Math]::Min($scanRows, $r + $blockRows - 1)
        $block = $worksheet.Range($worksheet.Cells.Item($r,1), $worksheet.Cells.Item($last,$scanColumns))
        $values = $block.Value2; $array = $values -is [Array] -and $values.Rank -eq 2
        for ($row=$r; $row -le $last; $row++) {
            for ($column=$scanColumns; $column -ge 1; $column--) {
                if (-not $visible[$column]) { continue }
                $value = if ($array) { $values.GetValue($row - $r + $values.GetLowerBound(0), $column - 1 + $values.GetLowerBound(1)) } else { $values }
                if ((Test-VisualNull $value) -or [string]$value -eq '') { continue }
                if ($value -is [string]) { Add-LiveVisualTextCell $worksheet ($worksheet.Cells.Item($row,$column)) ([string]$value) $state $graphics $fonts }
                break
            }
        }
        if ($timer.Elapsed.TotalSeconds -ge 2 -or $last -eq $scanRows) { Write-ProgressLine "bounds fallback rows=$last total=$scanRows"; $timer.Restart() }
    }
}
function Expand-VisualCaptureRange($worksheet, $baseRange) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $before = Range-Address $baseRange
    $state = @{
        maxRow = [int]$baseRange.Row + [int]$baseRange.Rows.Count - 1
        maxColumn = [int]$baseRange.Column + [int]$baseRange.Columns.Count - 1
        right = [double]$baseRange.Left + [double]$baseRange.Width
        bottom = [double]$baseRange.Top + [double]$baseRange.Height
        comShapes = 0; fastText = 0; liveText = 0
    }
    Write-ProgressLine "bounds scan start worksheet='$($worksheet.Name)' base='$before'"
    $plan = Get-WorksheetVisualPlan $worksheet $baseRange
    Add-VisualShapes $worksheet $plan $state
    $bitmap = [Drawing.Bitmap]::new(1,1); $graphics = [Drawing.Graphics]::FromImage($bitmap); $fonts = @{}
    try {
        if ($null -eq $plan) { Add-VisualTextFallback $worksheet $baseRange $state $graphics $fonts }
        else { Add-VisualTextFromPlan $worksheet $plan $state $graphics $fonts }
    } finally {
        foreach ($font in $fonts.Values) { $font.Dispose() }
        $graphics.Dispose(); $bitmap.Dispose()
    }
    $maxColumn = Find-VisualEndIndex $worksheet $state.right $true $state.maxColumn
    $maxRow = Find-VisualEndIndex $worksheet $state.bottom $false $state.maxRow
    $expanded = $worksheet.Range($worksheet.Cells.Item(1,1), $worksheet.Cells.Item($maxRow,$maxColumn))
    Write-ProgressLine "range expanded worksheet='$($worksheet.Name)' before='$before' after='$(Range-Address $expanded)' com_shapes=$($state.comShapes) fast_text=$($state.fastText) live_text=$($state.liveText)"
    Write-ProgressLine "visual step phase=excel.bounds duration_ms=$($timer.ElapsedMilliseconds) label='$($worksheet.Name)'"
    return ,$expanded
}
function Test-CaptureGeometry($capture, $range) {
    if ($null -eq $capture -or $null -eq $capture.capture_info) { return $false }
    $info = $capture.capture_info
    return ([int]$info.start_row -eq [int]$range.Row -and [int]$info.start_column -eq [int]$range.Column -and
        [int]$info.rows -eq [int]$range.Rows.Count -and [int]$info.columns -eq [int]$range.Columns.Count)
}
function Test-CompatibleCachedPair($left, $right) {
    if ($null -eq $left -or $null -eq $right -or $null -eq $left.capture_info -or $null -eq $right.capture_info) { return $false }
    foreach ($field in @('start_row', 'start_column', 'rows', 'columns')) {
        if ([int]$left.capture_info.$field -ne [int]$right.capture_info.$field) { return $false }
    }
    $a = @($left.tiles); $b = @($right.tiles)
    if ($a.Count -ne $b.Count) { return $false }
    for ($j = 0; $j -lt $a.Count; $j++) {
        if ([int]$a[$j].tile_index -ne [int]$b[$j].tile_index -or [string]$a[$j].capture_range -ne [string]$b[$j].capture_range) { return $false }
    }
    return $true
}
function Resolve-VisualPairRange($origSheet, $shrunkSheet, $limit, $cachedOriginal, $cachedShrunk) {
    $a = if ($null -ne $cachedOriginal -and $null -ne $cachedOriginal.capture_info) {
        $v = $cachedOriginal.capture_info
        Worksheet-RangeByBounds $origSheet ([int]$v.start_row) ([int]$v.start_column) ([int]$v.rows) ([int]$v.columns)
    } else { Resolve-CaptureRange $origSheet $limit }
    $b = if ($null -ne $cachedShrunk -and $null -ne $cachedShrunk.capture_info) {
        $v = $cachedShrunk.capture_info
        Worksheet-RangeByBounds $shrunkSheet ([int]$v.start_row) ([int]$v.start_column) ([int]$v.rows) ([int]$v.columns)
    } else { Resolve-CaptureRange $shrunkSheet $limit }
    $rows = [Math]::Max([int]$a.Row + [int]$a.Rows.Count - 1, [int]$b.Row + [int]$b.Rows.Count - 1)
    $columns = [Math]::Max([int]$a.Column + [int]$a.Columns.Count - 1, [int]$b.Column + [int]$b.Columns.Count - 1)
    return ,($origSheet.Range($origSheet.Cells.Item(1, 1), $origSheet.Cells.Item($rows, $columns)))
}

function Save-ChartSheetImage($chart, [string]$path, [string]$label) {
    $image = $null
    try {
        $image = Invoke-VisualClipboardImage {
            [void]$chart.Parent.Activate(); [void]$chart.Activate(); [void]$chart.CopyPicture(1,2,1)
        } $label ([uint32]$script:ExcelCaptureProcessId)
        $saveTimer = [Diagnostics.Stopwatch]::StartNew()
        $image.Save($path, [Drawing.Imaging.ImageFormat]::Png)
        Write-ProgressLine "visual step phase=excel.png_save duration_ms=$($saveTimer.ElapsedMilliseconds) label='$label'"
        Write-ProgressLine "chart capture saved $label path='$path' width=$($image.Width) height=$($image.Height)"
        return $path
    } finally { if ($null -ne $image) { $image.Dispose() } }
}
