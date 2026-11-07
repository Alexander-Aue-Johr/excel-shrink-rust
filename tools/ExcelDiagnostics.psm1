function Clear-ExcelDiagnosticLogs {
    param(
        [Parameter(Mandatory=$true)][string]$Root,
        [int]$MinimumAgeSeconds = 60,
        [string]$OwnershipPath,
        [ValidateRange(0, 4096)][int]$MaximumCompressionFiles = 32
    )
    $rootPath = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    if ([IO.Path]::GetFileName($rootPath) -ne 'EXCEL' -or [IO.Path]::GetFileName([IO.Path]::GetDirectoryName($rootPath)) -ne 'Diagnostics') {
        throw 'Cleanup is restricted to a Diagnostics\EXCEL directory.'
    }
    if (-not ('ExcelDiagnosticFiles' -as [type])) { Add-Type -Path (Join-Path $PSScriptRoot 'ExcelDiagnosticFiles.cs') }
    $observations = @{}
    if ($OwnershipPath -and (Test-Path -LiteralPath $OwnershipPath)) {
        try {
            foreach ($entry in @(Get-Content -LiteralPath $OwnershipPath -Raw | ConvertFrom-Json)) {
                if ($entry.path -and $entry.identity -and $entry.owners) { $observations[[string]$entry.path] = $entry }
            }
        } catch { Write-Warning 'Could not read diagnostic ownership history; unknown logs will be retained.' }
    }
    $observedThisPass = @{}
    $ownerQueries = 0
    $compressionAttempts = 0
    $result = [ordered]@{
        root=$rootPath; sampled_utc=[DateTime]::UtcNow.ToString('o'); removed_files=0; removed_bytes=[Int64]0;
        skipped_files=0; retained_bytes=[Int64]0; retained_live_bytes=[Int64]0; retained_unknown_bytes=[Int64]0;
        retained_locked_bytes=[Int64]0; retained_allocated_bytes=[Int64]0; allocation_unknown_files=0;
        compressed_files=0; compression_attempts=0; compression_saved_bytes=[Int64]0;
        outcomes=@{}; observed_files=0; ownership_queries=0; free_disk_bytes=$null; warning=$null
    }
    try {
        $drive = New-Object IO.DriveInfo ([IO.Path]::GetPathRoot($rootPath))
        $result.free_disk_bytes = $drive.AvailableFreeSpace
        if ($result.free_disk_bytes -lt 2GB) { $result.warning = 'Diagnostic volume has less than 2 GiB free. Unknown or live log owners are retained.' }
    } catch { $result.warning = 'Could not determine free space on the diagnostic volume.' }
    if (-not (Test-Path -LiteralPath $rootPath -PathType Container)) { return [pscustomobject]$result }
    $directories = New-Object 'System.Collections.Generic.Stack[string]'
    $directories.Push($rootPath)
    while ($directories.Count) {
        $directory = Get-Item -LiteralPath $directories.Pop() -ErrorAction Stop
        if ($directory.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
        # Observe fresh logger files first when the per-pass query budget is used.
        foreach ($item in @(Get-ChildItem -LiteralPath $directory.FullName -Force -ErrorAction SilentlyContinue | Sort-Object CreationTimeUtc -Descending)) {
            $path = [IO.Path]::GetFullPath($item.FullName)
            if (-not $path.StartsWith($rootPath + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Cleanup escaped its root.' }
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            if ($item.PSIsContainer) { $directories.Push($path); continue }
            if ($item.Extension -ne '.log') { continue }
            try {
                $observed = $observations[$path]
                $owners = @()
                if ($observed) {
                    foreach ($owner in $observed.owners) {
                        $identity = New-Object 'ExcelDiagnosticFiles+Owner'
                        $identity.Pid = [int]$owner.Pid
                        $identity.StartedTicks = [long]$owner.StartedTicks
                        $owners += $identity
                    }
                }
                $inspection = [ExcelDiagnosticFiles]::InspectAndRemove($path, $rootPath, $MinimumAgeSeconds, [string]$observed.identity, [ExcelDiagnosticFiles+Owner[]]$owners, ($compressionAttempts -lt $MaximumCompressionFiles))
                $outcome = $inspection.Outcome
                if ($inspection.CompressionAttempted) { $compressionAttempts++ }
                if ($outcome -eq 'unknown-compressed') { $result.compressed_files++; $result.compression_saved_bytes += $inspection.CompressionSavedBytes }
                if (-not $outcome.StartsWith('removed-') -and $OwnershipPath) {
                    if ($observed) { $observedThisPass[$path] = $observed }
                    # Only previously unknown files need Restart Manager. Bound
                    # registry work per pass; never ask it to shut anything down.
                    if ((-not $observed -or ($inspection.FileIdentity -and $inspection.FileIdentity -ne $observed.identity)) -and
                        $outcome -in @('locked','unknown') -and $ownerQueries -lt 32) {
                        $ownerQueries++
                        $mapped = [ExcelDiagnosticFiles]::ObserveOwners($path)
                        if ($mapped) { $observedThisPass[$path] = @{path=$path;identity=$mapped.FileIdentity;owners=$mapped.Owners} }
                    }
                }
                if (-not $result.outcomes.ContainsKey($outcome)) { $result.outcomes[$outcome] = 0 }
                $result.outcomes[$outcome]++
                if ($outcome.StartsWith('removed-')) {
                    $result.removed_files++
                    $result.removed_bytes += $inspection.Length
                } else {
                    $result.skipped_files++
                    $result.retained_bytes += $item.Length
                    if ($null -ne $inspection.AllocatedBytes) { $result.retained_allocated_bytes += $inspection.AllocatedBytes }
                    else { $result.allocation_unknown_files++; $result.retained_allocated_bytes += $item.Length }
                    if ($outcome -eq 'live') { $result.retained_live_bytes += $item.Length }
                    elseif ($outcome -eq 'locked') { $result.retained_locked_bytes += $item.Length }
                    elseif ($outcome -ne 'recent') { $result.retained_unknown_bytes += $item.Length }
                }
            } catch {
                $result.skipped_files++
                $result.retained_bytes += $item.Length
                $result.retained_unknown_bytes += $item.Length
                $result.retained_allocated_bytes += $item.Length
                $result.allocation_unknown_files++
            }
        }
    }
    if ($OwnershipPath) {
        $pending = $OwnershipPath + '.pending'
        [object[]]$records = @($observedThisPass.GetEnumerator() | ForEach-Object { $_.Value })
        ConvertTo-Json -InputObject $records -Depth 4 | Set-Content -LiteralPath $pending -Encoding UTF8
        Move-Item -LiteralPath $pending -Destination $OwnershipPath -Force
    }
    $result.observed_files = $observedThisPass.Count
    $result.ownership_queries = $ownerQueries
    $result.compression_attempts = $compressionAttempts
    try { $result.free_disk_bytes = (New-Object IO.DriveInfo ([IO.Path]::GetPathRoot($rootPath))).AvailableFreeSpace } catch {}
    if ($result.free_disk_bytes -ge 2GB) { $result.warning = $null }
    [pscustomobject]$result
}
function Clear-OfficeGraphicsCache {
    <# .SYNOPSIS
    Retires old graphics cache files only while Office is closed.
    #>
    param([string]$Root = (Join-Path $env:LOCALAPPDATA 'Microsoft/Windows/INetCache/Content.MSO'))
    $rootPath = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    if ([IO.Path]::GetFileName($rootPath) -ne 'Content.MSO' -or
        [IO.Path]::GetFileName([IO.Path]::GetDirectoryName($rootPath)) -ne 'INetCache') {
        throw 'Graphics cleanup is restricted to INetCache\Content.MSO.'
    }
    $result = [ordered]@{root=$rootPath; removed_files=0; removed_bytes=[long]0; skipped_files=0; status='complete'}
    if (Get-Process EXCEL,WINWORD,POWERPNT,OUTLOOK,MSACCESS,ONENOTE,MSPUB -ErrorAction SilentlyContinue) {
        $result.status = 'office-running'
        return [pscustomobject]$result
    }
    if (-not (Test-Path -LiteralPath $rootPath)) { return [pscustomobject]$result }
    if ((Get-Item -LiteralPath $rootPath -Force -ErrorAction Stop).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Graphics cache root is a reparse point.' }
    if (-not ('ExcelDiagnosticFiles' -as [type])) { Add-Type -Path (Join-Path $PSScriptRoot 'ExcelDiagnosticFiles.cs') }
    # Content.MSO files are direct children. Do not traverse nested directories.
    foreach ($item in Get-ChildItem -LiteralPath $rootPath -File -Force) {
        $outcome = [ExcelDiagnosticFiles]::RemoveOfficeCacheFile($item.FullName, $rootPath, 3600)
        if ($outcome -eq 'office-running') { $result.status = $outcome; break }
        if ($outcome -eq 'removed') { $result.removed_files++; $result.removed_bytes += $item.Length }
        else { $result.skipped_files++ }
    }
    [pscustomobject]$result
}

Export-ModuleMember -Function Clear-ExcelDiagnosticLogs,Clear-OfficeGraphicsCache
