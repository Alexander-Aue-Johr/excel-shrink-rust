param(
    [Parameter(Mandatory=$true)]
    [string]$CorpusRoot,

    [string]$OutputPath,

    [string]$ImageOutputDir,

    [ValidateRange(1, 20)]
    [int]$SheetsPerWorkbook = 3,

    [switch]$RepresentativeSheetsOnly
)

$ErrorActionPreference = 'Stop'
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

$ExperimentRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = [IO.Path]::GetFullPath(
    (Join-Path $ExperimentRoot '..\..\..\..')
)

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $ExperimentRoot 'results\vba.json'
}
if ([string]::IsNullOrWhiteSpace($ImageOutputDir)) {
    $ImageOutputDir = Join-Path `
        $RepoRoot `
        '.tmp\visual-validation\range-image\images\vba'
}

$ResultsDir = Split-Path -Parent $OutputPath
$FileListPath = Join-Path $ResultsDir 'files-vba.txt'
$PlanPath = Join-Path $ResultsDir 'capture-plan-vba.tsv'
$PlanBuilder = Join-Path $PSScriptRoot 'build-capture-plan.ps1'
$BasPath = Join-Path $PSScriptRoot 'image-capture.bas'

$CacheRoot = Join-Path $RepoRoot '.tmp\visual-validation\range-image\vba-host'
$HostPath = Join-Path $CacheRoot 'range-image-host.xlsm'
$FingerprintPath = Join-Path $CacheRoot 'source.sha256'

function Log([string]$Message) {
    Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $Message)
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

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, $Utf8NoBom)
}

function Ensure-VbaHost {
    New-Item -ItemType Directory -Path $CacheRoot -Force | Out-Null
    $fingerprint = (
        Get-FileHash -LiteralPath $BasPath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    $cached = if (Test-Path -LiteralPath $FingerprintPath) {
        [IO.File]::ReadAllText($FingerprintPath).Trim().ToLowerInvariant()
    } else { '' }

    if ((Test-Path -LiteralPath $HostPath) -and $cached -eq $fingerprint) {
        Log "Reusing cached VBA host: $HostPath"
        return
    }

    Log 'Building cached VBA host...'
    $localHost = Join-Path `
        $env:TEMP `
        ('range-image-host-' + [Guid]::NewGuid().ToString('N') + '.xlsm')
    $excel = $null
    $workbooks = $null
    $book = $null
    $component = $null

    try {
        $excel = New-Object -ComObject Excel.Application
        $excel.Visible = $false
        $excel.DisplayAlerts = $false
        $workbooks = $excel.Workbooks
        $book = $workbooks.Add()
        try {
            $component = $book.VBProject.VBComponents.Import($BasPath)
        }
        catch {
            throw @"
Could not import the VBA module. Enable:
Excel -> Options -> Trust Center -> Macro Settings ->
Trust access to the VBA project object model.
"@
        }
        $book.SaveAs($localHost, 52)
        $book.Close($false)
        Release-ComObject $component
        $component = $null
        Release-ComObject $book
        $book = $null

        Copy-Item -LiteralPath $localHost -Destination $HostPath -Force
        Write-Utf8NoBom $FingerprintPath ($fingerprint + "`n")
    }
    finally {
        if ($null -ne $book) { try { $book.Close($false) } catch {} }
        Release-ComObject $component
        Release-ComObject $book
        Release-ComObject $workbooks
        if ($null -ne $excel) { try { $excel.Quit() } catch {} }
        Release-ComObject $excel
        Remove-Item -LiteralPath $localHost -Force -ErrorAction SilentlyContinue
    }
}

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

New-Item -ItemType Directory -Path $ResultsDir -Force | Out-Null
Write-Utf8NoBom `
    $FileListPath `
    ((($files | ForEach-Object { $_.FullName }) -join "`n") + "`n")

Log "Preparing VBA capture plan for $($files.Count) workbook(s)..."

$planArgs = @{
    FileListPath = $FileListPath
    PlanPath = $PlanPath
    SheetsPerWorkbook = $SheetsPerWorkbook
}

if (-not $RepresentativeSheetsOnly) {
    $planArgs.AllVisibleSheets = $true
}

& $PlanBuilder @planArgs

Ensure-VbaHost

$temp = Join-Path `
    $env:TEMP `
    ('range-image-vba-' + [Guid]::NewGuid().ToString('N'))
$stagedHost = Join-Path $temp 'host.xlsm'
$stagedPlan = Join-Path $temp 'capture-plan.tsv'
$stagedImages = Join-Path $temp 'images'
$stagedResult = Join-Path $temp 'result.json'
New-Item -ItemType Directory -Path $stagedImages -Force | Out-Null
Copy-Item -LiteralPath $HostPath -Destination $stagedHost
Copy-Item -LiteralPath $PlanPath -Destination $stagedPlan

$excel = $null
$workbooks = $null
$hostBook = $null

try {
    Log 'Running VBA CopyPicture benchmark...'
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $true
    $excel.DisplayAlerts = $false
    $excel.EnableEvents = $false
    $excel.AskToUpdateLinks = $false

    $workbooks = $excel.Workbooks
    $hostBook = $workbooks.Open($stagedHost, 0, $true)
    $macro = "'$($hostBook.Name)'!RangeImageCapture.RunRangeImageCaptureBatch"

    $null = $excel.Run(
        $macro,
        $stagedPlan,
        $stagedImages,
        $stagedResult
    )

    if (-not (Test-Path -LiteralPath $stagedResult)) {
        throw 'VBA benchmark produced no result JSON.'
    }

    Copy-Item -LiteralPath $stagedResult -Destination $OutputPath -Force
    Remove-Item -LiteralPath $ImageOutputDir -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $ImageOutputDir -Force | Out-Null
    foreach ($item in Get-ChildItem -LiteralPath $stagedImages -Force) {
        Copy-Item -LiteralPath $item.FullName -Destination $ImageOutputDir -Recurse
    }

    Log "VBA result: $OutputPath"
}
finally {
    if ($null -ne $hostBook) { try { $hostBook.Close($false) } catch {} }
    Release-ComObject $hostBook
    Release-ComObject $workbooks
    if ($null -ne $excel) { try { $excel.Quit() } catch {} }
    Release-ComObject $excel
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
