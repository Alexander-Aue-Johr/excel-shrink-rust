
param(
    [Parameter(Mandatory=$true)][string]$OriginalPath,
    [Parameter(Mandatory=$true)][string]$ShrunkPath,
    [Parameter(Mandatory=$true)][string]$OutputDir,
    [Parameter(Mandatory=$true)][string]$CaptureManifestPath,
    [Parameter(Mandatory=$false)][string]$RangeLimitPath = '',
    [Parameter(Mandatory=$true)][string]$OriginalCacheDir,
    [Parameter(Mandatory=$true)][string]$ShrunkCacheDir,
    [Parameter(Mandatory=$false)][string]$OriginalSheetCachePath = '',
    [Parameter(Mandatory=$false)][string]$ShrunkSheetCachePath = '',
    [Parameter(Mandatory=$false)][string]$OriginalAlreadyCached = 'false',
    [Parameter(Mandatory=$false)][string]$ShrunkAlreadyCached = 'false'
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

if (-not ('ExcelShrinkScreenshotWin32' -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class ExcelShrinkScreenshotWin32 {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
}
"@
}

function Write-ProgressLine([string]$message) {
    $timestamp = Get-Date -Format o
    Add-Content -Path $ProgressPath -Value "$timestamp $message"
}

function File-Size([string]$path) {
    if (Test-Path $path) {
        return (Get-Item $path).Length
    }
    return 0
}

function Range-Address($range) {
    try {
        $startRow = [int]$range.Row
        $startColumn = [int]$range.Column
        $endRow = $startRow + [int]$range.Rows.Count - 1
        $endColumn = $startColumn + [int]$range.Columns.Count - 1
        return "{0}{1}:{2}{3}" -f (Column-Name $startColumn), $startRow, (Column-Name $endColumn), $endRow
    }
    catch {
        return '<unknown>'
    }
}

function Column-Name([int]$index) {
    $name = ''
    while ($index -gt 0) {
        $index--
        $name = [char](65 + ($index % 26)) + $name
        $index = [Math]::Floor($index / 26)
    }
    return $name
}

function Matching-WorksheetRange($worksheet, $sourceRange) {
    $startRow = [int]$sourceRange.Row
    $startColumn = [int]$sourceRange.Column
    $endRow = $startRow + [int]$sourceRange.Rows.Count - 1
    $endColumn = $startColumn + [int]$sourceRange.Columns.Count - 1
    return ,$worksheet.Range($worksheet.Cells.Item($startRow, $startColumn), $worksheet.Cells.Item($endRow, $endColumn))
}

function RangeLimit-Address($rangeLimit, $range) {
    if ($null -ne $rangeLimit) {
        $maxCell = [string]$rangeLimit.max_cell
        if (-not [string]::IsNullOrWhiteSpace($maxCell) -and $maxCell -match '^[A-Za-z]+[0-9]+$') {
            return "A1:$maxCell"
        }
    }

    return Range-Address $range
}

function Worksheet-RangeFromA1ThroughBottomRight($worksheet, $range) {
    $startRow = [int]$range.Row
    $startColumn = [int]$range.Column
    $endRow = $startRow + [int]$range.Rows.Count - 1
    $endColumn = $startColumn + [int]$range.Columns.Count - 1
    $endRow = [Math]::Max(1, [Math]::Min([int]$worksheet.Rows.Count, $endRow))
    $endColumn = [Math]::Max(1, [Math]::Min([int]$worksheet.Columns.Count, $endColumn))
    return ,$worksheet.Range($worksheet.Cells.Item(1, 1), $worksheet.Cells.Item($endRow, $endColumn))
}

function Max-WorksheetRange($worksheet, $leftRange, $rightRange) {
    if ($null -eq $rightRange) {
        return ,$leftRange
    }

    $leftEndRow = [int]$leftRange.Row + [int]$leftRange.Rows.Count - 1
    $leftEndColumn = [int]$leftRange.Column + [int]$leftRange.Columns.Count - 1
    $rightEndRow = [int]$rightRange.Row + [int]$rightRange.Rows.Count - 1
    $rightEndColumn = [int]$rightRange.Column + [int]$rightRange.Columns.Count - 1

    $startRow = [Math]::Max(1, [Math]::Min([int]$leftRange.Row, [int]$rightRange.Row))
    $startColumn = [Math]::Max(1, [Math]::Min([int]$leftRange.Column, [int]$rightRange.Column))
    $endRow = [Math]::Min([int]$worksheet.Rows.Count, [Math]::Max($leftEndRow, $rightEndRow))
    $endColumn = [Math]::Min([int]$worksheet.Columns.Count, [Math]::Max($leftEndColumn, $rightEndColumn))

    return ,$worksheet.Range($worksheet.Cells.Item($startRow, $startColumn), $worksheet.Cells.Item($endRow, $endColumn))
}

function Try-SelectAllRangeFromMaxCell($worksheet, $baseRange, [string]$label) {
    try {
        $maxRow = [int]$baseRange.Row + [int]$baseRange.Rows.Count - 1
        $maxColumn = [int]$baseRange.Column + [int]$baseRange.Columns.Count - 1
        $maxRow = [Math]::Max(1, [Math]::Min([int]$worksheet.Rows.Count, $maxRow))
        $maxColumn = [Math]::Max(1, [Math]::Min([int]$worksheet.Columns.Count, $maxColumn))

        [void]$worksheet.Parent.Activate()
        [void]$worksheet.Activate()
        [void]$worksheet.Cells.Item($maxRow, $maxColumn).Select()
        Start-Sleep -Milliseconds 50

        $method = 'execute_mso_select_all'
        try {
            [void]$worksheet.Application.CommandBars.ExecuteMso("SelectAll")
        }
        catch {
            $method = 'sendkeys_ctrl_a'
            [System.Windows.Forms.SendKeys]::SendWait("^a")
        }
        Start-Sleep -Milliseconds 100

        $selection = $worksheet.Application.Selection
        if ($null -eq $selection) {
            Write-ProgressLine "select all range ignored $label reason='empty_selection'"
            return $null
        }

        $selectedRows = [int]$selection.Rows.Count
        $selectedColumns = [int]$selection.Columns.Count
        if ($selectedRows -ge [int]$worksheet.Rows.Count -or $selectedColumns -ge [int]$worksheet.Columns.Count) {
            Write-ProgressLine "select all range ignored $label method=$method reason='whole_sheet' selected='$(Range-Address $selection)'"
            return $null
        }

        Write-ProgressLine "select all range $label method=$method max_cell='$((Column-Name $maxColumn))$maxRow' selected='$(Range-Address $selection)'"
        return ,$selection
    }
    catch {
        Write-ProgressLine "select all range failed $label error='$($_.Exception.Message)'"
        return $null
    }
}

function Resolve-CaptureRange($worksheet, $rangeLimit) {
    if ($null -ne $rangeLimit) {
        $maxRow = [int]$rangeLimit.max_row
        $maxColumn = [int]$rangeLimit.max_column
        if ($maxRow -gt 0 -and $maxColumn -gt 0) {
            return ,$worksheet.Range($worksheet.Cells.Item(1, 1), $worksheet.Cells.Item($maxRow, $maxColumn))
        }

        $maxCell = [string]$rangeLimit.max_cell
        if (-not [string]::IsNullOrWhiteSpace($maxCell) -and $maxCell -match '^[A-Za-z]+[0-9]+$') {
            return ,$worksheet.Range("A1:$maxCell")
        }
    }

    $usedRange = $worksheet.UsedRange
    $baseRange = Worksheet-RangeFromA1ThroughBottomRight $worksheet $usedRange
    $selectAllRange = Try-SelectAllRangeFromMaxCell $worksheet $baseRange "worksheet='$($worksheet.Name)'"
    return ,(Max-WorksheetRange $worksheet $baseRange $selectAllRange)
}

function Hide-ActiveGridlines($worksheet, [string]$label) {
    try {
        $window = $worksheet.Application.ActiveWindow
        if ($null -ne $window) {
            $window.DisplayGridlines = $false
        }
    }
    catch {
        Write-ProgressLine "hide gridlines failed $label error='$($_.Exception.Message)'"
    }
}

function Default-ScreenshotBackgroundRgba() {
    return [pscustomobject]@{
        r = 255
        g = 255
        b = 255
        a = 255
    }
}

function Convert-ExcelColorToRgba($color) {
    if ($null -eq $color) {
        return $null
    }

    try {
        $value = [int64]$color
        if ($value -lt 0) {
            return $null
        }

        return [pscustomobject]@{
            r = [int]($value -band 255)
            g = [int](($value -shr 8) -band 255)
            b = [int](($value -shr 16) -band 255)
            a = 255
        }
    }
    catch {
        return $null
    }
}

function Get-InteriorBackgroundRgba($interior) {
    if ($null -eq $interior) {
        return $null
    }

    try {
        $colorIndex = $interior.ColorIndex
        if ($null -ne $colorIndex -and [int]$colorIndex -eq -4142) {
            return $null
        }
    }
    catch {
    }

    try {
        return Convert-ExcelColorToRgba $interior.Color
    }
    catch {
        return $null
    }
}

function Get-RangePaddingBackgroundRgba($range) {
    try {
        $row = [Math]::Max(1, [int]$range.Rows.Count)
        $column = [Math]::Max(1, [int]$range.Columns.Count)
        $cell = $range.Cells.Item($row, $column)

        try {
            $rgba = Get-InteriorBackgroundRgba $cell.DisplayFormat.Interior
            if ($null -ne $rgba) {
                return $rgba
            }
        }
        catch {
        }

        $rgba = Get-InteriorBackgroundRgba $cell.Interior
        if ($null -ne $rgba) {
            return $rgba
        }
    }
    catch {
    }

    return (Default-ScreenshotBackgroundRgba)
}

function Save-RangeImage($worksheet, $range, [string]$path, [string]$label, [string]$addressLabel) {
    $address = if (-not [string]::IsNullOrWhiteSpace($addressLabel)) { $addressLabel } else { Range-Address $range }
    Write-ProgressLine "capture start $label worksheet='$($worksheet.Name)' range='$address'"
    $backgroundRgba = Get-RangePaddingBackgroundRgba $range
    Write-ProgressLine "capture background $label rgba=$($backgroundRgba.r),$($backgroundRgba.g),$($backgroundRgba.b),$($backgroundRgba.a)"

    $image = $null
    $lastError = $null
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        try {
            [void]$worksheet.Parent.Activate()
            [void]$worksheet.Activate()
            Hide-ActiveGridlines $worksheet $label
            [void]$range.Select()
            Start-Sleep -Milliseconds 100
            Write-ProgressLine "clipboard clear $label attempt=$attempt"
            [System.Windows.Forms.Clipboard]::Clear()
            Write-ProgressLine "copy picture $label attempt=$attempt"
            $selection = $worksheet.Application.Selection
            [void]$selection.CopyPicture(1, 2)
        }
        catch {
            $lastError = $_.Exception.Message
            Write-ProgressLine "copy failed $label attempt=$attempt error='$lastError'"
            Start-Sleep -Milliseconds 250
            continue
        }

        Start-Sleep -Milliseconds 250
        Write-ProgressLine "clipboard read $label attempt=$attempt"
        $image = [System.Windows.Forms.Clipboard]::GetImage()
        if ($null -ne $image) {
            break
        }
    }

    if ($null -eq $image) {
        $address = Range-Address $range
        if ($lastError -ne $null) {
            throw "CopyPicture failed for worksheet '$($worksheet.Name)' range '$address': $lastError"
        }
        throw "Clipboard did not contain an image for worksheet '$($worksheet.Name)' range '$address'"
    }
    $width = [int]$image.Width
    $height = [int]$image.Height
    $image.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-ProgressLine "capture image $label width=$width height=$height"
    $image.Dispose()
    [System.Windows.Forms.Clipboard]::Clear()
    Write-ProgressLine "capture saved $label path='$path' bytes=$(File-Size $path)"
    return [pscustomobject]@{
        path = $path
        width = $width
        height = $height
        pixels = [int64]$width * [int64]$height
        background_rgba = $backgroundRgba
    }
}

function Use-CachedRangeImage([string]$path, [string]$label) {
    if (-not (Test-Path $path)) {
        throw "cached screenshot missing for $label path='$path'"
    }
    Write-ProgressLine "capture cache hit $label path='$path' bytes=$(File-Size $path)"
    $image = [System.Drawing.Image]::FromFile($path)
    try {
        return [pscustomobject]@{
            path = $path
            width = [int]$image.Width
            height = [int]$image.Height
            pixels = [int64]$image.Width * [int64]$image.Height
            background_rgba = $null
        }
    }
    finally {
        $image.Dispose()
    }
}

function Wait-ExcelReady($excel, [string]$label, [int]$maxAttempts = 60) {
    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
        try {
            if ($excel.Ready) {
                return
            }
        }
        catch {
        }
        Write-ProgressLine "excel busy label='$label' wait_attempt=$attempt"
        Start-Sleep -Milliseconds 500
    }
}

