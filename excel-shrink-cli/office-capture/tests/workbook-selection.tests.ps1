$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -Path (Join-Path $PSScriptRoot '..\CaptureWorkbook.cs') `
    -ReferencedAssemblies @('System.IO.Compression', 'System.IO.Compression.FileSystem', 'System.Xml')

Import-Module (Join-Path $PSScriptRoot '../CaptureSetup.psm1') -Force -DisableNameChecking

function Assert-Equal($actual, $expected, [string]$message) {
    if ($actual -cne $expected) { throw "$message`: expected '$expected', got '$actual'" }
}

function Parse-Xml([string]$text) {
    $document = New-Object Xml.XmlDocument
    $document.PreserveWhitespace = $true
    $document.LoadXml($text)
    return ,$document
}

function Semantic-Xml($node) {
    $result = New-Object Text.StringBuilder
    [void]$result.Append('(' + $node.NodeType + ':' + $node.NamespaceURI + ':' + $node.LocalName)
    foreach ($attribute in @($node.Attributes | Where-Object NamespaceURI -ne 'http://www.w3.org/2000/xmlns/' | Sort-Object NamespaceURI, LocalName)) {
        [void]$result.Append('[' + $attribute.NamespaceURI + ':' + $attribute.LocalName + '=' + ($attribute.Value | ConvertTo-Json -Compress) + ']')
    }
    if ($null -ne $node.Value) { [void]$result.Append(($node.Value | ConvertTo-Json -Compress)) }
    foreach ($child in $node.ChildNodes) {
        if ($child.NodeType -ne [Xml.XmlNodeType]::Whitespace) { [void]$result.Append((Semantic-Xml $child)) }
    }
    [void]$result.Append(')')
    return $result.ToString()
}

function New-Workbook([string]$path, $parts) {
    $archive = [IO.Compression.ZipFile]::Open($path, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($name in $parts.Keys) {
            $entry = $archive.CreateEntry($name)
            $writer = New-Object IO.StreamWriter($entry.Open(), [Text.UTF8Encoding]::new($false))
            try { $writer.Write($parts[$name]) } finally { $writer.Dispose() }
        }
    } finally { $archive.Dispose() }
}

function Read-Parts([string]$path) {
    $parts = @{}
    $archive = [IO.Compression.ZipFile]::OpenRead($path)
    try {
        foreach ($entry in $archive.Entries) {
            $reader = New-Object IO.StreamReader($entry.Open())
            try { $parts[$entry.FullName] = $reader.ReadToEnd() } finally { $reader.Dispose() }
        }
    } finally { $archive.Dispose() }
    return $parts
}

function Worksheet([string]$ns, [string]$prefix, [string]$views) {
    $p = if ($prefix) { $prefix + ':' } else { '' }
    $declaration = if ($prefix) { 'xmlns:' + $prefix + '="' + $ns + '"' } else { 'xmlns="' + $ns + '"' }
    return @'
<@P@worksheet @DECLARE@ xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:meta="urn:test" meta:note="  &#x9; metadata &#xD;&#xA; ">
<@P@sheetPr codeName="Preserved"/><@P@dimension ref="A1:D4"/>@VIEWS@
<@P@sheetFormatPr defaultRowHeight="13.5"/><@P@cols><@P@col min="1" max="1" width="8.5" customWidth="1"/></@P@cols>
<@P@sheetData><@P@row r="1" ht="18.75" customHeight="1"><@P@c r="A1" t="inlineStr"><@P@is><@P@t xml:space="preserve">  text &#x9; &#xD;&#xA;  </@P@t></@P@is></@P@c><@P@c r="B1"><@P@f>SUM(C1:D1)&amp;" preserved "</@P@f><@P@v>1</@P@v></@P@c></@P@row></@P@sheetData>
<@P@mergeCells count="1"><@P@mergeCell ref="A3:B3"/></@P@mergeCells><@P@drawing r:id="rId99"/>
</@P@worksheet>
'@.Replace('@P@', $p).Replace('@DECLARE@', $declaration).Replace('@VIEWS@', $views)
}

