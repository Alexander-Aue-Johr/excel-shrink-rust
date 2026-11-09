# Fail explicitly before a targeted comparison partitions the entire Excel grid.
function Get-TargetedFullGridPlanFailure {
    param([Parameter(Mandatory=$true)][string]$Path)
    $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
    $reader = [IO.StreamReader]::new($stream)
    try {
        $header = $reader.ReadLine() | ConvertFrom-Json
        if ($header.version -ne 1 -or $header.stage -ne 'before_geometry_partition') { return $null }
        # This observer is bounded independently of the size of the plan index.
        for ($index = 0; $index -lt [Math]::Min([long]$header.total, 4096); $index++) {
            $line = $reader.ReadLine()
            if ($null -eq $line) { break }
            $region = $line | ConvertFrom-Json
            if ($region.range -eq 'A1:XFD1048576') {
                return @{
                    status='failed';kind='unbounded_capture_plan';coverage='unverified'
                    sheet=$region.sheet;range=$region.range
                    error="Targeted plan requires the entire worksheet $($region.sheet)!$($region.range); bounded background and conditional-format coverage is unresolved."
                }
            }
        }
        return $null
    } finally { $reader.Dispose() }
}

Export-ModuleMember -Function Get-TargetedFullGridPlanFailure
