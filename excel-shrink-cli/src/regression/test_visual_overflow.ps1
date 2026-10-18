$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Drawing
. (Join-Path $PSScriptRoot 'excel_visual_overflow.ps1')
function Write-ProgressLine($message) {}
function Column-Name($column) { return 'C' }
function Test-VisualRangeCaptureSplittableError($message) { return $message.Contains('[excel-shrink:range-copy-picture]') }

$bitmap=[Drawing.Bitmap]::new(40,40)
$graphics=[Drawing.Graphics]::FromImage($bitmap)
try {
 $graphics.Clear([Drawing.Color]::White)
 if([ExcelShrinkStripPixels]::HasContent($bitmap,[Drawing.Color]::White.ToArgb())){throw 'Blank detected as content'}
 $graphics.FillRectangle([Drawing.Brushes]::Black,1,1,2,4)
 if(-not [ExcelShrinkStripPixels]::HasContent($bitmap,[Drawing.Color]::White.ToArgb())){throw 'Edge overflow missed'}
}finally{$graphics.Dispose();$bitmap.Dispose()}

$script:OverflowCopyMs=500;$script:OverflowHeightMs=1
function Measure-OverflowHeight($sheet,$column,$rows) { $script:OverflowHeightMs=1;return $rows*20+1 }
if((Get-OverflowTileRows $null 3 10000) -ne 1599){throw 'Unsafe tile rows'}
if((Get-OverflowTileRows $null 3 80) -ne 80){throw 'Small range expanded beyond bounds'}

$sheet=New-Object PSObject
$sheet | Add-Member ScriptMethod Range {param($address) return [pscustomobject]@{Height=30;Address=$address}}
$script:visits=New-Object 'System.Collections.Generic.List[string]'
function Read-VerifiedRangeImage($sheet,$range,$label){
 $numbers=[regex]::Matches($range.Address,'\d+')
 $a=[int]$numbers[0].Value;$b=[int]$numbers[1].Value
 if($b-$a+1 -gt 2){throw '[excel-shrink:range-copy-picture] synthetic size failure'}
 $script:visits.Add("$a-$b")
 $image=[Drawing.Bitmap]::new(10,10);$g=[Drawing.Graphics]::FromImage($image)
 try{$g.Clear([Drawing.Color]::White);if($a -eq 1){$g.FillRectangle([Drawing.Brushes]::Black,2,2,4,4)}}finally{$g.Dispose()}
 return ,$image
}
$script:OverflowBackground=[Drawing.Color]::White.ToArgb()
if(-not (Test-OverflowStrip $sheet 3 1 8)){throw 'Missing content'}
if(($script:visits -join ',') -ne '1-2,3-4,5-6,7-8'){throw 'Incomplete coverage after positive first tile'}
function Read-VerifiedRangeImage($sheet,$range,$label){throw 'unrecoverable'}
$failed=$false
try{Test-OverflowStrip $sheet 3 1 1}catch{$failed=$true}
if(-not $failed){throw 'Failure treated as blank'}
Write-Output 'Overflow tests passed: pixels, height planning, full recursive coverage, leaf failure.'

# Regression: never visit thousands of hidden columns individually. Width is
# additive even if Hidden would return Null for a mixed range.
function Column-Name([int]$column) {
 $name=''
 while($column -gt 0){$column--;$name=[char](65+($column%26))+$name;$column=[int][Math]::Floor($column/26)}
 return $name
}
function New-WidthSheet([int[]]$visible) {
 $mock=[pscustomobject]@{Name='Hidden tail';VisibleColumns=$visible}
 $mock | Add-Member ScriptMethod Range {
  param($address)
  $script:WidthQueries++
  $ends=$address.Split(':');$coordinates=@()
  foreach($end in $ends){$index=0;foreach($letter in ([regex]::Match($end,'^[A-Z]+').Value).ToCharArray()){$index=$index*26+[int]$letter-64};$coordinates+= $index}
  $count=@($this.VisibleColumns | Where-Object {$_ -ge $coordinates[0] -and $_ -le $coordinates[-1]}).Count
  return [pscustomobject]@{Width=$count*8.0;Address=$address}
 }
 return $mock
}
$script:WidthQueries=0;$mock=New-WidthSheet (1..8)
if((Find-NextVisibleOverflowColumn $mock 9) -ne 0 -or $script:WidthQueries -ne 2){throw 'Hidden suffix must finish in two width queries'}
$script:WidthQueries=0;$mock=New-WidthSheet @(8,16384)
if((Find-NextVisibleOverflowColumn $mock 9) -ne 16384 -or $script:WidthQueries -gt 16){throw 'Visible island missed or linear scan'}
$script:WidthQueries=0;$mock=New-WidthSheet @(8,20,40,500)
if((Find-NextVisibleOverflowColumn $mock 9) -ne 20){throw 'Did not find first visible column after gap'}
if((Find-NextVisibleOverflowColumn $mock 21) -ne 40){throw 'Later visible island missed'}
$script:WidthQueries=0
if((Find-NextVisibleOverflowColumn $mock 8) -ne 8 -or $script:WidthQueries -ne 1){throw 'Visible immediate neighbour needs only one query'}
$script:WidthQueries=0
if((Find-NextVisibleOverflowColumn $mock 16385) -ne 0 -or $script:WidthQueries -ne 0){throw 'XFD boundary mishandled'}

