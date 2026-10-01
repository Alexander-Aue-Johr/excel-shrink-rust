$ErrorActionPreference = 'Stop'

$script:ExperimentStopwatch = [Diagnostics.Stopwatch]::StartNew()
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:StrictUtf8 = New-Object System.Text.UTF8Encoding($false, $true)

function Write-ExperimentLog(
    [string]$Message,
    [string]$Level = 'INFO'
) {
    $elapsed = $script:ExperimentStopwatch.Elapsed
    $stamp = '{0:00}:{1:00}.{2:000}' -f `
        [Math]::Floor($elapsed.TotalMinutes), `
        $elapsed.Seconds, `
        $elapsed.Milliseconds

    Write-Host "[$stamp] [$Level] $Message"
}

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    [IO.File]::WriteAllText($Path, $Text, $script:Utf8NoBom)
}

function Convert-WslUncToLinuxPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)

    if ($full -match '^\\\\wsl(?:\.localhost)?\\([^\\]+)\\(.*)$') {
        return [pscustomobject]@{
            Distribution = $Matches[1]
            LinuxPath = '/' + ($Matches[2] -replace '\\', '/')
        }
    }

    return $null
}

function Get-SourceFingerprint([string[]]$Paths) {
    $lines = New-Object 'System.Collections.Generic.List[string]'

    foreach ($path in ($Paths | Sort-Object)) {
        if (-not (Test-Path -LiteralPath $path)) {
            throw "Fingerprint source does not exist: $path"
        }

        if (Test-Path -LiteralPath $path -PathType Container) {
            foreach ($file in @(
                Get-ChildItem -LiteralPath $path -Recurse -File |
                Sort-Object FullName
            )) {
                $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                $lines.Add($file.FullName + '=' + $hash)
            }
        }
        else {
            $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
            $lines.Add($path + '=' + $hash)
        }
    }

    $bytes = [Text.Encoding]::UTF8.GetBytes(($lines -join "`n"))
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return (($sha.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join '')
    }
    finally {
        $sha.Dispose()
    }
}

function Invoke-JsonPost([string]$Uri, $Body) {
    $json = $Body | ConvertTo-Json -Depth 20 -Compress
    return Invoke-RestMethod `
        -Uri $Uri `
        -Method Post `
        -ContentType 'application/json' `
        -Body ([Text.Encoding]::UTF8.GetBytes($json))
}

function Release-ComObject($Object) {
    if (
        $null -ne $Object -and
        [Runtime.InteropServices.Marshal]::IsComObject($Object)
    ) {
        try {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($Object)
        }
        catch {
        }
    }
}



function Find-ExcelExecutable {
    $command = Get-Command excel.exe -ErrorAction SilentlyContinue

    if ($null -ne $command -and (Test-Path -LiteralPath $command.Source)) {
        return $command.Source
    }

    foreach ($registryPath in @(
        'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\excel.exe',
        'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\excel.exe',
        'Registry::HKEY_CURRENT_USER\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\excel.exe'
    )) {
        try {
            $key = Get-Item -LiteralPath $registryPath -ErrorAction Stop
            $candidate = [string]$key.GetValue('')

            if (
                -not [string]::IsNullOrWhiteSpace($candidate) -and
                (Test-Path -LiteralPath $candidate)
            ) {
                return $candidate
            }
        }
        catch {
        }
    }

    return $null
}



function Refresh-DeveloperAddInRegistration(
    [string]$AddInId,
    [string]$ManifestPath
) {
    $subKey = 'Software\Microsoft\Office\16.0\Wef\Developer'
    $key = $null

    try {
        $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($subKey, $true)

        if ($null -eq $key) {
            throw "Could not open HKCU\$subKey for writing."
        }

        # Reassert the same persistent registration immediately before launching a
        # fresh Excel process. This avoids relying on a stale Excel instance that
        # was already running before the developer manifest was registered.
        $key.SetValue(
            $AddInId,
            $ManifestPath,
            [Microsoft.Win32.RegistryValueKind]::String
        )
    }
    finally {
        if ($null -ne $key) {
            $key.Dispose()
        }
    }
}



function Stop-BenchmarkExcelProcess($Process) {
    if ($null -eq $Process) {
        return
    }

    try {
        $Process.Refresh()

        if ($Process.HasExited) {
            return
        }

        try {
            [void]$Process.CloseMainWindow()
            [void]$Process.WaitForExit(2000)
        }
        catch {
        }

        $Process.Refresh()

        if (-not $Process.HasExited) {
            Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue

            try {
                [void]$Process.WaitForExit(3000)
            }
            catch {
            }
        }
    }
    catch {
    }
}



