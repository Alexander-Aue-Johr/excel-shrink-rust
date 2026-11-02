param([string]$Registry, [string]$Gate, [int]$Writer = -1)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../src/regression/archive_processes.ps1') -RegistryPath ''

if ($Writer -ge 0) {
    [IO.File]::WriteAllText(($Gate + '.' + $Writer), 'ready')
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not [IO.File]::Exists($Gate)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Registration fixture gate timed out.' }
        Start-Sleep -Milliseconds 10
    }
    for ($sequence = 0; $sequence -lt 25; $sequence++) {
        Register-ArchiveOwner $Registry @{pid=$PID; started='1'; writer=$Writer; sequence=$sequence}
    }
    return
}

$root = Join-Path $env:TEMP ('es-registration-test-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $root
$Registry = Join-Path $root 'owners.jsonl'
$Gate = Join-Path $root 'start'
$children = @()
$generation = $env:EXCEL_SHRINK_ARCHIVE_GENERATION
try {
    $env:EXCEL_SHRINK_ARCHIVE_GENERATION = 'registration-test'
    [IO.File]::WriteAllText(($Registry + '.generation'), $env:EXCEL_SHRINK_ARCHIVE_GENERATION)
    [IO.File]::WriteAllText($Registry, '')
    for ($writer = 0; $writer -lt 12; $writer++) {
        $arguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -Registry "{1}" -Gate "{2}" -Writer {3}' -f $PSCommandPath, $Registry, $Gate, $writer
        $child = Start-Process powershell.exe -ArgumentList $arguments -WindowStyle Hidden -PassThru `
            -RedirectStandardError (Join-Path $root "$writer.stderr") -RedirectStandardOutput (Join-Path $root "$writer.stdout")
        $null = $child.Handle
        $children += $child
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    while (@(0..11 | Where-Object { -not [IO.File]::Exists($Gate + '.' + $_) }).Count) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Registration children did not become ready.' }
        Start-Sleep -Milliseconds 20
    }
    [IO.File]::WriteAllText($Gate, 'start')
    do {
        # Cleanup snapshots must never observe a torn JSON line during concurrent writes.
        foreach ($line in @(Read-ArchiveOwners $Registry)) { $null = $line | ConvertFrom-Json }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Concurrent registration did not finish.' }
        Start-Sleep -Milliseconds 10
    } while (@($children | Where-Object { -not $_.HasExited }).Count)
    foreach ($child in $children) {
        $child.WaitForExit()
        if ($child.ExitCode -ne 0) {
            $errors = Get-ChildItem $root -Filter '*.stderr' | ForEach-Object { [IO.File]::ReadAllText($_.FullName) }
            throw "Registration child failed: $($errors -join '; ')"
        }
    }
    $lines = @(Read-ArchiveOwners $Registry)
    $identities = @($lines | ForEach-Object { $r = $_ | ConvertFrom-Json; "$($r.writer)/$($r.sequence)" } | Sort-Object -Unique)
    if ($lines.Count -ne 300 -or $identities.Count -ne 300) { throw 'Concurrent registration lost or duplicated ownership entries.' }

    [IO.File]::WriteAllText(($Registry + '.stopping'), 'stopping')
    $refused = $false
    try { Register-ArchiveOwner $Registry @{pid=$PID; started='1'} } catch { $refused = $_.Exception.Message -like '*cleanup has started*' }
    if (-not $refused -or @(Read-ArchiveOwners $Registry).Count -ne 300) { throw 'Stopping marker failed to block a late registration.' }
    Remove-Item -LiteralPath ($Registry + '.stopping')
    $env:EXCEL_SHRINK_ARCHIVE_GENERATION = 'expired-generation'
    $refused = $false
    try { Register-ArchiveOwner $Registry @{pid=$PID; started='1'} } catch { $refused = $_.Exception.Message -like '*attempt changed*' }
    if (-not $refused) { throw 'An expired launcher registered against a later attempt.' }
    Write-Output 'PASS: 12 parallel writers, 300 complete unique entries, concurrent snapshots, stopping and generation guards.'
} finally {
    $env:EXCEL_SHRINK_ARCHIVE_GENERATION = $generation
    foreach ($child in $children) {
        if (-not $child.HasExited) { $child.Kill(); $child.WaitForExit() }
        $child.Dispose()
    }
    $resolved = [IO.Path]::GetFullPath($root)
    $tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($resolved) -match '^es-registration-test-[0-9a-f]{32}$') {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
