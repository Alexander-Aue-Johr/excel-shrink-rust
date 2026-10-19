param(
    [Parameter(Mandatory=$true)][string]$WorkbookPath,
    [Parameter(Mandatory=$true)][string]$ConfigPath,
    [Parameter(Mandatory=$true)][string]$OutputDir,
    [string]$PublishedOutputDir,
    [ValidateRange(10, 600)][int]$StartupTimeoutSeconds = 180,
    [ValidateRange(10, 7200)][int]$CaptureTimeoutSeconds = 1800
)

$ErrorActionPreference = 'Stop'
$utf8 = New-Object System.Text.UTF8Encoding($false, $true)

function Get-DotNet8 {
    $candidates = New-Object 'System.Collections.Generic.List[string]'
    if ($env:EXCEL_SHRINK_DOTNET) { $candidates.Add($env:EXCEL_SHRINK_DOTNET) }
    $globalCommand = Get-Command dotnet.exe -ErrorAction SilentlyContinue
    if ($null -ne $globalCommand) { $candidates.Add([string]$globalCommand.Source) }
    if ($env:LOCALAPPDATA) {
        $toolchains = Join-Path $env:LOCALAPPDATA 'excel-shrink-rust\repo-toolchains'
        if (Test-Path -LiteralPath $toolchains -PathType Container) {
            foreach ($file in @(Get-ChildItem -LiteralPath $toolchains -Recurse -File -Filter dotnet.exe -ErrorAction SilentlyContinue)) {
                $candidates.Add($file.FullName)
            }
        }
    }
    foreach ($candidate in ($candidates | Select-Object -Unique)) {
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) { continue }
        $sdks = @(& $candidate --list-sdks 2>$null)
        if ($LASTEXITCODE -eq 0 -and @($sdks | Where-Object { $_ -match '^8\.' }).Count -gt 0) {
            return $candidate
        }
    }
    throw 'A Windows .NET 8 SDK is required for worksheet capture.'
}