function Get-ManifestId([string]$Path) {
    [xml]$manifest = [IO.File]::ReadAllText($Path, $StrictUtf8)
    $node = $manifest.SelectSingleNode(
        "/*[local-name()='OfficeApp']/*[local-name()='Id']"
    )

    if ($null -eq $node -or [string]::IsNullOrWhiteSpace($node.InnerText)) {
        throw "Office Add-in manifest has no Id element: $Path"
    }

    return $node.InnerText.Trim()
}



function Get-ManifestVersion([string]$Path) {
    [xml]$manifest = [IO.File]::ReadAllText($Path, $StrictUtf8)
    $node = $manifest.SelectSingleNode(
        "/*[local-name()='OfficeApp']/*[local-name()='Version']"
    )

    if ($null -eq $node -or [string]::IsNullOrWhiteSpace($node.InnerText)) {
        throw "Office Add-in manifest has no Version element: $Path"
    }

    return $node.InnerText.Trim()
}


function Register-DeveloperAddIn(
    [string]$AddInId,
    [string]$ManifestPath
) {
    $subKey = 'Software\Microsoft\Office\16.0\Wef\Developer'
    $key = $null

    try {
        $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($subKey, $true)

        if ($null -eq $key) {
            throw "Could not open HKCU\$subKey for writing."
        }

        $names = @($key.GetValueNames())
        $previousExists = $names -contains $AddInId
        $previousValue = $null

        if ($previousExists) {
            $previousValue = [string]$key.GetValue($AddInId)
        }

        $key.SetValue(
            $AddInId,
            $ManifestPath,
            [Microsoft.Win32.RegistryValueKind]::String
        )

        return [pscustomobject]@{
            PreviousExists = $previousExists
            PreviousValue = $previousValue
        }
    }
    finally {
        if ($null -ne $key) {
            $key.Dispose()
        }
    }
}


function Restore-DeveloperAddIn(
    [string]$AddInId,
    $PreviousState
) {
    if ($null -eq $PreviousState) {
        return
    }

    $subKey = 'Software\Microsoft\Office\16.0\Wef\Developer'
    $key = $null

    try {
        $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($subKey, $true)

        if ($null -eq $key) {
            return
        }

        if ($PreviousState.PreviousExists) {
            $key.SetValue(
                $AddInId,
                [string]$PreviousState.PreviousValue,
                [Microsoft.Win32.RegistryValueKind]::String
            )
        }
        else {
            $key.DeleteValue($AddInId, $false)
        }
    }
    finally {
        if ($null -ne $key) {
            $key.Dispose()
        }
    }
}



