function Clear-ExcelDiagnosticLogs {
    param([Parameter(Mandatory=$true)][string]$Root, [int]$MinimumAgeSeconds = 60)
    $rootPath = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    if ([IO.Path]::GetFileName($rootPath) -ne 'EXCEL' -or [IO.Path]::GetFileName([IO.Path]::GetDirectoryName($rootPath)) -ne 'Diagnostics') {
        throw 'Cleanup is restricted to a Diagnostics\EXCEL directory.'
    }
    $result = [ordered]@{ root=$rootPath; sampled_utc=[DateTime]::UtcNow.ToString('o'); removed_files=0; removed_bytes=[Int64]0; skipped_files=0; retained_bytes=[Int64]0 }
    if (-not (Test-Path -LiteralPath $rootPath -PathType Container)) { return [pscustomobject]$result }
    $directories = New-Object 'System.Collections.Generic.Stack[string]'
    $directories.Push($rootPath)
    $cutoff = [DateTime]::UtcNow.AddSeconds(-$MinimumAgeSeconds)
    while ($directories.Count) {
        $directory = Get-Item -LiteralPath $directories.Pop() -ErrorAction Stop
        if ($directory.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
        foreach ($item in @(Get-ChildItem -LiteralPath $directory.FullName -Force -ErrorAction SilentlyContinue)) {
            $path = [IO.Path]::GetFullPath($item.FullName)
            if (-not $path.StartsWith($rootPath + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Cleanup escaped its root.' }
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            if ($item.PSIsContainer) { $directories.Push($path); continue }
            if ($item.Extension -ne '.log') { continue }
            if ($item.LastWriteTimeUtc -gt $cutoff) { $result.retained_bytes += $item.Length; continue }
            try {
                $length = $item.Length
                Remove-Item -LiteralPath $path -Force -ErrorAction Stop
                $result.removed_files++
                $result.removed_bytes += $length
            } catch {
                $result.skipped_files++
                $result.retained_bytes += $item.Length
            }
        }
    }
    [pscustomobject]$result
}
Export-ModuleMember -Function Clear-ExcelDiagnosticLogs
