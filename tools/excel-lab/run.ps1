[CmdletBinding()]
param(
    [ValidateSet('ui','build','self-test')][string]$Action='ui',
    [string]$WslRepo,
    [string]$WslDistro,
    [switch]$DeferDiff,
    [string]$ResultFile,
    [string]$CancelFile
)
$toolRoot=$PSScriptRoot
$repo=[IO.Path]::GetFullPath((Join-Path $toolRoot '../..'))
# Reuse the existing streaming/quoting implementation, now with a VBA progress
# file tail and idempotent owned-process cleanup. No Python in the runtime.
. (Join-Path $repo 'tools/excel-height-check/json_diff.ps1') -FunctionsOnly -LinuxRepoRoot $WslRepo -Distro $WslDistro -DeferDiff:$DeferDiff -ResultFile $ResultFile -CancelFile $CancelFile
. (Join-Path $toolRoot 'ExcelInterop.ps1')
. (Join-Path $toolRoot 'Plan.ps1')
. (Join-Path $toolRoot 'Options.ps1')
$ErrorActionPreference='Stop'
$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$exitCode=0
function Invoke-LabWorker([string]$WorkerAction,[string]$Plan,[int]$Timeout) {
    $arguments=@('-NoLogo','-NoProfile','-STA','-ExecutionPolicy','Bypass','-File',(Join-Path $toolRoot 'worker.ps1'),
        '-Action',$WorkerAction,'-ToolRoot',$toolRoot,'-RunRoot',$run,'-StateFile',$script:ExcelState,'-OwnerToken',$script:OwnerToken)
    if ($Plan) { $arguments+=@('-PlanFile',$Plan,'-ProgressFile',$progress) }
    if ($CancelFile) { $arguments+=@('-CancelFile',$CancelFile) }
    try {
        [void](Invoke-LoggedProcess -Executable $shell -Arguments $arguments -Stage ('excel-'+$WorkerAction) -TimeoutSeconds $Timeout -WorkingDirectory $env:TEMP -ProgressPath $progress)
    } finally { Stop-OwnedExcel }
}
function Invoke-LabCargo([string[]]$CargoArguments,[string]$Stage,[int]$Timeout) {
    if ($LinuxRepoRoot) {
        $wslHome=Invoke-WslText @('printenv','HOME') 'wsl'
        [void](Invoke-LoggedProcess -Executable 'wsl.exe' -Arguments (@('-d',$Distro,'--cd',$LinuxRepoRoot,'--exec',($wslHome+'/.cargo/bin/cargo'))+$CargoArguments) -Stage $Stage -TimeoutSeconds $Timeout -WorkingDirectory $env:TEMP)
    } else {
        $cargo=Get-Command cargo.exe -ErrorAction SilentlyContinue
        $cargoPath=if ($cargo) { $cargo.Source } else { Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe' }
        if (-not [IO.File]::Exists($cargoPath)) { throw 'No native cargo.exe found; use the WSL checkout or install Rust for Windows.' }
        [void](Invoke-LoggedProcess -Executable $cargoPath -Arguments $CargoArguments -Stage $Stage -TimeoutSeconds $Timeout -WorkingDirectory $repo)
    }
}
function Resolve-LabInput([string]$Path) {
    if ($LinuxRepoRoot -and $Path.StartsWith('/')) { $Path=Invoke-WslText @('wslpath','-w',$Path) 'path' }
    return (Get-Item -LiteralPath $Path).FullName
}
try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Launch Windows PowerShell with desktop Excel.' }
    if (-not $LinuxRepoRoot -and $repo -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.+)$') {
        $Distro=$Matches[1]; $LinuxRepoRoot='/'+$Matches[2].Replace('\','/')
    }
    $profileData=[IO.File]::ReadAllText((Join-Path $repo 'tools/excel-height-check/json_diff_profiles.json')) | ConvertFrom-Json
    $sourceText=[IO.File]::ReadAllText((Join-Path $repo 'excel-shrink-cli/src/visual_profile.rs')).Replace("`r`n","`n")
    $definition=[regex]::Match($sourceText,'(?s)pub const PREDEFINED_VISUAL_PROFILES[^=]*=\s*\[(.*?)\];')
    if (-not $definition.Success -or (Get-TextSha256 $definition.Groups[1].Value) -cne $profileData.definition_sha256) { throw 'Rust profiles changed; refresh the profile snapshot with the patch installer.' }
    $configuration=$null
    $workerAction=$Action
    if ($Action -eq 'ui') {
        Write-LabStatus 'Waiting for the WPF options window.'
        $configuration=Show-ExcelLabOptions -ToolRoot $toolRoot -Profiles @($profileData.profiles | ForEach-Object {$_.name})
        if ($null -eq $configuration) { Write-LabStatus 'Cancelled; nothing started.'; return }
        $workerAction=[string]$configuration.action
    }
    $run=Join-Path $repo ('.tmp/excel-lab/'+(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[guid]::NewGuid().ToString('N').Substring(0,8))
    [void][IO.Directory]::CreateDirectory($run)
    $script:RunLog=[IO.StreamWriter]::new((Join-Path $run 'runner.log'),$false,[Text.UTF8Encoding]::new($false)); $script:RunLog.AutoFlush=$true
    $script:OwnerToken=[guid]::NewGuid().ToString('N'); $script:ExcelState=Join-Path $run 'excel-process.json'
    $progress=Join-Path $run 'vba-progress.log'
    $shell=Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    Write-RunLog 'start' "Results/log: $run"
    if ($workerAction -eq 'build' -or $workerAction -eq 'self-test') { Invoke-LabWorker $workerAction '' 1800; return }
    Write-LabJson $configuration (Join-Path $run 'options.json')
    # Validate host and explicit plan before doing a possibly expensive shrink.
    [void](Get-LabHost $toolRoot)
    $externalPath=[string]$configuration.plan
    if ($externalPath) { $externalPath=Resolve-LabInput $externalPath }
    $external=Get-LabExternalPlan $externalPath
    if (@($external.captures).Count -gt 0 -or $configuration.compare_visual_vba -or $configuration.compare_visual_rust -or $configuration.copy_matrix -or $configuration.transparent_emf) { $configuration.screenshots=$true }
    $source=Resolve-LabInput ([string]$configuration.input)
    if ([IO.File]::Exists($source)) { $files=@(Get-Item -LiteralPath $source) } else { $files=@(Get-ChildItem -LiteralPath $source -File) }
    $files=@($files | Where-Object {$_.Extension -ieq '.xlsx' -and -not $_.Name.StartsWith('~$')} | Sort-Object FullName)
    if ($files.Count -eq 0) { throw 'No XLSX files selected (folders are non-recursive).' }
    if ($files.Count -gt 999999) { throw 'Too many selected workbooks.' }
    $other=''
    if ($configuration.mode -eq 'compare') {
        if ($files.Count -ne 1 -or -not [IO.File]::Exists($source)) { throw 'Two-file comparison requires one original XLSX.' }
        $other=Resolve-LabInput ([string]$configuration.other)
        if (-not [IO.File]::Exists($other) -or [IO.Path]::GetExtension($other) -ine '.xlsx') { throw 'Counterpart must be an XLSX file.' }
    }
    if ($configuration.mode -eq 'inspect' -and ($configuration.compare_values -or $configuration.compare_visual_vba -or $configuration.compare_visual_rust)) { throw 'Comparison options require two workbooks; use Shrink or Compare mode.' }
    $inputDir=Join-Path $run 'input'; $shrunkDir=Join-Path $run 'shrunk'
    [void][IO.Directory]::CreateDirectory($inputDir); [void][IO.Directory]::CreateDirectory($shrunkDir)
    $pairs=[Collections.Generic.List[object]]::new()
    for ($i=0; $i -lt $files.Count; $i++) {
        $id=$i.ToString('D6'); $destination=Join-Path $inputDir ($id+'.xlsx')
        $hash=(Get-FileHash -LiteralPath $files[$i].FullName -Algorithm SHA256).Hash
        [IO.File]::Copy($files[$i].FullName,$destination,$false)
        if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -cne $hash) { throw 'Input changed during staging.' }
        $right=if ($configuration.mode -eq 'inspect') { '' } else { Join-Path $shrunkDir ($id+'.xlsx') }
        if ($other) {
            $rightHash=(Get-FileHash -LiteralPath $other -Algorithm SHA256).Hash
            [IO.File]::Copy($other,$right,$false)
            if ((Get-FileHash -LiteralPath $right -Algorithm SHA256).Hash -cne $rightHash) { throw 'Counterpart changed during staging.' }
        }
        $pairs.Add([ordered]@{id=$id;name=$files[$i].Name;original=$destination;shrunk=$right;source=$files[$i].FullName;source_sha256=$hash})
        Write-RunLog 'stage' "Copied $($files[$i].Name) id=$id"
    }
    # Build the complete range plan now, so bad pair IDs cannot trigger a shrink.
    $plan=New-LabPlan -Configuration $configuration -Pairs $pairs.ToArray() -External $external
    $planPath=Join-Path $run 'request.xml'; $plan.Save($planPath)
    Write-LabJson $pairs.ToArray() (Join-Path $run 'manifest.json')
    $timeout=[int]$configuration.stall_seconds
    if ($configuration.mode -eq 'shrink') {
        $selected=@($profileData.profiles | Where-Object {$_.name -ceq [string]$configuration.profile})
        if ($selected.Count -ne 1) { throw 'Unknown Rust profile.' }
        $aPath=$inputDir; $bPath=$shrunkDir
        if ($LinuxRepoRoot) { $aPath=To-LinuxPath $aPath; $bPath=To-LinuxPath $bPath }
        Invoke-LabCargo (@('run','--release','--locked','-p','excel-shrink-cli','--bin','excel-shrink-cli','--',$aPath,$bPath)+@($selected[0].flags)+@('--console-log-level','info')) 'shrink' $timeout
    }
    foreach ($pair in $pairs) {
        if ($pair.shrunk -and (-not [IO.File]::Exists($pair.shrunk) -or ([IO.FileInfo]$pair.shrunk).Length -eq 0)) { throw "Missing nonempty Shrink output: $($pair.shrunk)" }
    }
    Invoke-LabWorker 'measure' $planPath $timeout
    # Pretty JSON keeps the comparison useful; runtime/timing/image paths live
    # in separate reports rather than creating false metadata differences.
    foreach ($name in @('original','shrunk')) {
        $path=Join-Path $run ($name+'.json')
        $data=[IO.File]::ReadAllText($path) | ConvertFrom-Json
        [IO.File]::WriteAllText($path, (ConvertTo-Json -InputObject $data -Depth 100)+"`n", [Text.UTF8Encoding]::new($false))
    }
    $captures=[IO.File]::ReadAllText((Join-Path $run 'captures.json')) | ConvertFrom-Json
    $bmp=@($captures.captures | Where-Object {$_.PSObject.Properties['storage'] -and $_.storage -eq 'bmp'})
    if ($bmp.Count -gt 0 -or $configuration.compare_visual_rust) {
        $manifest=Join-Path $toolRoot 'native/Cargo.toml'; $target=Join-Path $repo '.tmp/excel-lab-rust-target'; $capturePath=Join-Path $run 'captures.json'
        if ($LinuxRepoRoot) { $manifest=To-LinuxPath $manifest; $target=To-LinuxPath $target; $capturePath=To-LinuxPath $capturePath }
        $native=@('run','--release','--manifest-path',$manifest,'--target-dir',$target)
        if ([IO.File]::Exists((Join-Path $toolRoot 'native/Cargo.lock'))) { $native+=@('--locked') }
        $native+=@('--','--manifest',$capturePath,'--workers',[string]$configuration.workers)
        if ($configuration.compare_visual_rust) { $native+=@('--compare') }
        Invoke-LabCargo $native 'rust-images' $timeout
    }
    $failed=@($captures.captures | Where-Object {$_.PSObject.Properties['status'] -and $_.status -eq 'failed'})
    if ($failed.Count -gt 0) { throw "$($failed.Count) capture variants failed. See captures.json; failures were not treated as equal/blank images." }
    Write-RunLog 'done' "ORIGINAL JSON: $(Join-Path $run 'original.json')"
    Write-RunLog 'done' "SHRUNK JSON: $(Join-Path $run 'shrunk.json')"
    Write-RunLog 'done' 'Reports: value-comparison.json, captures.json, overflow.json; image-report.json when Rust image processing was requested.'
    if ($configuration.open_diff -and $configuration.mode -ne 'inspect') {
        $left=Join-Path $run 'original.json'; $right=Join-Path $run 'shrunk.json'
        if ($DeferDiff) {
            if (-not $ResultFile) { throw 'DeferDiff requires ResultFile.' }
            [IO.File]::WriteAllText($ResultFile,$left+"`n"+$right+"`n",[Text.UTF8Encoding]::new($false))
        } else { Open-WindowsDiff $left $right }
    }
} catch {
    $exitCode=1
    Write-RunLog 'ERROR' (Format-LabException $_)
    if ($_.ScriptStackTrace) { Write-RunLog 'ERROR' $_.ScriptStackTrace }
} finally {
    Stop-OwnedExcel
    if ($null -ne $script:RunLog) { $script:RunLog.Dispose(); $script:RunLog=$null }
}
exit $exitCode