function Open-WorkbookWithRetry($excel, [string]$path, [string]$label) {
    $lastError = $null
    for ($attempt = 1; $attempt -le 20; $attempt++) {
        try {
            Wait-ExcelReady $excel "before open $label" 10
            Write-ProgressLine "open workbook start label='$label' attempt=$attempt path='$path'"
            $openedAt = Get-Date -Format o
            $openTimer = [System.Diagnostics.Stopwatch]::StartNew()
            $workbook = $excel.Workbooks.Open($path, 0, $false)
            if ($null -ne $workbook) {
                Write-ProgressLine "opened $label workbook '$path' attempt=$attempt"
                Wait-ExcelReady $excel "after open $label" 60
                $openTimer.Stop()
                [void]$script:OpenTimings.Add([pscustomobject]@{
                    label = $label
                    path = $path
                    attempt = $attempt
                    duration_ms = [int64]$openTimer.ElapsedMilliseconds
                    opened_at = $openedAt
                })
                Write-ProgressLine "open workbook timing label='$label' attempt=$attempt duration_ms=$([int64]$openTimer.ElapsedMilliseconds)"
                return $workbook
            }
            $lastError = "Excel returned null workbook"
        }
        catch {
            $lastError = $_.Exception.Message
            Write-ProgressLine "open workbook failed label='$label' attempt=$attempt error='$lastError'"
        }

        Start-Sleep -Milliseconds ([Math]::Min(5000, 250 * $attempt))
    }

    throw "Excel did not open $label workbook '$path': $lastError"
}

