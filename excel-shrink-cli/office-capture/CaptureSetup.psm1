Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false, $true)

function Get-CaptureHostExecutable([string]$PreferredPath) {
    $candidate = $PreferredPath
    if (-not $candidate) { $candidate = $env:EXCEL_SHRINK_CAPTURE_HOST }
    if (-not $candidate) {
        $candidate = Join-Path $PSScriptRoot '../../target/release/excel-shrink-cli.exe'
    }
    $candidate = [IO.Path]::GetFullPath($candidate)
    if (-not [IO.File]::Exists($candidate) -or [IO.Path]::GetExtension($candidate) -ine '.exe') {
        throw "The Windows capture host is missing: $candidate. Build excel-shrink-cli for Windows or set EXCEL_SHRINK_CAPTURE_HOST."
    }
    return $candidate
}

function Quote-Argument([string]$value) {
    if ($value.Contains('"')) { throw 'Double quotes are not supported in launch paths.' }
    return '"' + $value + '"'
}

function Find-ExcelExecutable {
    $command = Get-Command excel.exe -ErrorAction SilentlyContinue
    if ($null -ne $command -and (Test-Path -LiteralPath $command.Source -PathType Leaf)) { return $command.Source }
    foreach ($path in @(
        'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\excel.exe',
        'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\excel.exe',
        'Registry::HKEY_CURRENT_USER\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\excel.exe'
    )) {
        try {
            $candidate = [string](Get-Item -LiteralPath $path -ErrorAction Stop).GetValue('')
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
        } catch {}
    }
    throw 'Excel executable was not found.'
}

function Read-ZipText($archive, [string]$name) {
    $entry = $archive.GetEntry($name)
    if ($null -eq $entry) { throw "Workbook part is missing: $name" }
    $reader = New-Object IO.StreamReader($entry.Open(), $utf8, $true)
    try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
}

function Set-ZipText($archive, [string]$name, [string]$content) {
    $old = $archive.GetEntry($name)
    if ($null -ne $old) { $old.Delete() }
    $entry = $archive.CreateEntry($name, [IO.Compression.CompressionLevel]::Optimal)
    $writer = New-Object IO.StreamWriter($entry.Open(), $utf8)
    try { $writer.Write($content) } finally { $writer.Dispose() }
}

function Add-AutoOpenTaskpane([string]$workbook, [string]$addInId, [string]$version) {
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::Open($workbook, [IO.Compression.ZipArchiveMode]::Update)
    try {
        foreach ($name in @('xl/webextensions/taskpanes.xml', 'xl/webextensions/webextension1.xml')) {
            if ($null -ne $archive.GetEntry($name)) { throw "Workbook already has a taskpane at $name" }
        }
        [xml]$types = Read-ZipText $archive '[Content_Types].xml'
        $typeNamespace = 'http://schemas.openxmlformats.org/package/2006/content-types'
        foreach ($item in @(
            @{ Part = '/xl/webextensions/taskpanes.xml'; Content = 'application/vnd.ms-office.webextensiontaskpanes+xml' },
            @{ Part = '/xl/webextensions/webextension1.xml'; Content = 'application/vnd.ms-office.webextension+xml' }
        )) {
            $node = $types.CreateElement('Override', $typeNamespace)
            [void]$node.SetAttribute('PartName', $item.Part)
            [void]$node.SetAttribute('ContentType', $item.Content)
            [void]$types.DocumentElement.AppendChild($node)
        }
        Set-ZipText $archive '[Content_Types].xml' ($types.OuterXml + "`n")

        [xml]$relations = Read-ZipText $archive '_rels/.rels'
        $maximum = 0
        foreach ($existing in @($relations.DocumentElement.ChildNodes)) {
            if ($existing.NodeType -eq [Xml.XmlNodeType]::Element -and [string]$existing.GetAttribute('Id') -match '^rId([0-9]+)$') {
                $maximum = [Math]::Max($maximum, [int]$Matches[1])
            }
        }
        $relation = $relations.CreateElement('Relationship', 'http://schemas.openxmlformats.org/package/2006/relationships')
        [void]$relation.SetAttribute('Id', ('rId' + ($maximum + 1)))
        [void]$relation.SetAttribute('Type', 'http://schemas.microsoft.com/office/2011/relationships/webextensiontaskpanes')
        [void]$relation.SetAttribute('Target', '/xl/webextensions/taskpanes.xml')
        [void]$relations.DocumentElement.AppendChild($relation)
        Set-ZipText $archive '_rels/.rels' ($relations.OuterXml + "`n")

        $taskpanes = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><wetp:taskpanes xmlns:wetp="http://schemas.microsoft.com/office/webextensions/taskpanes/2010/11"><wetp:taskpane dockstate="right" visibility="1" width="350" row="4"><wetp:webextensionref xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"/></wetp:taskpane></wetp:taskpanes>'
        $taskpaneRelations = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.microsoft.com/office/2011/relationships/webextension" Target="webextension1.xml"/></Relationships>'
        $instance = '{' + [Guid]::NewGuid().ToString().ToUpperInvariant() + '}'
        $reference = [Security.SecurityElement]::Escape($addInId)
        $referenceVersion = [Security.SecurityElement]::Escape($version)
        $extension = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><we:webextension xmlns:we="http://schemas.microsoft.com/office/webextensions/webextension/2010/11" id="' + $instance + '"><we:reference id="' + $reference + '" version="' + $referenceVersion + '" store="developer" storeType="Registry"/><we:alternateReferences/><we:properties><we:property name="Office.AutoShowTaskpaneWithDocument" value="true"/></we:properties><we:bindings/><we:snapshot xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></we:webextension>'
        Set-ZipText $archive 'xl/webextensions/taskpanes.xml' $taskpanes
        Set-ZipText $archive 'xl/webextensions/_rels/taskpanes.xml.rels' $taskpaneRelations
        Set-ZipText $archive 'xl/webextensions/webextension1.xml' $extension
    } finally { $archive.Dispose() }
}

