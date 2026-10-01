param(
    [Parameter(Mandatory=$true)]
    [string]$FileListPath,

    [Parameter(Mandatory=$true)]
    [string]$OutputPath,

    [switch]$SkipIfUnavailable,

    [switch]$ConfigureWebViewLoopback,

    [switch]$EnableRuntimeLogging,

    [switch]$TemporaryDeveloperRegistration,

    [ValidateRange(5, 120)]
    [int]$FirstActivationTimeoutSeconds = 120,

    [ValidateRange(1, 10)]
    [int]$FirstActivationAttempts = 5,

    [ValidateRange(0, 60)]
    [int]$ActivationRetryDelaySeconds = 10,

    [ValidateRange(0, 60)]
    [int]$RegistrationSettleSeconds = 10,

    [ValidateRange(0, 30)]
    [int]$PostResultHoldSeconds = 0,

    [ValidateRange(0, 60)]
    [int]$InterWorkbookDelaySeconds = 0,

    [ValidateSet('DrawingBounds', 'RangeImage')]
    [string]$BenchmarkMode = 'DrawingBounds',

    [string]$CapturePlanPath,

    [string]$ImageOutputDir,

    [ValidateRange(1, 64)]
    [int]$ImageBatchSize = 8,

    [ValidateRange(0, 9)]
    [int]$PerWorkbookRetries = 1,

    [ValidateRange(5, 300)]
    [int]$ResultTimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
$env:DOTNET_CLI_UI_LANGUAGE = 'en-us'
$env:DOTNET_NOLOGO = '1'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'

$StatusFileName = if ($BenchmarkMode -eq 'RangeImage') {
    'officejs-getimage-status.json'
}
else {
    'office-addin-status.json'
}

$StatusPath = Join-Path (Split-Path -Parent $OutputPath) $StatusFileName
$PartialOutputPath = Join-Path `
    (Split-Path -Parent $OutputPath) `
    (([IO.Path]::GetFileNameWithoutExtension($OutputPath)) + '.partial.json')
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$StrictUtf8 = New-Object System.Text.UTF8Encoding($false, $true)

function Write-Status([string]$Status, [string]$Message) {
    $data = [ordered]@{
        status = $Status
        message = $Message
    }

    [IO.File]::WriteAllText(
        $StatusPath,
        (($data | ConvertTo-Json -Depth 4) + "`n"),
        $Utf8NoBom
    )
}

function Skip-Or-Throw([string]$Message) {
    Write-Status 'skipped' $Message

    if ($SkipIfUnavailable) {
        Write-Host "Office.js add-in: SKIPPED - $Message"
        return $true
    }

    throw $Message
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


function Copy-DirectoryContents(
    [string]$Source,
    [string]$Destination
) {
    if (-not (Test-Path -LiteralPath $Source)) {
        return
    }

    Remove-Item `
        -LiteralPath $Destination `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue

    New-Item -ItemType Directory -Path $Destination -Force | Out-Null

    foreach ($item in Get-ChildItem -LiteralPath $Source -Force) {
        Copy-Item `
            -LiteralPath $item.FullName `
            -Destination $Destination `
            -Recurse `
            -Force
    }
}


function Test-DotNet8Sdk([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path -LiteralPath $Path)) {
        return $false
    }

    try {
        $sdks = @(& $Path --list-sdks 2>$null)
        return @($sdks | Where-Object { $_ -match '^8\.' }).Count -gt 0
    }
    catch {
        return $false
    }
}

function Find-DotNet8Sdk {
    $global = Get-Command dotnet.exe -ErrorAction SilentlyContinue

    if ($null -ne $global -and (Test-DotNet8Sdk $global.Source)) {
        return $global.Source
    }

    if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        $toolchainRoot = Join-Path $env:LOCALAPPDATA 'excel-shrink-rust\repo-toolchains'

        if (Test-Path -LiteralPath $toolchainRoot) {
            $candidates = @(
                Get-ChildItem `
                    -LiteralPath $toolchainRoot `
                    -Recurse `
                    -File `
                    -Filter 'dotnet.exe' `
                    -ErrorAction SilentlyContinue |
                Where-Object {
                    $_.FullName -like '*\dotnet-win-x64\dotnet.exe'
                } |
                Sort-Object FullName
            )

            foreach ($candidate in $candidates) {
                if (Test-DotNet8Sdk $candidate.FullName) {
                    return $candidate.FullName
                }
            }
        }
    }

    return $null
}


function Get-WebViewLoopbackState {
    $name = 'microsoft.win32webviewhost_cw5n1h2txyewy'
    $exe = Join-Path $env:SystemRoot 'System32\CheckNetIsolation.exe'

    if (-not (Test-Path -LiteralPath $exe)) {
        return [pscustomobject]@{
            Available = $false
            Enabled = $null
            Name = $name
            Exe = $exe
            Raw = ''
        }
    }

    try {
        $raw = (& $exe LoopbackExempt -s 2>&1 | Out-String)
        $enabled = $raw -match [Regex]::Escape($name)

        return [pscustomobject]@{
            Available = $true
            Enabled = $enabled
            Name = $name
            Exe = $exe
            Raw = $raw
        }
    }
    catch {
        return [pscustomobject]@{
            Available = $true
            Enabled = $null
            Name = $name
            Exe = $exe
            Raw = $_.Exception.Message
        }
    }
}


function Ensure-WebViewLoopback {
    $state = Get-WebViewLoopbackState

    if (-not $state.Available) {
        Write-Warning (
            'CheckNetIsolation.exe is unavailable; WebView localhost loopback ' +
            'could not be checked.'
        )
        return
    }

    if ($state.Enabled -eq $true) {
        Write-Host 'Microsoft Desktop App Web Viewer localhost loopback: enabled'
        return
    }

    if (-not $ConfigureWebViewLoopback) {
        Write-Host (
            'Microsoft Desktop App Web Viewer localhost loopback exemption: not listed. ' +
            'Continuing without changing system state; use -ConfigureWebViewLoopback ' +
            'only if localhost loading actually fails on this machine.'
        )
        return
    }

    Write-Host (
        'Requesting one-time localhost loopback exemption for Microsoft Desktop ' +
        'App Web Viewer. A Windows UAC prompt can appear.'
    )

    $process = Start-Process `
        -FilePath $state.Exe `
        -ArgumentList @(
            'LoopbackExempt',
            '-a',
            ('-n=' + $state.Name)
        ) `
        -Verb RunAs `
        -Wait `
        -PassThru

    if ($process.ExitCode -ne 0) {
        throw (
            'CheckNetIsolation failed to add the WebView loopback exemption. ' +
            'Exit code: ' +
            $process.ExitCode
        )
    }

    $verified = Get-WebViewLoopbackState

    if ($verified.Enabled -ne $true) {
        throw (
            'CheckNetIsolation returned success, but the WebView loopback ' +
            'exemption is still not listed.'
        )
    }

    Write-Host 'Microsoft Desktop App Web Viewer localhost loopback: enabled'
}


function Enable-OfficeRuntimeLogging([string]$LogPath) {
    $subKey =
        'Software\Microsoft\Office\16.0\Wef\Developer\RuntimeLogging'

    $key = $null

    try {
        $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey(
            $subKey,
            $true
        )

        if ($null -eq $key) {
            throw "Could not open HKCU\$subKey for writing."
        }

        $names = @($key.GetValueNames())
        $previousExists = $names -contains ''
        $previousValue = $null
        $previousKind = $null

        if ($previousExists) {
            $previousValue = $key.GetValue('')
            $previousKind = $key.GetValueKind('')
        }

        $key.SetValue(
            '',
            $LogPath,
            [Microsoft.Win32.RegistryValueKind]::String
        )

        return [pscustomobject]@{
            PreviousExists = $previousExists
            PreviousValue = $previousValue
            PreviousKind = $previousKind
        }
    }
    finally {
        if ($null -ne $key) {
            $key.Dispose()
        }
    }
}


function Restore-OfficeRuntimeLogging($PreviousState) {
    if ($null -eq $PreviousState) {
        return
    }

    $subKey =
        'Software\Microsoft\Office\16.0\Wef\Developer\RuntimeLogging'

    $key = $null

    try {
        $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey(
            $subKey,
            $true
        )

        if ($null -eq $key) {
            return
        }

        if ($PreviousState.PreviousExists) {
            $kind = if ($null -ne $PreviousState.PreviousKind) {
                $PreviousState.PreviousKind
            }
            else {
                [Microsoft.Win32.RegistryValueKind]::String
            }

            $key.SetValue(
                '',
                $PreviousState.PreviousValue,
                $kind
            )
        }
        else {
            $key.DeleteValue('', $false)
        }
    }
    finally {
        if ($null -ne $key) {
            $key.Dispose()
        }
    }
}


function Get-OfficeRuntimeLogSummary(
    [string]$LogPath,
    [string]$AddInId
) {
    if (
        [string]::IsNullOrWhiteSpace($LogPath) -or
        -not (Test-Path -LiteralPath $LogPath)
    ) {
        return ''
    }

    try {
        $lines = @([IO.File]::ReadAllLines($LogPath))

        if ($lines.Count -eq 0) {
            return 'Office runtime log exists but is empty.'
        }

        $interesting = @(
            $lines |
            Where-Object {
                $_ -match [Regex]::Escape($AddInId) -or
                $_ -match '(?i)\b(error|unexpected|warning|monitorable|solutionid)\b'
            } |
            Select-Object -Last 24
        )

        if ($interesting.Count -eq 0) {
            $interesting = @($lines | Select-Object -Last 16)
        }

        return (
            'Office runtime log tail: ' +
            (($interesting | ForEach-Object { $_.Trim() }) -join ' | ')
        )
    }
    catch {
        return (
            'Office runtime log could not be read: ' +
            $_.Exception.Message
        )
    }
}

function Close-OfficeBenchmarkWorkbookIfOpen([string]$FullName) {
    $excelObject = $null
    $workbooksObject = $null
    $candidate = $null
    $target = [IO.Path]::GetFullPath($FullName)

    try {
        $excelObject = [Runtime.InteropServices.Marshal]::GetActiveObject(
            'Excel.Application'
        )
        $workbooksObject = $excelObject.Workbooks

        for ($index = $workbooksObject.Count; $index -ge 1; $index--) {
            $candidate = $null

            try {
                $candidate = $workbooksObject.Item($index)
                $candidateFullName = [IO.Path]::GetFullPath(
                    [string]$candidate.FullName
                )

                if (
                    [string]::Equals(
                        $candidateFullName,
                        $target,
                        [StringComparison]::OrdinalIgnoreCase
                    )
                ) {
                    $candidate.Close($false)
                    return $true
                }
            }
            catch {
            }
            finally {
                Release-ComObject $candidate
                $candidate = $null
            }
        }
    }
    catch {
        # Excel may already have closed the document or may not be in the ROT.
    }
    finally {
        Release-ComObject $candidate
        Release-ComObject $workbooksObject
        Release-ComObject $excelObject
    }

    return $false
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


function Start-BenchmarkExcelProcess(
    [string]$ExcelExecutable,
    [string]$DocumentPath
) {
    if (-not [string]::IsNullOrWhiteSpace($ExcelExecutable)) {
        $quotedDocument = '"' + $DocumentPath.Replace('"', '""') + '"'

        return Start-Process `
            -FilePath $ExcelExecutable `
            -ArgumentList @('/x', $quotedDocument) `
            -PassThru
    }

    # Fallback only. Microsoft documents excel.exe /x as the way to start a new
    # Excel process; use the shell only if EXCEL.EXE could not be resolved.
    return Start-Process `
        -FilePath $DocumentPath `
        -PassThru
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

function Format-OfficeAddInDiagnostics($Status) {
    if ($null -eq $Status) {
        return 'No status response was available from the local Office.js host.'
    }

    $events = @($Status.client_events)
    $errors = @($Status.errors)
    $eventText = ($events -join "`n")

    $parts = New-Object 'System.Collections.Generic.List[string]'

    if ($events.Count -eq 0) {
        $parts.Add(
            'No task-pane request or client event reached the local host. ' +
            'Excel most likely did not open the embedded add-in task pane.'
        )
    }
    else {
        if (
            $eventText -match 'GET /taskpane.html' -and
            $eventText -notmatch 'GET /bootstrap.js'
        ) {
            $parts.Add(
                'Excel requested taskpane.html, but its first local script resource ' +
                '(bootstrap.js) was never requested. The Office host reached the URL, ' +
                'but the web view did not begin normal page subresource loading.'
            )
        }
        elseif (
            $eventText -match 'GET /bootstrap.js' -and
            $eventText -notmatch 'BEACON bootstrap_loaded'
        ) {
            $parts.Add(
                'bootstrap.js was fetched from localhost but did not execute far enough ' +
                'to send its first same-origin beacon.'
            )
        }
        elseif (
            $eventText -match 'BEACON resource_load_error' -and
            $eventText -match 'officeapis.*onecdn.*microsoft'
        ) {
            $parts.Add(
                'The local bootstrap executed, but the Microsoft Office.js CDN resource ' +
                'reported a browser resource-load error.'
            )
        }
        elseif (
            $eventText -match 'BEACON office_js_watchdog_15s' -and
            $eventText -match 'office_defined.*false'
        ) {
            $parts.Add(
                'The task-pane bootstrap is executing, but the Office global was still ' +
                'undefined after 15 seconds.'
            )
        }
        elseif (
            $eventText -match 'GET /taskpane.js' -and
            $eventText -notmatch '"stage":"script_loaded"'
        ) {
            $parts.Add(
                'taskpane.js was requested but did not reach its script_loaded event.'
            )
        }
        elseif (
            $eventText -match '"stage":"script_loaded"' -and
            $eventText -notmatch '"stage":"office_ready"'
        ) {
            $parts.Add(
                'taskpane.js executed, but Office.onReady() did not resolve.'
            )
        }
        elseif (
            $eventText -match '"stage":"office_ready"' -and
            $eventText -notmatch '"stage":"measurement_started"'
        ) {
            $parts.Add(
                'Office.onReady() completed, but the benchmark did not start measuring.'
            )
        }
        elseif (
            $eventText -match '"stage":"measurement_started"' -and
            $eventText -notmatch '"stage":"measurement_completed"'
        ) {
            $parts.Add(
                'Office.js reached the benchmark, but the batched Excel API measurement ' +
                'did not complete.'
            )
        }

        $parts.Add(
            'Task-pane/server events: ' +
            (($events | Select-Object -Last 24) -join ' | ')
        )
    }

    if ($errors.Count -gt 0) {
        $parts.Add(
            'Task-pane errors: ' +
            (($errors | Select-Object -Last 8) -join ' | ')
        )
    }

    return ($parts -join ' ')
}

$dotnet = Find-DotNet8Sdk

if ($null -eq $dotnet) {
    $message = (
        'A Windows .NET 8 SDK is required for the small local Kestrel host. ' +
        'Run the base drawing-bounds experiment once to populate the repo-specific ' +
        '.NET SDK cache, or install a Windows .NET 8 SDK.'
    )

    if (Skip-Or-Throw $message) {
        return
    }
}

$env:DOTNET_ROOT = Split-Path -Parent $dotnet

$paths = @(
    [IO.File]::ReadAllLines($FileListPath, $StrictUtf8) |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_.Length -gt 0 }
)

if ($paths.Count -eq 0) {
    throw "File list is empty: $FileListPath"
}

if ($BenchmarkMode -eq 'RangeImage') {
    if ([string]::IsNullOrWhiteSpace($CapturePlanPath)) {
        throw '-CapturePlanPath is required for BenchmarkMode RangeImage.'
    }

    if (-not (Test-Path -LiteralPath $CapturePlanPath)) {
        throw "Capture plan does not exist: $CapturePlanPath"
    }

    if ([string]::IsNullOrWhiteSpace($ImageOutputDir)) {
        throw '-ImageOutputDir is required for BenchmarkMode RangeImage.'
    }

    $CapturePlanPath = [IO.Path]::GetFullPath($CapturePlanPath)
    $ImageOutputDir = [IO.Path]::GetFullPath($ImageOutputDir)
}

Remove-Item -LiteralPath $PartialOutputPath -Force -ErrorAction SilentlyContinue

Write-Host 'Checking trusted ASP.NET Core HTTPS development certificate...'
& $dotnet dev-certs https --check --trust --quiet *> $null

if ($LASTEXITCODE -ne 0) {
    Write-Host 'Creating/trusting the ASP.NET Core HTTPS development certificate...'
    Write-Host 'A one-time Windows trust confirmation can appear.'

    & $dotnet dev-certs https --trust

    if ($LASTEXITCODE -ne 0) {
        if (Skip-Or-Throw 'A trusted localhost HTTPS development certificate could not be established.') {
            return
        }
    }
}

$tempRoot = Join-Path `
    $env:TEMP `
    ('drawing-bounds-office-addin-' + [Guid]::NewGuid().ToString('N'))

$stage = Join-Path $tempRoot 'app'
$serverStage = Join-Path $stage 'server'
$documents = Join-Path $tempRoot 'documents'
$stagedOutput = Join-Path $tempRoot 'office-addin.json'
$stagedList = Join-Path $tempRoot 'files.txt'
$stagedCapturePlan = Join-Path $tempRoot 'capture-plan.tsv'
$stagedImageRoot = Join-Path $tempRoot 'images'
$serverOut = Join-Path $tempRoot 'server.stdout.txt'
$serverErr = Join-Path $tempRoot 'server.stderr.txt'

$diagnosticDirectoryName = if ($BenchmarkMode -eq 'RangeImage') {
    'officejs-getimage-diagnostics'
}
else {
    'office-addin-diagnostics'
}

$diagnosticRoot = Join-Path `
    (Split-Path -Parent $OutputPath) `
    $diagnosticDirectoryName

$server = $null
$registrationState = $null
$runtimeLoggingState = $null
$runtimeLogPath = $null
$addInId = $null
$addInVersion = $null
$excel = $null
$workbooks = $null
$workbook = $null

$oldOutput = $env:DRAWING_BOUNDS_OUTPUT_PATH
$oldList = $env:DRAWING_BOUNDS_FILE_LIST
$oldStaticRoot = $env:DRAWING_BOUNDS_STATIC_ROOT
$oldCapturePlan = $env:DRAWING_BOUNDS_CAPTURE_PLAN_PATH
$oldImageRoot = $env:DRAWING_BOUNDS_IMAGE_OUTPUT_ROOT
$oldImageBatchSize = $env:DRAWING_BOUNDS_IMAGE_BATCH_SIZE
$oldAspNetEnvironment = $env:ASPNETCORE_ENVIRONMENT

New-Item -ItemType Directory -Path $stage -Force | Out-Null
New-Item -ItemType Directory -Path $serverStage -Force | Out-Null
New-Item -ItemType Directory -Path $documents -Force | Out-Null
New-Item -ItemType Directory -Path $stagedImageRoot -Force | Out-Null
Remove-Item `
    -LiteralPath $diagnosticRoot `
    -Recurse `
    -Force `
    -ErrorAction SilentlyContinue

New-Item `
    -ItemType Directory `
    -Path $diagnosticRoot `
    -Force | Out-Null

$taskpaneScriptSource = if ($BenchmarkMode -eq 'RangeImage') {
    Join-Path $PSScriptRoot 'getimage-taskpane.js'
}
else {
    Join-Path $PSScriptRoot 'taskpane.js'
}

$serverProgramSource = if ($BenchmarkMode -eq 'RangeImage') {
    Join-Path $PSScriptRoot 'server\GetImageProgram.cs'
}
else {
    Join-Path $PSScriptRoot 'server\Program.cs'
}

Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'manifest.xml') -Destination $stage
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'taskpane.html') -Destination $stage
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'bootstrap.js') -Destination $stage
Copy-Item -LiteralPath $taskpaneScriptSource -Destination (Join-Path $stage 'taskpane.js')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'commands.html') -Destination $stage
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'server\DrawingBoundsOfficeAddinServer.csproj') -Destination $serverStage
Copy-Item -LiteralPath $serverProgramSource -Destination (Join-Path $serverStage 'Program.cs')
Copy-Item -LiteralPath $FileListPath -Destination $stagedList -Force

if ($BenchmarkMode -eq 'RangeImage') {
    Copy-Item -LiteralPath $CapturePlanPath -Destination $stagedCapturePlan -Force
}

try {
    $serverProject = Join-Path $serverStage 'DrawingBoundsOfficeAddinServer.csproj'

    Write-Host 'Building local Office.js Kestrel host...'
    & $dotnet build $serverProject -c Release --nologo

    if ($LASTEXITCODE -ne 0) {
        throw "Office.js Kestrel host build failed with exit code $LASTEXITCODE"
    }

    $serverDll = Join-Path `
        $serverStage `
        'bin\Release\net8.0\DrawingBoundsOfficeAddinServer.dll'

    if (-not (Test-Path -LiteralPath $serverDll)) {
        throw "Kestrel host build completed without producing: $serverDll"
    }

    $env:DRAWING_BOUNDS_OUTPUT_PATH = $stagedOutput
    $env:DRAWING_BOUNDS_FILE_LIST = $stagedList
    $env:DRAWING_BOUNDS_STATIC_ROOT = $stage

    if ($BenchmarkMode -eq 'RangeImage') {
        $env:DRAWING_BOUNDS_CAPTURE_PLAN_PATH = $stagedCapturePlan
        $env:DRAWING_BOUNDS_IMAGE_OUTPUT_ROOT = $stagedImageRoot
        $env:DRAWING_BOUNDS_IMAGE_BATCH_SIZE = [string]$ImageBatchSize
    }
    else {
        $env:DRAWING_BOUNDS_CAPTURE_PLAN_PATH = $null
        $env:DRAWING_BOUNDS_IMAGE_OUTPUT_ROOT = $null
        $env:DRAWING_BOUNDS_IMAGE_BATCH_SIZE = $null
    }

    $env:ASPNETCORE_ENVIRONMENT = 'Development'

    $server = Start-Process `
        -FilePath $dotnet `
        -ArgumentList @($serverDll) `
        -WorkingDirectory $stage `
        -RedirectStandardOutput $serverOut `
        -RedirectStandardError $serverErr `
        -WindowStyle Hidden `
        -PassThru

    $ready = $false

    for ($attempt = 0; $attempt -lt 150; $attempt++) {
        Start-Sleep -Milliseconds 100

        if ($server.HasExited) {
            break
        }

        try {
            $health = Invoke-RestMethod `
                -Uri 'http://127.0.0.1:3001/health' `
                -Method Get

            if ($health.ok) {
                $ready = $true
                break
            }
        }
        catch {
        }
    }

    if (-not $ready) {
        $stderr = if (Test-Path -LiteralPath $serverErr) {
            [IO.File]::ReadAllText($serverErr)
        }
        else {
            ''
        }

        throw "Office.js Kestrel host did not become ready. $stderr"
    }

    try {
        $taskpanePreflight = Invoke-WebRequest `
            -Uri 'https://localhost:3000/taskpane.html' `
            -UseBasicParsing

        if ($taskpanePreflight.StatusCode -ne 200) {
            throw (
                'Unexpected HTTPS preflight status: ' +
                $taskpanePreflight.StatusCode
            )
        }
    }
    catch {
        throw (
            'The Office.js HTTPS endpoint is not reachable/trusted from Windows: ' +
            $_.Exception.Message
        )
    }

    $officeJsCdn = 'https://officeapis.public.onecdn.static.microsoft/1/office.js'
    Write-Host 'Checking current Microsoft Office.js CDN endpoint...'

    try {
        $officeJsPreflight = Invoke-WebRequest `
            -Uri $officeJsCdn `
            -UseBasicParsing `
            -TimeoutSec 30

        if ($officeJsPreflight.StatusCode -ne 200) {
            throw (
                'Unexpected Office.js CDN status: ' +
                $officeJsPreflight.StatusCode
            )
        }

        $officeJsBytes = [Text.Encoding]::UTF8.GetByteCount(
            [string]$officeJsPreflight.Content
        )

        if ($officeJsBytes -lt 10000) {
            throw (
                'Office.js CDN response was unexpectedly small: ' +
                $officeJsBytes +
                ' bytes'
            )
        }

        Write-Host (
            '  Windows can reach Office.js CDN: ' +
            $officeJsBytes +
            ' bytes'
        )
    }
    catch {
        Write-Warning (
            'Windows PowerShell could not fetch the current Office.js CDN endpoint. ' +
            'Excel WebView2 may fail for the same proxy/firewall reason. ' +
            $_.Exception.Message
        )
    }

    $manifest = Join-Path $stage 'manifest.xml'
    $sourceManifest = Join-Path $PSScriptRoot 'manifest.xml'
    $addInId = Get-ManifestId $sourceManifest
    $addInVersion = Get-ManifestVersion $sourceManifest

    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw 'LOCALAPPDATA is required for the persistent Office.js developer manifest.'
    }

    $registrationRoot = Join-Path `
        $env:LOCALAPPDATA `
        'excel-shrink-rust\office-addins\visual-validation'

    New-Item -ItemType Directory -Path $registrationRoot -Force | Out-Null

    $registrationManifest = Join-Path $registrationRoot 'manifest.xml'
    Copy-Item `
        -LiteralPath $sourceManifest `
        -Destination $registrationManifest `
        -Force

    # Mirror Microsoft's office-addin-dev-settings Windows registration:
    # HKCU\Software\Microsoft\Office\16.0\Wef\Developer
    #   <manifest-id> = <absolute manifest path>
    #
    # Use a stable local-Windows manifest instead of the temporary server staging
    # directory (and instead of a WSL UNC path). By default the developer
    # registration survives the benchmark so subsequent runs remain sideloaded.
    $registrationState = Register-DeveloperAddIn $addInId $registrationManifest

    if ($TemporaryDeveloperRegistration) {
        Write-Host 'Office.js developer registration: temporary for this run'
    }
    else {
        Write-Host 'Office.js developer registration: persistent'
        Write-Host "  $registrationManifest"
    }

    Ensure-WebViewLoopback

    if ($EnableRuntimeLogging) {
        $runtimeLogPath = Join-Path $diagnosticRoot 'office-runtime.log'
        Remove-Item `
            -LiteralPath $runtimeLogPath `
            -Force `
            -ErrorAction SilentlyContinue

        $runtimeLoggingState = Enable-OfficeRuntimeLogging $runtimeLogPath

        Write-Host 'Office runtime logging: enabled for this diagnostic run'
        Write-Host "  $runtimeLogPath"
        Write-Host (
            '  Runtime logging affects performance; do not enable it for final ' +
            'benchmark timing runs.'
        )
    }

    $excelExecutable = Find-ExcelExecutable

    Write-Host ('Office.js benchmark mode: ' + $BenchmarkMode)

    if (-not [string]::IsNullOrWhiteSpace($excelExecutable)) {
        Write-Host 'Document launch mode: dedicated Excel process (excel.exe /x).'
        Write-Host "  $excelExecutable"
    }
    else {
        Write-Warning (
            'EXCEL.EXE could not be resolved from PATH/App Paths. Falling back to ' +
            'Windows shell document launch, which can reuse an already-running Excel ' +
            'instance and is less reliable for development add-in activation.'
        )
    }

    $workbookOrdinal = 0

    foreach ($path in $paths) {
        $workbookOrdinal++
        $workbookName = [IO.Path]::GetFileName($path)
        $isFirstWorkbook = $workbookOrdinal -eq 1

        Write-Host "Office.js: $workbookName"

        if ($isFirstWorkbook) {
            Write-Host (
                '  The first workbook is the add-in activation probe. The developer ' +
                'registration will be reasserted before every launch.'
            )
            Write-Host (
                "  Activation plan: $FirstActivationAttempts attempt(s), " +
                "$FirstActivationTimeoutSeconds seconds each, " +
                "$ActivationRetryDelaySeconds seconds between attempts."
            )

            if ($RegistrationSettleSeconds -gt 0) {
                Write-Host (
                    "  Waiting $RegistrationSettleSeconds seconds after registration " +
                    'before the first fresh Excel process...'
                )
                Start-Sleep -Seconds $RegistrationSettleSeconds
            }
        }

        $received = $false
        $encodedName = [Uri]::EscapeDataString($workbookName)
        $lastStatus = $null
        $totalAttempts = if ($isFirstWorkbook) {
            $FirstActivationAttempts
        }
        else {
            $PerWorkbookRetries + 1
        }
        $timeoutSeconds = if ($isFirstWorkbook) {
            $FirstActivationTimeoutSeconds
        }
        else {
            $ResultTimeoutSeconds
        }

        for (
            $runAttempt = 0;
            $runAttempt -lt $totalAttempts -and -not $received;
            $runAttempt++
        ) {
            $attemptNumber = $runAttempt + 1
            $launchProcess = $null

            if ($runAttempt -gt 0 -and $ActivationRetryDelaySeconds -gt 0) {
                Write-Host (
                    "  Waiting $ActivationRetryDelaySeconds seconds before activation " +
                    "attempt $attemptNumber/$totalAttempts..."
                )
                Start-Sleep -Seconds $ActivationRetryDelaySeconds
            }
            elseif ($runAttempt -gt 0) {
                Write-Host (
                    "  Starting activation attempt $attemptNumber/$totalAttempts..."
                )
            }

            # Reassert the persistent developer registration before every fresh
            # Excel process, not only for the first workbook. A later Excel launch
            # can otherwise observe stale development-add-in state as well.
            Refresh-DeveloperAddInRegistration `
                -AddInId $addInId `
                -ManifestPath $registrationManifest

            # Use a fresh full path on every attempt while preserving Workbook.Name.
            # This prevents Excel/DDE from merely reactivating a failed prior open.
            $attemptDirectory = Join-Path `
                $documents `
                ('workbook-{0:D2}\attempt-{1:D2}' -f $workbookOrdinal, $attemptNumber)

            New-Item `
                -ItemType Directory `
                -Path $attemptDirectory `
                -Force | Out-Null

            $documentCopy = Join-Path $attemptDirectory $workbookName

            Copy-Item `
                -LiteralPath $path `
                -Destination $documentCopy `
                -Force

            Add-OfficeAddInAutoOpenToWorkbook `
                -WorkbookPath $documentCopy `
                -AddInId $addInId `
                -AddInVersion $addInVersion

            if ($attemptNumber -eq 1) {
                Write-Host (
                    '  Embedded Office.AutoShowTaskpaneWithDocument into the ' +
                    'temporary copy.'
                )
            }

            Write-Host (
                "  Launching activation attempt $attemptNumber/$totalAttempts..."
            )

            try {
                try {
                    Invoke-RestMethod `
                        -Uri 'http://127.0.0.1:3001/reset-diagnostics' `
                        -Method Post | Out-Null
                }
                catch {
                }

                # Prefer a brand-new Excel process. Microsoft documents /x as the
                # command-line switch that starts Excel in a new process. That makes
                # each activation attempt re-read the persistent developer registration
                # instead of inheriting stale add-in state from an older Excel process.
                $launchProcess = Start-BenchmarkExcelProcess `
                    -ExcelExecutable $excelExecutable `
                    -DocumentPath $documentCopy

                if ($null -ne $launchProcess) {
                    Write-Host ("    Excel PID: " + $launchProcess.Id)
                }

                $pollMilliseconds = 200
                $maxAttempts = [Math]::Ceiling(
                    ($timeoutSeconds * 1000.0) / $pollMilliseconds
                )

                for ($attempt = 0; $attempt -lt $maxAttempts; $attempt++) {
                    Start-Sleep -Milliseconds $pollMilliseconds

                    # Do not treat the Process object returned by Start-Process as
                    # authoritative for workbook lifetime. Office/Click-to-Run can hand
                    # the document to another Excel process while the launcher PID exits.
                    # Keep polling the benchmark host for the full workbook timeout.

                    try {
                        $lastStatus = Invoke-RestMethod `
                            -Uri ("http://127.0.0.1:3001/status?workbook=$encodedName") `
                            -Method Get

                        if ($lastStatus.done) {
                            $received = $true
                            break
                        }

                        if (@($lastStatus.errors).Count -gt 0) {
                            throw (
                                'Office.js task pane reported an error: ' +
                                (@($lastStatus.errors) -join '; ')
                            )
                        }
                    }
                    catch {
                        if ($_.Exception.Message -like 'Office.js task pane reported*') {
                            throw
                        }
                    }
                }
            }
            finally {
                if ($received) {
                    # /result has already been accepted by the local host at this point.
                    # Keep the workbook visible briefly so the task-pane runtime can
                    # finish its final event traffic and Excel/WebView can settle.
                    if ($PostResultHoldSeconds -gt 0) {
                        Write-Host (
                            "  Result received. Keeping workbook open for " +
                            "$PostResultHoldSeconds second(s) before host teardown..."
                        )
                        Start-Sleep -Seconds $PostResultHoldSeconds
                    }

                    # The task pane no longer closes the workbook itself. This dedicated
                    # /x process belongs only to this benchmark attempt, so the host owns
                    # teardown and there is only one closer.
                    Stop-BenchmarkExcelProcess $launchProcess
                }
                else {
                    # Failed attempts must not leak an Excel process into the next
                    # activation attempt.
                    Stop-BenchmarkExcelProcess $launchProcess
                }

                if ($null -ne $launchProcess) {
                    try { $launchProcess.Dispose() } catch {}
                    $launchProcess = $null
                }
            }
        }

        if (-not $received) {
            if (Test-Path -LiteralPath $serverOut) {
                Copy-Item `
                    -LiteralPath $serverOut `
                    -Destination (Join-Path $diagnosticRoot 'server.stdout.txt') `
                    -Force
            }

            if (Test-Path -LiteralPath $serverErr) {
                Copy-Item `
                    -LiteralPath $serverErr `
                    -Destination (Join-Path $diagnosticRoot 'server.stderr.txt') `
                    -Force
            }

            if (Test-Path -LiteralPath $stagedOutput) {
                Copy-Item `
                    -LiteralPath $stagedOutput `
                    -Destination $PartialOutputPath `
                    -Force
            }

            if (
                $BenchmarkMode -eq 'RangeImage' -and
                -not [string]::IsNullOrWhiteSpace($ImageOutputDir)
            ) {
                Copy-DirectoryContents $stagedImageRoot $ImageOutputDir
            }

            $diagnostics = Format-OfficeAddInDiagnostics $lastStatus
            $runtimeDiagnostics = ''

            if ($EnableRuntimeLogging) {
                Start-Sleep -Milliseconds 500
                $runtimeDiagnostics = Get-OfficeRuntimeLogSummary `
                    -LogPath $runtimeLogPath `
                    -AddInId $addInId

                if (-not [string]::IsNullOrWhiteSpace($runtimeDiagnostics)) {
                    $runtimeDiagnostics = ' ' + $runtimeDiagnostics
                }
            }

            $partialNote = if (Test-Path -LiteralPath $PartialOutputPath) {
                " Partial measurements were preserved at: $PartialOutputPath"
            }
            else {
                ''
            }

            $message = (
                "The Office.js task pane did not submit a result for $workbookName " +
                "within $timeoutSeconds seconds per attempt after " +
                "$totalAttempts attempt(s). $diagnostics" +
                "$runtimeDiagnostics$partialNote " +
                "Diagnostics are in: $diagnosticRoot"
            )

            Write-Status 'partial' $message

            if ($SkipIfUnavailable) {
                Write-Warning ("Office.js ${BenchmarkMode}: PARTIAL - $message")
                return
            }

            throw $message
        }

        if (
            $received -and
            $InterWorkbookDelaySeconds -gt 0 -and
            $workbookOrdinal -lt $paths.Count
        ) {
            Write-Host (
                "  Waiting $InterWorkbookDelaySeconds second(s) before starting " +
                'the next Excel process...'
            )
            Start-Sleep -Seconds $InterWorkbookDelaySeconds
        }
    }

    if (-not (Test-Path -LiteralPath $stagedOutput)) {
        throw "Office.js experiment completed without producing: $stagedOutput"
    }

    Copy-Item `
        -LiteralPath $stagedOutput `
        -Destination $OutputPath `
        -Force

    if (
        $BenchmarkMode -eq 'RangeImage' -and
        -not [string]::IsNullOrWhiteSpace($ImageOutputDir)
    ) {
        Copy-DirectoryContents $stagedImageRoot $ImageOutputDir
    }

    Remove-Item -LiteralPath $PartialOutputPath -Force -ErrorAction SilentlyContinue

    Write-Status `
        'ok' `
        ("Office.js $BenchmarkMode experiment completed with the local .NET/Kestrel host.")

    Write-Host 'Office.js add-in result:'
    Write-Host "  $OutputPath"

    if ($BenchmarkMode -eq 'RangeImage') {
        Write-Host 'Office.js persisted PNGs:'
        Write-Host "  $ImageOutputDir"
    }
}
catch {
    Write-Status 'error' $_.Exception.Message
    throw
}
finally {
    if ($TemporaryDeveloperRegistration) {
        Restore-DeveloperAddIn $addInId $registrationState
    }

    Restore-OfficeRuntimeLogging $runtimeLoggingState

    if ($null -ne $server -and -not $server.HasExited) {
        try {
            Stop-Process -Id $server.Id -Force
        }
        catch {
        }
    }

    $env:DRAWING_BOUNDS_OUTPUT_PATH = $oldOutput
    $env:DRAWING_BOUNDS_FILE_LIST = $oldList
    $env:DRAWING_BOUNDS_STATIC_ROOT = $oldStaticRoot
    $env:DRAWING_BOUNDS_CAPTURE_PLAN_PATH = $oldCapturePlan
    $env:DRAWING_BOUNDS_IMAGE_OUTPUT_ROOT = $oldImageRoot
    $env:DRAWING_BOUNDS_IMAGE_BATCH_SIZE = $oldImageBatchSize
    $env:ASPNETCORE_ENVIRONMENT = $oldAspNetEnvironment

    Remove-Item `
        -LiteralPath $tempRoot `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue
}
