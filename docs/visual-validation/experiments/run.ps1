param(
    [ValidateSet('range-image', 'shape-bounds')]
    [string]$Experiment,

    [string]$CorpusRoot,

    [string]$Workbook,

    [switch]$NonInteractive
)

$ErrorActionPreference = 'Stop'

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

if ([string]::IsNullOrWhiteSpace($Experiment)) {
    if ($NonInteractive) {
        $Experiment = 'range-image'
    }
    else {
        $choice = Choose 'Visual validation experiment' @(
            'Get picture',
            'Get shape bounds'
        ) 0

        $Experiment = if ($choice -eq 'Get picture') {
            'range-image'
        }
        else {
            'shape-bounds'
        }
    }
}

$runner = Join-Path $PSScriptRoot ($Experiment + '\run.ps1')
$args = @{}

if (-not [string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $args.CorpusRoot = $CorpusRoot
}

if (-not [string]::IsNullOrWhiteSpace($Workbook)) {
    if ($Experiment -ne 'range-image') {
        throw '-Workbook is only valid for the Get picture / range-image experiment.'
    }
    $args.Workbook = $Workbook
}

if ($NonInteractive) {
    $args.NonInteractive = $true
}

& $runner @args
