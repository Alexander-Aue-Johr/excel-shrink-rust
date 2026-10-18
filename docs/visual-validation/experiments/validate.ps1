param()
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
$repo=Get-VVRepo $PSScriptRoot
$code=0
if($repo -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$'){
 $distro=$Matches[1];$linuxRepo='/'+$Matches[2].Replace('\','/')
 & wsl.exe -d $distro --cd $linuxRepo --exec python3 tools/validate-visual-experiments.py
 $code=$LASTEXITCODE
}else{
 $python=Get-Command py -ErrorAction SilentlyContinue
 if($python){& $python.Source -3 (Join-Path $repo 'tools\validate-visual-experiments.py')}
 else{& python (Join-Path $repo 'tools\validate-visual-experiments.py')}
 $code=$LASTEXITCODE
}
if($code){throw 'Portable experiment tests failed.'}
# Parse source and run native file-I/O tests without starting Excel.
& (Join-Path $PSScriptRoot '_shared\suite\syntax-preflight.ps1') -RepoRoot $repo
& (Join-Path $PSScriptRoot '_shared\suite\preflight.ps1')