function Set-InitialWorksheet([string]$workbook, [int]$index, [bool]$hideErrorIndicators = $false) {
    # These paths are the private document copies, never shrink inputs or outputs.
    # activeTab and tabSelected must agree in every workbook window.
    [CaptureWorkbook]::PrepareCapture($workbook, $index, $hideErrorIndicators)
}

function Register-AddIn([string]$id, [string]$manifestPath) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Office\16.0\Wef\Developer', $true)
    if ($null -eq $key) { throw 'Cannot open Office developer add-in registry key.' }
    try {
        # Other capture sessions register concurrently. Read this known value
        # directly instead of enumerating a registry key that is changing.
        $previous = $key.GetValue($id, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $key.SetValue($id, $manifestPath, [Microsoft.Win32.RegistryValueKind]::String)
        return $previous
    } finally { $key.Dispose() }
}

function Restore-AddIn([string]$id, $previous) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Office\16.0\Wef\Developer', $true)
    if ($null -eq $key) { return }
    try {
        if ($null -eq $previous) { $key.DeleteValue($id, $false) }
        else { $key.SetValue($id, [string]$previous, [Microsoft.Win32.RegistryValueKind]::String) }
    } finally { $key.Dispose() }
}

function Stop-OwnedProcess($process) {
    if ($null -eq $process) { return }
    try {
        if (-not [CaptureProcesses]::Stop($process)) {
            Write-Warning "Owned capture process $($process.Id) has not confirmed exit after termination."
        }
    } catch { Write-Warning "Could not confirm owned capture process exit: $($_.Exception.Message)" }
}

function Copy-CompletedOutput([string]$from, [string]$to) {
    $imageFrom = Join-Path $from 'images'
    $imageTo = Join-Path $to 'images'
    if (Test-Path -LiteralPath $imageFrom -PathType Container) {
        New-Item -ItemType Directory -Path $imageTo -Force | Out-Null
        foreach ($image in @(Get-ChildItem -LiteralPath $imageFrom -File -Filter '*.png')) {
            Copy-VerifiedFile $image.FullName (Join-Path $imageTo $image.Name)
        }
    }
    Copy-VerifiedFile (Join-Path $from 'result.json') (Join-Path $to 'result.json')
}

function Copy-VerifiedFile([string]$source, [string]$target) {
    $expected = (Get-Item -LiteralPath $source).Length
    $expectedHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            Copy-Item -LiteralPath $source -Destination $target -Force
            if ((Get-Item -LiteralPath $target).Length -eq $expected -and
                (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -eq $expectedHash) { return }
            throw "Incomplete output copy: $target"
        } catch {
            if ($attempt -eq 3) { throw }
            Start-Sleep -Milliseconds 250
        }
    }
}

Export-ModuleMember -Function @(
    'Get-CaptureHostExecutable', 'Quote-Argument', 'Find-ExcelExecutable',
    'Add-AutoOpenTaskpane', 'Set-InitialWorksheet', 'Register-AddIn', 'Restore-AddIn',
    'Stop-OwnedProcess', 'Copy-CompletedOutput', 'Copy-VerifiedFile'
)
