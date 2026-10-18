[CmdletBinding()]
param(
    [ValidateSet('gui','cli')][string]$InterfaceMode='gui',
    [switch]$PlanOnly,[switch]$NoOpen,[switch]$ReportOnly,[string]$ResumeRun,
    [string]$ConfigPath,[string]$SelectionFile,[string]$CorpusRoot,[string[]]$Group,
    [string]$Experiment='range-image',[string]$Implementation,[string]$Workbook,
    [Alias('SheetNames')][string[]]$Sheet,[int[]]$SheetIndex,[string]$SheetsJson,
    [switch]$ListSheets,[switch]$ListExperiments,
    [ValidateSet('A','B','C','D','E','F')][string]$Strategy='F',
    [ValidateSet('rust','dotnet')][string]$HostKind='dotnet',
    [ValidateSet('heuristic','selective')][string]$OverflowMode='selective',
    [ValidateSet(16,32,64)][int]$TileMegapixels=32,[ValidateSet(1,2,4,8,16,32,64)][int]$BatchSize=1,
    [ValidateSet('budget','balanced')][string]$TilePacking='budget',
    [ValidateSet('excel-x','rust-com')][string]$ExcelLaunchMode='excel-x',
    [ValidateSet('off','first-seam')][string]$ContextValidation='off',
    [ValidateSet('normal','detailed')][string]$DiagnosticLevel='normal',
    [int]$OverflowProbeWidthPx=1920,[int]$OverflowProbeHeightPx=1080,
    [ValidateRange(30,1800)][int]$SetupTimeoutSeconds=600,[ValidateRange(10,1800)][int]$StartupTimeoutSeconds=300,
    [Alias('TimeoutSeconds')][ValidateRange(30,7200)][int]$CaptureTimeoutSeconds=1200,
    [ValidateRange(10,1800)][int]$InactivityTimeoutSeconds=120,[ValidateRange(10,600)][int]$NativeCallTimeoutSeconds=90,
    [ValidateRange(30,1800)][int]$PostprocessTimeoutSeconds=600,[ValidateRange(60,86400)][int]$SessionTimeoutSeconds=21600,
    [ValidateRange(1,5)][int]$Repetitions=3,[switch]$WaitForStart,[switch]$UseOfficeDebugJs
)
$ErrorActionPreference='Stop'
if($InterfaceMode -eq 'gui' -and (-not $PSBoundParameters.ContainsKey('Experiment') -or $PSBoundParameters.ContainsKey('InterfaceMode')) -and -not $PlanOnly -and -not $ListSheets -and -not $ListExperiments -and -not $SelectionFile -and -not $ResumeRun){
    $g=@{};if($CorpusRoot){$g.CorpusRoot=$CorpusRoot};if($Workbook){$g.Workbook=$Workbook}
    & (Join-Path $PSScriptRoot 'gui.ps1') @g;return
}
$forward=@{};foreach($k in $PSBoundParameters.Keys){if($k -ne 'InterfaceMode'){$forward[$k]=$PSBoundParameters[$k]}}
if(-not $ResumeRun){$forward.Single=$true}
& (Join-Path $PSScriptRoot 'reproduce.ps1') @forward