function Test-CachedCaptureFiles($capture) {
    if ($null -eq $capture) {
        return $false
    }
    if ($null -ne $capture.screenshot_error) {
        return $false
    }

    $tiles = @($capture.tiles)
    if ($tiles.Count -eq 0) {
        $image = [string]$capture.image
        return (-not [string]::IsNullOrWhiteSpace($image)) -and (Test-Path $image)
    }

    foreach ($tile in $tiles) {
        $image = [string]$tile.image
        if ([string]::IsNullOrWhiteSpace($image) -or -not (Test-Path $image)) {
            return $false
        }
    }
    return $true
}

function Read-SheetCaptureCache([string]$path) {
    $map = @{}
    if ([string]::IsNullOrWhiteSpace($path) -or -not (Test-Path $path)) {
        return $map
    }

    try {
        $json = Get-Content -Path $path -Raw | ConvertFrom-Json
        foreach ($capture in @($json.captures)) {
            if (Test-CachedCaptureFiles $capture) {
                $map[[int]$capture.sheet_index] = $capture
            }
        }
        Write-ProgressLine "sheet cache loaded path='$path' count=$($map.Count)"
    }
    catch {
        Write-ProgressLine "sheet cache ignored path='$path' error='$($_.Exception.Message)'"
    }

    return $map
}

function First-NonEmptyString($primary, $fallback) {
    if ($null -ne $primary -and -not [string]::IsNullOrWhiteSpace([string]$primary)) {
        return [string]$primary
    }
    if ($null -ne $fallback -and -not [string]::IsNullOrWhiteSpace([string]$fallback)) {
        return [string]$fallback
    }
    return $null
}

function First-NonNull($primary, $fallback) {
    if ($null -ne $primary) {
        return $primary
    }
    return $fallback
}

function Join-CachedCaptureTiles($originalCapture, $shrunkCapture) {
    $originalByIndex = @{}
    $shrunkByIndex = @{}
    $tileIndexes = @{}

    foreach ($tile in @($originalCapture.tiles)) {
        $index = [int]$tile.tile_index
        $originalByIndex[$index] = $tile
        $tileIndexes[$index] = $true
    }
    foreach ($tile in @($shrunkCapture.tiles)) {
        $index = [int]$tile.tile_index
        $shrunkByIndex[$index] = $tile
        $tileIndexes[$index] = $true
    }

    $tiles = New-Object System.Collections.ArrayList
    foreach ($index in @($tileIndexes.Keys | Sort-Object)) {
        $originalTile = $originalByIndex[[int]$index]
        $shrunkTile = $shrunkByIndex[[int]$index]
        $preferred = First-NonNull $originalTile $shrunkTile
        if ($null -eq $preferred) {
            continue
        }

        [void]$tiles.Add([pscustomobject]@{
            tile_index = [int]$index
            capture_range = $preferred.capture_range
            capture_info = $preferred.capture_info
            original_image = if ($null -ne $originalTile) { $originalTile.image } else { $null }
            shrunk_image = if ($null -ne $shrunkTile) { $shrunkTile.image } else { $null }
            original_background_rgba = if ($null -ne $originalTile) { $originalTile.background_rgba } else { $null }
            shrunk_background_rgba = if ($null -ne $shrunkTile) { $shrunkTile.background_rgba } else { $null }
            original_width = if ($null -ne $originalTile) { $originalTile.width } else { $null }
            original_height = if ($null -ne $originalTile) { $originalTile.height } else { $null }
            shrunk_width = if ($null -ne $shrunkTile) { $shrunkTile.width } else { $null }
            shrunk_height = if ($null -ne $shrunkTile) { $shrunkTile.height } else { $null }
        })
    }

    return @($tiles.ToArray())
}

function New-CachedCapturePair([int]$sheetIndex, [string]$sheetName, $originalCapture, $shrunkCapture) {
    $tiles = Join-CachedCaptureTiles $originalCapture $shrunkCapture
    $preferred = First-NonNull $originalCapture $shrunkCapture
    return [pscustomobject]@{
        sheet_index = $sheetIndex
        sheet_name = $sheetName
        capture_range = First-NonEmptyString $originalCapture.capture_range $shrunkCapture.capture_range
        capture_strategy = First-NonEmptyString $originalCapture.capture_strategy $shrunkCapture.capture_strategy
        used_range_error = First-NonEmptyString $originalCapture.used_range_error $shrunkCapture.used_range_error
        screenshot_error = $null
        capture_info = $preferred.capture_info
        original_image = if (@($originalCapture.tiles).Count -eq 0) { $originalCapture.image } else { $null }
        shrunk_image = if (@($shrunkCapture.tiles).Count -eq 0) { $shrunkCapture.image } else { $null }
        original_background_rgba = $originalCapture.background_rgba
        shrunk_background_rgba = $shrunkCapture.background_rgba
        tiles = @($tiles)
    }
}

