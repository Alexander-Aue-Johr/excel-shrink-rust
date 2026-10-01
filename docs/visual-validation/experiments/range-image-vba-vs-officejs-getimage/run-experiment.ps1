param(
    [string]$CorpusRoot,

    [ValidateRange(1, 20)]
    [int]$SheetsPerWorkbook = 3,

    [switch]$AllVisibleSheets,

    [switch]$RepresentativeSheetsOnly,

    [ValidateRange(1, 1048576)]
    [int]$MaxCaptureRows = 500,

    [ValidateRange(1, 16384)]
    [int]$MaxCaptureColumns = 64,

    [ValidateRange(1, 64)]
    [int]$OfficeJsBatchSize = 8,

    [ValidateRange(5, 300)]
    [int]$OfficeJsTimeoutSeconds = 120,

    [ValidateRange(5, 120)]
    [int]$OfficeJsFirstActivationTimeoutSeconds = 120,

    [ValidateRange(1, 10)]
    [int]$OfficeJsFirstActivationAttempts = 5,

    [ValidateRange(0, 60)]
    [int]$OfficeJsActivationRetryDelaySeconds = 20,

    [ValidateRange(0, 60)]
    [int]$OfficeJsRegistrationSettleSeconds = 20,

    [ValidateRange(1, 10)]
    [int]$OfficeJsPerWorkbookAttempts = 5,

    [ValidateRange(0, 30)]
    [int]$OfficeJsPostResultHoldSeconds = 5,

    [ValidateRange(0, 60)]
    [int]$OfficeJsInterWorkbookDelaySeconds = 3,

    [switch]$SkipVba,

    [switch]$SkipOfficeJs,

    [switch]$ConfigureWebViewLoopback,

    [switch]$EnableRuntimeLogging,

    [switch]$OfficeJsOnly,

    [switch]$TemporaryDeveloperRegistration
)

$ErrorActionPreference = 'Stop'

if ($OfficeJsOnly) {
    $SkipVba = $true
    $SkipOfficeJs = $false
}

if ($SkipVba -and $SkipOfficeJs) {
    throw 'Both VBA and Office.js are disabled; there is nothing to run.'
}

$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

$RepoRoot = [IO.Path]::GetFullPath(
    (Join-Path $PSScriptRoot '..\..\..\..')
)

if ([string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $CorpusRoot = Join-Path $RepoRoot 'corpus-local'
}
else {
    $CorpusRoot = [IO.Path]::GetFullPath($CorpusRoot)
}

if (-not (Test-Path -LiteralPath $CorpusRoot)) {
    throw "Corpus directory does not exist: $CorpusRoot"
}

$DrawingExperimentRoot = Join-Path `
    (Split-Path -Parent $PSScriptRoot) `
    'drawing-bounds-vba-vs-rust-com'

$OfficeRunner = Join-Path `
    $DrawingExperimentRoot `
    'office-addin\run-office-addin.ps1'

$PlanBuilder = Join-Path $PSScriptRoot 'build-capture-plan.ps1'
$BasPath = Join-Path $PSScriptRoot 'image-capture.bas'
$Summarizer = Join-Path $PSScriptRoot 'summarize-results.ps1'

$ResultsDir = Join-Path $PSScriptRoot 'results'
$FileListPath = Join-Path $ResultsDir 'files.txt'
$PlanPath = Join-Path $ResultsDir 'capture-plan.tsv'
$VbaResultPath = Join-Path $ResultsDir 'vba-copypicture.json'
$OfficeResultPath = Join-Path $ResultsDir 'officejs-getimage.json'
$SummaryPath = Join-Path $ResultsDir 'summary.json'

$CacheRoot = Join-Path `
    $RepoRoot `
    '.tmp\visual-validation\range-image-vba-vs-officejs-getimage'

$VbaHostRoot = Join-Path $CacheRoot 'vba-host'
$VbaHostPath = Join-Path $VbaHostRoot 'range-image-host.xlsm'
$VbaFingerprintPath = Join-Path $VbaHostRoot 'source.sha256'
$ImageRoot = Join-Path $CacheRoot 'images'
$VbaImageRoot = Join-Path $ImageRoot 'vba-copypicture'
$OfficeImageRoot = Join-Path `
    $ImageRoot `
    ("officejs-getimage-batch" + $OfficeJsBatchSize)

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, $Utf8NoBom)
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
    Remove-Item `
        -LiteralPath $Destination `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue

    New-Item -ItemType Directory -Path $Destination -Force | Out-Null

    if (-not (Test-Path -LiteralPath $Source)) {
        return
    }

    foreach ($item in Get-ChildItem -LiteralPath $Source -Force) {
        Copy-Item `
            -LiteralPath $item.FullName `
            -Destination $Destination `
            -Recurse `
            -Force
    }
}

