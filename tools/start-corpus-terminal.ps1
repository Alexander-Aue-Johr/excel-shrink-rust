[CmdletBinding()]
param(
    [string]$Corpus,
    [string]$Partition,
    [ValidateSet('priority','size-asc','size-desc')][string]$Order,
    [string]$ConfigurationPath,
    [switch]$Standalone,
    [string]$ExcelShrinkExe,
    [switch]$Build,
    [switch]$SkipBuild,
    [switch]$FullScanDeferred,
    [switch]$AggressiveFirst,
    [ValidateSet('default','maximum-cleaning','too-aggressive-cleaning')]
    [string]$PreparationHashProfile,
    [switch]$NoBrowser
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Threading.Thread]::CurrentThread.CurrentUICulture=[Globalization.CultureInfo]::GetCultureInfo('en-US')
[Threading.Thread]::CurrentThread.CurrentCulture=[Globalization.CultureInfo]::GetCultureInfo('en-US')
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Import-Module (Join-Path $PSScriptRoot 'corpus-launcher/Core.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'corpus-launcher/Launch.psm1') -Force

function Wait-CorpusObservatory([string]$Url, [string]$OutputRoot) {
    $expected = [IO.Path]::GetFullPath((Join-Path $OutputRoot 'control-room')).TrimEnd('\','/')
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
        $health = $null
        try { $health = Invoke-RestMethod ($Url + 'health') -TimeoutSec 2 -ErrorAction Stop } catch { }
        if ($health) {
            $actual = [string](Get-Value $health 'root' '')
            if ($actual.StartsWith('\\?\UNC\')) { $actual = '\\' + $actual.Substring(8) }
            elseif ($actual.StartsWith('\\?\')) { $actual = $actual.Substring(4) }
            if ((Get-Value $health 'kind') -ne 'excel-shrink-control-room' -or
                -not [IO.Path]::IsPathRooted($actual) -or
                -not [string]::Equals([IO.Path]::GetFullPath($actual).TrimEnd('\','/'), $expected, [StringComparison]::OrdinalIgnoreCase)) {
                throw "The Observatory port belongs to another run or service: $Url"
            }
            return
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "The Observatory is not ready: $Url. Check the printed runner logs."
}

function Open-CorpusObservatory([string]$Url) { Start-Process -FilePath $Url }

function Start-SavedCorpusRun {
    param([string]$Repo, [string]$ConfigurationPath, [string]$Executable,
        [string]$Corpus, [string]$Partition, [string]$Order,
        [switch]$Build, [switch]$SkipBuild, [switch]$FullScanDeferred, [switch]$NoBrowser,
        [switch]$Standalone, [switch]$AggressiveFirst,
        [string]$PreparationHashProfile)
    if ($Build -and $SkipBuild) { throw 'Choose either -Build or -SkipBuild.' }
    if ([string]::IsNullOrWhiteSpace($Corpus) -or
        (-not $Standalone -and [string]::IsNullOrWhiteSpace($Partition))) {
        throw 'Specify the corpus and size partition, for example: excel-shrink-visual-validation -Corpus Enron -Partition 3/4'
    }
    if (-not [IO.File]::Exists($ConfigurationPath)) {
        throw "No saved corpus configuration: $ConfigurationPath. Save one with tools/start-corpus.ps1 first."
    }
    $configuration = Import-StudyConfiguration $Repo $ConfigurationPath
    $selected = @($configuration.corpora | Where-Object { $_.name -eq $Corpus })
    if ($selected.Count -ne 1) { throw "Unknown or ambiguous corpus: $Corpus. Choose a configured corpus." }
    $isStandaloneConfiguration = Get-Value $configuration 'standalone_windows' $false
    $isStandaloneConfiguration = ($isStandaloneConfiguration -is [bool]) -and $isStandaloneConfiguration
    if ($Standalone -and -not $isStandaloneConfiguration) {
        throw 'The selected configuration is not a standalone Windows deployment.'
    }
    if ($isStandaloneConfiguration) {
        $update = "bash tools/start-corpus-wsl.sh -PrepareStandalone -Corpus $($selected[0].name)"
        if ($Build) { throw "This standalone Windows deployment uses prepared binaries. Update it from WSL with: $update" }
        if ($selected[0].name -notin @(Get-Value $configuration 'standalone_corpora' @())) {
            throw "This corpus has not been prepared for standalone Windows use. Prepare it from WSL with: $update"
        }
        $configuration.settings.build_release = $false
    }
    if ($Standalone -and [string]::IsNullOrWhiteSpace($Partition)) {
        if ($selected[0].name -ne $configuration.settings.corpus) {
            throw 'Specify -Partition all or N/M when changing the saved corpus.'
        }
        $Partition = if ($configuration.settings.partition -eq 0) { 'all' }
            else { "$($configuration.settings.partition)/$($configuration.settings.partitions)" }
    }
    $configuration.settings.corpus = $selected[0].name
    if ($Partition -eq 'all') {
        $configuration.settings.partition = 0
    } else {
        $index = 0; $count = 0
        if ($Partition -notmatch '\A([1-9][0-9]*)/([1-9][0-9]*)\z' -or
            -not [int]::TryParse($Matches[1], [ref]$index) -or
            -not [int]::TryParse($Matches[2], [ref]$count) -or
            $count -gt 1000 -or $index -gt $count) {
            throw 'Partition must be all or N/M, with 1 <= N <= M <= 1000 (for example 3/4).'
        }
        $configuration.settings.partition = $index
        $configuration.settings.partitions = $count
    }
    if ($Order) {
        if ($Order -notin @('priority','size-asc','size-desc')) { throw 'Order must be priority, size-asc or size-desc.' }
        $configuration.settings.order = $Order
    }
    if ($PreparationHashProfile) {
        $configuration.settings | Add-Member -NotePropertyName preparation_hash_profile -NotePropertyValue $PreparationHashProfile -Force
        $configuration.settings | Add-Member -NotePropertyName aggressive_first -NotePropertyValue $false -Force
    }
    if ($AggressiveFirst) {
        $configuration.settings | Add-Member -NotePropertyName aggressive_first -NotePropertyValue $true -Force
        $configuration.settings | Add-Member -NotePropertyName preparation_hash_profile -NotePropertyValue $null -Force
    }
    $configuration.settings | Add-Member -NotePropertyName full_scan_deferred -NotePropertyValue ([bool]$FullScanDeferred) -Force
    Assert-StudySettings $configuration.settings
    $output = Resolve-StudyPath $Repo $configuration.settings.output_root
    $url = "http://127.0.0.1:$($configuration.settings.control_room_port)/"
    $status = Get-StudyRunStatus $output
    $existing = $status.Busy -or $null -ne $status.Process
    if ($existing) {
        if (-not $status.Busy -or $null -eq $status.Process) {
            throw 'The output directory is busy, but its runner could not be verified. No second runner was started.'
        }
        $receipt = Read-StudyJson (Join-Path $output 'launch.json')
        $active = Get-Value $receipt 'Settings'
        if (-not $active -or (Get-Value $receipt 'Pid' 0) -ne $status.Process.Id) {
            throw 'The active run selection could not be verified. No second runner was started.'
        }
        if ((Get-Value $active 'aggressive_first' $false) -ne (Get-Value $configuration.settings 'aggressive_first' $false)) {
            throw 'The active run uses a different comparison strategy. Stop it before changing screening versus full validation.'
        }
        if ((Get-Value $active 'preparation_hash_profile' $null) -ne (Get-Value $configuration.settings 'preparation_hash_profile' $null)) {
            throw 'The active run uses a different preparation hash profile. Stop it before changing the profile.'
        }
        $samePartition = $active.partition -eq $configuration.settings.partition -and
            ($active.partition -eq 0 -or $active.partitions -eq $configuration.settings.partitions)
        if ($active.corpus -ne $configuration.settings.corpus -or -not $samePartition -or
            $active.order -ne $configuration.settings.order) {
            $activePartition = if ($active.partition -eq 0) { 'all' } else { "$($active.partition)/$($active.partitions)" }
            throw "A different visual validation run is already active: $($active.corpus), partition $activePartition, $($active.order). No second runner was started. Observatory: $url"
        }
        $result = [pscustomobject]@{
            Pid = $status.Process.Id; ControlRoom = $url; OutputRoot = $output; AlreadyRunning = $true
            Stdout = (Get-Value $receipt 'Stdout'); Stderr = (Get-Value $receipt 'Stderr')
        }
    } else {
        if ($Build) { $configuration.settings.build_release = $true }
        if ($SkipBuild) { $configuration.settings.build_release = $false }
        $stamp = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8)
        $directory = Join-Path ([IO.Path]::GetDirectoryName($ConfigurationPath)) ('runs/' + $stamp)
        $plan = New-StudyLaunchPlan $configuration $Repo $Executable $directory
        $receipt = Start-StudyLaunch $configuration $ConfigurationPath $Repo $plan
        $result = [pscustomobject]@{
            Pid = $receipt.Pid; ControlRoom = $receipt.ControlRoom; OutputRoot = $output; AlreadyRunning = $false
            Stdout = $receipt.Stdout; Stderr = $receipt.Stderr
        }
    }
    Write-Host "Visual validation: $($configuration.settings.corpus), partition $Partition, $($configuration.settings.order)."
    $label = if ($result.AlreadyRunning) { 'Already running' } else { 'Runner started' }
    Write-Host "$label (PID $($result.Pid))."
    Write-Host "Observatory: $($result.ControlRoom)"
    if ($result.Stdout) { Write-Host "Output log: $($result.Stdout)" }
    if ($result.Stderr) { Write-Host "Error log:  $($result.Stderr)" }
    Write-Host 'Waiting for the Observatory...'
    Wait-CorpusObservatory $result.ControlRoom $output
    if (-not $NoBrowser) { Open-CorpusObservatory $result.ControlRoom }
    return $result
}

if ($Standalone -and $ConfigurationPath) { throw 'Choose either -Standalone or -ConfigurationPath.' }
if (-not $ConfigurationPath) {
    $configurationName = if ($Standalone) { 'standalone.json' } else { 'config.json' }
    $ConfigurationPath = Join-Path $repo ".tmp/corpus-launcher/$configurationName"
}
$ConfigurationPath = Resolve-StudyPath $repo $ConfigurationPath
if ($Standalone -and -not [IO.File]::Exists($ConfigurationPath)) {
    throw 'No standalone deployment found. Prepare one from WSL with -PrepareStandalone.'
}
if (-not $ExcelShrinkExe) { $ExcelShrinkExe = Join-Path $repo 'target/release/excel-shrink-cli.exe' }
$null = Start-SavedCorpusRun $repo $ConfigurationPath $ExcelShrinkExe -Corpus $Corpus -Partition $Partition -Order $Order -Build:$Build -SkipBuild:$SkipBuild -FullScanDeferred:$FullScanDeferred -NoBrowser:$NoBrowser -Standalone:$Standalone -AggressiveFirst:$AggressiveFirst -PreparationHashProfile $PreparationHashProfile