function Capture-RangeImageSetWithPartialCache($origSheet, $shrunkSheet, $origRange, $shrunkRange, [string]$originalCacheDir, [string]$shrunkCacheDir, [string]$baseName, [string]$label, [string]$strategy, [string]$usedRangeError, $cachedOriginal, $cachedShrunk) {
    if ($null -eq $cachedOriginal -and $null -eq $cachedShrunk) {
        return Capture-RangeImageSet $origSheet $shrunkSheet $origRange $shrunkRange $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError
    }
    if ($null -ne $cachedOriginal -and $null -ne $cachedShrunk) {
        return (New-CachedCapturePair 0 "" $cachedOriginal $cachedShrunk)
    }

    $address = Range-Address $origRange
    if ($null -ne $cachedOriginal) {
        $cachedTiles = @($cachedOriginal.tiles)
        if ($cachedTiles.Count -gt 0) {
            $tiles = New-Object System.Collections.ArrayList
            foreach ($cachedTile in @($cachedTiles | Sort-Object {[int]$_.tile_index})) {
                $tileIndex = [int]$cachedTile.tile_index
                $info = $cachedTile.capture_info
                if ($null -eq $info) {
                    Write-ProgressLine "partial cache ignored $label side=original tile=$tileIndex reason='missing_capture_info'; recapturing pair"
                    return Capture-RangeImageSet $origSheet $shrunkSheet $origRange $shrunkRange $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError
                }
                $tileRange = Worksheet-RangeByBounds $shrunkSheet ([int]$info.start_row) ([int]$info.start_column) ([int]$info.rows) ([int]$info.columns)
                $shrunkPath = Join-Path $shrunkCacheDir ("{0}-tile-{1:D3}.png" -f $baseName, $tileIndex)
                $shrunkInfo = Save-RangeImage $shrunkSheet $tileRange $shrunkPath "shrunk $label partial-cache tile=$tileIndex" $cachedTile.capture_range
                [void]$tiles.Add([pscustomobject]@{
                    tile_index = $tileIndex
                    capture_range = $cachedTile.capture_range
                    capture_info = $cachedTile.capture_info
                    original_image = $cachedTile.image
                    shrunk_image = $shrunkPath
                    original_background_rgba = $cachedTile.background_rgba
                    shrunk_background_rgba = $shrunkInfo.background_rgba
                    original_width = $cachedTile.width
                    original_height = $cachedTile.height
                    shrunk_width = [int]$shrunkInfo.width
                    shrunk_height = [int]$shrunkInfo.height
                })
            }
            return [pscustomobject]@{
                capture_range = First-NonEmptyString $cachedOriginal.capture_range $address
                capture_strategy = First-NonEmptyString $cachedOriginal.capture_strategy $strategy
                capture_info = First-NonNull $cachedOriginal.capture_info (Range-CaptureInfo $origRange $strategy $address $usedRangeError)
                original_image = $null
                shrunk_image = $null
                original_background_rgba = $null
                shrunk_background_rgba = $null
                tiles = @($tiles.ToArray())
            }
        }

        $origPath = [string]$cachedOriginal.image
        if ([string]::IsNullOrWhiteSpace($origPath)) {
            Write-ProgressLine "partial cache ignored $label side=original reason='missing_image'; recapturing pair"
            return Capture-RangeImageSet $origSheet $shrunkSheet $origRange $shrunkRange $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError
        }
        $origInfo = Use-CachedRangeImage $origPath "original $label partial-cache"
        $shrunkPath = Join-Path $shrunkCacheDir ("{0}.png" -f $baseName)
        $shrunkInfo = Save-RangeImage $shrunkSheet $shrunkRange $shrunkPath "shrunk $label partial-cache" $address
        $maxPixels = [Math]::Max([int64]$origInfo.pixels, [int64]$shrunkInfo.pixels)
        if ($maxPixels -gt $MaxScreenshotPixels) {
            Write-ProgressLine "partial cache full image too large $label pixels=$maxPixels max_pixels=$MaxScreenshotPixels; recapturing pair"
            Remove-Item $shrunkPath -Force -ErrorAction SilentlyContinue
            return Capture-RangeImageSet $origSheet $shrunkSheet $origRange $shrunkRange $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError
        }
        return [pscustomobject]@{
            capture_range = First-NonEmptyString $cachedOriginal.capture_range $address
            capture_strategy = First-NonEmptyString $cachedOriginal.capture_strategy $strategy
            capture_info = First-NonNull $cachedOriginal.capture_info (Range-CaptureInfo $origRange $strategy $address $usedRangeError)
            original_image = $origPath
            shrunk_image = $shrunkPath
            original_background_rgba = $cachedOriginal.background_rgba
            shrunk_background_rgba = $shrunkInfo.background_rgba
            tiles = @()
        }
    }

    $cachedTiles = @($cachedShrunk.tiles)
    if ($cachedTiles.Count -gt 0) {
        $tiles = New-Object System.Collections.ArrayList
        foreach ($cachedTile in @($cachedTiles | Sort-Object {[int]$_.tile_index})) {
            $tileIndex = [int]$cachedTile.tile_index
            $info = $cachedTile.capture_info
            if ($null -eq $info) {
                Write-ProgressLine "partial cache ignored $label side=shrunk tile=$tileIndex reason='missing_capture_info'; recapturing pair"
                return Capture-RangeImageSet $origSheet $shrunkSheet $origRange $shrunkRange $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError
            }
            $tileRange = Worksheet-RangeByBounds $origSheet ([int]$info.start_row) ([int]$info.start_column) ([int]$info.rows) ([int]$info.columns)
            $origPath = Join-Path $originalCacheDir ("{0}-tile-{1:D3}.png" -f $baseName, $tileIndex)
            $origInfo = Save-RangeImage $origSheet $tileRange $origPath "original $label partial-cache tile=$tileIndex" $cachedTile.capture_range
            [void]$tiles.Add([pscustomobject]@{
                tile_index = $tileIndex
                capture_range = $cachedTile.capture_range
                capture_info = $cachedTile.capture_info
                original_image = $origPath
                shrunk_image = $cachedTile.image
                original_background_rgba = $origInfo.background_rgba
                shrunk_background_rgba = $cachedTile.background_rgba
                original_width = [int]$origInfo.width
                original_height = [int]$origInfo.height
                shrunk_width = $cachedTile.width
                shrunk_height = $cachedTile.height
            })
        }
        return [pscustomobject]@{
            capture_range = First-NonEmptyString $cachedShrunk.capture_range $address
            capture_strategy = First-NonEmptyString $cachedShrunk.capture_strategy $strategy
            capture_info = First-NonNull $cachedShrunk.capture_info (Range-CaptureInfo $origRange $strategy $address $usedRangeError)
            original_image = $null
            shrunk_image = $null
            original_background_rgba = $null
            shrunk_background_rgba = $null
            tiles = @($tiles.ToArray())
        }
    }

    $shrunkPath = [string]$cachedShrunk.image
    if ([string]::IsNullOrWhiteSpace($shrunkPath)) {
        Write-ProgressLine "partial cache ignored $label side=shrunk reason='missing_image'; recapturing pair"
        return Capture-RangeImageSet $origSheet $shrunkSheet $origRange $shrunkRange $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError
    }
    $shrunkInfo = Use-CachedRangeImage $shrunkPath "shrunk $label partial-cache"
    $origPath = Join-Path $originalCacheDir ("{0}.png" -f $baseName)
    $origInfo = Save-RangeImage $origSheet $origRange $origPath "original $label partial-cache" $address
    $maxPixels = [Math]::Max([int64]$origInfo.pixels, [int64]$shrunkInfo.pixels)
    if ($maxPixels -gt $MaxScreenshotPixels) {
        Write-ProgressLine "partial cache full image too large $label pixels=$maxPixels max_pixels=$MaxScreenshotPixels; recapturing pair"
        Remove-Item $origPath -Force -ErrorAction SilentlyContinue
        return Capture-RangeImageSet $origSheet $shrunkSheet $origRange $shrunkRange $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError
    }
    return [pscustomobject]@{
        capture_range = First-NonEmptyString $cachedShrunk.capture_range $address
        capture_strategy = First-NonEmptyString $cachedShrunk.capture_strategy $strategy
        capture_info = First-NonNull $cachedShrunk.capture_info (Range-CaptureInfo $origRange $strategy $address $usedRangeError)
        original_image = $origPath
        shrunk_image = $shrunkPath
        original_background_rgba = $origInfo.background_rgba
        shrunk_background_rgba = $cachedShrunk.background_rgba
        tiles = @()
    }
}

