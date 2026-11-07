$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../ExcelDiagnostics.psm1') -Force
$testBase = if ($env:EXCEL_SHRINK_TEST_TEMP) { $env:EXCEL_SHRINK_TEST_TEMP } else { $env:TEMP }
$root = Join-Path $testBase ('graphics-cache-test-' + [Guid]::NewGuid().ToString('N') + '/INetCache/Content.MSO')
[void][IO.Directory]::CreateDirectory($root)
$old = Join-Path $root 'old.tmp'
$fresh = Join-Path $root 'fresh.tmp'
$locked = Join-Path $root 'locked.tmp'
$other = Join-Path $root 'keep.png'
foreach ($path in @($old, $fresh, $locked, $other)) { [IO.File]::WriteAllText($path, 'fixture') }
foreach ($path in @($old, $locked, $other)) {
    [IO.File]::SetCreationTimeUtc($path, [DateTime]::UtcNow.AddHours(-2))
    [IO.File]::SetLastWriteTimeUtc($path, [DateTime]::UtcNow.AddHours(-2))
}
$handle = [IO.File]::Open($locked, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
try {
    $result = Clear-OfficeGraphicsCache -Root $root
    if ($result.status -eq 'office-running') {
        if ($result.removed_files -ne 0 -or -not (Test-Path -LiteralPath $old)) { throw 'Office guard removed a cache file.' }
        'PASS: running Office prevents graphics cache cleanup.'
        return
    }
    if ($result.removed_files -ne 1 -or (Test-Path -LiteralPath $old)) { throw 'Old cache file was not retired.' }
    foreach ($path in @($fresh, $locked, $other)) {
        if (-not (Test-Path -LiteralPath $path)) { throw "Protected fixture removed: $path" }
    }
    $rejected = $false
    try { Clear-OfficeGraphicsCache -Root $testBase | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw 'Unrelated directory accepted.' }
    $outside = Join-Path $testBase ('outside-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    [IO.File]::WriteAllText($outside, 'outside')
    $result = [ExcelDiagnosticFiles]::RemoveOfficeCacheFile($outside, $root, 0)
    if ($result -ne 'unsafe-path' -or -not (Test-Path -LiteralPath $outside)) { throw 'Native root guard failed.' }
    'PASS: old cache retired; recent, locked, unrelated-extension and outside-root files preserved.'
} finally { $handle.Dispose() }
