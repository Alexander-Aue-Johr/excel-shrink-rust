[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Corpus,
    [string]$ConfigurationPath,
    [switch]$SkipBuild
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
[Threading.Thread]::CurrentThread.CurrentUICulture=[Globalization.CultureInfo]::GetCultureInfo('en-US')
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if($repo -notmatch '^[A-Za-z]:[\\/]'){
    throw 'Prepare the native Windows workspace with: bash tools/start-corpus-wsl.sh -PrepareStandalone -Corpus Destatis'
}
Import-Module (Join-Path $PSScriptRoot 'corpus-launcher/Core.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'corpus-launcher/Launch.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'corpus-launcher/Standalone.psm1') -Force
if(-not $ConfigurationPath){$ConfigurationPath=Join-Path $repo '.tmp/corpus-launcher/config.json'}
$ConfigurationPath=Resolve-StudyPath $repo $ConfigurationPath
$configuration=Import-StudyConfiguration $repo $ConfigurationPath
$selected=@($configuration.corpora | Where-Object {$_.name -eq $Corpus})
if($selected.Count -ne 1){throw "Unknown or ambiguous corpus: $Corpus"}
$output=Resolve-StudyPath $repo $configuration.settings.output_root
$gate=[Threading.Mutex]::new($false,('Local\ExcelShrink-CorpusLaunch-'+(Get-TextHash $output).Substring(0,20)))
$owned=$false
try {
    try{$owned=$gate.WaitOne(0)}catch [Threading.AbandonedMutexException]{$owned=$true}
    if(-not $owned -or -not (Get-StudyRunStatus $output).Ended){throw 'Stop the active corpus run and wait for its workbooks to finish before preparing the Windows deployment.'}
    $stamp=(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8)
    $directory=Join-Path $repo ('.tmp/corpus-launcher/deployments/'+$stamp)
    [void][IO.Directory]::CreateDirectory($directory)
    if(-not $SkipBuild){
        $executable=Invoke-StudyReleaseBuild $repo $directory 'cargo' (Get-Value $configuration 'wsl_workspace') -Versioned
    }else{
        $previousDeployment=Read-StudyJson (Join-Path $repo '.tmp/corpus-launcher/standalone-deployment.json')
        $executable=Get-Value $previousDeployment 'executable'
    }
    if(-not [IO.File]::Exists($executable)){throw "The Windows release executable is missing: $executable"}
    $source=Get-Value $selected[0] 'local_archive'
    if(-not $source){throw "Link a local archive for $Corpus before preparing its Windows deployment."}
    $source=Resolve-StudyPath $repo $source
    $archives=Join-Path $repo '.tmp/corpus-launcher/standalone-archives'
    $destination=Join-Path $archives ($selected[0].id.Substring(0,20)+'.zip')
    Write-Host "Preparing local Windows archive for $Corpus..."
    $copy=Copy-StandaloneArchive $source $destination (Get-Value $selected[0] 'archive_sha256')
    $standalonePath=Join-Path $repo '.tmp/corpus-launcher/standalone.json'
    $previous=Read-StudyJson $standalonePath
    $available=@(@(Get-Value $previous 'standalone_corpora' @())+@($Corpus) | Select-Object -Unique)
    $standalone=New-StandaloneStudyConfiguration $configuration $repo $available $archives
    $standalone.settings.corpus=$Corpus
    foreach($entry in $standalone.corpora){
        if($entry.name -eq $Corpus){$entry | Add-Member -NotePropertyName archive_sha256 -NotePropertyValue $copy.sha256 -Force}
    }
    # Store the deployment separately; the WSL build configuration and all
    # existing records, prepared variants and provenance keep their locations.
    Save-StudyJson $standalonePath $standalone
    # Replace wrappers from older deployments that bound every invocation to
    # standalone.json. The current command selects it only with -Standalone.
    & (Join-Path $PSScriptRoot 'install-corpus-command.ps1')
    $commit=(& git -C $repo rev-parse HEAD).Trim()
    if($LASTEXITCODE -ne 0){throw 'Could not record the deployed source commit.'}
    $receipt=[pscustomobject][ordered]@{
        version=1;prepared_utc=[DateTime]::UtcNow.ToString('o');commit=$commit
        executable=$executable;binary_sha256=(Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
        configuration=$standalonePath;output_root=$output;corpora=$available;archive=$copy
    }
    Save-StudyJson (Join-Path $directory 'deployment.json') $receipt
    Save-StudyJson (Join-Path $repo '.tmp/corpus-launcher/standalone-deployment.json') $receipt
    Write-Host 'Windows deployment is ready. WSL is not needed to run it.'
    Write-Host "In Windows PowerShell, run: excel-shrink-visual-validation -Standalone -Corpus $Corpus"
    $receipt | ConvertTo-Json -Depth 6
} finally {if($owned){$gate.ReleaseMutex()};$gate.Dispose()}
