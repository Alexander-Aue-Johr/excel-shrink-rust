$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot '..\run.ps1'
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($script, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -Path (Join-Path $PSScriptRoot '..\CaptureWorkbook.cs') `
    -ReferencedAssemblies @('System.IO.Compression', 'System.IO.Compression.FileSystem', 'System.Xml')

function Hide-WorksheetErrorIndicators([string]$content) {
    $source = New-Object IO.MemoryStream(,[Text.Encoding]::UTF8.GetBytes($content))
    $destination = New-Object IO.MemoryStream
    try {
        [CaptureWorkbook]::HideWorksheetErrorIndicators($source, $destination)
        return [Text.Encoding]::UTF8.GetString($destination.ToArray())
    } finally { $source.Dispose(); $destination.Dispose() }
}

function Parse-Worksheet([string]$content) {
    $document = New-Object Xml.XmlDocument
    $document.PreserveWhitespace = $true
    $document.LoadXml($content)
    return ,$document
}

function Assert-Equal($actual, $expected, [string]$message) {
    if ($actual -cne $expected) { throw "$message`: expected '$expected', got '$actual'" }
}

$source = @'
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1"><c r="A1" t="inlineStr" xmlns:test="urn:test" test:note="  &#x9; attribute &#xD;&#xA; whitespace  "><is><t xml:space="preserve">  retained &#x9; text &#xD;&#xA; </t></is></c><c r="B1"><f>SUM(C1:D1)&amp;" &#xD; "</f><v>0</v></c></row></sheetData>
  <cellWatches><cellWatch r="A1"/></cellWatches>
  <drawing xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"/>
  <extLst/>
</worksheet>
'@
$before = Parse-Worksheet $source
$after = Parse-Worksheet (Hide-WorksheetErrorIndicators $source)
$originalData = $before.DocumentElement.SelectSingleNode('*[local-name()="sheetData"]')
$preparedData = $after.DocumentElement.SelectSingleNode('*[local-name()="sheetData"]')
Assert-Equal $preparedData.OuterXml $originalData.OuterXml 'Cell XML must be preserved'
$names = @($after.DocumentElement.ChildNodes | Where-Object NodeType -eq Element | ForEach-Object LocalName)
Assert-Equal ($names -join ',') 'sheetData,cellWatches,ignoredErrors,drawing,extLst' 'Worksheet child order'
$rule = $after.DocumentElement.SelectSingleNode('*[local-name()="ignoredErrors"]/*')
Assert-Equal $rule.GetAttribute('sqref') 'A1:XFD1048576' 'Worksheet coverage'
Assert-Equal $rule.Attributes.Count 10 'All nine warning classes must be covered'
foreach ($attribute in $rule.Attributes) {
    if ($attribute.Name -ne 'sqref') { Assert-Equal $attribute.Value '1' 'Warning flag' }
}

$existing = '<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheetData/><x:ignoredErrors><x:ignoredError sqref="A1" numberStoredAsText="1"/></x:ignoredErrors><x:drawing/></x:worksheet>'
$replaced = Parse-Worksheet (Hide-WorksheetErrorIndicators $existing)
Assert-Equal $replaced.DocumentElement.SelectNodes('*[local-name()="ignoredErrors"]').Count 1 'Existing ignoredErrors must be replaced'
Assert-Equal $replaced.DocumentElement.ChildNodes[1].Prefix 'x' 'Worksheet namespace prefix'
Assert-Equal $replaced.DocumentElement.ChildNodes[2].LocalName 'drawing' 'Existing order must be preserved'
Assert-Equal (Hide-WorksheetErrorIndicators $replaced.OuterXml) $replaced.OuterXml 'Preparation must be idempotent'

$withoutTrailing = '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>'
$appended = Parse-Worksheet (Hide-WorksheetErrorIndicators $withoutTrailing)
Assert-Equal $appended.DocumentElement.LastChild.LocalName 'ignoredErrors' 'Append when no trailing elements exist'

$workbook = Join-Path ([IO.Path]::GetTempPath()) ('capture-preparation-' + [Guid]::NewGuid().ToString('N') + '.xlsx')
try {
    $archive = [IO.Compression.ZipFile]::Open($workbook, [IO.Compression.ZipArchiveMode]::Create)
    try {
        $entries = @{
            '[Content_Types].xml' = '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>'
            'xl/worksheets/sheet1.xml' = $source
            'xl/sharedStrings.xml' = '<sst><si><t xml:space="preserve">  shared text  </t></si></sst>'
        }
        foreach ($name in $entries.Keys) {
            $entry = $archive.CreateEntry($name)
            $stream = $entry.Open()
            try {
                $bytes = [Text.Encoding]::UTF8.GetBytes($entries[$name])
                $stream.Write($bytes, 0, $bytes.Length)
            } finally { $stream.Dispose() }
        }
    } finally { $archive.Dispose() }
    [CaptureWorkbook]::HideErrorIndicators($workbook)
    $archive = [IO.Compression.ZipFile]::OpenRead($workbook)
    try {
        foreach ($name in $entries.Keys) {
            $reader = New-Object IO.StreamReader($archive.GetEntry($name).Open())
            try { $actual = $reader.ReadToEnd() } finally { $reader.Dispose() }
            $expected = if ($name -eq 'xl/worksheets/sheet1.xml') { Hide-WorksheetErrorIndicators $source } else { $entries[$name] }
            Assert-Equal $actual $expected "Prepared ZIP member $name"
        }
    } finally { $archive.Dispose() }
    Assert-Equal ([IO.File]::Exists($workbook + '.prepared')) $false 'Prepared ZIP must be published'
} finally { Remove-Item -LiteralPath $workbook -Force -ErrorAction SilentlyContinue }
Write-Host 'Worksheet preparation tests passed.'