function Read-ZipEntryText(
    [System.IO.Compression.ZipArchive]$Archive,
    [string]$EntryName
) {
    $entry = $Archive.GetEntry($EntryName)

    if ($null -eq $entry) {
        throw "Required OOXML part is missing: $EntryName"
    }

    $stream = $entry.Open()

    try {
        $reader = New-Object IO.StreamReader(
            $stream,
            $StrictUtf8,
            $true
        )

        try {
            return $reader.ReadToEnd()
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}


function Write-ZipEntryText(
    [System.IO.Compression.ZipArchive]$Archive,
    [string]$EntryName,
    [string]$Text
) {
    $existing = $Archive.GetEntry($EntryName)

    if ($null -ne $existing) {
        $existing.Delete()
    }

    $entry = $Archive.CreateEntry(
        $EntryName,
        [System.IO.Compression.CompressionLevel]::Optimal
    )

    $stream = $entry.Open()

    try {
        $writer = New-Object IO.StreamWriter(
            $stream,
            $Utf8NoBom
        )

        try {
            $writer.Write($Text)
        }
        finally {
            $writer.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}


function Add-XmlOverride(
    [xml]$Document,
    [string]$PartName,
    [string]$ContentType
) {
    $namespace = 'http://schemas.openxmlformats.org/package/2006/content-types'
    $manager = New-Object Xml.XmlNamespaceManager($Document.NameTable)
    $manager.AddNamespace('ct', $namespace)

    $existing = $Document.SelectSingleNode(
        "/ct:Types/ct:Override[@PartName='$PartName']",
        $manager
    )

    if ($null -ne $existing) {
        throw "Workbook already contains OOXML content-type override: $PartName"
    }

    $node = $Document.CreateElement('Override', $namespace)
    [void]$node.SetAttribute('PartName', $PartName)
    [void]$node.SetAttribute('ContentType', $ContentType)
    [void]$Document.DocumentElement.AppendChild($node)
}


function Get-NextRelationshipId([xml]$Relationships) {
    $max = 0

    foreach ($node in @($Relationships.DocumentElement.ChildNodes)) {
        if ($node.NodeType -ne [Xml.XmlNodeType]::Element) {
            continue
        }

        $id = [string]$node.GetAttribute('Id')

        if ($id -match '^rId([0-9]+)$') {
            $value = [int]$Matches[1]

            if ($value -gt $max) {
                $max = $value
            }
        }
    }

    return 'rId' + ($max + 1)
}


function Add-OfficeAddInAutoOpenToWorkbook(
    [string]$WorkbookPath,
    [string]$AddInId,
    [string]$AddInVersion
) {
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $archive = [IO.Compression.ZipFile]::Open(
        $WorkbookPath,
        [IO.Compression.ZipArchiveMode]::Update
    )

    try {
        $reservedEntries = @(
            'xl/webextensions/taskpanes.xml',
            'xl/webextensions/_rels/taskpanes.xml.rels',
            'xl/webextensions/webextension1.xml'
        )

        foreach ($entryName in $reservedEntries) {
            if ($null -ne $archive.GetEntry($entryName)) {
                throw (
                    'The temporary workbook copy already contains Office web-extension ' +
                    "markup at $entryName. This benchmark currently refuses to replace it."
                )
            }
        }

        [xml]$contentTypes = Read-ZipEntryText $archive '[Content_Types].xml'
        Add-XmlOverride `
            $contentTypes `
            '/xl/webextensions/taskpanes.xml' `
            'application/vnd.ms-office.webextensiontaskpanes+xml'
        Add-XmlOverride `
            $contentTypes `
            '/xl/webextensions/webextension1.xml' `
            'application/vnd.ms-office.webextension+xml'

        Write-ZipEntryText `
            $archive `
            '[Content_Types].xml' `
            ($contentTypes.OuterXml + "`n")

        [xml]$packageRelationships = Read-ZipEntryText `
            $archive `
            '_rels/.rels'

        $relationshipsNamespace =
            'http://schemas.openxmlformats.org/package/2006/relationships'

        $taskpaneRelationshipId = Get-NextRelationshipId $packageRelationships
        $relationship = $packageRelationships.CreateElement(
            'Relationship',
            $relationshipsNamespace
        )

        [void]$relationship.SetAttribute('Id', $taskpaneRelationshipId)
        [void]$relationship.SetAttribute(
            'Type',
            'http://schemas.microsoft.com/office/2011/relationships/webextensiontaskpanes'
        )
        [void]$relationship.SetAttribute(
            'Target',
            '/xl/webextensions/taskpanes.xml'
        )
        [void]$packageRelationships.DocumentElement.AppendChild($relationship)

        Write-ZipEntryText `
            $archive `
            '_rels/.rels' `
            ($packageRelationships.OuterXml + "`n")

        $webExtensionInstanceId =
            '{' + [Guid]::NewGuid().ToString().ToUpperInvariant() + '}'

        $taskpanesXml = @"
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<wetp:taskpanes xmlns:wetp="http://schemas.microsoft.com/office/webextensions/taskpanes/2010/11">
  <wetp:taskpane dockstate="right" visibility="1" width="350" row="4">
    <wetp:webextensionref xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"/>
  </wetp:taskpane>
</wetp:taskpanes>
"@

        $taskpanesRelationshipsXml = @"
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.microsoft.com/office/2011/relationships/webextension" Target="webextension1.xml"/>
</Relationships>
"@

        $escapedInstanceId = [Security.SecurityElement]::Escape(
            $webExtensionInstanceId
        )
        $escapedAddInId = [Security.SecurityElement]::Escape($AddInId)
        $escapedVersion = [Security.SecurityElement]::Escape($AddInVersion)

        $webExtensionXml = @"
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<we:webextension xmlns:we="http://schemas.microsoft.com/office/webextensions/webextension/2010/11" id="$escapedInstanceId">
  <we:reference id="$escapedAddInId" version="$escapedVersion" store="developer" storeType="Registry"/>
  <we:alternateReferences/>
  <we:properties>
    <we:property name="Office.AutoShowTaskpaneWithDocument" value="true"/>
  </we:properties>
  <we:bindings/>
  <we:snapshot xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/>
</we:webextension>
"@

        Write-ZipEntryText `
            $archive `
            'xl/webextensions/taskpanes.xml' `
            ($taskpanesXml.TrimStart() + "`n")

        Write-ZipEntryText `
            $archive `
            'xl/webextensions/_rels/taskpanes.xml.rels' `
            ($taskpanesRelationshipsXml.TrimStart() + "`n")

        Write-ZipEntryText `
            $archive `
            'xl/webextensions/webextension1.xml' `
            ($webExtensionXml.TrimStart() + "`n")
    }
    finally {
        $archive.Dispose()
    }
}


function Start-ExcelX([string]$ExcelExecutable, [string]$DocumentPath) {
    $quoted = '"' + $DocumentPath.Replace('"', '""') + '"'
    return Start-Process `
        -FilePath $ExcelExecutable `
        -ArgumentList @('/x', $quoted) `
        -PassThru
}