function Range-CaptureInfo($range, [string]$strategy, [string]$address, [string]$usedRangeError) {
    return [pscustomobject]@{
        strategy = $strategy
        address = $address
        start_row = [int]$range.Row
        start_column = [int]$range.Column
        rows = [Math]::Max(1, [int]$range.Rows.Count)
        columns = [Math]::Max(1, [int]$range.Columns.Count)
        used_range_error = $usedRangeError
    }
}

function Is-MemoryLikeCaptureError([string]$message) {
    if ([string]::IsNullOrWhiteSpace($message)) {
        return $false
    }
    return $message -match '(?i)(memory|speicher|ressource|resource|not enough|nicht genug|nicht genügend|out of|8007000e|e_outofmemory)'
}

function Get-RangePointSize($range) {
    $width = [double]0
    $height = [double]0
    try {
        $width = [double]$range.Width
    }
    catch {
        $width = [double]$range.Columns.Count
    }
    try {
        $height = [double]$range.Height
    }
    catch {
        $height = [double]$range.Rows.Count
    }
    return [pscustomobject]@{
        width = $width
        height = $height
    }
}

function Prefer-ColumnSplitForRangePair($origRange, $shrunkRange) {
    $origSize = Get-RangePointSize $origRange
    $shrunkSize = Get-RangePointSize $shrunkRange
    $maxWidth = [Math]::Max([double]$origSize.width, [double]$shrunkSize.width)
    $maxHeight = [Math]::Max([double]$origSize.height, [double]$shrunkSize.height)
    if ($maxWidth -gt 0 -or $maxHeight -gt 0) {
        return $maxWidth -gt $maxHeight
    }
    return [int]$shrunkRange.Columns.Count -gt [int]$shrunkRange.Rows.Count
}

function Worksheet-RangeByBounds($worksheet, [int]$startRow, [int]$startColumn, [int]$rows, [int]$columns) {
    $endRow = $startRow + $rows - 1
    $endColumn = $startColumn + $columns - 1
    return ,$worksheet.Range($worksheet.Cells.Item($startRow, $startColumn), $worksheet.Cells.Item($endRow, $endColumn))
}

function Split-RangePair($origSheet, $shrunkSheet, $origRange, $shrunkRange, [bool]$preferColumns) {
    $startRow = [int]$origRange.Row
    $startColumn = [int]$origRange.Column
    $rows = [int]$origRange.Rows.Count
    $columns = [int]$origRange.Columns.Count
    $splitColumns = $false

    if ($preferColumns -and $columns -gt 1) {
        $splitColumns = $true
    }
    elseif ((-not $preferColumns) -and $rows -gt 1) {
        $splitColumns = $false
    }
    elseif ($columns -gt 1) {
        $splitColumns = $true
    }
    elseif ($rows -gt 1) {
        $splitColumns = $false
    }
    else {
        return @()
    }

    if ($splitColumns) {
        $leftColumns = [Math]::Max(1, [int][Math]::Floor($columns / 2))
        $rightColumns = $columns - $leftColumns
        if ($rightColumns -le 0) { return @() }
        return @(
            [pscustomobject]@{
                original = Worksheet-RangeByBounds $origSheet $startRow $startColumn $rows $leftColumns
                shrunk = Worksheet-RangeByBounds $shrunkSheet $startRow $startColumn $rows $leftColumns
            },
            [pscustomobject]@{
                original = Worksheet-RangeByBounds $origSheet $startRow ($startColumn + $leftColumns) $rows $rightColumns
                shrunk = Worksheet-RangeByBounds $shrunkSheet $startRow ($startColumn + $leftColumns) $rows $rightColumns
            }
        )
    }

    $topRows = [Math]::Max(1, [int][Math]::Floor($rows / 2))
    $bottomRows = $rows - $topRows
    if ($bottomRows -le 0) { return @() }
    return @(
        [pscustomobject]@{
            original = Worksheet-RangeByBounds $origSheet $startRow $startColumn $topRows $columns
            shrunk = Worksheet-RangeByBounds $shrunkSheet $startRow $startColumn $topRows $columns
        },
        [pscustomobject]@{
            original = Worksheet-RangeByBounds $origSheet ($startRow + $topRows) $startColumn $bottomRows $columns
            shrunk = Worksheet-RangeByBounds $shrunkSheet ($startRow + $topRows) $startColumn $bottomRows $columns
        }
    )
}

