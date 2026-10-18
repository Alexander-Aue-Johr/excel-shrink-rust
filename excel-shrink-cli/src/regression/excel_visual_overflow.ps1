# Rust supplies data/merge/drawing bounds. Excel only resolves physical geometry
# and reads image strips; no Cells.Find, Value2 scan or text-width estimation.
if (-not ('ExcelShrinkStripPixels' -as [type])) {
    Add-Type -ReferencedAssemblies System.Drawing.dll @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class ExcelShrinkStripPixels {
    public static bool HasContent(Bitmap source, int background) {
        using (var bitmap = new Bitmap(source.Width, source.Height, PixelFormat.Format32bppArgb)) {
            using (var g = Graphics.FromImage(bitmap)) g.DrawImageUnscaled(source, 0, 0);
            var data = bitmap.LockBits(new Rectangle(0,0,bitmap.Width,bitmap.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            try {
                var row = new byte[bitmap.Width * 4];
                int br = (background >> 16) & 255, bg = (background >> 8) & 255, bb = background & 255;
                // CopyPicture's outer boundary is not worksheet content. Keep
                // the first interior pixel, including narrow edge overflow.
                for (int y = 1; y < bitmap.Height-1; ++y) {
                    Marshal.Copy(IntPtr.Add(data.Scan0, y * data.Stride), row, 0, row.Length);
                    for (int x = 1; x < bitmap.Width-1; ++x) {
                        int p=x*4;
                        if (Math.Abs(row[p]-bb)>14 || Math.Abs(row[p+1]-bg)>14 || Math.Abs(row[p+2]-br)>14) return true;
                    }
                }
                return false;
            } finally { bitmap.UnlockBits(data); }
        }
    }
}
'@
}

$script:OverflowReleaseCount = 0L
$script:OverflowReleaseMs = 0.0
function Release-OverflowCom($obj) {
    if ($null -ne $obj -and [Runtime.InteropServices.Marshal]::IsComObject($obj)) {
        $watch=[Diagnostics.Stopwatch]::StartNew()
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($obj)
        # Aggregate per sheet: writing one log line per temporary range can
        # dominate the operation that we are trying to measure.
        $script:OverflowReleaseCount++
        $script:OverflowReleaseMs += $watch.Elapsed.TotalMilliseconds
    }
}
function Write-OverflowTiming([string]$phase, $watch, [string]$detail) {
    Write-ProgressLine "visual step phase=excel.overflow.$phase duration_ms=$($watch.Elapsed.TotalMilliseconds) $detail"
}
# Compare against Excel's actual active sheet: opening another workbook can
# change it outside this helper. Cache only successful gridline preparation.
$script:CaptureGridlineSheets = New-Object System.Collections.ArrayList
function Prepare-VisualCapture($worksheet, $range, [string]$label) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    $activated=$false;$gridlines=$false
    try {
        $active=$worksheet.Application.ActiveSheet
        if(-not [object]::ReferenceEquals($active,$worksheet)) {
            $book=$worksheet.Parent
            if($null -eq $active -or -not [object]::ReferenceEquals($active.Parent,$book)) {
                [void]$book.Activate()
            }
            [void]$worksheet.Activate()
            $activated=$true
        }
        $prepared=$false
        foreach($known in $script:CaptureGridlineSheets) {
            if([object]::ReferenceEquals($known,$worksheet)){$prepared=$true;break}
        }
        if(-not $prepared) {
            $window=$worksheet.Application.ActiveWindow
            if($null -eq $window){throw 'No active Excel window for capture'}
            $window.DisplayGridlines=$false
            [void]$script:CaptureGridlineSheets.Add($worksheet)
            $gridlines=$true
        }
        # Select each new capture range once, not on every clipboard retry.
        [void]$range.Select()
    } finally {
        Write-OverflowTiming 'prepare' $watch "label='$label' activated=$activated gridlines_set=$gridlines"
    }
}
function Initialize-OverflowCalibration($sheet) {
    $watch=[Diagnostics.Stopwatch]::StartNew();$range=$null;$image=$null
    try {
        $range=$sheet.Range('A1:A80')
        Prepare-VisualCapture $sheet $range 'overflow reference'
        $image=Invoke-VisualClipboardImage { [void]$range.CopyPicture(1,2) } 'overflow reference' ([uint32]$script:ExcelCaptureProcessId)
        $script:OverflowScaleY=($image.Height-1)/[double]$range.Height
        $script:OverflowScaleX=($image.Width-1)/[double]$range.Width
        $script:OverflowBackground=$image.GetPixel(2,2).ToArgb()
        if ($script:OverflowScaleY -le 0 -or $script:OverflowScaleX -le 0) { throw 'Invalid image scale calibration' }
    } finally {
        if($image){$image.Dispose()};Release-OverflowCom $range
        $script:OverflowCopyMs=[Math]::Max(1,$watch.Elapsed.TotalMilliseconds)
        Write-OverflowTiming 'calibration' $watch ''
    }
}
function Read-VerifiedRangeImage($worksheet, $range, [string]$label) {
    $watch=[Diagnostics.Stopwatch]::StartNew();$image=$null
    try {
        $expectedHeight=[double]$range.Height*$script:OverflowScaleY+1
        $expectedWidth=[double]$range.Width*$script:OverflowScaleX+1
        if($expectedHeight -le 1 -or $expectedWidth -le 1){throw '[excel-shrink:range-copy-picture] Zero-size visible range'}
        if($expectedHeight -gt 32000 -or $expectedWidth -gt 32000){
            throw "[excel-shrink:range-copy-picture] planned size exceeds safe bitmap axis: ${expectedWidth}x${expectedHeight}"
        }
        Prepare-VisualCapture $worksheet $range $label
        $image=Invoke-VisualClipboardImage {
            try { [void]$range.CopyPicture(1,2) }
            catch {
                # Recover preparation after an actual copy failure, not after
                # a delayed clipboard read or every successful tile.
                Prepare-VisualCapture $worksheet $range $label
                throw
            }
        } $label ([uint32]$script:ExcelCaptureProcessId)
        if($image.Height+3 -lt $expectedHeight*0.99 -or $image.Width+3 -lt $expectedWidth*0.99 -or
           ($image.Height -ge 32766 -and $expectedHeight -gt $image.Height+3) -or
           ($image.Width -ge 32766 -and $expectedWidth -gt $image.Width+3)) {
            throw "[excel-shrink:range-copy-picture] TRUNCATED expected=${expectedWidth}x${expectedHeight} actual=$($image.Width)x$($image.Height)"
        }
        $result=$image;$image=$null;return ,$result
    } finally {
        if($image){$image.Dispose()}
        $script:OverflowCopyMs=[Math]::Max(1,$watch.Elapsed.TotalMilliseconds)
        Write-OverflowTiming 'copy' $watch "label='$label'"
    }
}
function Measure-OverflowHeight($sheet,[int]$column,[int]$rows) {
    $watch=[Diagnostics.Stopwatch]::StartNew();$r=$null
    try {$letter=Column-Name $column;$r=$sheet.Range("${letter}1:${letter}$rows");return [double]$r.Height*$script:OverflowScaleY+1}
    finally {Release-OverflowCom $r;$script:OverflowHeightMs=$watch.Elapsed.TotalMilliseconds;Write-OverflowTiming 'height_query' $watch "rows=$rows column=$column"}
}
function Get-OverflowTileRows($sheet,[int]$column,[int]$lastRow) {
    $watch=[Diagnostics.Stopwatch]::StartNew()
    # Thousands of hidden rows may have the physical height of a single page.
    # Check the actual whole height before assuming that many rows need tiles.
    $wholeHeight=Measure-OverflowHeight $sheet $column $lastRow
    if($wholeHeight -le 32000){
        Write-OverflowTiming 'plan' $watch "rows_per_image=$lastRow last_row=$lastRow pixels=$wholeHeight whole_height_fits=True"
        return $lastRow
    }
    $n=[Math]::Min(1600,$lastRow)
    $h=if($n -eq $lastRow){$wholeHeight}else{Measure-OverflowHeight $sheet $column $n}
    # Oversize must become safe. Proportional guesses are validated, since rows
    # may have different heights. A strict decrease guarantees termination.
    while($h -gt 32000 -and $n -gt 1){
        $n=[Math]::Max(1,[Math]::Min($n-1,[int][Math]::Floor($n*31999/[Math]::Max(1,$h-1))))
        $h=Measure-OverflowHeight $sheet $column $n
    }
    for($i=0;$i -lt 2 -and $n -lt $lastRow -and $h -lt 31980;$i++){
        $candidate=[Math]::Min($lastRow,[int][Math]::Floor($n*31999/[Math]::Max(1,$h-1)))
        if($candidate -le $n){break}
        $saved=[Math]::Ceiling($lastRow/[double]$n)-[Math]::Ceiling($lastRow/[double]$candidate)
        if($saved*$script:OverflowCopyMs -le $script:OverflowHeightMs){break}
        $next=Measure-OverflowHeight $sheet $column $candidate
        if($next -gt 32000){break}
        $n=$candidate;$h=$next
    }
    Write-OverflowTiming 'plan' $watch "rows_per_image=$n last_row=$lastRow pixels=$h"
    return $n
}
function Get-OverflowColumnsWidth($sheet,[int]$first,[int]$last) {
    $range=$null
    try {
        $range=$sheet.Range("$(Column-Name $first):$(Column-Name $last)")
        $value=$range.Width
        if($null -eq $value -or $value -is [DBNull]){throw 'Unavailable column width during overflow visibility search'}
        $width=[double]$value
        if([double]::IsNaN($width) -or [double]::IsInfinity($width) -or $width -lt 0){throw 'Invalid column width during overflow visibility search'}
        return $width
    } finally {Release-OverflowCom $range}
}
function Find-NextVisibleOverflowColumn($sheet,[int]$first) {
    if($first -gt 16384){return 0}
    if($first -lt 1){throw 'Invalid first overflow column'}
    $watch=[Diagnostics.Stopwatch]::StartNew();$queries=0;$result=0
    try {
        $queries++
        if((Get-OverflowColumnsWidth $sheet $first $first) -gt 0){$result=$first;return $result}
        if($first -eq 16384){return 0}
        $low=$first+1;$high=16384;$queries++
        # Width is additive, including mixed Hidden values: a zero-width suffix
        # proves there is no visible column left. Never scan it column by column.
        if((Get-OverflowColumnsWidth $sheet $low $high) -le 0){return 0}
        # There is a visible column somewhere beyond this hidden block. Search
        # prefix widths, a monotone predicate even with several visible islands.
        while($low -lt $high){
            $middle=[int][Math]::Floor(($low+$high)/2.0);$queries++
            if((Get-OverflowColumnsWidth $sheet $low $middle) -gt 0){$high=$middle}else{$low=$middle+1}
        }
        $result=$low;return $result
    } finally {Write-OverflowTiming 'visible_column' $watch "first=$first next=$result width_queries=$queries"}
}
function Test-OverflowStrip($sheet,[int]$column,[int]$first,[int]$last) {
    $range=$null;$image=$null;$failure=$null;$content=$false
    try {
        $letter=Column-Name $column;$address="${letter}${first}:${letter}$last"
        $watch=[Diagnostics.Stopwatch]::StartNew();$range=$sheet.Range($address)
        Write-OverflowTiming 'range' $watch "address=$address"
        # A completely hidden row interval has no visible pixels to inspect.
        if([double]$range.Height -le 0){return $false}
        try {$image=Read-VerifiedRangeImage $sheet $range "overflow $address"}catch{$failure=$_.Exception.Message}
        if($null -eq $failure){
            $watch=[Diagnostics.Stopwatch]::StartNew()
            $content=[ExcelShrinkStripPixels]::HasContent($image,$script:OverflowBackground)
            Write-OverflowTiming 'pixels' $watch "address=$address content=$content"
        }
    } finally {if($image){$image.Dispose()};Release-OverflowCom $range}
    if($null -ne $failure){
        if(-not (Test-VisualRangeCaptureSplittableError $failure)){throw $failure}
        Write-ProgressLine "overflow split address=$address rows=$($last-$first+1) error='$failure'"
        if($first -eq $last){throw "Overflow undecidable at $address : $failure"}
        $middle=[int][Math]::Floor(($first+$last)/2)
        $a=Test-OverflowStrip $sheet $column $first $middle
        $b=Test-OverflowStrip $sheet $column ($middle+1) $last
        return ($a -or $b)
    }
    return $content
}
function Expand-RustVisualCaptureRange($worksheet,$baseRange,$limit) {
    $releaseStartCount=$script:OverflowReleaseCount
    $releaseStartMs=$script:OverflowReleaseMs
    try {
        $watch=[Diagnostics.Stopwatch]::StartNew()
        $maxRow=[int]$baseRange.Row+[int]$baseRange.Rows.Count-1
        $maxColumn=[int]$baseRange.Column+[int]$baseRange.Columns.Count-1
        # Rust row/column bounds are the starting approximation. No drawing
        # point conversion, Shapes access or COM geometry search is permitted.
        $tileRows=0;$lastContent=$maxColumn
        $column=Find-NextVisibleOverflowColumn $worksheet ($maxColumn+1)
        while($column -gt 0){
            if($tileRows -eq 0){$tileRows=Get-OverflowTileRows $worksheet $column $maxRow}
            $hasContent=$false
            for($row=1;$row -le $maxRow;$row+=$tileRows){
                $hit=Test-OverflowStrip $worksheet $column $row ([Math]::Min($maxRow,$row+$tileRows-1))
                $hasContent=$hasContent -or $hit
            }
            Write-ProgressLine "overflow column sheet='$($worksheet.Name)' column=$column last_row=$maxRow content=$hasContent"
            if(-not $hasContent){break}
            $lastContent=$column
            $column=Find-NextVisibleOverflowColumn $worksheet ($column+1)
        }
        Write-OverflowTiming 'total' $watch "sheet='$($worksheet.Name)' before_column=$maxColumn after_column=$lastContent last_row=$maxRow"
        return ,$worksheet.Range("A1:$(Column-Name $lastContent)$maxRow")
    } finally {
        $count=$script:OverflowReleaseCount-$releaseStartCount
        $ms=$script:OverflowReleaseMs-$releaseStartMs
        Write-ProgressLine "visual step phase=excel.overflow.com_release duration_ms=$ms count=$count aggregate=sheet"
    }
}
