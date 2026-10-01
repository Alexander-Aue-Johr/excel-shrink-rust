function Test-LabAddress([string]$Address) {
    $m=[regex]::Match($Address,'^\$?([A-Za-z]{1,3})\$?([1-9][0-9]*)(?::\$?([A-Za-z]{1,3})\$?([1-9][0-9]*))?$')
    if (-not $m.Success) { throw "Expected a single A1 cell/rectangle, not a formula or external reference: $Address" }
    $parts=@()
    foreach ($index in @(1,3)) {
        if (-not $m.Groups[$index].Success) { continue }
        $column=0
        foreach ($ch in $m.Groups[$index].Value.ToUpperInvariant().ToCharArray()) { $column=$column*26+([int]$ch-[int][char]'A'+1) }
        [long]$row=0
        if (-not [long]::TryParse($m.Groups[$index+1].Value,[ref]$row) -or $column -gt 16384 -or $row -gt 1048576) { throw "Address outside XLSX capacity: $Address" }
        $parts+=,@($row,$column)
    }
    if ($parts.Count -eq 2 -and ($parts[1][0] -lt $parts[0][0] -or $parts[1][1] -lt $parts[0][1])) { throw "Reversed rectangle: $Address" }
}
function Get-LabExternalPlan([string]$Path) {
    if (-not $Path) { return [pscustomobject]@{captures=@();overflow=@()} }
    $value=[IO.File]::ReadAllText($Path) | ConvertFrom-Json
    foreach ($property in $value.PSObject.Properties.Name) { if ($property -notin @('captures','overflow')) { throw "Unknown plan field: $property" } }
    if (-not $value.PSObject.Properties['captures']) { Add-Member -InputObject $value -NotePropertyName captures -NotePropertyValue @() }
    if (-not $value.PSObject.Properties['overflow']) { Add-Member -InputObject $value -NotePropertyName overflow -NotePropertyValue @() }
    foreach ($kind in @('captures','overflow')) {
        if ($null -eq $value.$kind -or $value.$kind -isnot [array]) { throw "Plan $kind must be an array, including for one entry." }
        foreach ($item in $value.$kind) {
            $allowed=if ($kind -eq 'captures') { @('pair','sheet','range','appearance','format','storage','transparent') } else { @('pair','sheet','capture','probe','source','variant','method') }
            foreach ($property in $item.PSObject.Properties.Name) { if ($property -notin $allowed) { throw "Unknown $kind field: $property" } }
            if (-not $item.PSObject.Properties['sheet'] -or [string]::IsNullOrWhiteSpace([string]$item.sheet)) { throw 'Each plan entry needs a worksheet name.' }
            if (-not $item.PSObject.Properties['pair']) { Add-Member -InputObject $item -NotePropertyName pair -NotePropertyValue '000000' }
            if ([string]$item.pair -notmatch '^\d{6}$') { throw 'pair must be a six-digit string such as 000000.' }
            if ($kind -eq 'captures') {
                if (-not $item.PSObject.Properties['range']) { throw 'Capture entry needs range.' }
                Test-LabAddress ([string]$item.range)
                if ($item.PSObject.Properties['appearance'] -and $item.appearance -notin @(1,2)) { throw 'appearance must be 1 (screen) or 2 (printer).' }
                if ($item.PSObject.Properties['format'] -and $item.format -notin @(2,-4147)) { throw 'format must be 2 (bitmap) or -4147 (picture).' }
                if ($item.PSObject.Properties['storage'] -and $item.storage -notin @('png','bmp')) { throw 'storage must be png or bmp.' }
                if ($item.PSObject.Properties['transparent']) {
                    if ($item.transparent -isnot [bool]) { throw 'transparent must be a JSON boolean.' }
                    if ($item.transparent -and (-not $item.PSObject.Properties['format'] -or $item.format -ne -4147)) { throw 'transparent requires format -4147.' }
                    if ($item.transparent -and $item.PSObject.Properties['storage'] -and $item.storage -eq 'bmp') { throw 'Transparent canvas requires PNG; BMP uses RGB24-white.' }
                }
            } else {
                foreach ($field in @('capture','probe')) {
                    if (-not $item.PSObject.Properties[$field]) { throw "Overflow entry needs $field." }
                    Test-LabAddress ([string]$item.$field)
                }
                $method=if ($item.PSObject.Properties['method']) { [string]$item.method } else { 'counterfactual' }
                if ($method -notin @('counterfactual','nonuniform')) { throw 'Unknown overflow method.' }
                if ($method -eq 'counterfactual') {
                    if (-not $item.PSObject.Properties['source']) { throw 'Counterfactual overflow needs source.' }
                    Test-LabAddress ([string]$item.source)
                }
                if ($item.PSObject.Properties['variant'] -and $item.variant -notin @('original','shrunk')) { throw 'variant must be original or shrunk.' }
            }
        }
    }
    return $value
}
function New-LabPlan {
    param($Configuration, $Pairs, $External)
    $doc=[Xml.XmlDocument]::new()
    $root=$doc.CreateElement('excel-lab'); [void]$doc.AppendChild($root)
    [void]$root.SetAttribute('version','1')
    foreach ($key in @('shapes','hash_values','compare_values','row_scope','screenshots','storage','copy_matrix','transparent_emf','compare_visual_vba','compare_visual_rust','block_cells','max_cells','tile_points','max_tiles','max_differences')) {
        $value=$Configuration[$key]
        if ($value -is [bool]) { $value=$value.ToString().ToLowerInvariant() }
        [void]$root.SetAttribute($key,[string]$value)
    }
    [void]$root.SetAttribute('batch_rows','8192'); [void]$root.SetAttribute('max_pixels','32000000'); [void]$root.SetAttribute('emf_dpi','96')
    $collection=$doc.CreateElement('pairs'); [void]$root.AppendChild($collection)
    $ids=@{}
    foreach ($pair in $Pairs) {
        $ids[[string]$pair.id]=$true
        $node=$doc.CreateElement('pair'); [void]$collection.AppendChild($node)
        foreach ($key in @('id','name','original','shrunk')) { [void]$node.SetAttribute($key,[string]$pair[$key]) }
    }
    foreach ($kind in @('captures','overflow')) {
        $collection=$doc.CreateElement($kind); [void]$root.AppendChild($collection)
        foreach ($item in $External.$kind) {
            if (-not $ids.ContainsKey([string]$item.pair)) { throw "Plan refers to an unselected pair: $($item.pair)" }
            $nodeName=if ($kind -eq 'captures') { 'capture' } else { 'probe' }
            $node=$doc.CreateElement($nodeName); [void]$collection.AppendChild($node)
            foreach ($property in $item.PSObject.Properties) {
                $value=$property.Value
                if ($value -is [bool]) { $value=$value.ToString().ToLowerInvariant() }
                [void]$node.SetAttribute($property.Name,[string]$value)
            }
        }
    }
    return ,$doc
}