function Capture-RangeTileRecursive($origSheet, $shrunkSheet, $origRange, $shrunkRange, [string]$originalCacheDir, [string]$shrunkCacheDir, [string]$baseName, [string]$label, [string]$strategy, [string]$usedRangeError, [ref]$nextTileIndex, [int]$depth) {
    $tileIndex = [int]$nextTileIndex.Value
    $nextTileIndex.Value = $tileIndex + 1
    $origPath = Join-Path $originalCacheDir ("{0}-tile-{1:D3}.png" -f $baseName, $tileIndex)
    $shrunkPath = Join-Path $shrunkCacheDir ("{0}-tile-{1:D3}.png" -f $baseName, $tileIndex)
    $address = Range-Address $origRange
    $tileLabel = "$label tile=$tileIndex depth=$depth"

    try {
        $origInfo = Save-RangeImage $origSheet $origRange $origPath "original $tileLabel" $address
        $shrunkInfo = Save-RangeImage $shrunkSheet $shrunkRange $shrunkPath "shrunk $tileLabel" $address
    }
    catch {
        $errorMessage = $_.Exception.Message
        Remove-Item $origPath -Force -ErrorAction SilentlyContinue
        Remove-Item $shrunkPath -Force -ErrorAction SilentlyContinue
        if (-not (Is-MemoryLikeCaptureError $errorMessage) -or $depth -ge 16) {
            throw
        }

        $preferColumns = Prefer-ColumnSplitForRangePair $origRange $shrunkRange
        $parts = @(Split-RangePair $origSheet $shrunkSheet $origRange $shrunkRange $preferColumns)
        if ($parts.Count -eq 0) {
            throw
        }

        Write-ProgressLine "tile split $tileLabel range='$address' reason='memory_error' error='$errorMessage' split_axis=$(if ($preferColumns) { 'columns' } else { 'rows' })"
        $tiles = New-Object System.Collections.ArrayList
        foreach ($part in $parts) {
            foreach ($tile in @(Capture-RangeTileRecursive $origSheet $shrunkSheet $part.original $part.shrunk $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError $nextTileIndex ($depth + 1))) {
                [void]$tiles.Add($tile)
            }
        }
        return @($tiles.ToArray())
    }

    $maxPixels = [Math]::Max([int64]$origInfo.pixels, [int64]$shrunkInfo.pixels)

    if ($maxPixels -le $MaxScreenshotPixels -or $depth -ge 16) {
        return @([pscustomobject]@{
            tile_index = $tileIndex
            capture_range = $address
            capture_info = Range-CaptureInfo $origRange "${strategy}_tile" $address $usedRangeError
            original_image = $origPath
            shrunk_image = $shrunkPath
            original_background_rgba = $origInfo.background_rgba
            shrunk_background_rgba = $shrunkInfo.background_rgba
            original_width = [int]$origInfo.width
            original_height = [int]$origInfo.height
            shrunk_width = [int]$shrunkInfo.width
            shrunk_height = [int]$shrunkInfo.height
        })
    }

    $maxWidth = [Math]::Max([int]$origInfo.width, [int]$shrunkInfo.width)
    $maxHeight = [Math]::Max([int]$origInfo.height, [int]$shrunkInfo.height)
    $preferColumns = $maxWidth -gt $maxHeight
    $parts = @(Split-RangePair $origSheet $shrunkSheet $origRange $shrunkRange $preferColumns)
    if ($parts.Count -eq 0) {
        return @([pscustomobject]@{
            tile_index = $tileIndex
            capture_range = $address
            capture_info = Range-CaptureInfo $origRange "${strategy}_tile" $address $usedRangeError
            original_image = $origPath
            shrunk_image = $shrunkPath
            original_background_rgba = $origInfo.background_rgba
            shrunk_background_rgba = $shrunkInfo.background_rgba
            original_width = [int]$origInfo.width
            original_height = [int]$origInfo.height
            shrunk_width = [int]$shrunkInfo.width
            shrunk_height = [int]$shrunkInfo.height
        })
    }

    Write-ProgressLine "tile split $tileLabel range='$address' width=$maxWidth height=$maxHeight pixels=$maxPixels max_pixels=$MaxScreenshotPixels split_axis=$(if ($preferColumns) { 'columns' } else { 'rows' })"
    Remove-Item $origPath -Force -ErrorAction SilentlyContinue
    Remove-Item $shrunkPath -Force -ErrorAction SilentlyContinue

    $tiles = New-Object System.Collections.ArrayList
    foreach ($part in $parts) {
        foreach ($tile in @(Capture-RangeTileRecursive $origSheet $shrunkSheet $part.original $part.shrunk $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError $nextTileIndex ($depth + 1))) {
            [void]$tiles.Add($tile)
        }
    }
    return @($tiles.ToArray())
}

function Capture-RangeImageSet($origSheet, $shrunkSheet, $origRange, $shrunkRange, [string]$originalCacheDir, [string]$shrunkCacheDir, [string]$baseName, [string]$label, [string]$strategy, [string]$usedRangeError) {
    $origPath = Join-Path $originalCacheDir ("{0}.png" -f $baseName)
    $shrunkPath = Join-Path $shrunkCacheDir ("{0}.png" -f $baseName)
    $address = Range-Address $origRange
    try {
        $origInfo = Save-RangeImage $origSheet $origRange $origPath "original $label" $address
        $shrunkInfo = Save-RangeImage $shrunkSheet $shrunkRange $shrunkPath "shrunk $label" $address
    }
    catch {
        $errorMessage = $_.Exception.Message
        Remove-Item $origPath -Force -ErrorAction SilentlyContinue
        Remove-Item $shrunkPath -Force -ErrorAction SilentlyContinue
        if (-not (Is-MemoryLikeCaptureError $errorMessage)) {
            throw
        }

        $preferColumns = Prefer-ColumnSplitForRangePair $origRange $shrunkRange
        $parts = @(Split-RangePair $origSheet $shrunkSheet $origRange $shrunkRange $preferColumns)
        if ($parts.Count -eq 0) {
            throw
        }

        Write-ProgressLine "capture split $label range='$address' reason='memory_error' error='$errorMessage' split_axis=$(if ($preferColumns) { 'columns' } else { 'rows' })"
        $tiles = New-Object System.Collections.ArrayList
        $nextTileIndex = 1
        foreach ($part in $parts) {
            foreach ($tile in @(Capture-RangeTileRecursive $origSheet $shrunkSheet $part.original $part.shrunk $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError ([ref]$nextTileIndex) 1)) {
                [void]$tiles.Add($tile)
            }
        }

        return [pscustomobject]@{
            capture_range = $address
            capture_strategy = "${strategy}_tiled_after_memory_error"
            capture_info = Range-CaptureInfo $origRange "${strategy}_tiled_after_memory_error" $address $usedRangeError
            original_image = $null
            shrunk_image = $null
            original_background_rgba = $null
            shrunk_background_rgba = $null
            tiles = @($tiles.ToArray())
        }
    }

    $maxPixels = [Math]::Max([int64]$origInfo.pixels, [int64]$shrunkInfo.pixels)
    $captureInfo = Range-CaptureInfo $origRange $strategy $address $usedRangeError

    if ($maxPixels -le $MaxScreenshotPixels) {
        return [pscustomobject]@{
            capture_range = $address
            capture_strategy = $strategy
            capture_info = $captureInfo
            original_image = $origPath
            shrunk_image = $shrunkPath
            original_background_rgba = $origInfo.background_rgba
            shrunk_background_rgba = $shrunkInfo.background_rgba
            tiles = @()
        }
    }

    $maxWidth = [Math]::Max([int]$origInfo.width, [int]$shrunkInfo.width)
    $maxHeight = [Math]::Max([int]$origInfo.height, [int]$shrunkInfo.height)
    $preferColumns = $maxWidth -gt $maxHeight
    $parts = @(Split-RangePair $origSheet $shrunkSheet $origRange $shrunkRange $preferColumns)
    if ($parts.Count -eq 0) {
        return [pscustomobject]@{
            capture_range = $address
            capture_strategy = $strategy
            capture_info = $captureInfo
            original_image = $origPath
            shrunk_image = $shrunkPath
            original_background_rgba = $origInfo.background_rgba
            shrunk_background_rgba = $shrunkInfo.background_rgba
            tiles = @()
        }
    }

    Write-ProgressLine "capture split $label range='$address' width=$maxWidth height=$maxHeight pixels=$maxPixels max_pixels=$MaxScreenshotPixels split_axis=$(if ($preferColumns) { 'columns' } else { 'rows' })"
    Remove-Item $origPath -Force -ErrorAction SilentlyContinue
    Remove-Item $shrunkPath -Force -ErrorAction SilentlyContinue

    $tiles = New-Object System.Collections.ArrayList
    $nextTileIndex = 1
    foreach ($part in $parts) {
        foreach ($tile in @(Capture-RangeTileRecursive $origSheet $shrunkSheet $part.original $part.shrunk $originalCacheDir $shrunkCacheDir $baseName $label $strategy $usedRangeError ([ref]$nextTileIndex) 1)) {
            [void]$tiles.Add($tile)
        }
    }

    return [pscustomobject]@{
        capture_range = $address
        capture_strategy = "${strategy}_tiled"
        capture_info = Range-CaptureInfo $origRange "${strategy}_tiled" $address $usedRangeError
        original_image = $null
        shrunk_image = $null
        original_background_rgba = $null
        shrunk_background_rgba = $null
        tiles = @($tiles.ToArray())
    }
}