function Measure-OverflowHeight($sheet,$column,$rows){$script:HeightQueries++;return 1200.0}
$script:HeightQueries=0
if((Get-OverflowTileRows $null 8 1048576) -ne 1048576 -or $script:HeightQueries -ne 1){throw 'Hidden rows should fit into a single physical image'}

function Find-VisualEndIndex { throw 'COM drawing point resolution must never run' }
$base=[pscustomobject]@{Row=1;Column=1;Rows=@{Count=40};Columns=@{Count=8};Left=0;Top=0;Width=64;Height=800}
function Add-VisualShapes { throw 'COM drawing enumeration must not run, even for legacy fallback plans' }
$limit=[pscustomobject]@{drawing_points=@([pscustomobject]@{row=999;column=999;right=10000;bottom=10000});drawing_com_fallback=$true}
function Test-OverflowStrip($sheet,$column,$first,$last){throw 'No image should be requested for a hidden suffix'}
$script:WidthQueries=0;$mock=New-WidthSheet (1..8)
$result=Expand-RustVisualCaptureRange $mock $base $limit
if($result.Address -ne 'A1:H40' -or $script:WidthQueries -ne 3){throw 'Hidden suffix changed capture bounds or scanned individually'}
$script:TestedColumns=New-Object 'System.Collections.Generic.List[int]'
function Test-OverflowStrip($sheet,$column,$first,$last){$script:TestedColumns.Add($column);return $column -eq 20}
$mock=New-WidthSheet @(8,20,40)
$result=Expand-RustVisualCaptureRange $mock $base $limit
if($result.Address -ne 'A1:T40' -or ($script:TestedColumns -join ',') -ne '20,40'){throw 'Visible gap traversal or blank stopping incorrect'}
Write-Output 'Hidden-area tests passed: whole suffix, distant visible islands, XFD, full-height fitting and actual traversal.'

# Capture setup must follow actual active-sheet changes, including workbook opens.
$app=[pscustomobject]@{ActiveSheet=$null;ActiveWindow=[pscustomobject]@{DisplayGridlines=$true}}
$script:BookActivations=0;$script:SheetActivations=0;$script:Selections=0
function New-CaptureBook { $b=[pscustomobject]@{}; $b | Add-Member ScriptMethod Activate {$script:BookActivations++}; return $b }
function New-CaptureSheet($book,$app) {
 $s=[pscustomobject]@{Parent=$book;Application=$app}
 $s | Add-Member ScriptMethod Activate {$script:SheetActivations++;$this.Application.ActiveSheet=$this}
 return $s
}
$book1=New-CaptureBook;$book2=New-CaptureBook
$s1=New-CaptureSheet $book1 $app;$s2=New-CaptureSheet $book1 $app;$s3=New-CaptureSheet $book2 $app
$range=[pscustomobject]@{}
$range | Add-Member ScriptMethod Select {$script:Selections++}
$script:CaptureGridlineSheets.Clear()
Prepare-VisualCapture $s1 $range 'first'
Prepare-VisualCapture $s1 $range 'second tile'
if($script:BookActivations -ne 1 -or $script:SheetActivations -ne 1 -or $script:CaptureGridlineSheets.Count -ne 1){throw 'Repeated tile reactivated or prepared sheet'}
Prepare-VisualCapture $s2 $range 'same book'
if($script:BookActivations -ne 1 -or $script:SheetActivations -ne 2){throw 'Same-book sheet switch reactivated workbook'}
Prepare-VisualCapture $s3 $range 'other book'
if($script:BookActivations -ne 2 -or $script:SheetActivations -ne 3){throw 'Cross-book activation missing'}
$app.ActiveSheet=$s2
Prepare-VisualCapture $s3 $range 'external active-sheet change'
if($script:BookActivations -ne 3 -or $script:SheetActivations -ne 4 -or $script:CaptureGridlineSheets.Count -ne 3 -or $script:Selections -ne 5){throw 'Actual Excel activation state was ignored'}
Write-Output 'Capture preparation tests passed: repeated tiles, sheet/book switches and external activation.'