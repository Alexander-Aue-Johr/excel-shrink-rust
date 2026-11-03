Set-StrictMode -Version Latest

function ConvertTo-WindowsLongPath {
    <#
    .SYNOPSIS
    Adds the Windows extended-path prefix only when a file path approaches MAX_PATH.
    .DESCRIPTION
    File APIs can reject a long absolute path even when its directory exists.
    Preserve short paths and already-prefixed paths. Native drive paths and UNC
    paths, including WSL shares, use their respective extended-path forms.
    Apply this at file I/O boundaries, after Join-Path has composed the path;
    PowerShell's Join-Path does not accept the extended prefix as a drive.
    #>
    param([Parameter(Mandatory=$true)][string]$Path)

    if ($Path.StartsWith('\\?\')) { return $Path }
    $full = if ($Path -match '^[A-Za-z]:[\\/]' -or $Path.StartsWith('\\')) {
        $Path
    } else {
        [IO.Path]::GetFullPath($Path)
    }
    if ($full.Length -lt 240) { return $full }
    if ($full.StartsWith('\\')) { return '\\?\UNC\' + $full.Substring(2) }
    if ($full -match '^[A-Za-z]:[\\/]') { return '\\?\' + $full }
    return $full
}

Export-ModuleMember -Function ConvertTo-WindowsLongPath
