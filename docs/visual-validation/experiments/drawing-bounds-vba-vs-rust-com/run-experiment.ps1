param(
    [Parameter(Mandatory=$true)]
    [string]$CorpusRoot,

    [string]$BasPath,

    [string]$RustExe
)

$ErrorActionPreference = 'Stop'

if (-not $BasPath) {
    $BasPath = Join-Path $PSScriptRoot 'drawing-bounds.bas'
}

if (-not $RustExe) {
    $RustExe = Join-Path $PSScriptRoot 'rust-com\target\x86_64-pc-windows-gnu\release\drawing-bounds-rust-com.exe'
}

function Resolve-ExistingPath([string]$Path, [string]$Label) {
    try {
        return (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
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
    if ($bytes.Length -ge 3 -and
        $bytes[0] -eq 0xEF -and
        $bytes[1] -eq 0xBB -and
        $bytes[2] -eq 0xBF) {
        $withoutBom = New-Object byte[] ($bytes.Length - 3)
        [Array]::Copy($bytes, 3, $withoutBom, 0, $withoutBom.Length)
        [IO.File]::WriteAllBytes($Destination, $withoutBom)
    }
    else {
        [IO.File]::WriteAllBytes($Destination, $bytes)
    }
}

$CorpusRoot = Resolve-ExistingPath $CorpusRoot 'Corpus root'
$BasPath = Resolve-ExistingPath $BasPath 'BAS module'
$RustExe = Resolve-ExistingPath $RustExe 'Rust COM executable'

$results = Join-Path $PSScriptRoot 'results'
New-Item -ItemType Directory -Path $results -Force | Out-Null

$fileListPath = Join-Path $results 'files.txt'
$vbaOutputPath = Join-Path $results 'vba.json'
$rustOutputPath = Join-Path $results 'rust-com.json'

# Deliberately no Sort-Object: materialize one provider traversal and reuse this
# exact order for both variants.
$files = @(
    Get-ChildItem -LiteralPath $CorpusRoot -Recurse -File -Filter '*.xlsx'
)

if ($files.Count -eq 0) {
    throw "No .xlsx files found below: $CorpusRoot"
}

$fileListText = (($files | ForEach-Object { $_.FullName }) -join "`n") + "`n"
Write-Utf8NoBom $fileListPath $fileListText
Write-Host "Prepared $($files.Count) workbook paths without sorting:"
Write-Host "  $fileListPath"

$tempRoot = Join-Path $env:TEMP ("drawing-bounds-experiment-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

$stagedBas = Join-Path $tempRoot 'drawing-bounds.bas'
$stagedList = Join-Path $tempRoot 'files.txt'
$stagedVbaOutput = Join-Path $tempRoot 'vba.json'
$stagedRustOutput = Join-Path $tempRoot 'rust-com.json'
$stagedRustExe = Join-Path $tempRoot 'drawing-bounds-rust-com.exe'

Copy-Item -LiteralPath $BasPath -Destination $stagedBas
Copy-Item -LiteralPath $fileListPath -Destination $stagedList
Copy-Item -LiteralPath $RustExe -Destination $stagedRustExe

$excel = $null
$hostBook = $null
$component = $null

try {
    Write-Host
    Write-Host '=== VBA batch inside Excel ==='

    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $excel.EnableEvents = $false

    Write-Host ("Excel version: " + $excel.Version)
    Write-Host 'Creating temporary macro host...'

    $hostBook = $excel.Workbooks.Add()

    try {
        $null = $hostBook.VBProject.VBComponents.Count
        $component = $hostBook.VBProject.VBComponents.Import($stagedBas)
    }
    catch {
        throw @"
Could not import the BAS module into Excel.

For this controlled experiment Excel must allow:
  Options -> Trust Center -> Trust Center Settings -> Macro Settings
  -> Trust access to the VBA project object model

Original error:
$($_.Exception.Message)
"@
    }

    $macro = "'$($hostBook.Name)'!DrawingBounds.RunDrawingBoundsBatch"
    $null = $excel.Run($macro, $stagedList, $stagedVbaOutput)

    if (-not (Test-Path -LiteralPath $stagedVbaOutput)) {
        throw "VBA completed without producing $stagedVbaOutput"
    }

    Copy-Utf8WithoutBom $stagedVbaOutput $vbaOutputPath
    Write-Host "VBA result:"
    Write-Host "  $vbaOutputPath"
}
finally {
    if ($null -ne $hostBook) {
        try { $hostBook.Close($false) } catch { }
    }
    if ($null -ne $excel) {
        try { $excel.Quit() } catch { }
    }

    if ($null -ne $component) {
        try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($component) } catch { }
    }
    if ($null -ne $hostBook) {
        try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($hostBook) } catch { }
    }
    if ($null -ne $excel) {
        try { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel) } catch { }
    }

    $component = $null
    $hostBook = $null
    $excel = $null
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}

try {
    Write-Host
    Write-Host '=== External Rust -> Excel COM ==='

    & $stagedRustExe $stagedList $stagedRustOutput
    if ($LASTEXITCODE -ne 0) {
        throw "Rust COM experiment failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path -LiteralPath $stagedRustOutput)) {
        throw "Rust COM experiment completed without producing $stagedRustOutput"
    }

    Copy-Item -LiteralPath $stagedRustOutput -Destination $rustOutputPath -Force

    Write-Host
    Write-Host 'Done.'
    Write-Host "VBA:"
    Write-Host "  $vbaOutputPath"
    Write-Host "Rust COM:"
    Write-Host "  $rustOutputPath"
    Write-Host
    Write-Host 'Compare in VS Code:'
    Write-Host "  code --diff `"$vbaOutputPath`" `"$rustOutputPath`""
}
finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