function Get-WindowProcessId($hwnd) {
    $processId = [uint32]0
    [void][ExcelShrinkScreenshotWin32]::GetWindowThreadProcessId([IntPtr]$hwnd, [ref]$processId)
    return [int]$processId
}

$excel = $null
$original = $null
$shrunk = $null
$captures = New-Object System.Collections.ArrayList
$rangeLimitsByIndex = @{}
$originalSheetCacheByIndex = @{}
$shrunkSheetCacheByIndex = @{}
$ProgressPath = Join-Path $OutputDir "screenshot-progress.log"
$OpenTimingsPath = Join-Path $OutputDir "screenshot-open-timings.json"
$script:OpenTimings = New-Object System.Collections.ArrayList
$originalAlreadyCachedEnabled = $OriginalAlreadyCached -match '^(1|true|yes)$'
$shrunkAlreadyCachedEnabled = $ShrunkAlreadyCached -match '^(1|true|yes)$'
$MaxScreenshotPixels = [int64]80000000

if (-not [string]::IsNullOrWhiteSpace($RangeLimitPath) -and (Test-Path $RangeLimitPath)) {
    $rangeLimits = Get-Content -Path $RangeLimitPath -Raw | ConvertFrom-Json
    foreach ($rangeLimit in @($rangeLimits)) {
        $rangeLimitsByIndex[[int]$rangeLimit.sheet_index] = $rangeLimit
    }
}

