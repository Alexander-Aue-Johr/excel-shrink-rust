param(
    [string]$CorpusRoot,

    [ValidateSet('A', 'B', 'C', 'D', 'E', 'F', 'all')]
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

    [string]$Workbook,

    [switch]$AutoOpenDevTools,

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

function Convert-WslUncToLinuxPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if ($full -match '^\\\\wsl(?:\.localhost|\$)?\\([^\\]+)\\(.*)$') {
        return [pscustomobject]@{
            Distribution = $Matches[1]
            LinuxPath = '/' + ($Matches[2] -replace '\\', '/')
        }
    }
    return $null
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
            'A - UsedRange prefetch',
            'B - UsedRange immediately before each picture batch',
            'C - content + drawings + 10 row/column margin',
            'D - C bounds + measured horizontal overflow',
            'E - D + measured vertical overflow',
            'F - WSL Rust OOXML bounds + horizontal render overflow',
            'Run all strategies'
        ) 0
        $Strategy = switch -Regex ($choice) {
            '^A ' { 'A' }
            '^B ' { 'B' }
            '^C ' { 'C' }
            '^D ' { 'D' }
            '^E ' { 'E' }
            '^F ' { 'F' }
            default { 'all' }
        }
    }
}

if ([string]::IsNullOrWhiteSpace($Implementation)) {
    if ($NonInteractive) { $Implementation = 'officejs-rust' }
    else {
        $choice = Choose 'Implementation' @(
            'Office.js - Rust host',
            'Office.js - ASP.NET host',
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

# Source Context patch: Office.js DevTools CLI option.
if (
    -not $NonInteractive -and
    ($Implementation -like 'officejs-*' -or $Implementation -eq 'all') -and
    -not $PSBoundParameters.ContainsKey('AutoOpenDevTools')
) {
    $devToolsChoice = Choose 'Office.js DevTools' @(
        'Off',
        'Auto-open DevTools'
    ) 0

    if ($devToolsChoice -eq 'Auto-open DevTools') {
        $AutoOpenDevTools = $true
    }
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

# Workbook selection: default to the complete corpus. The interactive VS Code
# terminal task reaches this runner through experiments/run.ps1, so no extra
# tasks.json input is needed.
$allFiles = @($files)

if (-not [string]::IsNullOrWhiteSpace($Workbook) -and $Workbook -notin @('all', '*')) {
    $matches = @(
        $allFiles | Where-Object {
            $_.Name -ieq $Workbook -or
            $_.FullName -ieq $Workbook
        }
    )

    if ($matches.Count -eq 0) {
        throw "Requested workbook was not found below the corpus: $Workbook"
    }
    if ($matches.Count -gt 1) {
        throw "Workbook name is ambiguous; pass the full path instead: $Workbook"
    }

    $files = @($matches[0])
}
elseif (-not $NonInteractive) {
    $workbookOptions = @('All workbooks') + @(
        $allFiles | ForEach-Object { $_.Name }
    )
    $workbookChoice = Choose 'Workbook' $workbookOptions 0

    if ($workbookChoice -ne 'All workbooks') {
        $matches = @($allFiles | Where-Object { $_.Name -ieq $workbookChoice })
        if ($matches.Count -ne 1) {
            throw "Workbook selection is ambiguous: $workbookChoice"
        }
        $files = @($matches[0])
    }
}

$selectionSuffix = ''
if ($files.Count -eq 1 -and $allFiles.Count -gt 1) {
    $baseName = [IO.Path]::GetFileNameWithoutExtension($files[0].Name)
    $safeName = $baseName -replace '[^A-Za-z0-9._-]', '_'
    if ($safeName.Length -gt 80) {
        $safeName = $safeName.Substring(0, 80)
    }
    $selectionSuffix = '-workbook-' + $safeName
}

$FileList = Join-Path $ResultsRoot ("files$selectionSuffix.txt")
$utf8 = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText(
    $FileList,
    ((($files | ForEach-Object { $_.FullName }) -join "`n") + "`n"),
    $utf8
)

Write-Host "Prepared $($files.Count) workbook path(s):"
foreach ($file in $files) {
    Write-Host "  $($file.Name)"
}
Write-Host "File list: $FileList"

$strategies = if ($Strategy -eq 'all') {
    @('A', 'B', 'C', 'D', 'E', 'F')
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

$FVisualPlanPath = $null
$needsFOfficeJs = (
    $strategies -contains 'F' -and
    @($implementations | Where-Object { $_ -like 'officejs-*' }).Count -gt 0
)

if ($needsFOfficeJs) {
    $repoWsl = Convert-WslUncToLinuxPath $RepoRoot
    $fileListWsl = Convert-WslUncToLinuxPath $FileList
    $prepareScript = Join-Path $PSScriptRoot 'prepare-visual-plan.sh'
    $prepareScriptWsl = Convert-WslUncToLinuxPath $prepareScript

    if ($null -eq $repoWsl -or $null -eq $fileListWsl -or $null -eq $prepareScriptWsl) {
        throw (
            'Strategy F requires the repository and selected file list to live inside ' +
            'WSL so the native Rust OOXML preplanner can run before Excel starts.'
        )
    }

    if (
        $repoWsl.Distribution -ne $fileListWsl.Distribution -or
        $repoWsl.Distribution -ne $prepareScriptWsl.Distribution
    ) {
        throw 'Strategy F paths resolve to different WSL distributions.'
    }

    $planRoot = Join-Path $RepoRoot '.tmp\visual-validation\range-image\plans'
    New-Item -ItemType Directory -Path $planRoot -Force | Out-Null
    $FVisualPlanPath = Join-Path $planRoot ("strategy-f$selectionSuffix.json")
    $planWsl = Convert-WslUncToLinuxPath $FVisualPlanPath
    if ($null -eq $planWsl -or $planWsl.Distribution -ne $repoWsl.Distribution) {
        throw 'Could not convert the Strategy F visual-plan output path to WSL.'
    }

    Write-Host ''
    Write-Host 'Preparing Strategy F OOXML metadata with native Rust in WSL...'
    $planStarted = [Diagnostics.Stopwatch]::StartNew()
    & wsl.exe `
        -d $repoWsl.Distribution `
        -- bash `
        $prepareScriptWsl.LinuxPath `
        $fileListWsl.LinuxPath `
        $planWsl.LinuxPath

    if ($LASTEXITCODE -ne 0) {
        throw "Strategy F WSL Rust preplanner failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path -LiteralPath $FVisualPlanPath -PathType Leaf)) {
        throw "Strategy F preplanner did not produce: $FVisualPlanPath"
    }

    Write-Host (
        'Strategy F OOXML plan prepared before host startup in ' +
        ('{0:0.00}' -f $planStarted.Elapsed.TotalMilliseconds) +
        " ms: $FVisualPlanPath"
    )
}

foreach ($currentStrategy in $strategies) {
    foreach ($currentImplementation in $implementations) {
        Write-Host ''
        Write-Host ('=' * 76)
        Write-Host (
            "Get picture - strategy $currentStrategy - " +
            "$currentImplementation - batch $BatchSize"
        )
        Write-Host ('=' * 76)

        if ($currentImplementation -eq 'vba') {
            if ($currentStrategy -ne 'A') {
                Write-Warning (
                    "VBA currently serves as the CopyPicture baseline for strategy A. " +
                    "Strategies B-F are Office.js planning experiments; skipping VBA " +
                    "for strategy $currentStrategy."
                )
                continue
            }

            $output = Join-Path `
                $ResultsRoot `
                ("a-used-range-prefetch\vba$selectionSuffix.json")
            $images = Join-Path $ImageRoot ("a\vba$selectionSuffix")
            $vbaArgs = @{
                CorpusRoot = $CorpusRoot
                OutputPath = $output
                ImageOutputDir = $images
            }
            if ($files.Count -lt $allFiles.Count) {
                $vbaArgs.OnlyWorkbookPath = $files[0].FullName
            }
            & $VbaRunner @vbaArgs
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
            'F' { 'f-rust-xml-horizontal-overflow' }
        }

        $hostLabel = if ($hostKind -eq 'rust') { 'rust' } else { 'dotnet' }
        $output = Join-Path `
            $ResultsRoot `
            ($strategyDirectory + "\officejs-$hostLabel-batch$BatchSize$selectionSuffix.json")
        $images = Join-Path `
            $ImageRoot `
            ($currentStrategy.ToLowerInvariant() + "\officejs-$hostLabel-batch$BatchSize$selectionSuffix")

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

        if ($AutoOpenDevTools) {
            $args.AutoOpenDevTools = $true
        }

        if ($hostKind -eq 'rust') {
            $args.ExcelLaunchMode = $RustExcelLaunchMode
        }
        else {
            $args.ExcelLaunchMode = 'excel-x'
        }

        if ($currentStrategy -eq 'F') {
            $args.VisualPlanPath = $FVisualPlanPath
        }

        & $SharedOfficeRunner @args
    }
}
