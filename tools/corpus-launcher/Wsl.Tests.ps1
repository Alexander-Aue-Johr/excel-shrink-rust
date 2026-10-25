# Run with Windows PowerShell; no production workspace or run is changed.
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot 'Core.psm1') -Force
$tokens=$null; $errors=$null
$bridge=Join-Path $PSScriptRoot '../start-corpus-wsl.ps1'
$ast=[Management.Automation.Language.Parser]::ParseFile($bridge,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Cannot parse the WSL bridge.' }
foreach ($function in $ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst]},$false)) {
    . ([scriptblock]::Create($function.Extent.Text))
}
$root=Join-Path ([IO.Path]::GetTempPath()) ('excel-shrink-wsl-copy-'+[Guid]::NewGuid().ToString('N'))
$source=Join-Path $root 'source'; $destination=Join-Path $root 'destination'
$held=$null
try {
    [void][IO.Directory]::CreateDirectory((Join-Path $source 'records'))
    $original=Join-Path $source 'records/kept.json'
    [IO.File]::WriteAllText($original,'{"identity":"unchanged-corpus:1:book.xlsx","complete":true}')
    [IO.File]::WriteAllText((Join-Path $source '.run.lock'),'')
    $before=(Get-FileHash -LiteralPath $original).Hash
    Copy-StudyResults $source $destination
    $copied=Join-Path $destination 'records/kept.json'
    if ((Get-FileHash -LiteralPath $copied).Hash -ne $before) { throw 'Result content changed during copying.' }
    if ((Get-FileHash -LiteralPath $original).Hash -ne $before) { throw 'The original result changed.' }
    if (Test-Path (Join-Path $destination '.run.lock')) { throw 'A stale run lock was copied.' }
    Copy-StudyResults $source $destination
    [IO.File]::WriteAllText($copied,'different existing result')
    $rejected=$false
    try { Copy-StudyResults $source $destination } catch { $rejected=$_.Exception.Message -like '*refusing to overwrite*' }
    if (-not $rejected -or [IO.File]::ReadAllText($copied) -ne 'different existing result') { throw 'Conflicting results were overwritten.' }
    $held=[IO.File]::Open((Join-Path $source '.run.lock'),[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::ReadWrite)
    $held.Lock(0,1)
    $rejected=$false
    try { Copy-StudyResults $source (Join-Path $root 'must-not-exist') } catch { $rejected=$_.Exception.Message -like '*source output directory is active*' }
    if (-not $rejected -or (Test-Path (Join-Path $root 'must-not-exist'))) { throw 'An active source output was copied.' }
    $held.Unlock(0,1); $held.Dispose(); $held=$null
    'WSL result migration: copy, original retention, lock exclusion, repeat, conflict, and active-source checks passed.'
} finally {
    if ($held) { $held.Dispose() }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
