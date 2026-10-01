param(
    [string]$CorpusRoot,

    [ValidateSet(
        'run-all',
        'vba',
        'excel-dna',
        'vsto',
        'powershell-com',
        'rust-com',
        'officejs-rust',
        'officejs-dotnet'
    )]
    [string]$Implementation,

    [ValidateSet('excel-x', 'rust-com')]
    [string]$RustExcelLaunchMode = 'rust-com',

    [switch]$NonInteractive
)

$ErrorActionPreference = 'Stop'

$ExperimentsRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $ExperimentsRoot '..\..\..'))
$SharedOfficeRunner = Join-Path $ExperimentsRoot '_shared\office-addin\run.ps1'
$Results = Join-Path $PSScriptRoot 'results'
$FileList = Join-Path $Results 'files.txt'

function Choose([string]$Title, [string[]]$Options, [int]$DefaultIndex = 0) {
    Write-Host ''
    Write-Host $Title
    for ($i = 0; $i -lt $Options.Count; $i++) {
        $marker = if ($i -eq $DefaultIndex) { '*' } else { ' ' }
        Write-Host ("  [{0}] {1} {2}" -f ($i + 1), $marker, $Options[$i])
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

if ([string]::IsNullOrWhiteSpace($Implementation)) {
    if ($NonInteractive) {
        $Implementation = 'run-all'
    }
    else {
        $choice = Choose 'Get shape bounds implementation' @(
            'Run all',
            'VBA',
            'Excel-DNA',
            'VSTO',
            'PowerShell COM',
            'Rust COM',
            'Office.js · Rust host',
            'Office.js · ASP.NET host'
        ) 0

        $Implementation = switch ($choice) {
            'Run all' { 'run-all' }
            'VBA' { 'vba' }
            'Excel-DNA' { 'excel-dna' }
            'VSTO' { 'vsto' }
            'PowerShell COM' { 'powershell-com' }
            'Rust COM' { 'rust-com' }
            'Office.js · Rust host' { 'officejs-rust' }
            'Office.js · ASP.NET host' { 'officejs-dotnet' }
        }
    }
}

New-Item -ItemType Directory -Path $Results -Force | Out-Null

# The existing base runner remains the smallest source of truth for the established
# VBA / Excel-DNA / PowerShell COM / Rust COM comparison.
if ($Implementation -in @('run-all', 'vba', 'excel-dna', 'powershell-com', 'rust-com')) {
    if ($Implementation -eq 'run-all') {
        & (Join-Path $PSScriptRoot 'run-experiment.ps1') `
            -CorpusRoot $CorpusRoot `
            -DeferValidation
    }
    else {
        Write-Warning (
            "The established shape-bounds runner currently executes its four base " +
            "implementations together. Running the base set so geometry validation " +
            "stays identical; inspect the requested variant in results/."
        )
        & (Join-Path $PSScriptRoot 'run-experiment.ps1') `
            -CorpusRoot $CorpusRoot `
            -DeferValidation
    }
}

if ($Implementation -in @('run-all', 'vsto')) {
    & (Join-Path $PSScriptRoot 'vsto-addin\run-vsto.ps1') `
        -FileListPath $FileList `
        -OutputPath (Join-Path $Results 'vsto.json') `
        -SkipIfUnavailable
}

foreach ($office in @(
    [pscustomobject]@{ Name='officejs-rust'; Host='rust'; File='officejs-rust.json' },
    [pscustomobject]@{ Name='officejs-dotnet'; Host='dotnet'; File='officejs-dotnet.json' }
)) {
    if ($Implementation -notin @('run-all', $office.Name)) {
        continue
    }

    if (-not (Test-Path -LiteralPath $FileList)) {
        $files = @(
            Get-ChildItem `
                -LiteralPath $CorpusRoot `
                -Recurse `
                -File `
                -Filter '*.xlsx' |
            Where-Object { -not $_.Name.StartsWith('~$') } |
            Sort-Object FullName
        )
        $utf8 = New-Object Text.UTF8Encoding($false)
        [IO.File]::WriteAllText(
            $FileList,
            ((($files | ForEach-Object { $_.FullName }) -join "`n") + "`n"),
            $utf8
        )
    }

    $imageRoot = Join-Path `
        $RepoRoot `
        ('.tmp\visual-validation\shape-bounds\' + $office.Name)

    $args = @{
        FileListPath = $FileList
        OutputPath = (Join-Path $Results $office.File)
        ImageOutputDir = $imageRoot
        Experiment = 'shape-bounds'
        Strategy = 'A'
        HostKind = $office.Host
        BatchSize = 8
    }

    if ($office.Host -eq 'rust') {
        $args.ExcelLaunchMode = $RustExcelLaunchMode
    }
    else {
        $args.ExcelLaunchMode = 'excel-x'
    }

    & $SharedOfficeRunner @args
}

if (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'summarize-results.ps1')) {
    try {
        & (Join-Path $PSScriptRoot 'summarize-results.ps1') `
            -ResultsDir $Results
    }
    catch {
        Write-Warning (
            'Summary did not accept the expanded Office.js result set yet: ' +
            $_.Exception.Message
        )
    }
}
