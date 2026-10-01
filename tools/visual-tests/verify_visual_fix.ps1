param(
    [string]$Repository = (Join-Path $PSScriptRoot '..\..'),
    [switch]$LiveExcel
)
# Windows PowerShell 5.1 / .NET Framework checks. No Excel or clipboard access
# occurs unless -LiveExcel is supplied. Native declarations are only compiled.
$ErrorActionPreference = 'Stop'
$Repository = (Resolve-Path -LiteralPath $Repository).Path
$source = Join-Path $Repository 'excel-shrink-cli\src\regression'
$script:Assertions = 0
$script:Messages = New-Object 'System.Collections.Generic.List[string]'
function Assert-Visual($condition, [string]$message) {
    if (-not $condition) { throw "FAIL: $message" }
    $script:Assertions++
    Write-Host "PASS: $message"
}
function Write-ProgressLine([string]$message) { [void]$script:Messages.Add($message) }
foreach ($file in @(Get-ChildItem -LiteralPath $source -Filter '*.ps1') + @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1')) {
    $tokens=$null; $errors=$null
    [void][System.Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref]$tokens, [ref]$errors)
    Assert-Visual ($errors.Count -eq 0) "PowerShell parser: $($file.Name) $($errors | Out-String)"
}
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Xml.Linq
Add-Type -AssemblyName System.Drawing
if (-not ('ExcelShrink.Visual.WorkbookPlan' -as [type])) {
    Add-Type -Path (Join-Path $source 'excel_visual_plan.cs') -ReferencedAssemblies @(
        'System.dll','System.Core.dll','System.Xml.dll','System.Xml.Linq.dll',
        'System.IO.Compression.dll','System.IO.Compression.FileSystem.dll')
}
Assert-Visual ($null -ne ('ExcelShrink.Visual.WorkbookPlan' -as [type])) 'C# OOXML planner compiled'
. (Join-Path $source 'excel_visual_bounds.ps1')
. (Join-Path $source 'excel_visual_clipboard.ps1')

function New-PlanFixture([string]$path, [string]$sheetXml, [string]$drawingXml) {
    $parts = [ordered]@{
        'xl/workbook.xml' = '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="r1"/></sheets></workbook>'
        'xl/_rels/workbook.xml.rels' = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="ss" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="st" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>'
        'xl/worksheets/sheet1.xml' = $sheetXml
        'xl/worksheets/_rels/sheet1.xml.rels' = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="d" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>'
        'xl/sharedStrings.xml' = '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><r><t>Small </t></r><r><rPr><sz val="28"/></rPr><t>LARGE</t></r><rPh sb="0" eb="2"><t>phonetic annotation</t></rPh></si></sst>'
        'xl/styles.xml' = '<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cellStyleXfs><xf numFmtId="0"/></cellStyleXfs><cellXfs><xf numFmtId="0"/><xf numFmtId="165"/></cellXfs></styleSheet>'
        'xl/drawings/drawing1.xml' = $drawingXml
    }
    $file=[IO.File]::Open($path,[IO.FileMode]::Create)
    $zip=[IO.Compression.ZipArchive]::new($file,[IO.Compression.ZipArchiveMode]::Create,$false)
    try {
        foreach ($item in $parts.GetEnumerator()) {
            $entry=$zip.CreateEntry([string]$item.Key)
            $stream=$entry.Open()
            try { $bytes=[Text.Encoding]::UTF8.GetBytes([string]$item.Value); $stream.Write($bytes,0,$bytes.Length) }
            finally { $stream.Dispose() }
        }
    } finally { $zip.Dispose(); $file.Dispose() }
}
$sheet = @'
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
 <cols><col min="3" max="3" hidden="1"/></cols><sheetData>
 <row r="1"><c r="A1" t="inlineStr"><is><t>Blocked by B1</t></is></c><c r="B1"><v>3</v></c></row>
 <row r="2"><c r="A2" t="s"><v>0</v></c></row>
 <row r="3" hidden="1"><c r="A3" t="inlineStr"><is><t>Hidden row</t></is></c></row>
 <row r="4"><c r="A4" t="inlineStr"><is><t>_x005F_x0041_</t></is></c><c r="B4"/></row>
 <row r="5"><c r="A5" t="inlineStr"><is><t>Visible</t></is></c><c r="C5"><v>7</v></c></row>
 <row r="6"><c r="A6" s="1" t="str"><v>Custom format</v></c></row>
 <row r="7"><c r="A7" t="inlineStr"><is><t>Merged</t></is></c></row>
 <row r="8"><c r="D8" t="inlineStr"><is><t>Outside requested columns</t></is></c></row>
 <row r="999999"><c r="A999999" t="inlineStr"><is><t>Unrelated tail</t></is></c></row>
 </sheetData><mergeCells><mergeCell ref="A7:B8"/></mergeCells><drawing r:id="d"/>
