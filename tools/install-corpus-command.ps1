[CmdletBinding()]
param([string]$BinDirectory, [switch]$NoPathUpdate)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function New-CorpusCommandWrapper([string]$EntryPoint, [string]$PowerShellExe) {
    foreach ($path in @($EntryPoint, $PowerShellExe)) {
        if ($path.IndexOf('"') -ge 0 -or ($path.ToCharArray() | Where-Object { [char]::IsControl($_) })) {
            throw 'The corpus command path contains unsupported characters.'
        }
    }
    # Batch files expand percent signs even inside quotes. Delayed expansion is
    # disabled so paths and forwarded arguments containing ! survive unchanged.
    $entry = $EntryPoint.Replace('%', '%%')
    $powershell = $PowerShellExe.Replace('%', '%%')
    return @"
@echo off
setlocal DisableDelayedExpansion
rem Managed by excel-shrink-rust/tools/install-corpus-command.ps1
for /f "tokens=2 delims=:" %%C in ('"%SystemRoot%\System32\chcp.com"') do set "_excel_run_codepage=%%C"
"%SystemRoot%\System32\chcp.com" 65001 >nul
"$powershell" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "$entry" %*
set "_excel_run_exit=%errorlevel%"
"%SystemRoot%\System32\chcp.com" %_excel_run_codepage% >nul
endlocal & exit /b %_excel_run_exit%
"@.Replace("`r`n", "`n").Replace("`n", "`r`n")
}

function Add-CorpusCommandPath([string]$Path, [string]$Directory) {
    $target = [IO.Path]::GetFullPath($Directory).TrimEnd('\','/')
    foreach ($entry in $Path.Split(';')) {
        if ([string]::IsNullOrWhiteSpace($entry)) { continue }
        $expanded = [Environment]::ExpandEnvironmentVariables($entry.Trim().Trim('"')).TrimEnd('\','/')
        if ([string]::Equals($expanded, $target, [StringComparison]::OrdinalIgnoreCase)) { return $Path }
    }
    if (-not $Path) { return $Directory }
    if ($Path.EndsWith(';')) { return $Path + $Directory }
    return $Path + ';' + $Directory
}

function Install-CorpusCommand([string]$Directory, [string]$EntryPoint, [switch]$NoPathUpdate) {
    if (-not [IO.File]::Exists($EntryPoint)) { throw "Corpus terminal entry point is missing: $EntryPoint" }
    $Directory = [IO.Path]::GetFullPath($Directory)
    $target = Join-Path $Directory 'excel-shrink-visual-validation.cmd'
    $marker = 'rem Managed by excel-shrink-rust/tools/install-corpus-command.ps1'
    $exists = [IO.File]::Exists($target)
    $existing = if ($exists) { [IO.File]::ReadAllText($target) } else { '' }
    if ($exists -and -not $existing.Contains($marker)) {
        throw "An unrelated command already exists at $target. It was left unchanged."
    }
    $powershell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    $content = New-CorpusCommandWrapper $EntryPoint $powershell
    [void][IO.Directory]::CreateDirectory($Directory)
    if ($content -cne $existing) { [IO.File]::WriteAllText($target, $content, [Text.UTF8Encoding]::new($false)) }
    if (-not $NoPathUpdate) {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $updated = Add-CorpusCommandPath $userPath $Directory
        if ($updated -cne $userPath) { [Environment]::SetEnvironmentVariable('Path', $updated, 'User') }
        $env:PATH = Add-CorpusCommandPath $env:PATH $Directory
    }
    return $target
}

if (-not $BinDirectory) { $BinDirectory = Join-Path $env:USERPROFILE '.local/bin' }
$entryPoint = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'start-corpus-terminal.ps1'))
$command = Install-CorpusCommand $BinDirectory $entryPoint -NoPathUpdate:$NoPathUpdate
Write-Host "Installed: $command"
Write-Host 'In a new terminal, run: excel-shrink-visual-validation -Corpus Enron -Partition 3/4'