function Get-SourceKey([string]$sourceDirectory) {
    $parts = foreach ($file in @(Get-ChildItem -LiteralPath $sourceDirectory -File | Where-Object { $_.Extension -in @('.cs', '.csproj') } | Sort-Object Name)) {
        $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        "$($file.Name)=$hash"
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes(($parts -join "`n"))
        return (([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()).Substring(0, 20)
    } finally { $sha.Dispose() }
}

function Quote-Argument([string]$value) {
    if ($value.Contains('"')) { throw 'Double quotes are not supported in launch paths.' }
    return '"' + $value + '"'
}

function Build-Host([string]$dotnet, [string]$stage, [string]$outputDirectory) {
    $source = Join-Path $PSScriptRoot 'host'
    $project = Join-Path $source 'OfficeCaptureHost.csproj'
    if (-not (Test-Path -LiteralPath $project -PathType Leaf)) { throw 'Capture host project is missing.' }
    $cacheRoot = Join-Path $env:LOCALAPPDATA 'excel-shrink-rust\office-capture\host'
    $cache = Join-Path $cacheRoot (Get-SourceKey $source)
    $dll = Join-Path $cache 'OfficeCaptureHost.dll'
    if ((Test-Path -LiteralPath $dll -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $cache 'OfficeCaptureHost.runtimeconfig.json') -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $cache 'OfficeCaptureHost.deps.json') -PathType Leaf)) {
        return $dll
    }

    $localSource = Join-Path $stage 'host-source'
    $localBuild = Join-Path $stage 'host-build'
    New-Item -ItemType Directory -Path $localSource, $localBuild -Force | Out-Null
    Copy-Item -LiteralPath $project -Destination $localSource
    foreach ($file in @(Get-ChildItem -LiteralPath $source -File -Filter '*.cs')) {
        Copy-Item -LiteralPath $file.FullName -Destination $localSource
    }
    $localProject = Join-Path $localSource 'OfficeCaptureHost.csproj'
    $stdout = Join-Path $stage 'build.stdout.log'
    $stderr = Join-Path $stage 'build.stderr.log'
    $arguments = 'build {0} -c Release -o {1} --nologo' -f (Quote-Argument $localProject), (Quote-Argument $localBuild)
    $process = Start-Process -FilePath $dotnet -ArgumentList $arguments -WorkingDirectory $localSource `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(180000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw 'Capture host build exceeded 180 seconds.'
    }
    $process.Refresh()
    $buildOutputs = @(
        (Join-Path $localBuild 'OfficeCaptureHost.dll'),
        (Join-Path $localBuild 'OfficeCaptureHost.runtimeconfig.json'),
        (Join-Path $localBuild 'OfficeCaptureHost.deps.json')
    )
    $missingOutputs = @($buildOutputs | Where-Object { -not (Test-Path -LiteralPath $_ -PathType Leaf) })
    if (($null -ne $process.ExitCode -and $process.ExitCode -ne 0) -or $missingOutputs.Count -gt 0) {
        Copy-Item -LiteralPath $stdout -Destination (Join-Path $outputDirectory 'host-build.stdout.log') -Force
        Copy-Item -LiteralPath $stderr -Destination (Join-Path $outputDirectory 'host-build.stderr.log') -Force
        $exitCode = if ($null -eq $process.ExitCode) { 'unavailable' } else { [string]$process.ExitCode }
        throw "Capture host build failed (exit code: $exitCode; missing outputs: $($missingOutputs -join ', ')). See host-build.*.log in $outputDirectory."
    }
    New-Item -ItemType Directory -Path $cache -Force | Out-Null
    Copy-Item -Path (Join-Path $localBuild '*') -Destination $cache -Recurse -Force
    return $dll
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

function Register-AddIn([string]$id, [string]$manifestPath) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Office\16.0\Wef\Developer', $true)
    if ($null -eq $key) { throw 'Cannot open Office developer add-in registry key.' }
    try {
        $previous = if (@($key.GetValueNames()) -contains $id) { [string]$key.GetValue($id) } else { $null }
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
        $process.Refresh()
        if ($process.HasExited) { return }
        [void]$process.CloseMainWindow()
        if (-not $process.WaitForExit(3000)) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
    } catch {}
}

function Copy-CompletedOutput([string]$from, [string]$to) {
    $imageFrom = Join-Path $from 'images'
    $imageTo = Join-Path $to 'images'
    New-Item -ItemType Directory -Path $imageTo -Force | Out-Null
    foreach ($image in @(Get-ChildItem -LiteralPath $imageFrom -File -Filter '*.png')) {
        Copy-VerifiedFile $image.FullName (Join-Path $imageTo $image.Name)
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

$sourceWorkbook = [IO.Path]::GetFullPath($WorkbookPath)
$sourceConfig = [IO.Path]::GetFullPath($ConfigPath)
$destination = [IO.Path]::GetFullPath($OutputDir)
if (-not $PublishedOutputDir) { $PublishedOutputDir = $destination }
if (-not (Test-Path -LiteralPath $sourceWorkbook -PathType Leaf)) { throw "Workbook missing: $sourceWorkbook" }
if (-not (Test-Path -LiteralPath $sourceConfig -PathType Leaf)) { throw "Capture config missing: $sourceConfig" }
New-Item -ItemType Directory -Path $destination -Force | Out-Null
if (Test-Path -LiteralPath (Join-Path $destination 'result.json')) { throw 'Output directory already has a result.json.' }

$stage = Join-Path $env:TEMP ('es-office-capture-' + [Guid]::NewGuid().ToString('N'))
$runId = [Guid]::NewGuid().ToString('N')
$hostProcess = $null
$excelProcess = $null
$addInId = $null
$registrationDone = $false
$previousRegistration = $null
New-Item -ItemType Directory -Path $stage -Force | Out-Null
try {
    $dotnet = Get-DotNet8
    $env:DOTNET_ROOT = Split-Path -Parent $dotnet
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $dll = Build-Host $dotnet $stage $destination

    $webSource = Join-Path $PSScriptRoot 'web'
    $webStage = Join-Path $stage 'web'
    $hostOutput = Join-Path $stage 'output'
    $documentDirectory = Join-Path $stage 'document'
    foreach ($path in @($webStage, $hostOutput, $documentDirectory)) { New-Item -ItemType Directory -Path $path -Force | Out-Null }
    Copy-Item -Path (Join-Path $webSource '*') -Destination $webStage -Recurse -Force
    foreach ($asset in @('manifest.xml', 'taskpane.html', 'commands.html', 'app.js', 'capture-geometry.js', 'capture-overflow.js', 'taskpane.css', 'icon-32.png')) {
        if (-not (Test-Path -LiteralPath (Join-Path $webStage $asset) -PathType Leaf)) {
            throw "Office add-in asset is missing: $asset"
        }
    }
    $manifestSource = Join-Path $webStage 'manifest.xml'
    [xml]$manifest = [IO.File]::ReadAllText($manifestSource, $utf8)
    $idNode = $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Id']")
    $versionNode = $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Version']")
    if ($null -eq $idNode -or $null -eq $versionNode) { throw 'Office add-in manifest is incomplete.' }
    $addInId = [Guid]::NewGuid().ToString()
    $idNode.InnerText = $addInId
    $manifestPath = Join-Path $stage 'manifest.xml'
    [IO.File]::WriteAllText($manifestPath, $manifest.OuterXml, $utf8)

    $workbookCopy = Join-Path $documentDirectory ([IO.Path]::GetFileName($sourceWorkbook))
    Copy-Item -LiteralPath $sourceWorkbook -Destination $workbookCopy
    Add-AutoOpenTaskpane $workbookCopy $addInId $versionNode.InnerText.Trim()
    $configCopy = Join-Path $stage 'config.json'
    Copy-Item -LiteralPath $sourceConfig -Destination $configCopy

    $hostStdout = Join-Path $stage 'host.stdout.log'
    $hostStderr = Join-Path $stage 'host.stderr.log'
    $hostArguments = '{0} --config {1} --output {2} --published-output {3} --web {4} --run-id {5}' -f `
        (Quote-Argument $dll), (Quote-Argument $configCopy), (Quote-Argument $hostOutput), `
        (Quote-Argument $PublishedOutputDir), (Quote-Argument $webStage), (Quote-Argument $runId)
    $hostProcess = Start-Process -FilePath $dotnet -ArgumentList $hostArguments -WorkingDirectory $stage `
        -RedirectStandardOutput $hostStdout -RedirectStandardError $hostStderr -WindowStyle Hidden -PassThru
    $hostReady = $false
    $startup = [Diagnostics.Stopwatch]::StartNew()
    while ($startup.Elapsed.TotalSeconds -lt 30) {
        Start-Sleep -Milliseconds 200
        $hostProcess.Refresh()
        if ($hostProcess.HasExited) { break }
        try {
            $health = Invoke-RestMethod -Uri 'http://127.0.0.1:3001/health' -TimeoutSec 2
            if ($health.ok -and $health.run_id -eq $runId) { $hostReady = $true; break }
        } catch {}
    }
    if (-not $hostReady) { throw 'Worksheet capture host did not become ready within 30 seconds.' }
    $taskpaneResponse = Invoke-WebRequest -UseBasicParsing -Uri 'http://localhost:3000/taskpane.html' -TimeoutSec 10
    if ($taskpaneResponse.StatusCode -ne 200) { throw 'Office add-in taskpane is unavailable over HTTP.' }

    $previousRegistration = Register-AddIn $addInId $manifestPath
    $registrationDone = $true
    $excelExecutable = Find-ExcelExecutable
    $excelProcess = Start-Process -FilePath $excelExecutable -ArgumentList ('/x ' + (Quote-Argument $workbookCopy)) -PassThru

    $startup.Restart()
    $captureStarted = $false
    $result = Join-Path $hostOutput 'result.json'
    $captureTimer = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $result -PathType Leaf)) {
        $hostProcess.Refresh()
        if ($hostProcess.HasExited) { throw 'Worksheet capture host exited before writing a result.' }
        if (-not $captureStarted) {
            try {
                $health = Invoke-RestMethod -Uri 'http://127.0.0.1:3001/health' -TimeoutSec 2
                if ($health.run_id -eq $runId -and $health.config_seen) {
                    $captureStarted = $true
                    $captureTimer.Restart()
                }
            } catch {}
            if ($startup.Elapsed.TotalSeconds -gt $StartupTimeoutSeconds) { throw 'Office add-in did not start before the startup timeout.' }
        } elseif ($captureTimer.Elapsed.TotalSeconds -gt $CaptureTimeoutSeconds) {
            throw 'Worksheet capture exceeded the capture timeout.'
        }
        Start-Sleep -Milliseconds 250
    }

    Copy-CompletedOutput $hostOutput $destination
    $captureResult = [IO.File]::ReadAllText((Join-Path $destination 'result.json'), $utf8) | ConvertFrom-Json
    if ($null -ne $captureResult.PSObject.Properties['error'] -and $null -ne $captureResult.error) {
        throw "Worksheet capture failed: $($captureResult.error | ConvertTo-Json -Compress -Depth 5)"
    }
    Write-Host ('RESULT: ' + (Join-Path $destination 'result.json'))
} catch {
    foreach ($name in @('host.stdout.log', 'host.stderr.log')) {
        $log = Join-Path $stage $name
        if (Test-Path -LiteralPath $log -PathType Leaf) {
            Copy-Item -LiteralPath $log -Destination (Join-Path $destination $name) -Force -ErrorAction SilentlyContinue
        }
    }
    throw
} finally {
    Stop-OwnedProcess $excelProcess
    Stop-OwnedProcess $hostProcess
    if ($registrationDone) { Restore-AddIn $addInId $previousRegistration }
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