try {
    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
    New-Item -ItemType Directory -Path $OriginalCacheDir -Force | Out-Null
    New-Item -ItemType Directory -Path $ShrunkCacheDir -Force | Out-Null
    Set-Content -Path $ProgressPath -Value "screenshot capture started $(Get-Date -Format o)"
    Write-ProgressLine "input original='$OriginalPath'"
    Write-ProgressLine "input shrunk='$ShrunkPath'"
    Write-ProgressLine "output_dir='$OutputDir'"
    Write-ProgressLine "original_cache_dir='$OriginalCacheDir' already_cached=$OriginalAlreadyCached"
    Write-ProgressLine "shrunk_cache_dir='$ShrunkCacheDir' already_cached=$ShrunkAlreadyCached"
    Write-ProgressLine "capture_manifest_path='$CaptureManifestPath'"
    Write-ProgressLine "range_limit_path='$RangeLimitPath' loaded_count=$($rangeLimitsByIndex.Count)"
    $originalSheetCacheByIndex = Read-SheetCaptureCache $OriginalSheetCachePath
    $shrunkSheetCacheByIndex = Read-SheetCaptureCache $ShrunkSheetCachePath
    Write-ProgressLine "sheet cache summary original_hits=$($originalSheetCacheByIndex.Count) shrunk_hits=$($shrunkSheetCacheByIndex.Count)"
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $true
    $excel.ScreenUpdating = $true
    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $excel.AutomationSecurity = 3
    $excelProcessId = Get-WindowProcessId $excel.Hwnd
    Write-ProgressLine "excel pid=$excelProcessId"

    $original = Open-WorkbookWithRetry $excel $OriginalPath "original"
    $shrunk = Open-WorkbookWithRetry $excel $ShrunkPath "shrunk"
    $sheetCount = [Math]::Min($original.Worksheets.Count, $shrunk.Worksheets.Count)
    Write-ProgressLine "sheet count $sheetCount"

    for ($i = 1; $i -le $sheetCount; $i++) {
        $origSheet = $original.Worksheets.Item($i)
        $shrunkSheet = $shrunk.Worksheets.Item($i)
        $rangeLimit = $rangeLimitsByIndex[$i]
        $imageBase = "{0:D3}" -f $i
        $imageName = "$imageBase.png"
        $origImage = Join-Path $OriginalCacheDir $imageName
        $shrunkImage = Join-Path $ShrunkCacheDir $imageName
        $captureRange = $null
        $captureInfo = $null
        $captureStrategy = $null
        $captureTiles = @()
        $origBackgroundRgba = $null
        $shrunkBackgroundRgba = $null
        $usedRangeError = $null

        Write-ProgressLine "sheet start index=$i name='$($origSheet.Name)'"

        $cachedOriginal = $originalSheetCacheByIndex[$i]
        $cachedShrunk = $shrunkSheetCacheByIndex[$i]
        if ($null -ne $cachedOriginal -and $null -ne $cachedShrunk) {
            [void]$captures.Add((New-CachedCapturePair $i $origSheet.Name $cachedOriginal $cachedShrunk))
            Write-ProgressLine "sheet cache hit index=$i original_hash='$($cachedOriginal.sheet_xml_hash)' shrunk_hash='$($cachedShrunk.sheet_xml_hash)'"
            Write-ProgressLine "sheet done index=$i"
            continue
        }
        elseif ($null -ne $cachedOriginal -or $null -ne $cachedShrunk) {
            Write-ProgressLine "sheet cache partial index=$i original_hit=$($null -ne $cachedOriginal) shrunk_hit=$($null -ne $cachedShrunk); capturing missing side only"
        }

        try {
            $primaryStrategy = if ($null -ne $rangeLimit) { "rust_bounds" } else { "used_range" }
            try {
                $origRange = Resolve-CaptureRange $origSheet $rangeLimit
                $shrunkRange = Matching-WorksheetRange $shrunkSheet $origRange
                $captureRange = RangeLimit-Address $rangeLimit $origRange
                $captureInfo = Range-CaptureInfo $origRange $primaryStrategy $captureRange $null
                Write-ProgressLine "sheet primary range index=$i strategy=$primaryStrategy range='$captureRange' start_row=$($captureInfo.start_row) start_column=$($captureInfo.start_column) rows=$($captureInfo.rows) columns=$($captureInfo.columns)"
                $captureSet = Capture-RangeImageSetWithPartialCache $origSheet $shrunkSheet $origRange $shrunkRange $OriginalCacheDir $ShrunkCacheDir $imageBase "sheet $i $primaryStrategy" $primaryStrategy $null $cachedOriginal $cachedShrunk
                $captureRange = $captureSet.capture_range
                $captureStrategy = $captureSet.capture_strategy
                $captureInfo = $captureSet.capture_info
                $origImage = $captureSet.original_image
                $shrunkImage = $captureSet.shrunk_image
                $origBackgroundRgba = $captureSet.original_background_rgba
                $shrunkBackgroundRgba = $captureSet.shrunk_background_rgba
                $captureTiles = @($captureSet.tiles)
            }
            catch {
                $usedRangeError = $_.Exception.Message
                Write-ProgressLine "primary range failed index=$i strategy=$primaryStrategy error='$usedRangeError'"
                if ($null -eq $cachedOriginal -and -not $originalAlreadyCachedEnabled) {
                    Remove-Item $origImage -Force -ErrorAction SilentlyContinue
                    Remove-Item (Join-Path $OriginalCacheDir ("{0}-tile-*.png" -f $imageBase)) -Force -ErrorAction SilentlyContinue
                }
                if ($null -eq $cachedShrunk -and -not $shrunkAlreadyCachedEnabled) {
                    Remove-Item $shrunkImage -Force -ErrorAction SilentlyContinue
                    Remove-Item (Join-Path $ShrunkCacheDir ("{0}-tile-*.png" -f $imageBase)) -Force -ErrorAction SilentlyContinue
                }

                $origRange = Resolve-CaptureRange $origSheet $null
                $shrunkRange = Matching-WorksheetRange $shrunkSheet $origRange
                $captureRange = Range-Address $origRange
                $captureInfo = Range-CaptureInfo $origRange "used_range_after_primary_failure" $captureRange $usedRangeError
                Write-ProgressLine "sheet used range fallback index=$i range='$captureRange' start_row=$($captureInfo.start_row) start_column=$($captureInfo.start_column) rows=$($captureInfo.rows) columns=$($captureInfo.columns)"
                $captureSet = Capture-RangeImageSetWithPartialCache $origSheet $shrunkSheet $origRange $shrunkRange $OriginalCacheDir $ShrunkCacheDir $imageBase "sheet $i used range fallback" "used_range_after_primary_failure" $usedRangeError $cachedOriginal $cachedShrunk
                $captureRange = $captureSet.capture_range
                $captureStrategy = $captureSet.capture_strategy
                $captureInfo = $captureSet.capture_info
                $origImage = $captureSet.original_image
                $shrunkImage = $captureSet.shrunk_image
                $origBackgroundRgba = $captureSet.original_background_rgba
                $shrunkBackgroundRgba = $captureSet.shrunk_background_rgba
                $captureTiles = @($captureSet.tiles)
            }

            [void]$captures.Add([pscustomobject]@{
                sheet_index = $i
                sheet_name = $origSheet.Name
                capture_range = $captureRange
                capture_strategy = $captureStrategy
                used_range_error = $usedRangeError
                capture_info = $captureInfo
                original_image = $origImage
                shrunk_image = $shrunkImage
                original_background_rgba = $origBackgroundRgba
                shrunk_background_rgba = $shrunkBackgroundRgba
                tiles = @($captureTiles)
            })
            Write-ProgressLine "sheet captured index=$i strategy=$captureStrategy range='$captureRange' tiles=$(@($captureTiles).Count)"
            Write-ProgressLine "sheet done index=$i"
        }
        catch {
            $capturedOriginalImage = if ($null -ne $origImage -and (Test-Path $origImage)) { $origImage } else { $null }
            $capturedShrunkImage = if ($null -ne $shrunkImage -and (Test-Path $shrunkImage)) { $shrunkImage } else { $null }
            $errorCaptureStrategy = if ($null -ne $captureStrategy) { $captureStrategy } elseif ($null -ne $captureInfo) { $captureInfo.strategy } else { $null }
            [void]$captures.Add([pscustomobject]@{
                sheet_index = $i
                sheet_name = $origSheet.Name
                capture_range = $captureRange
                capture_strategy = $errorCaptureStrategy
                used_range_error = $usedRangeError
                screenshot_error = $_.Exception.Message
                capture_info = $captureInfo
                original_image = $capturedOriginalImage
                shrunk_image = $capturedShrunkImage
                original_background_rgba = $origBackgroundRgba
                shrunk_background_rgba = $shrunkBackgroundRgba
                tiles = @($captureTiles)
            })
            Write-ProgressLine "sheet error index=$i error='$($_.Exception.Message)'"
        }
    }
}
finally {
    if ($original -ne $null) { $original.Close($false) }
    if ($shrunk -ne $null) { $shrunk.Close($false) }
    if ($excel -ne $null) { $excel.Quit() }
}

if ($captures.Count -eq 0) {
    '[]' | Set-Content -Path $CaptureManifestPath -Encoding UTF8
}
else {
    @($captures) | ConvertTo-Json -Depth 8 | Set-Content -Path $CaptureManifestPath -Encoding UTF8
}
@($script:OpenTimings) | ConvertTo-Json -Depth 4 | Set-Content -Path $OpenTimingsPath -Encoding UTF8
Write-ProgressLine "screenshot capture finished captures=$($captures.Count) manifest='$CaptureManifestPath'"
