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

    [switch]$ConfigureWebViewLoopback,

    [switch]$EnableRuntimeLogging,

    [switch]$TemporaryDeveloperRegistration
)

$ErrorActionPreference = 'Stop'

$runner = Join-Path $PSScriptRoot 'run-experiment.ps1'

$arguments = @{
    OfficeJsOnly = $true
    SheetsPerWorkbook = $SheetsPerWorkbook
    MaxCaptureRows = $MaxCaptureRows
    MaxCaptureColumns = $MaxCaptureColumns
    OfficeJsBatchSize = $OfficeJsBatchSize
    OfficeJsTimeoutSeconds = $OfficeJsTimeoutSeconds
    OfficeJsFirstActivationTimeoutSeconds = $OfficeJsFirstActivationTimeoutSeconds
    OfficeJsFirstActivationAttempts = $OfficeJsFirstActivationAttempts
    OfficeJsActivationRetryDelaySeconds = $OfficeJsActivationRetryDelaySeconds
    OfficeJsRegistrationSettleSeconds = $OfficeJsRegistrationSettleSeconds
    OfficeJsPerWorkbookAttempts = $OfficeJsPerWorkbookAttempts
    OfficeJsPostResultHoldSeconds = $OfficeJsPostResultHoldSeconds
    OfficeJsInterWorkbookDelaySeconds = $OfficeJsInterWorkbookDelaySeconds
}

if (-not [string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $arguments.CorpusRoot = $CorpusRoot
}

if ($AllVisibleSheets) {
    $arguments.AllVisibleSheets = $true
}

if ($RepresentativeSheetsOnly) {
    $arguments.RepresentativeSheetsOnly = $true
}

if ($ConfigureWebViewLoopback) {
    $arguments.ConfigureWebViewLoopback = $true
}

if ($EnableRuntimeLogging) {
    $arguments.EnableRuntimeLogging = $true
}

if ($TemporaryDeveloperRegistration) {
    $arguments.TemporaryDeveloperRegistration = $true
}

& $runner @arguments