function Fixture([string]$ns, [string]$prefix, [bool]$withViews = $true) {
    $p = if ($prefix) { $prefix + ':' } else { '' }
    $declaration = if ($prefix) { 'xmlns:' + $prefix + '="' + $ns + '"' } else { 'xmlns="' + $ns + '"' }
    $relationships = if ($ns.StartsWith('http://purl.')) { 'http://purl.oclc.org/ooxml/officeDocument/relationships' } else { 'http://schemas.openxmlformats.org/officeDocument/2006/relationships' }
    $views = '<@P@sheetViews><@P@sheetView workbookViewId="0" tabSelected="1" zoomScale="85"><@P@pane xSplit="1" ySplit="2" state="frozen"/><@P@selection activeCell="C4" sqref="C4"/></@P@sheetView><@P@sheetView workbookViewId="1" tabSelected="1"/></@P@sheetViews>'.Replace('@P@', $p)
    $workbookViews = if ($withViews) { '<@P@bookViews><@P@workbookView activeTab="0" firstSheet="0" xWindow="25"/><@P@workbookView activeTab="1" firstSheet="0" xWindow="50"/></@P@bookViews>' } else { '' }
    $targetViews = if ($withViews) { '<@P@sheetViews><@P@sheetView workbookViewId="0" tabSelected="0" zoomScale="90"><@P@selection activeCell="B3" sqref="B3"/></@P@sheetView></@P@sheetViews>'.Replace('@P@', $p) } else { '' }
    return @{
        '[Content_Types].xml' = '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/worksheets/hidden.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/alpha.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/custom-name.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>'
        'xl/workbook.xml' = ('<@P@workbook @DECLARE@ xmlns:rel="@REL@"><@P@workbookPr checkCompatibility="1" date1904="1" showObjects="all"/>@VIEWS@<@P@sheets><@P@sheet name="Hidden" sheetId="41" state="hidden" rel:id="rId8"/><@P@sheet name="Alpha" sheetId="3" rel:id="rId4"/><@P@sheet name="Target" sheetId="97" rel:id="rId12"/></@P@sheets><@P@definedNames><@P@definedName name="Test">Target!$A$1</@P@definedName></@P@definedNames></@P@workbook>'.Replace('@VIEWS@', $workbookViews).Replace('@P@', $p).Replace('@DECLARE@', $declaration).Replace('@REL@', $relationships))
        'xl/_rels/workbook.xml.rels' = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId4" Target="worksheets/alpha.xml" Type="worksheet"/><Relationship Id="rId12" Target="/xl/worksheets/custom-name.xml" Type="worksheet"/><Relationship Id="rId8" Target="worksheets/hidden.xml" Type="worksheet"/></Relationships>'
        'xl/worksheets/hidden.xml' = (Worksheet $ns $prefix $(if ($withViews) { $views } else { '' }))
        'xl/worksheets/alpha.xml' = (Worksheet $ns $prefix $(if ($withViews) { $views } else { '' }))
        'xl/worksheets/custom-name.xml' = (Worksheet $ns $prefix $targetViews)
        'xl/sharedStrings.xml' = '<sst><si><t xml:space="preserve">  unchanged shared text  </t></si></sst>'
        'custom/payload.bin' = 'Opaque payload stays byte-identical.'
    }
}

