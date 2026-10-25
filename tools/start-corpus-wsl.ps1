# Invoked by start-corpus-wsl.sh using Windows PowerShell. Unknown arguments
# remain in $args and are forwarded to the normal launcher as native arguments.
param([string]$LinuxRepo, [string]$Distribution, [switch]$PrepareOnly)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$forward=@($args)
[Threading.Thread]::CurrentThread.CurrentUICulture=[Globalization.CultureInfo]::GetCultureInfo('en-US')
$source=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Import-Module (Join-Path $PSScriptRoot 'corpus-launcher/Core.psm1') -Force
if (-not $LinuxRepo.StartsWith('/') -or -not $Distribution) { throw 'A WSL repository path and distribution are required.' }
$base=Join-Path $env:LOCALAPPDATA ('excel-shrink-rust/wsl-workspaces/'+(Get-TextHash $source.ToLowerInvariant()).Substring(0,20))
$workspace=Join-Path $base 'repo'
$markerPath=Join-Path $base 'workspace.json'
$configPath=Join-Path $workspace '.tmp/corpus-launcher/config.json'
$sourceConfigPath=Join-Path $source '.tmp/corpus-launcher/config.json'

function Invoke-WorkspaceGit([string[]]$GitArguments) {
    $stdout=Join-Path $base 'git.stdout.log'; $stderr=Join-Path $base 'git.stderr.log'
    $sourceGit=$source.Replace('\','/')
    $arguments=@('-c',('safe.directory='+$sourceGit),'-c',('safe.directory='+$sourceGit+'/.git'))+$GitArguments
    $line=($arguments | ForEach-Object {ConvertTo-WindowsArgument ([string]$_)}) -join ' '
    $process=Start-Process -FilePath 'git.exe' -ArgumentList $line -WorkingDirectory $base -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Windows workspace sync failed. See $stderr." }
    return [IO.File]::ReadAllText($stdout,[Text.Encoding]::UTF8)
}

function Assert-SourceOutputIdle([string]$Output) {
    if (-not [IO.File]::Exists((Join-Path $Output '.run.lock'))) { return }
    if ($Output -notmatch '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)(\\.*)$') {
        if (Test-StudyOutputBusy $Output) { throw "The source output directory is active: $Output" }
        return
    }
    $arguments=@('-d',$Matches[1],'--exec','flock','--nonblock','--shared',($Matches[2].Replace('\','/')+'/.run.lock'),'true')
    # wsl.exe parses switches before CRT-style unquoting. Quote only arguments
    # that need it; a quoted "-d" is interpreted as a Linux command.
    $line=($arguments | ForEach-Object {if ($_ -match '[\s"]') {ConvertTo-WindowsArgument ([string]$_)} else {[string]$_}}) -join ' '
    $probeLog=Join-Path $base 'source-lock.log'; $probeError=Join-Path $base 'source-lock.stderr.log'
    $probe=Start-Process -FilePath (Join-Path $env:SystemRoot 'System32/wsl.exe') -ArgumentList $line -WorkingDirectory $base -WindowStyle Hidden -RedirectStandardOutput $probeLog -RedirectStandardError $probeError -Wait -PassThru
    if ($probe.ExitCode -ne 0) { throw "The WSL output directory is active or its lock could not be checked (code $($probe.ExitCode)): $Output. See $probeError." }
}

function Copy-StudyResults([string]$From, [string]$To) {
    if (-not [IO.Directory]::Exists($From)) { return }
    Assert-SourceOutputIdle $From
    foreach ($file in Get-ChildItem -LiteralPath $From -Recurse -File -Force) {
        $relative=$file.FullName.Substring($From.TrimEnd('\').Length).TrimStart('\')
        if ($relative -eq '.run.lock') { continue }
        $destination=Join-Path $To $relative
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
        $hash=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        if ([IO.File]::Exists($destination)) {
            if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $hash) { throw "Existing Windows results differ; refusing to overwrite: $destination" }
        } else { [IO.File]::Copy($file.FullName,$destination,$false) }
        if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ne $hash) { throw "Result copy verification failed: $destination" }
    }
    Assert-SourceOutputIdle $From
}

[void][IO.Directory]::CreateDirectory($base)
Write-Host "Preparing Windows workspace: $workspace"
$gate=[Threading.Mutex]::new($false,('Local\ExcelShrink-WslWorkspace-'+(Get-TextHash $source).Substring(0,20)))
$owned=$false
$runGate=$null; $runOwned=$false
try {
    try { $owned=$gate.WaitOne(0) } catch [Threading.AbandonedMutexException] { $owned=$true }
    if (-not $owned) { throw 'This WSL workspace is already being prepared.' }
    $marker=Read-StudyJson $markerPath
    if ([IO.Directory]::Exists($workspace) -and (-not $marker -or $marker.source_repo -cne $source)) { throw "The Windows workspace is not owned by this checkout: $workspace" }
    $existing=Read-StudyJson $configPath
    if ($existing) {
        $output=Resolve-StudyPath $workspace $existing.settings.output_root
        $runGate=[Threading.Mutex]::new($false,('Local\ExcelShrink-CorpusLaunch-'+(Get-TextHash $output).Substring(0,20)))
        try { $runOwned=$runGate.WaitOne(0) } catch [Threading.AbandonedMutexException] { $runOwned=$true }
        if (-not $runOwned -or (Test-StudyOutputBusy $output)) { throw "The Windows run is already starting or active. Open http://127.0.0.1:$($existing.settings.control_room_port)/" }
    }
    if (-not $marker) { Save-StudyJson $markerPath ([pscustomobject]@{source_repo=$source;linux_repo=$LinuxRepo;distribution=$Distribution}) }
    if (-not [IO.Directory]::Exists((Join-Path $workspace '.git'))) {
        [void](Invoke-WorkspaceGit @('clone','--quiet','--no-local','--',$source,$workspace))
    } else {
        [void](Invoke-WorkspaceGit @('-C',$workspace,'fetch','--quiet','origin','HEAD'))
        [void](Invoke-WorkspaceGit @('-C',$workspace,'reset','--quiet','--hard','FETCH_HEAD'))
    }
    # Include edits in tracked source files. Ignored archives, results, build
    # artifacts and credentials are never mirrored with the source checkout.
    $tracked=Invoke-WorkspaceGit @('-c','core.quotepath=false','-C',$source,'ls-files')
    foreach ($relative in $tracked.Split("`n")) {
        $relative=$relative.TrimEnd("`r")
        if (-not $relative) { continue }
        $from=Join-Path $source $relative; $to=Join-Path $workspace $relative
        if ([IO.File]::Exists($from)) {
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($to))
            if (-not [IO.File]::Exists($to) -or (Get-FileHash -LiteralPath $from).Hash -ne (Get-FileHash -LiteralPath $to).Hash) { [IO.File]::Copy($from,$to,$true) }
        } elseif ([IO.File]::Exists($to)) { [IO.File]::Delete($to) }
    }
    if (-not $existing) {
        $configuration=Import-StudyConfiguration $source $sourceConfigPath
        $oldOutput=Resolve-StudyPath $source $configuration.settings.output_root
        $newOutput=Join-Path $workspace '.tmp/regression/study'
        Copy-StudyResults $oldOutput $newOutput
        foreach ($entry in $configuration.corpora) {
            if ($entry.local_archive) { $entry.local_archive=Resolve-StudyPath $source $entry.local_archive }
        }
        if ($configuration.settings.import_report) { $configuration.settings.import_report=Resolve-StudyPath $source $configuration.settings.import_report }
        $configuration.settings.output_root=$newOutput
        $configuration.settings.download_root=Join-Path $workspace '.tmp/archive-regression/downloads'
        $configuration | Add-Member -NotePropertyName wsl_workspace -NotePropertyValue ([pscustomobject]@{source_repo=$source;linux_repo=$LinuxRepo;distribution=$Distribution;original_output=$oldOutput}) -Force
        Save-StudyJson $configPath $configuration
    }
    Write-Host "Windows workspace: $workspace"
    Write-Host "Existing results are retained. ZIP archives remain at their original locations."
} finally {
    if ($runGate) { if ($runOwned) { $runGate.ReleaseMutex() }; $runGate.Dispose() }
    if ($owned) { $gate.ReleaseMutex() }; $gate.Dispose()
}
if ($PrepareOnly) { return }
$powerShell=Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
& $powerShell -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File (Join-Path $workspace 'tools/start-corpus.ps1') @forward
exit $LASTEXITCODE
