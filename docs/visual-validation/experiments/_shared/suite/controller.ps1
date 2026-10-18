# Windows / WSL Python bridge. JSON request files preserve exact worksheet names.
function Invoke-VVEvaluationPython([string]$Repo,[string[]]$Extra) {
    $script=Join-Path $Repo 'docs\visual-validation\experiments\_shared\suite\cli.py'
    if($Repo -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$') {
        $distro=$Matches[1];$linuxRepo='/'+$Matches[2].Replace('\','/')
        $nativeArgs=@('-d',$distro,'--cd',$linuxRepo,'--exec','python3','-u',($linuxRepo+'/docs/visual-validation/experiments/_shared/suite/cli.py'),'--repo-root',$linuxRepo)
        $translated=@();$isPath=$false
        foreach($item in $Extra){
            if($isPath){
                if($item -notmatch '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$' -or $Matches[1] -cne $distro){throw 'Controller files must be in the repository WSL distribution.'}
                $translated+='/'+$Matches[2].Replace('\','/');$isPath=$false
            }else{$translated+=$item;$isPath=$item -in @('--config','--request','--resolved','--write-resolved','--run-root','--publish-run')}
        }
        & wsl.exe @nativeArgs @translated | ForEach-Object{Write-Host $_}
        return [int]$LASTEXITCODE
    }
    $python=Get-Command py -ErrorAction SilentlyContinue;$prefix=@()
    if($null -ne $python){$prefix=@('-3')}else{$python=Get-Command python -ErrorAction SilentlyContinue}
    if($null -eq $python){throw 'Python 3.10+ is required. No extra Python package is needed.'}
    & $python.Source @prefix -u $script --repo-root $Repo @Extra | ForEach-Object{Write-Host $_}
    return [int]$LASTEXITCODE
}
function Convert-VVPlainObject($Object) {
    if($null -eq $Object){return $null}
    if($Object -is [string] -or $Object -is [ValueType]){return $Object}
    if($Object -is [Collections.IDictionary]){$h=@{};foreach($k in $Object.Keys){$h[$k]=Convert-VVPlainObject $Object[$k]};return $h}
    if($Object -is [pscustomobject]){$h=@{};foreach($p in $Object.PSObject.Properties){$h[$p.Name]=Convert-VVPlainObject $p.Value};return $h}
    if($Object -is [array]){return ,@($Object|ForEach-Object{Convert-VVPlainObject $_})}
    return $Object
}
