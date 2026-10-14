param(
    [string]$CorpusRoot,
    [switch]$DeferValidation
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\_shared\reporting.ps1')
$vvReportOwner = Start-VVReport -Start $PSScriptRoot -Experiment 'shape-bounds-native' -Arguments $PSBoundParameters
try {
$ErrorActionPreference = 'Stop'

# Keep repository-authored experiment output stable across localized Windows hosts.
$env:DOTNET_CLI_UI_LANGUAGE = 'en-us'
$env:DOTNET_NOLOGO = '1'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'

$RepoRoot = [IO.Path]::GetFullPath(
    (Join-Path $PSScriptRoot '..\..\..\..')
)

if ([string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $CorpusRoot = Join-Path $RepoRoot 'corpus-local'
}
else {
    $CorpusRoot = [IO.Path]::GetFullPath($CorpusRoot)
}

$BasPath = Join-Path $PSScriptRoot 'drawing-bounds.bas'
$PowerShellComScript = Join-Path $PSScriptRoot 'powershell-com.ps1'

$RustProjectRoot = Join-Path $PSScriptRoot 'rust-com'
$RustCargoTomlPath = Join-Path $RustProjectRoot 'Cargo.toml'
$RustCargoLockPath = Join-Path $RustProjectRoot 'Cargo.lock'
$RustSourcePath = Join-Path $RustProjectRoot 'src\main.rs'
$RustReleaseRoot = Join-Path `
    $RustProjectRoot `
    'target\x86_64-pc-windows-gnu\release'
$RustExe = Join-Path $RustReleaseRoot 'drawing-bounds-rust-com.exe'
$RustFingerprintPath = Join-Path $RustReleaseRoot 'source.sha256'

$CSharpProjectRoot = Join-Path $PSScriptRoot 'csharp-addin'
$CSharpProjectPath = Join-Path $CSharpProjectRoot 'DrawingBoundsAddIn.csproj'
$CSharpSourcePath = Join-Path $CSharpProjectRoot 'DrawingBoundsCommands.cs'

$ExperimentCacheRoot = Join-Path `
    $RepoRoot `
    '.tmp\visual-validation\shape-bounds'

$VbaCacheRoot = Join-Path $ExperimentCacheRoot 'vba-host'
$VbaHostPath = Join-Path $VbaCacheRoot 'drawing-bounds-host.xlsm'
$VbaHostFingerprintPath = Join-Path $VbaCacheRoot 'source.sha256'

$CSharpCacheRoot = Join-Path $ExperimentCacheRoot 'csharp-addin'
$CSharpXllPath = Join-Path $CSharpCacheRoot 'DrawingBoundsAddIn-AddIn64-packed.xll'
$CSharpFingerprintPath = Join-Path $CSharpCacheRoot 'source.sha256'

function Get-StableStringSha256([string]$Value) {
    $sha256 = [Security.Cryptography.SHA256]::Create()

    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
        $hash = $sha256.ComputeHash($bytes)

        return (
            [BitConverter]::ToString($hash)
        ).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    throw @"
LOCALAPPDATA is not available.

The experiment needs a writable local Windows directory for its repo-specific
.NET SDK fallback because installing a Windows SDK directly below a
\\wsl.localhost\... path is unreliable.
"@
}

$RepoToolchainId = (
    Get-StableStringSha256 $RepoRoot
).Substring(0, 16)

$WindowsToolchainRoot = Join-Path `
    $env:LOCALAPPDATA `
    "excel-shrink-rust\repo-toolchains\$RepoToolchainId"

$RepoDotNetRoot = Join-Path `
    $WindowsToolchainRoot `
    'dotnet-win-x64'

$RepoDotNetExe = Join-Path `
    $RepoDotNetRoot `
    'dotnet.exe'

$DotNetInstallScript = Join-Path `
    $WindowsToolchainRoot `
    'dotnet-install.ps1'


function Resolve-ExistingPath([string]$Path, [string]$Label) {
    try {
        $item = Get-Item -LiteralPath $Path -ErrorAction Stop
        return [IO.Path]::GetFullPath($item.FullName)
    }
    catch {
        throw "$Label does not exist: $Path"
    }
}


function Write-Utf8NoBom([string]$Path, [string]$Text) {
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($Path, $Text, $encoding)
}


function Copy-Utf8WithoutBom([string]$Source, [string]$Destination) {
    $bytes = [IO.File]::ReadAllBytes($Source)

    if (
        $bytes.Length -ge 3 -and
        $bytes[0] -eq 0xEF -and
        $bytes[1] -eq 0xBB -and
        $bytes[2] -eq 0xBF
    ) {
        $withoutBom = New-Object byte[] ($bytes.Length - 3)
        [Array]::Copy($bytes, 3, $withoutBom, 0, $withoutBom.Length)
        [IO.File]::WriteAllBytes($Destination, $withoutBom)
    }
    else {
        [IO.File]::WriteAllBytes($Destination, $bytes)
    }
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


function Get-SourceFingerprint([string[]]$Paths) {
    $parts = @()

    foreach ($path in $Paths) {
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        $parts += "$path=$hash"
    }

    return ($parts -join "`n")
}


function Read-CachedFingerprint([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return $null
    }

    return (Get-Content -LiteralPath $Path -Raw).Trim()
}


function Get-RustSourceHashes {
    return [ordered]@{
        'Cargo.toml' = (
            Get-FileHash -LiteralPath $RustCargoTomlPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()

        'Cargo.lock' = (
            Get-FileHash -LiteralPath $RustCargoLockPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()

        'src/main.rs' = (
            Get-FileHash -LiteralPath $RustSourcePath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
    }
}


function Read-RustSourceHashes([string]$Path) {
    $result = @{}
    $allowedKeys = @(
        'Cargo.toml',
        'Cargo.lock',
        'src/main.rs'
    )

    foreach ($line in (Get-Content -LiteralPath $Path)) {
        if ([string]::IsNullOrWhiteSpace($line)) {
            continue
        }

        if ($line -notmatch '^([^=]+)=([0-9A-Fa-f]{64})$') {
            throw "Malformed Rust source fingerprint line: $line"
        }

        $key = $Matches[1]
        $hash = $Matches[2].ToLowerInvariant()

        if ($key -notin $allowedKeys) {
            throw "Unexpected Rust source fingerprint key: $key"
        }

        if ($result.ContainsKey($key)) {
            throw "Duplicate Rust source fingerprint key: $key"
        }

        $result[$key] = $hash
    }

    foreach ($key in $allowedKeys) {
        if (-not $result.ContainsKey($key)) {
            throw "Rust source fingerprint is missing key: $key"
        }
    }

    if ($result.Count -ne $allowedKeys.Count) {
        throw (
            "Rust source fingerprint contains an unexpected number of entries: " +
            $result.Count
        )
    }

    return $result
}


function Assert-RustHelperFresh {
    $buildCommand = (
        'bash ./docs/visual-validation/experiments/' +
        'shape-bounds/build-rust-com.sh'
    )

    if (-not (Test-Path -LiteralPath $RustExe)) {
        throw @"
Rust COM executable is missing:
  $RustExe

Build it from WSL before running the experiment:
  $buildCommand
"@
    }

    if (-not (Test-Path -LiteralPath $RustFingerprintPath)) {
        throw @"
Rust COM source fingerprint is missing:
  $RustFingerprintPath

The executable may predate source-freshness tracking. Rebuild it from WSL:
  $buildCommand
"@
    }

    $expected = Get-RustSourceHashes
    $actual = Read-RustSourceHashes $RustFingerprintPath
    $mismatches = New-Object 'System.Collections.Generic.List[string]'

    foreach ($key in $expected.Keys) {
        if ($actual[$key] -ne $expected[$key]) {
            $mismatches.Add(
                (
                    "{0}: expected {1}, built {2}" -f
                    $key,
                    $expected[$key],
                    $actual[$key]
                )
            )
        }
    }

    if ($mismatches.Count -gt 0) {
        $details = ($mismatches | ForEach-Object { "  $_" }) -join "`n"

        throw @"
Rust COM executable is stale relative to its recorded source hashes:
$details

Rebuild it from WSL:
  $buildCommand
"@
    }
}


# Repo-specific Windows .NET SDK fallback for the C# Excel-DNA experiment.
function Test-DotNet8Sdk([string]$DotNetPath) {
    if (-not (Test-Path -LiteralPath $DotNetPath)) {
        return $false
    }

    try {
        $sdkLines = @(& $DotNetPath --list-sdks 2>$null)

        if ($LASTEXITCODE -ne 0) {
            return $false
        }

        foreach ($line in $sdkLines) {
            if ([string]$line -match '^\s*8\.') {
                return $true
            }
        }

        return $false
    }
    catch {
        return $false
    }
}


function Find-GlobalDotNet8Sdk {
    foreach ($candidate in @('dotnet.exe', 'dotnet')) {
        $command = Get-Command $candidate -ErrorAction SilentlyContinue

        if (
            $null -ne $command -and
            (Test-DotNet8Sdk $command.Source)
        ) {
            return $command.Source
        }
    }

    return $null
}


function Install-RepoLocalDotNet8Sdk {
    New-Item `
        -ItemType Directory `
        -Path $WindowsToolchainRoot `
        -Force |
        Out-Null

    if (
        (Test-Path -LiteralPath $RepoDotNetRoot) -and
        -not (Test-DotNet8Sdk $RepoDotNetExe)
    ) {
        Write-Host 'Removing incomplete repo-specific .NET SDK cache:'
        Write-Host "  $RepoDotNetRoot"

        Remove-Item `
            -LiteralPath $RepoDotNetRoot `
            -Recurse `
            -Force
    }

    New-Item `
        -ItemType Directory `
        -Path $RepoDotNetRoot `
        -Force |
        Out-Null

    if (-not (Test-Path -LiteralPath $DotNetInstallScript)) {
        Write-Host 'Downloading Microsoft dotnet-install.ps1:'
        Write-Host "  $DotNetInstallScript"

        try {
            $previousSecurityProtocol = [Net.ServicePointManager]::SecurityProtocol

            try {
                [Net.ServicePointManager]::SecurityProtocol = `
                    [Net.SecurityProtocolType]::Tls12

                Invoke-WebRequest `
                    -UseBasicParsing `
                    -Uri 'https://dot.net/v1/dotnet-install.ps1' `
                    -OutFile $DotNetInstallScript
            }
            finally {
                [Net.ServicePointManager]::SecurityProtocol = `
                    $previousSecurityProtocol
            }
        }
        catch {
            throw @"
Failed to download Microsoft's dotnet-install.ps1.

URL:
  https://dot.net/v1/dotnet-install.ps1

Destination:
  $DotNetInstallScript

Original error:
$($_.Exception.Message)
"@
        }
    }
    else {
        Write-Host 'Reusing cached Microsoft dotnet-install.ps1:'
        Write-Host "  $DotNetInstallScript"
    }

    Unblock-File `
        -LiteralPath $DotNetInstallScript `
        -ErrorAction SilentlyContinue

    Write-Host 'Installing repo-specific Windows .NET 8 SDK without admin rights:'
    Write-Host "  $RepoDotNetRoot"

    $windowsPowerShell = Join-Path $PSHOME 'powershell.exe'

    & $windowsPowerShell `
        -NoLogo `
        -NoProfile `
        -ExecutionPolicy Bypass `
        -File $DotNetInstallScript `
        -Channel '8.0' `
        -Quality 'GA' `
        -Architecture 'x64' `
        -InstallDir $RepoDotNetRoot `
        -NoPath `
        -Verbose

    $installExitCode = $LASTEXITCODE

    if ($installExitCode -ne 0) {
        throw @"
Repo-specific .NET 8 SDK installation failed with exit code $installExitCode.

Installer:
  $DotNetInstallScript

Install directory:
  $RepoDotNetRoot

The installer is run with -Verbose, so its detailed failure should appear above
this message.
"@
    }

    if (-not (Test-DotNet8Sdk $RepoDotNetExe)) {
        throw @"
The repo-specific .NET installer completed, but a usable .NET 8 SDK was not found.

Expected executable:
  $RepoDotNetExe

Install directory:
  $RepoDotNetRoot

The incomplete SDK directory will be removed automatically on the next run.
"@
    }
}


function Get-DotNet8Sdk {
    $globalDotNet = Find-GlobalDotNet8Sdk

    if ($null -ne $globalDotNet) {
        Write-Host 'Using global Windows .NET 8 SDK:'
        Write-Host "  $globalDotNet"
        return $globalDotNet
    }

    if (Test-DotNet8Sdk $RepoDotNetExe) {
        Write-Host 'Reusing repo-specific Windows .NET 8 SDK:'
        Write-Host "  $RepoDotNetExe"
        return $RepoDotNetExe
    }

    Write-Host 'No global Windows .NET 8 SDK is available.'
    Write-Host 'Falling back to the repo-specific Windows toolchain cache:'
    Write-Host "  $WindowsToolchainRoot"

    Install-RepoLocalDotNet8Sdk

    Write-Host 'Using newly installed repo-specific Windows .NET 8 SDK:'
    Write-Host "  $RepoDotNetExe"
    return $RepoDotNetExe
}


function Ensure-VbaHost {
    New-Item -ItemType Directory -Path $VbaCacheRoot -Force | Out-Null

    $fingerprint = Get-SourceFingerprint @($BasPath)
    $cachedFingerprint = Read-CachedFingerprint $VbaHostFingerprintPath

    if (
        (Test-Path -LiteralPath $VbaHostPath) -and
        $cachedFingerprint -eq $fingerprint
    ) {
        Write-Host 'Reusing cached VBA host:'
        Write-Host "  $VbaHostPath"
        return
    }

    Write-Host 'Building cached VBA host workbook...'

    $localHost = Join-Path `
        $env:TEMP `
        ("drawing-bounds-host-" + [Guid]::NewGuid().ToString('N') + '.xlsm')

    $excel = $null
    $workbooks = $null
    $book = $null
    $component = $null

    try {
        $excel = New-Object -ComObject Excel.Application
        if(Get-Command Register-VVDecisionExcel -ErrorAction SilentlyContinue){Register-VVDecisionExcel $excel}
        $excel.Visible = $false
        $excel.DisplayAlerts = $false
        $excel.EnableEvents = $false

        $workbooks = $excel.Workbooks
        $book = $workbooks.Add()

        try {
            $null = $book.VBProject.VBComponents.Count
            $component = $book.VBProject.VBComponents.Import($BasPath)
        }
        catch {
            throw @"
Could not build the cached VBA host.

Excel must allow:
  Options -> Trust Center -> Trust Center Settings -> Macro Settings
  -> Trust access to the VBA project object model

This permission is only needed when the cached host must be created or rebuilt.

Original error:
$($_.Exception.Message)
"@
        }

        # xlOpenXMLWorkbookMacroEnabled = 52
        $book.SaveAs($localHost, 52)
        $book.Close($false)

        Release-ComObject $component
        $component = $null

        Release-ComObject $book
        $book = $null

        Copy-Item -LiteralPath $localHost -Destination $VbaHostPath -Force
        Write-Utf8NoBom $VbaHostFingerprintPath ($fingerprint + "`n")

        Write-Host 'Cached VBA host:'
        Write-Host "  $VbaHostPath"
    }
    finally {
        if ($null -ne $book) {
            try { $book.Close($false) } catch {}
        }

        Release-ComObject $component
        Release-ComObject $book
        Release-ComObject $workbooks

        if ($null -ne $excel) {
            try { $excel.Quit() } catch {}
        }

        Release-ComObject $excel

        Remove-Item -LiteralPath $localHost -Force -ErrorAction SilentlyContinue

        [GC]::Collect()
        [GC]::WaitForPendingFinalizers()
    }
}


function Ensure-CSharpAddIn {
    New-Item -ItemType Directory -Path $CSharpCacheRoot -Force | Out-Null

    # Resolve the .NET 8 runtime before checking the XLL cache.
    #
    # A cached net8.0 Excel-DNA add-in still needs DOTNET_ROOT when the SDK/runtime
    # lives only in the repo-specific LOCALAPPDATA toolchain cache. Previously this
    # initialization happened only on rebuilds, so a cache hit could leave Excel
    # unable to load/register the managed ExcelCommand.
    $dotnet = Get-DotNet8Sdk

    if ($dotnet -eq $RepoDotNetExe) {
        $env:DOTNET_ROOT = $RepoDotNetRoot
    }

    $fingerprint = Get-SourceFingerprint @(
        $CSharpProjectPath,
        $CSharpSourcePath
    )

    $cachedFingerprint = Read-CachedFingerprint $CSharpFingerprintPath

    if (
        (Test-Path -LiteralPath $CSharpXllPath) -and
        $cachedFingerprint -eq $fingerprint
    ) {
        Write-Host 'Reusing cached C# Excel-DNA add-in:'
        Write-Host "  $CSharpXllPath"
        return
    }

    Write-Host 'Building C# Excel-DNA add-in...'

    $buildRoot = Join-Path `
        $env:TEMP `
        ("drawing-bounds-csharp-" + [Guid]::NewGuid().ToString('N'))

    New-Item -ItemType Directory -Path $buildRoot -Force | Out-Null

    try {
        Copy-Item `
            -LiteralPath $CSharpProjectPath `
            -Destination (Join-Path $buildRoot 'DrawingBoundsAddIn.csproj')

        Copy-Item `
            -LiteralPath $CSharpSourcePath `
            -Destination (Join-Path $buildRoot 'DrawingBoundsCommands.cs')

        & $dotnet `
            build `
            (Join-Path $buildRoot 'DrawingBoundsAddIn.csproj') `
            -c Release `
            --nologo

        if ($LASTEXITCODE -ne 0) {
            throw "C# Excel-DNA build failed with exit code $LASTEXITCODE"
        }

        $builtXll = Get-ChildItem `
            -LiteralPath $buildRoot `
            -Recurse `
            -File `
            -Filter '*-AddIn64-packed.xll' |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1

        if ($null -eq $builtXll) {
            throw "C# build completed but no *-AddIn64-packed.xll was found below: $buildRoot"
        }

        Copy-Item `
            -LiteralPath $builtXll.FullName `
            -Destination $CSharpXllPath `
            -Force

        Write-Utf8NoBom $CSharpFingerprintPath ($fingerprint + "`n")

        Write-Host 'Cached C# Excel-DNA add-in:'
        Write-Host "  $CSharpXllPath"
    }
    finally {
        Remove-Item `
            -LiteralPath $buildRoot `
            -Recurse `
            -Force `
            -ErrorAction SilentlyContinue
    }
}


$RepoRoot = Resolve-ExistingPath $RepoRoot 'Repository root'
$CorpusRoot = Resolve-ExistingPath $CorpusRoot 'Corpus root'
$BasPath = Resolve-ExistingPath $BasPath 'BAS module'
$PowerShellComScript = Resolve-ExistingPath $PowerShellComScript 'PowerShell COM helper'
$RustCargoTomlPath = Resolve-ExistingPath $RustCargoTomlPath 'Rust Cargo.toml'
$RustCargoLockPath = Resolve-ExistingPath $RustCargoLockPath 'Rust Cargo.lock'
$RustSourcePath = Resolve-ExistingPath $RustSourcePath 'Rust source'
Assert-RustHelperFresh
$RustExe = Resolve-ExistingPath $RustExe 'Rust COM executable'
$CSharpProjectPath = Resolve-ExistingPath $CSharpProjectPath 'C# project'
$CSharpSourcePath = Resolve-ExistingPath $CSharpSourcePath 'C# source'

Write-Host 'Repository:'
Write-Host "  $RepoRoot"

Write-Host 'Corpus:'
Write-Host "  $CorpusRoot"

Ensure-VbaHost
Ensure-CSharpAddIn

$results = Join-Path $env:VV_REPORT_DIR 'data\shape-bounds'

New-Item -ItemType Directory -Path $results -Force | Out-Null

foreach ($stale in @(
    'vsto.json',
    'vsto-status.json',
    'office-addin.json',
    'office-addin-status.json',
    'geometry-validation.json',
    'timing-summary.json',
    'timing-summary.csv'
)) {
    Remove-Item `
        -LiteralPath (Join-Path $results $stale) `
        -Force `
        -ErrorAction SilentlyContinue
}

foreach ($staleDirectory in @(
    'compare',
    'geometry-mismatches'
)) {
    Remove-Item `
        -LiteralPath (Join-Path $results $staleDirectory) `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue
}

$fileListPath = Join-Path $results 'files.txt'

$vbaOutputPath = Join-Path $results 'vba.json'
$csharpOutputPath = Join-Path $results 'csharp-addin.json'
$powershellOutputPath = Join-Path $results 'powershell-com.json'
$rustOutputPath = Join-Path $results 'rust-com.json'

$files = @(
    Get-ChildItem `
        -LiteralPath $CorpusRoot `
        -Recurse `
        -File `
        -Filter '*.xlsx'
)

if ($files.Count -eq 0) {
    throw "No .xlsx files found below: $CorpusRoot"
}

$fileListText = (
    ($files | ForEach-Object { $_.FullName }) -join "`n"
) + "`n"

Write-Utf8NoBom $fileListPath $fileListText

Write-Host
Write-Host "Prepared $($files.Count) workbook paths without sorting:"
Write-Host "  $fileListPath"

$tempRoot = Join-Path `
    $env:TEMP `
    ("drawing-bounds-experiment-" + [Guid]::NewGuid().ToString('N'))

New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

$stagedList = Join-Path $tempRoot 'files.txt'

$stagedVbaHost = Join-Path $tempRoot 'drawing-bounds-host.xlsm'
$stagedVbaOutput = Join-Path $results 'vba.json'

$stagedCSharpXll = Join-Path $tempRoot 'DrawingBoundsAddIn-AddIn64-packed.xll'
$stagedCSharpOutput = Join-Path $results 'csharp-addin.json'

$stagedPowerShellOutput = Join-Path $results 'powershell-com.json'

$stagedRustExe = Join-Path $tempRoot 'drawing-bounds-rust-com.exe'
$stagedRustOutput = Join-Path $results 'rust-com.json'

Copy-Item -LiteralPath $fileListPath -Destination $stagedList
Copy-Item -LiteralPath $VbaHostPath -Destination $stagedVbaHost
Copy-Item -LiteralPath $CSharpXllPath -Destination $stagedCSharpXll
Copy-Item -LiteralPath $RustExe -Destination $stagedRustExe


try {
    Write-Host
    Write-Host '=== VBA inside Excel ==='
    Write-VVEvent 'native_implementation_started' @{implementation='vba'}

    $excel = $null
    $workbooks = $null
    $hostBook = $null

    try {
        $excel = New-Object -ComObject Excel.Application
        if(Get-Command Register-VVDecisionExcel -ErrorAction SilentlyContinue){Register-VVDecisionExcel $excel}
        $excel.Visible = $false
        $excel.DisplayAlerts = $false
        $excel.EnableEvents = $false

        Write-Host ("Excel version: " + $excel.Version)

        $workbooks = $excel.Workbooks
        $hostBook = $workbooks.Open($stagedVbaHost, 0, $true)

        $macro = "'$($hostBook.Name)'!DrawingBounds.RunDrawingBoundsBatch"

        $null = $excel.Run(
            $macro,
            $stagedList,
            $stagedVbaOutput
        )

        if (-not (Test-Path -LiteralPath $stagedVbaOutput)) {
            throw "VBA completed without producing $stagedVbaOutput"
        }

        Copy-Utf8WithoutBom $stagedVbaOutput $vbaOutputPath

        Write-Host 'VBA result:'
        Write-Host "  $vbaOutputPath"
    }
    finally {
        if ($null -ne $hostBook) {
            try { $hostBook.Close($false) } catch {}
        }

        Release-ComObject $hostBook
        Release-ComObject $workbooks

        if ($null -ne $excel) {
            try { $excel.Quit() } catch {}
        }

        Release-ComObject $excel

        [GC]::Collect()
        [GC]::WaitForPendingFinalizers()
    }


    Write-Host
    Write-Host '=== C# Excel-DNA inside Excel ==='
    Write-VVEvent 'native_implementation_started' @{implementation='excel-dna'}

    $excel = $null

    try {
        $excel = New-Object -ComObject Excel.Application
        if(Get-Command Register-VVDecisionExcel -ErrorAction SilentlyContinue){Register-VVDecisionExcel $excel}
        $excel.Visible = $false
        $excel.DisplayAlerts = $false
        $excel.EnableEvents = $false

        Write-Host ("Excel version: " + $excel.Version)

        $registered = $excel.RegisterXLL($stagedCSharpXll)

        if (-not $registered) {
            throw "Excel refused to register the C# Excel-DNA add-in: $stagedCSharpXll"
        }

        $runResult = $excel.Run(
            'DrawingBoundsCSharp_RunDrawingBoundsBatch',
            $stagedList,
            $stagedCSharpOutput
        )

        if ("$runResult" -ne 'OK') {
            throw "C# Excel-DNA experiment failed: $runResult"
        }

        if (-not (Test-Path -LiteralPath $stagedCSharpOutput)) {
            throw "C# Excel-DNA completed without producing $stagedCSharpOutput"
        }

        Copy-Utf8WithoutBom $stagedCSharpOutput $csharpOutputPath

        Write-Host 'C# Excel-DNA result:'
        Write-Host "  $csharpOutputPath"
    }
    finally {
        if ($null -ne $excel) {
            try { $excel.Quit() } catch {}
        }

        Release-ComObject $excel

        [GC]::Collect()
        [GC]::WaitForPendingFinalizers()
    }


    Write-Host
    Write-Host '=== External PowerShell -> Excel COM ==='

    & $PowerShellComScript `
        -FileListPath $stagedList `
        -OutputPath $stagedPowerShellOutput

    if (-not (Test-Path -LiteralPath $stagedPowerShellOutput)) {
        throw "PowerShell COM completed without producing $stagedPowerShellOutput"
    }

    Copy-Utf8WithoutBom $stagedPowerShellOutput $powershellOutputPath

    Write-Host 'PowerShell COM result:'
    Write-Host "  $powershellOutputPath"


    Write-Host
    Write-Host '=== External Rust -> Excel COM ==='

    & $stagedRustExe $stagedList $stagedRustOutput

    if ($LASTEXITCODE -ne 0) {
        throw "Rust COM experiment failed with exit code $LASTEXITCODE"
    }

    if (-not (Test-Path -LiteralPath $stagedRustOutput)) {
        throw "Rust COM experiment completed without producing $stagedRustOutput"
    }

    Copy-Utf8WithoutBom $stagedRustOutput $rustOutputPath

    Write-Host 'Rust COM result:'
    Write-Host "  $rustOutputPath"


    Write-Host
    Write-Host 'Done.'
    Write-Host
    Write-Host 'Full timing results:'
    Write-Host "  VBA:             $vbaOutputPath"
    Write-Host "  C# Excel-DNA:    $csharpOutputPath"
    Write-Host "  PowerShell COM:  $powershellOutputPath"
    Write-Host "  Rust COM:        $rustOutputPath"

    if (-not $DeferValidation) {
        & (Join-Path $PSScriptRoot 'summarize-results.ps1') `
            -ResultsDir $results `
            -BaseOnly `
            -FailOnMismatch
    }
}
finally {
    foreach($partial in @(Get-ChildItem -LiteralPath $tempRoot -File -ErrorAction SilentlyContinue | Where-Object {$_.Extension -in @('.json','.jsonl','.log','.txt')})) {
        Copy-Item -LiteralPath $partial.FullName -Destination $results -Force -ErrorAction Continue
    }
    Remove-Item `
        -LiteralPath $tempRoot `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue
}

} catch {
    $vvCaughtError = $_
    try { Write-VVFailure $vvCaughtError $PSCommandPath } catch { Write-Warning ('Could not persist error: ' + $_.Exception.Message) }
    throw $vvCaughtError
} finally {
    Stop-VVReport $vvReportOwner
}
