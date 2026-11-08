$ErrorActionPreference = 'Stop'

function Get-TargetedSessionDesktop {
    <# .SYNOPSIS Resolve the private desktop belonging to this workbook session. #>
    param($Entry)
    $name = [string]$Entry.desktop_name
    if ([string]::IsNullOrWhiteSpace($name)) { $name = $env:EXCEL_SHRINK_EXPECTED_DESKTOP }
    if ($name -notmatch '^[a-zA-Z0-9_-]{1,128}$' -or $name -eq 'Default') {
        throw 'Targeted workbook session requires its private desktop identity.'
    }
    return $name
}

Export-ModuleMember -Function Get-TargetedSessionDesktop