$root = Join-Path ([IO.Path]::GetTempPath()) ('capture-selection-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
try {
    foreach ($ns in @('http://schemas.openxmlformats.org/spreadsheetml/2006/main', 'http://purl.oclc.org/ooxml/spreadsheetml/main')) {
        foreach ($prefix in @('', 'x')) {
            foreach ($hideErrors in @($false, $true)) {
                $source = Join-Path $root 'source.xlsx'; $copy = Join-Path $root 'copy.xlsx'
                $parts = Fixture $ns $prefix
                New-Workbook $source $parts
                $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
                Copy-Item -LiteralPath $source -Destination $copy
                Set-InitialWorksheet $copy 3 $hideErrors
                Assert-Equal (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash $sourceHash 'Input source must remain unchanged'
                $prepared = Read-Parts $copy
                $book = Parse-Xml $prepared['xl/workbook.xml']
                $properties = $book.SelectSingleNode('//*[local-name()="workbookPr"]')
                Assert-Equal $properties.GetAttribute('checkCompatibility') '0' 'Disable only the automatic compatibility check'
                Assert-Equal $properties.GetAttribute('date1904') '1' 'Preserve the date system'
                Assert-Equal $properties.GetAttribute('showObjects') 'all' 'Preserve object visibility'
                foreach ($view in $book.SelectNodes('//*[local-name()="workbookView"]')) {
                    Assert-Equal $view.GetAttribute('activeTab') '2' 'Active ordinal must not be sheetId or filename'
                    Assert-Equal $view.GetAttribute('firstSheet') '0' 'Tab strip position must remain unchanged'
                }
                foreach ($name in $parts.Keys) {
                    if (-not $name.StartsWith('xl/worksheets/') -and $name -ne 'xl/workbook.xml') {
                        Assert-Equal $prepared[$name] $parts[$name] "Untouched ZIP payload $name"
                        continue
                    }
                    if ($name -eq 'xl/workbook.xml') { continue }
                    $before = Parse-Xml $parts[$name]; $after = Parse-Xml $prepared[$name]
                    $selected = $name.EndsWith('custom-name.xml')
                    foreach ($view in $after.SelectNodes('//*[local-name()="sheetView"]')) {
                        Assert-Equal $view.GetAttribute('tabSelected') $(if ($selected) { '1' } else { '0' }) 'Each workbook window must select only the target'
                    }
                    Assert-Equal $after.SelectNodes('//*[local-name()="sheetView"]').Count 2 'Both workbook windows need consistent sheet views'
                    $beforeView = $before.SelectSingleNode('//*[local-name()="sheetView"][@workbookViewId="0"]')
                    $afterView = $after.SelectSingleNode('//*[local-name()="sheetView"][@workbookViewId="0"]')
                    $beforeView.RemoveAttribute('tabSelected'); $afterView.RemoveAttribute('tabSelected')
                    Assert-Equal (Semantic-Xml $afterView) (Semantic-Xml $beforeView) 'Zoom, panes and cell selections must be preserved'
                    foreach ($document in @($before, $after)) {
                        foreach ($node in @($document.SelectNodes('//*[local-name()="sheetViews" or local-name()="ignoredErrors"]'))) { [void]$node.ParentNode.RemoveChild($node) }
                    }
                    Assert-Equal (Semantic-Xml $after.DocumentElement) (Semantic-Xml $before.DocumentElement) 'All cell values, formulas, row heights and non-UI metadata must be preserved'
                }
                Set-InitialWorksheet $copy 3 $hideErrors
                $again = Read-Parts $copy
                foreach ($name in $prepared.Keys) { Assert-Equal $again[$name] $prepared[$name] 'Preparation must be idempotent' }
                Remove-Item -LiteralPath $source, $copy -Force
            }
        }
    }

    foreach ($flag in @('1', 'true', '0', 'false', 'absent')) {
        $source = Join-Path $root ('compatibility-' + $flag + '.xlsx')
        $parts = Fixture 'http://schemas.openxmlformats.org/spreadsheetml/2006/main' ''
        $replacement = if ($flag -eq 'absent') { '' } else { 'checkCompatibility="' + $flag + '"' }
        $parts['xl/workbook.xml'] = $parts['xl/workbook.xml'].Replace('checkCompatibility="1"', $replacement)
        New-Workbook $source $parts
        Set-InitialWorksheet $source 3
        $prepared = Read-Parts $source
        $book = Parse-Xml $prepared['xl/workbook.xml']
        $properties = $book.SelectSingleNode('//*[local-name()="workbookPr"]')
        $expected = if ($flag -in @('1', 'true')) { '0' } elseif ($flag -eq 'absent') { '' } else { $flag }
        Assert-Equal $properties.GetAttribute('checkCompatibility') $expected 'Preserve false/absent checks and disable enabled checks'
        Assert-Equal $properties.GetAttribute('date1904') '1' 'Compatibility setup must preserve date1904'
        Remove-Item -LiteralPath $source -Force
    }
    foreach ($flag in @('1', 'true', '0', 'false', 'absent')) {
        $source = Join-Path $root ('read-only-recommendation-' + $flag + '.xlsx')
        $parts = Fixture 'http://schemas.openxmlformats.org/spreadsheetml/2006/main' ''
        $attribute = if ($flag -eq 'absent') { '' } else { ' readOnlyRecommended="' + $flag + '"' }
        $sharing = '<fileSharing userName="Fixture Author" reservationPassword="ABCD"' + $attribute + '/>'
        $parts['xl/workbook.xml'] = $parts['xl/workbook.xml'].Replace('<workbookPr', $sharing + '<workbookPr')
        New-Workbook $source $parts
        Set-InitialWorksheet $source 3
        $prepared = Read-Parts $source
        $book = Parse-Xml $prepared['xl/workbook.xml']
        $properties = $book.SelectSingleNode('//*[local-name()="fileSharing"]')
        $expected = if ($flag -in @('1', 'true')) { '0' } elseif ($flag -eq 'absent') { '' } else { $flag }
        Assert-Equal $properties.GetAttribute('readOnlyRecommended') $expected 'Disable only enabled read-only recommendations'
        Assert-Equal $properties.GetAttribute('reservationPassword') 'ABCD' 'Preserve the write-reservation password'
        Assert-Equal $properties.GetAttribute('userName') 'Fixture Author' 'Preserve sharing attribution'
        Remove-Item -LiteralPath $source -Force
    }
    $source = Join-Path $root 'missing-views.xlsx'
    New-Workbook $source (Fixture 'http://schemas.openxmlformats.org/spreadsheetml/2006/main' '' $false)
    Set-InitialWorksheet $source 3
    $parts = Read-Parts $source
    $sheet = Parse-Xml $parts['xl/worksheets/custom-name.xml']
    $children = @($sheet.DocumentElement.ChildNodes | Where-Object NodeType -eq Element | ForEach-Object LocalName)
    Assert-Equal $children[2] 'sheetViews' 'Missing views must precede sheet format/data'
    Assert-Equal $sheet.SelectSingleNode('//*[local-name()="sheetView"]').GetAttribute('workbookViewId') '0' 'Missing workbook window defaults to view zero'
    Remove-Item -LiteralPath $source -Force

    $source = Join-Path $root 'with-chartsheet.xlsx'
    $parts = Fixture 'http://purl.oclc.org/ooxml/spreadsheetml/main' 'x'
    $chart = '<x:chartsheet xmlns:x="http://purl.oclc.org/ooxml/spreadsheetml/main"><x:sheetPr codeName="KeepChart"/><x:sheetViews><x:sheetView workbookViewId="0" tabSelected="1" zoomScale="75"/></x:sheetViews><x:drawing xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships" r:id="rId1"/></x:chartsheet>'
    $parts['xl/worksheets/hidden.xml'] = $chart
    $parts['[Content_Types].xml'] = $parts['[Content_Types].xml'].Replace('PartName="/xl/worksheets/hidden.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"', 'PartName="/xl/worksheets/hidden.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"')
    New-Workbook $source $parts
    Set-InitialWorksheet $source 3 $true
    $prepared = Read-Parts $source
    $before = Parse-Xml $chart; $after = Parse-Xml $prepared['xl/worksheets/hidden.xml']
    Assert-Equal $after.DocumentElement.LocalName 'chartsheet' 'A non-worksheet root must keep its sheet type'
    $view = $after.SelectSingleNode('//*[local-name()="sheetView"]')
    Assert-Equal $view.GetAttribute('tabSelected') '0' 'Chartsheet must not remain selected with the target'
    $view.RemoveAttribute('tabSelected')
    $before.SelectSingleNode('//*[local-name()="sheetView"]').RemoveAttribute('tabSelected')
    Assert-Equal (Semantic-Xml $after.DocumentElement) (Semantic-Xml $before.DocumentElement) 'Chartsheet metadata must otherwise remain unchanged'
    Remove-Item -LiteralPath $source -Force

    foreach ($placeholder in @(
        @{ state='veryHidden'; id='r:id=""'; succeeds=$true },
        @{ state='hidden'; id='r:id=""'; succeeds=$false },
        @{ state='visible'; id='r:id=""'; succeeds=$false },
        @{ state='veryHidden'; id=''; succeeds=$false },
        @{ state='veryHidden'; id='r:id="missing"'; succeeds=$false }
    )) {
        $source = Join-Path $root ('macro-placeholder-' + [Guid]::NewGuid().ToString('N') + '.xlsx')
        $parts = Fixture 'http://schemas.openxmlformats.org/spreadsheetml/2006/main' ''
        $stub = '<sheet name="Module1" sheetId="4" state="' + $placeholder.state + '" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" ' + $placeholder.id + '/>'
        $parts['xl/workbook.xml'] = $parts['xl/workbook.xml'].Replace('</sheets>', ($stub + '</sheets>'))
        New-Workbook $source $parts
        $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
        $succeeded = $true
        try { Set-InitialWorksheet $source 3 $true } catch { $succeeded = $false; if ($placeholder.succeeds) { throw } }
        Assert-Equal $succeeded $placeholder.succeeds 'Only explicit empty veryHidden macro placeholders may have no part'
        if (-not $succeeded) {
            Assert-Equal (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash $hash 'Invalid real sheet relationships must preserve the original ZIP'
        }
        Remove-Item -LiteralPath $source -Force
    }
    # Original formulas/cached values must survive controlled manual startup.
    # Enron 3.295893 (47 sheets) stalled in automatic open but became ready in <10 s manually.
    foreach ($namespace in @('http://schemas.openxmlformats.org/spreadsheetml/2006/main', 'http://purl.oclc.org/ooxml/spreadsheetml/main')) {
        foreach ($existing in @($false,$true)) {
            $source = Join-Path $root ('manual-' + [Guid]::NewGuid().ToString('N') + '.xlsx')
            $parts = Fixture $namespace 'x'
            $calculation = if($existing){'<x:calcPr calcId="123" calcMode="auto" fullCalcOnLoad="1" forceFullCalc="1" calcOnSave="1" iterateCount="42"/>'}else{''}
            $parts['xl/workbook.xml']=$parts['xl/workbook.xml'].Replace('</x:workbook>',($calculation+'<x:extLst/></x:workbook>'))
            New-Workbook $source $parts
            Set-InitialWorksheet $source 3 $false $true
            $prepared=Read-Parts $source
            $xml=Parse-Xml $prepared['xl/workbook.xml']
            $node=$xml.DocumentElement['calcPr',$namespace]
            Assert-Equal $node.GetAttribute('calcMode') 'manual' 'Open must not trigger automatic calculation'
            foreach($attribute in @('fullCalcOnLoad','forceFullCalc','calcOnSave')){Assert-Equal $node.GetAttribute($attribute) '0' 'Implicit recalculation must be disabled'}
            Assert-Equal $node.NextSibling.LocalName 'extLst' 'calcPr schema order must precede extLst'
            if($existing){Assert-Equal $node.GetAttribute('iterateCount') '42' 'Unrelated calculation settings must survive'}
            foreach($part in @('xl/worksheets/alpha.xml','xl/worksheets/custom-name.xml')) {
                $before=Parse-Xml $parts[$part];$after=Parse-Xml $prepared[$part]
                Assert-Equal ($after.SelectSingleNode('//*[local-name()="sheetData"]').OuterXml) ($before.SelectSingleNode('//*[local-name()="sheetData"]').OuterXml) 'Formulas and cached cell values must be preserved'
            }
            $before=Read-Parts $source
            Set-InitialWorksheet $source 3 $false $true
            $after=Read-Parts $source
            Assert-Equal $after['xl/workbook.xml'] $before['xl/workbook.xml'] 'Manual preparation must be idempotent'
            Remove-Item -LiteralPath $source -Force
        }
    }

    foreach ($failure in @('hidden', 'outside', 'missing-relationship', 'missing-part', 'invalid-view-id')) {
        $source = Join-Path $root ($failure + '.xlsx')
        $parts = Fixture 'http://schemas.openxmlformats.org/spreadsheetml/2006/main' ''
        $index = 3
        switch ($failure) {
            hidden { $index = 1 }
            outside { $index = 4 }
            missing-relationship { $parts['xl/workbook.xml'] = $parts['xl/workbook.xml'].Replace('rId12', 'missing') }
            missing-part { $parts.Remove('xl/worksheets/custom-name.xml') }
            invalid-view-id { $parts['xl/worksheets/alpha.xml'] = $parts['xl/worksheets/alpha.xml'].Replace('workbookViewId="1"', 'workbookViewId="99"') }
        }
        New-Workbook $source $parts
        $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
        $failed = $false
        try { Set-InitialWorksheet $source $index } catch { $failed = $true }
        Assert-Equal $failed $true "Invalid selection must fail: $failure"
        Assert-Equal (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash $hash 'Failed preparation must preserve the complete old workbook'
        Assert-Equal ([IO.File]::Exists($source + '.prepared')) $false 'Failed preparation must clean its unpublished ZIP'
        Remove-Item -LiteralPath $source -Force
    }
} finally {
    # Every recursive removal stays inside the unique test-owned directory.
    $resolved = [IO.Path]::GetFullPath($root)
    if (-not $resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notlike 'capture-selection-*') { throw 'Unsafe test cleanup path' }
    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host 'Capture workbook selection tests passed.'
