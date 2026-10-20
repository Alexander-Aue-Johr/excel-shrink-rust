$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
$registryScript = Join-Path $captureRoot '../src/regression/archive_processes.ps1'
$root = Join-Path $env:TEMP ('es-archive-recovery-test-' + [Guid]::NewGuid().ToString('N'))
$stage = Join-Path $root 'stage'
$registry = Join-Path $root 'owners.jsonl'
$childRecord = Join-Path $root 'child.txt'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$launcher = Join-Path $root 'launcher.ps1'
@'
param($RegistryScript, $RegistryPath, $StagePath, $CaptureRoot, $ChildRecord)
$ErrorActionPreference = 'Stop'
& $RegistryScript -RegistryPath $RegistryPath -StagePath $StagePath
Add-Type -Path (Join-Path $CaptureRoot 'CaptureProcesses.cs')
$job = New-Object CaptureProcesses
$child = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -Command Start-Sleep 120' -PassThru -WindowStyle Hidden
$job.Add($child)
Set-Content -LiteralPath $ChildRecord -Value $child.Id
Start-Sleep 120
'@ | Set-Content -LiteralPath $launcher -Encoding UTF8
$owner = $null
try {
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $launcher,
        '-RegistryScript', $registryScript, '-RegistryPath', $registry,
        '-StagePath', $stage, '-CaptureRoot', $captureRoot, '-ChildRecord', $childRecord)
    $quoted = $arguments | ForEach-Object { '"' + $_ + '"' }
    $owner = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList $quoted -PassThru -WindowStyle Hidden
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not (Test-Path -LiteralPath $childRecord)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Test launcher did not start.' }
        Start-Sleep -Milliseconds 100
    }
    $childId = [int](Get-Content -LiteralPath $childRecord)
    # A reused PID with a different birth time must be ignored.
    $stale = @{ pid = $PID; started = '1'; stage = $null } | ConvertTo-Json -Compress
    Add-Content -LiteralPath $registry -Value $stale
    & $registryScript -RegistryPath $registry -Stop
    Start-Sleep -Milliseconds 300
    if (Get-Process -Id $owner.Id -ErrorAction SilentlyContinue) { throw 'Launcher survived cleanup.' }
    if (Get-Process -Id $childId -ErrorAction SilentlyContinue) { throw 'Owned child survived cleanup.' }
    if (Test-Path -LiteralPath $stage) { throw 'Staged files survived cleanup.' }
    Write-Host 'PASS: stopped owned launcher and child, removed staging, ignored stale PID.'
}
finally {
    if ($null -ne $owner) { Stop-Process -Id $owner.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
