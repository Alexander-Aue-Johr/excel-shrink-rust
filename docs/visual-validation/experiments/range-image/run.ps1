param(
    [string]$CorpusRoot,

    [ValidateSet('A', 'B', 'C', 'D', 'E', 'all')]
    [string]$Strategy,

    [ValidateSet('vba', 'officejs-rust', 'officejs-dotnet', 'all')]
    [string]$Implementation,

    [ValidateRange(1, 64)]
    [int]$BatchSize = 8,

    [ValidateRange(64, 8192)]
    [int]$OverflowProbeWidthPx = 1920,

    [ValidateRange(64, 8192)]
    [int]$OverflowProbeHeightPx = 1080,

    [ValidateSet('excel-x', 'rust-com')]
    [string]$RustExcelLaunchMode = 'rust-com',

    [switch]$NonInteractive
)

$ErrorActionPreference = 'Stop'

$ExperimentsRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $ExperimentsRoot '..\..\..'))
$SharedOfficeRunner = Join-Path $ExperimentsRoot '_shared\office-addin\run.ps1'
$VbaRunner = Join-Path $PSScriptRoot 'vba\run.ps1'
$ResultsRoot = Join-Path $PSScriptRoot 'results'
$FileList = Join-Path $ResultsRoot 'files.txt'
$ImageRoot = Join-Path $RepoRoot '.tmp\visual-validation\range-image\images'

function Choose(
    [string]$Title,
    [string[]]$Options,
    [int]$DefaultIndex = 0
) {
    Write-Host ''
    Write-Host $Title
    for ($index = 0; $index -lt $Options.Count; $index++) {
        $marker = if ($index -eq $DefaultIndex) { '*' } else { ' ' }
        Write-Host ("  [{0}] {1} {2}" -f ($index + 1), $marker, $Options[$index])
    }

    while ($true) {
        $raw = Read-Host ("Choose 1-$($Options.Count) [default $($DefaultIndex + 1)]")
        if ([string]::IsNullOrWhiteSpace($raw)) {
            return $Options[$DefaultIndex]
        }
        $value = 0
        if ([int]::TryParse($raw, [ref]$value)) {
            if ($value -ge 1 -and $value -le $Options.Count) {
                return $Options[$value - 1]
            }
        }
    }
}

if ([string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $CorpusRoot = Join-Path $RepoRoot 'corpus-local'
}
else {
    $CorpusRoot = [IO.Path]::GetFullPath($CorpusRoot)
}

if (-not (Test-Path -LiteralPath $CorpusRoot)) {
    throw "Corpus directory does not exist: $CorpusRoot"
}

if ([string]::IsNullOrWhiteSpace($Strategy)) {
    if ($NonInteractive) { $Strategy = 'A' }
    else {
        $choice = Choose 'Bounds strategy' @(
            'A · UsedRange prefetch',
            'B · UsedRange immediately before each picture batch',
            'C · content + drawings + 10 row/column margin',
            'D · C bounds + measured horizontal overflow',
            'E · D + measured vertical overflow',
            'Run all strategies'
        ) 0
        $Strategy = switch -Regex ($choice) {
            '^A ' { 'A' }
            '^B ' { 'B' }
            '^C ' { 'C' }
            '^D ' { 'D' }
            '^E ' { 'E' }
            default { 'all' }
        }
    }
}

if ([string]::IsNullOrWhiteSpace($Implementation)) {
    if ($NonInteractive) { $Implementation = 'officejs-rust' }
    else {
        $choice = Choose 'Implementation' @(
            'Office.js · Rust host',
            'Office.js · ASP.NET host',
            'VBA CopyPicture baseline',
            'Run all implementations'
        ) 0
        $Implementation = switch -Regex ($choice) {
            'Rust host' { 'officejs-rust' }
            'ASP.NET' { 'officejs-dotnet' }
            '^VBA' { 'vba' }
            default { 'all' }
        }
    }
}

if (
    -not $NonInteractive -and
    ($Implementation -like 'officejs-*' -or $Implementation -eq 'all')
) {
    $batchChoice = Choose 'Office.js batch size' @(
        '1', '2', '4', '8', '16', '32', '64'
    ) 3
    $BatchSize = [int]$batchChoice
}

New-Item -ItemType Directory -Path $ResultsRoot -Force | Out-Null

$files = @(
    Get-ChildItem `
        -LiteralPath $CorpusRoot `
        -Recurse `
        -File `
        -Filter '*.xlsx' |
    Where-Object { -not $_.Name.StartsWith('~$') } |
    Sort-Object FullName
)

if ($files.Count -eq 0) {
    throw "No .xlsx files found below corpus: $CorpusRoot"
}

$utf8 = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText(
    $FileList,
    ((($files | ForEach-Object { $_.FullName }) -join "`n") + "`n"),
    $utf8
)

Write-Host "Prepared $($files.Count) workbook paths:"
Write-Host "  $FileList"

$strategies = if ($Strategy -eq 'all') {
    @('A', 'B', 'C', 'D', 'E')
}
else {
    @($Strategy)
}

$implementations = if ($Implementation -eq 'all') {
    @('vba', 'officejs-rust', 'officejs-dotnet')
}
else {
    @($Implementation)
}

foreach ($currentStrategy in $strategies) {
    foreach ($currentImplementation in $implementations) {
        Write-Host ''
        Write-Host ('=' * 76)
        Write-Host (
            "Get picture · strategy $currentStrategy · " +
            "$currentImplementation · batch $BatchSize"
        )
        Write-Host ('=' * 76)

        if ($currentImplementation -eq 'vba') {
            if ($currentStrategy -ne 'A') {
                Write-Warning (
                    "VBA currently serves as the CopyPicture baseline for strategy A. " +
                    "Strategies B-E are Office.js planning experiments; skipping VBA " +
                    "for strategy $currentStrategy."
                )
                continue
            }

            $output = Join-Path $ResultsRoot 'a-used-range-prefetch\vba.json'
            $images = Join-Path $ImageRoot 'a\vba'
            & $VbaRunner `
                -CorpusRoot $CorpusRoot `
                -OutputPath $output `
                -ImageOutputDir $images
            continue
        }

        $hostKind = if ($currentImplementation -eq 'officejs-rust') {
            'rust'
        } else {
            'dotnet'
        }

        $strategyDirectory = switch ($currentStrategy) {
            'A' { 'a-used-range-prefetch' }
            'B' { 'b-used-range-inline' }
            'C' { 'c-content-shapes-margin' }
            'D' { 'd-horizontal-overflow-xor' }
            'E' { 'e-horizontal-vertical-overflow-xor' }
        }

        $hostLabel = if ($hostKind -eq 'rust') { 'rust' } else { 'dotnet' }
        $output = Join-Path `
            $ResultsRoot `
            ($strategyDirectory + "\officejs-$hostLabel-batch$BatchSize.json")
        $images = Join-Path `
            $ImageRoot `
            ($currentStrategy.ToLowerInvariant() + "\officejs-$hostLabel-batch$BatchSize")

        $args = @{
            FileListPath = $FileList
            OutputPath = $output
            ImageOutputDir = $images
            Experiment = 'range-image'
            Strategy = $currentStrategy
            HostKind = $hostKind
            BatchSize = $BatchSize
            OverflowProbeWidthPx = $OverflowProbeWidthPx
            OverflowProbeHeightPx = $OverflowProbeHeightPx
        }

        if ($hostKind -eq 'rust') {
            $args.ExcelLaunchMode = $RustExcelLaunchMode
        }
        else {
            $args.ExcelLaunchMode = 'excel-x'
        }

        & $SharedOfficeRunner @args
    }
}