function Ensure-VbaHost {
    New-Item -ItemType Directory -Path $VbaHostRoot -Force | Out-Null

    $fingerprint = (
        Get-FileHash -LiteralPath $BasPath -Algorithm SHA256
    ).Hash.ToLowerInvariant()

    $cachedFingerprint = if (Test-Path -LiteralPath $VbaFingerprintPath) {
        [IO.File]::ReadAllText($VbaFingerprintPath).Trim().ToLowerInvariant()
    }
    else {
        ''
    }

    if (
        (Test-Path -LiteralPath $VbaHostPath) -and
        $cachedFingerprint -eq $fingerprint
    ) {
        Write-Host 'Reusing cached VBA image-capture host:'
        Write-Host "  $VbaHostPath"
        return
    }

    Write-Host 'Building cached VBA image-capture host...'

    $localHost = Join-Path `
        $env:TEMP `
        ("range-image-host-" + [Guid]::NewGuid().ToString('N') + '.xlsm')

    $excel = $null
    $workbooks = $null
    $book = $null
    $component = $null

    try {
        $excel = New-Object -ComObject Excel.Application
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
Could not build the cached VBA image-capture host.

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
        Write-Utf8NoBom $VbaFingerprintPath ($fingerprint + "`n")

        Write-Host 'Cached VBA image-capture host:'
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

function Run-VbaCapture {
    Ensure-VbaHost

    $tempRoot = Join-Path `
        $env:TEMP `
        ('range-image-vba-' + [Guid]::NewGuid().ToString('N'))

    $stagedHost = Join-Path $tempRoot 'range-image-host.xlsm'
    $stagedPlan = Join-Path $tempRoot 'capture-plan.tsv'
    $stagedImages = Join-Path $tempRoot 'images'
    $stagedResult = Join-Path $tempRoot 'vba-copypicture.json'

    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $stagedImages -Force | Out-Null

    Copy-Item -LiteralPath $VbaHostPath -Destination $stagedHost
    Copy-Item -LiteralPath $PlanPath -Destination $stagedPlan

    $excel = $null
    $workbooks = $null
    $hostBook = $null

    try {
        Write-Host ''
        Write-Host '=== VBA Range.CopyPicture -> Chart.Export PNG ==='

        $excel = New-Object -ComObject Excel.Application
        $excel.Visible = $true
        $excel.DisplayAlerts = $false
        $excel.EnableEvents = $false
        $excel.AskToUpdateLinks = $false

        Write-Host ('Excel version: ' + $excel.Version)

        $workbooks = $excel.Workbooks
        $hostBook = $workbooks.Open($stagedHost, 0, $true)

        $macro =
            "'$($hostBook.Name)'!RangeImageCapture.RunRangeImageCaptureBatch"

        $null = $excel.Run(
            $macro,
            $stagedPlan,
            $stagedImages,
            $stagedResult
        )

        if (-not (Test-Path -LiteralPath $stagedResult)) {
            throw "VBA image-capture experiment produced no result: $stagedResult"
        }

        Copy-Item -LiteralPath $stagedResult -Destination $VbaResultPath -Force
        Copy-DirectoryContents $stagedImages $VbaImageRoot

        Write-Host 'VBA image-capture result:'
        Write-Host "  $VbaResultPath"
        Write-Host 'VBA persisted PNGs:'
        Write-Host "  $VbaImageRoot"
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

        Remove-Item `
            -LiteralPath $tempRoot `
            -Recurse `
            -Force `
            -ErrorAction SilentlyContinue

        [GC]::Collect()
        [GC]::WaitForPendingFinalizers()
    }
}

New-Item -ItemType Directory -Path $ResultsDir -Force | Out-Null
New-Item -ItemType Directory -Path $ImageRoot -Force | Out-Null

# Never let a skipped producer make a new run look complete because an old result
# happened to still exist.
Remove-Item -LiteralPath $SummaryPath -Force -ErrorAction SilentlyContinue

if (-not $SkipVba) {
    Remove-Item -LiteralPath $VbaResultPath -Force -ErrorAction SilentlyContinue
    Remove-Item `
        -LiteralPath $VbaImageRoot `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue
}

if (-not $SkipOfficeJs) {
    foreach ($staleOfficeResult in @(
        $OfficeResultPath,
        (Join-Path $ResultsDir 'officejs-getimage.partial.json'),
        (Join-Path $ResultsDir 'officejs-getimage-status.json')
    )) {
        Remove-Item `
            -LiteralPath $staleOfficeResult `
            -Force `
            -ErrorAction SilentlyContinue
    }

    Remove-Item `
        -LiteralPath $OfficeImageRoot `
        -Recurse `
        -Force `
        -ErrorAction SilentlyContinue
}

$corpusFiles = @(
    Get-ChildItem `
        -LiteralPath $CorpusRoot `
        -Recurse `
        -File `
        -Filter '*.xlsx' |
    Where-Object { -not $_.Name.StartsWith('~$') } |
    Sort-Object FullName
)

if ($corpusFiles.Count -eq 0) {
    throw "No .xlsx files found below corpus: $CorpusRoot"
}

Write-Utf8NoBom `
    $FileListPath `
    ((($corpusFiles | ForEach-Object { $_.FullName }) -join "`n") + "`n")

Write-Host 'Repository:'
Write-Host "  $RepoRoot"
Write-Host 'Corpus:'
Write-Host "  $CorpusRoot"
Write-Host "Prepared $($corpusFiles.Count) workbook paths:"
Write-Host "  $FileListPath"

$planArgs = @{
    FileListPath = $FileListPath
    PlanPath = $PlanPath
    SheetsPerWorkbook = $SheetsPerWorkbook
    MaxCaptureRows = $MaxCaptureRows
    MaxCaptureColumns = $MaxCaptureColumns
}

if ($AllVisibleSheets -and $RepresentativeSheetsOnly) {
    throw (
        '-AllVisibleSheets and -RepresentativeSheetsOnly are mutually exclusive.'
    )
}

# For performance comparison, use one PNG capture from every visible worksheet by
# default. Hidden/very-hidden sheets stay excluded because VBA CopyPicture requires
# an activatable visible worksheet and changing visibility would alter the workbook.
if (-not $RepresentativeSheetsOnly) {
    $planArgs.AllVisibleSheets = $true
    Write-Host 'Capture selection: every visible worksheet'
}
else {
    Write-Host (
        "Capture selection: representative worksheets " +
        "(up to $SheetsPerWorkbook per workbook)"
    )
}

& $PlanBuilder @planArgs

if (-not $SkipVba) {
    Run-VbaCapture
}

if (-not $SkipOfficeJs) {
    Write-Host ''
    Write-Host '=== Office.js Range.getImage() -> PNG ==='

    $officeArgs = @{
        FileListPath = $FileListPath
        OutputPath = $OfficeResultPath
        BenchmarkMode = 'RangeImage'
        CapturePlanPath = $PlanPath
        ImageOutputDir = $OfficeImageRoot
        ImageBatchSize = $OfficeJsBatchSize
        ResultTimeoutSeconds = $OfficeJsTimeoutSeconds
        FirstActivationTimeoutSeconds = $OfficeJsFirstActivationTimeoutSeconds
        FirstActivationAttempts = $OfficeJsFirstActivationAttempts
        ActivationRetryDelaySeconds = $OfficeJsActivationRetryDelaySeconds
        RegistrationSettleSeconds = $OfficeJsRegistrationSettleSeconds
        PerWorkbookRetries = [Math]::Max(0, $OfficeJsPerWorkbookAttempts - 1)
        PostResultHoldSeconds = $OfficeJsPostResultHoldSeconds
        InterWorkbookDelaySeconds = $OfficeJsInterWorkbookDelaySeconds
    }

    if ($ConfigureWebViewLoopback) {
        $officeArgs.ConfigureWebViewLoopback = $true
    }

    if ($EnableRuntimeLogging) {
        $officeArgs.EnableRuntimeLogging = $true
    }

    if ($TemporaryDeveloperRegistration) {
        $officeArgs.TemporaryDeveloperRegistration = $true
    }

    & $OfficeRunner @officeArgs
}

if (
    -not $SkipVba -and
    -not $SkipOfficeJs -and
    (Test-Path -LiteralPath $VbaResultPath) -and
    (Test-Path -LiteralPath $OfficeResultPath)
) {
    & $Summarizer `
        -VbaResultPath $VbaResultPath `
        -OfficeResultPath $OfficeResultPath `
        -VbaImageRoot $VbaImageRoot `
        -OfficeImageRoot $OfficeImageRoot `
        -OutputPath $SummaryPath
}

Write-Host ''

if ($SkipVba -and -not $SkipOfficeJs) {
    Write-Host 'Office.js-only Range.getImage run complete.'
}
else {
    Write-Host 'Range-image experiment complete.'
}
Write-Host "  Plan: $PlanPath"
Write-Host "  Results: $ResultsDir"
Write-Host "  Images: $ImageRoot"