</worksheet>
'@
$header='<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">'
$markerFrom='<xdr:from><xdr:col>2</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>'
$markerTo='<xdr:to><xdr:col>3</xdr:col><xdr:colOff>12700</xdr:colOff><xdr:row>9</xdr:row><xdr:rowOff>25400</xdr:rowOff></xdr:to>'
$drawing=[Text.StringBuilder]::new(); [void]$drawing.Append($header)
for ($i=1; $i -le 1920; $i++) {
    [void]$drawing.Append('<xdr:twoCellAnchor>' + $markerFrom + $markerTo + '<xdr:sp><xdr:nvSpPr><xdr:cNvPr id="' + $i + '" name="Line ' + $i + '"/></xdr:nvSpPr><xdr:spPr><a:prstGeom prst="line"/><a:extLst><a:ext uri="{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}"><a16:creationId xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main" id="{00000000-0000-0000-0000-000000000001}"/></a:ext><a:ext uri="{909E8E84-426E-40DD-AFC4-6F175D3DCCD1}"><a14:hiddenFill xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"/></a:ext><a:ext uri="{91240B29-F687-4F45-9708-019B960494DF}"><a14:hiddenLine xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"/></a:ext></a:extLst></xdr:spPr><xdr:txBody><a:bodyPr/><a:p/></xdr:txBody></xdr:sp><xdr:clientData/></xdr:twoCellAnchor>')
}
[void]$drawing.Append('</xdr:wsDr>')
$temp=Join-Path ([IO.Path]::GetTempPath()) ('es-planner-tests-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temp | Out-Null
$planObject=$null
try {
    $fixture=Join-Path $temp 'fixture.xlsx'
    New-PlanFixture $fixture $sheet $drawing.ToString()
    $planObject=[ExcelShrink.Visual.WorkbookPlan]::new($fixture)
    [Action[string]]$progress={ param([string]$message) Write-ProgressLine $message }
    $plan=$planObject.Get('Data',10,2,$progress)
    Assert-Visual ($plan.FastTwoCellShapes -eq 1920 -and $plan.FallbackShapes.Count -eq 0 -and -not $plan.AllShapesNeedCom) '1920 unrotated line anchors with known metadata extensions need no per-shape COM fallback'
    Assert-Visual ($plan.MaxMarkerRow -eq 10 -and $plan.MaxMarkerColumn -eq 4 -and $plan.MaxRowOffset -eq 2 -and $plan.MaxColumnOffset -eq 1) 'Zero-based drawing markers and EMU offsets'
    Assert-Visual ($plan.Text.Count -eq 5) 'Only visible rightmost text candidates inside the requested range'
    $byRow=@{}; foreach ($c in $plan.Text) { $byRow[$c.Row]=$c }
    Assert-Visual ($byRow[2].RichText -and $byRow[2].Text -eq 'Small LARGE') 'Rich shared strings exclude phonetic annotation text'
    Assert-Visual ($byRow[4].Text -eq '_x0041_') 'Escaped underscore is not decoded twice'
    Assert-Visual ($byRow[6].NeedsLiveFormatting) 'Non-general number formats use Excel display text'
    Assert-Visual ($byRow[7].MergeEndRow -eq 8 -and $byRow[7].MergeEndColumn -eq 2 -and $byRow[7].NeedsLiveFormatting) 'Merged-cell geometry is retained'
    Assert-Visual ([object]::ReferenceEquals($plan,$planObject.Get('Data',10,2,$progress))) 'Worksheet plans are memoized by range'
    Assert-Visual ($planObject.Get('Data',10,4,$progress).Text.Count -eq 6) 'A changed requested range gets its own plan'
    $planObject.Dispose(); $planObject=$null
    $variants=$header + '<xdr:oneCellAnchor>' + $markerFrom + '<xdr:ext cx="1270000" cy="0"/><xdr:sp><xdr:nvSpPr><xdr:cNvPr id="1" name="Horizontal"/></xdr:nvSpPr></xdr:sp></xdr:oneCellAnchor>' +
        '<xdr:absoluteAnchor><xdr:pos x="2540000" y="1270000"/><xdr:ext cx="0" cy="1270000"/><xdr:sp><xdr:nvSpPr><xdr:cNvPr id="2" name="Vertical"/></xdr:nvSpPr></xdr:sp></xdr:absoluteAnchor>' +
        '<xdr:twoCellAnchor editAs="absolute">' + $markerFrom + $markerTo + '<xdr:sp><xdr:nvSpPr><xdr:cNvPr id="3" name="Fixed"/></xdr:nvSpPr><xdr:spPr><a:xfrm><a:off x="1270000" y="2540000"/><a:ext cx="127000" cy="0"/></a:xfrm></xdr:spPr></xdr:sp></xdr:twoCellAnchor>' +
        '<xdr:twoCellAnchor>' + $markerFrom + $markerTo + '<xdr:sp><xdr:nvSpPr><xdr:cNvPr id="4" name="Rotated"/></xdr:nvSpPr><xdr:spPr><a:xfrm rot="1800000"/></xdr:spPr></xdr:sp></xdr:twoCellAnchor>' +
        '<xdr:twoCellAnchor>' + $markerFrom + $markerTo + '<xdr:sp><xdr:nvSpPr><xdr:cNvPr id="5" name="UnknownExt"/></xdr:nvSpPr><xdr:spPr><a:extLst><a:ext uri="{11111111-2222-3333-4444-555555555555}"><u:payload xmlns:u="urn:unknown"/></a:ext></a:extLst></xdr:spPr></xdr:sp></xdr:twoCellAnchor>' +
        '<xdr:twoCellAnchor>' + $markerFrom + $markerTo + '<xdr:sp><xdr:nvSpPr><xdr:cNvPr id="6" name="Duplicate"/></xdr:nvSpPr><xdr:spPr><a:xfrm rot="1800000"/></xdr:spPr></xdr:sp></xdr:twoCellAnchor>' +
        '<xdr:twoCellAnchor>' + $markerFrom + $markerTo + '<xdr:sp><xdr:nvSpPr><xdr:cNvPr id="7" name="Duplicate"/></xdr:nvSpPr><xdr:spPr><a:prstGeom prst="line"/></xdr:spPr></xdr:sp></xdr:twoCellAnchor></xdr:wsDr>'
    New-PlanFixture $fixture $sheet $variants
    $planObject=[ExcelShrink.Visual.WorkbookPlan]::new($fixture); $v=$planObject.Get('Data',10,2,$progress)
    Assert-Visual ($v.PointAnchors.Count -eq 3 -and $v.PointAnchors[0].Height -eq 0 -and $v.PointAnchors[1].Width -eq 0) 'One-cell/absolute anchors accept zero-size line axes'
    Assert-Visual ($v.PointAnchors[2].X -eq 100 -and $v.PointAnchors[2].Y -eq 200 -and $v.PointAnchors[2].Width -eq 10) 'Fixed-position two-cell anchors use their saved transform'
    Assert-Visual ($v.FallbackShapes.Count -eq 3 -and $v.FallbackShapes.Contains('Rotated') -and $v.FallbackShapes.Contains('UnknownExt') -and $v.FallbackShapes.Contains('Duplicate')) 'Rotated and unknown-extension geometry falls back to Excel'
    Assert-Visual ($v.AmbiguousFallbackShapes.Count -eq 1 -and $v.AmbiguousFallbackShapes[0] -eq 'Duplicate') 'Duplicate OOXML shape names are marked for index enumeration instead of ambiguous name lookup' 
} finally { if ($null -ne $planObject) {$planObject.Dispose()}; Remove-Item -LiteralPath $temp -Recurse -Force }

Assert-Visual (Test-VisualNull ([DBNull]::Value)) 'DBNull is recognized explicitly'
Assert-Visual ($null -eq (Convert-VisualFiniteNumber ([DBNull]::Value))) 'DBNull numeric properties do not get cast'
Assert-Visual ($null -eq (Convert-VisualFiniteNumber ([double]::NaN))) 'NaN is rejected'
Assert-Visual ($null -eq (Convert-VisualFiniteNumber ([double]::PositiveInfinity))) 'Infinity is rejected'
Assert-Visual ($null -eq (Convert-VisualFiniteNumber '')) 'Blank numeric properties are not silently converted to zero'
Assert-Visual ((Convert-VisualFiniteNumber 0) -eq 0) 'Zero is a valid line extent'
$font=[pscustomobject]@{Name='Arial';Size=[DBNull]::Value;Bold=$false;Italic=$false}
Assert-Visual ($null -eq (Get-VisualFont $font @{})) 'Mixed font sizes select per-character measurement, not an 11pt guess'
$state=@{right=0.0;bottom=0.0;maxRow=1;maxColumn=1;comShapes=0}
$shape=[pscustomobject]@{Visible=-1;Left=[DBNull]::Value;Top=10;Width=0;Height=20;Rotation=0;BottomRightCell=[pscustomobject]@{Left=50;Top=100;Width=20;Height=15;Row=10;Column=3}}
Add-ComVisualShape $shape $state 'mock shape with mixed numeric property'
Assert-Visual ($state.right -ge 70 -and $state.bottom -ge 115 -and $state.comShapes -eq 1) 'Bad geometry uses a known BottomRightCell instead of losing the sheet'
$shape.BottomRightCell=$null; $threw=$false
try { Add-ComVisualShape $shape $state 'unresolvable mock shape' } catch {$threw=$true}
Assert-Visual $threw 'Unresolvable geometry stays an explicit failure, never a false pass'
$a=[pscustomobject]@{sequence=10;owner_pid=42}
$b=[pscustomobject]@{sequence=11;owner_pid=42}
$c=[pscustomobject]@{sequence=11;owner_pid=99}
$d=[pscustomobject]@{sequence=12;owner_pid=42}
Assert-Visual (Test-VisualClipboardImageIdentity $a $b $b 42) 'Fresh stable Excel-owned clipboard image accepted'
Assert-Visual (-not (Test-VisualClipboardImageIdentity $a $b $c 42)) 'Foreign clipboard owner rejected'
Assert-Visual (-not (Test-VisualClipboardImageIdentity $a $b $d 42)) 'Sequence changes during GetImage require rereading'
Assert-Visual (-not (Test-VisualClipboardImageIdentity $a $a $a 42)) 'An old Excel clipboard image is not mistaken for this capture'
Assert-Visual (-not (Test-VisualClipboardImageIdentity $a $b $b 0)) 'Unknown Excel PID cannot certify clipboard provenance'
Assert-Visual (Test-VisualRangeCaptureSplittableError '[excel-shrink:range-copy-picture] CopyPicture failed') 'Repeated Range.CopyPicture failures are splittable'
Assert-Visual (Test-VisualRangeCaptureSplittableError 'Exception: not enough memory') 'Memory-like range capture failures are splittable'
Assert-Visual (-not (Test-VisualRangeCaptureSplittableError 'Clipboard owner is not this Excel instance.')) 'Foreign clipboard ownership is not hidden by range splitting'
Assert-Visual (-not (Test-VisualRangeCaptureSplittableError 'PNG save failed: disk full')) 'File-system failures are not hidden by range splitting'
$utf8Path = [IO.Path]::GetTempFileName()
try {
    [IO.File]::WriteAllText($utf8Path, '{"sheet_name":"Inhaltsübersicht","description":"Erläuterung zu CSV-Tabellen"}', [Text.UTF8Encoding]::new($false))
    $utf8Json = Get-Content -LiteralPath $utf8Path -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-Visual ($utf8Json.sheet_name -eq 'Inhaltsübersicht' -and $utf8Json.description -eq 'Erläuterung zu CSV-Tabellen') 'UTF-8 without BOM preserves non-ASCII sheet names'
} finally { Remove-Item -LiteralPath $utf8Path -Force -ErrorAction SilentlyContinue }
$screenshotSource = Get-Content -LiteralPath (Join-Path $source 'excel_screenshots.ps1') -Raw -Encoding UTF8
Assert-Visual ($screenshotSource.Contains('Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json')) 'Production screenshot cache JSON is read explicitly as UTF-8'
Assert-Visual ($screenshotSource.Contains('Get-Content -LiteralPath $RangeLimitPath -Raw -Encoding UTF8 | ConvertFrom-Json')) 'Range-limit JSON is read explicitly as UTF-8'
Assert-Visual ($screenshotSource.Contains('Get-Content -LiteralPath $SkipSheetsPath -Raw -Encoding UTF8 | ConvertFrom-Json')) 'Skip-sheet JSON is read explicitly as UTF-8'
Write-Host "Native checks passed: $script:Assertions assertions."
if ($LiveExcel) {
    & (Join-Path $PSScriptRoot 'verify_excel_visuals.ps1') -Repository $Repository -LiveExcel
} else { Write-Host 'Excel rendering not tested. Add -LiveExcel on an interactive Windows desktop.' }
