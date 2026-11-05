param(
    [int]$OwnerPid,
    [long]$OwnerStartedTicks,
    [string]$StatusPath,
    [string]$Root = (Join-Path $env:LOCALAPPDATA 'Temp\Diagnostics\EXCEL'),
    [switch]$Once
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'ExcelDiagnostics.psm1') -Force
if (-not $Once -and ($OwnerPid -le 0 -or $OwnerStartedTicks -le 0)) { throw 'Watch mode requires an owner process identity.' }
while ($true) {
    if (-not $Once) {
        $owner = Get-Process -Id $OwnerPid -ErrorAction SilentlyContinue
        if (-not $owner -or $owner.StartTime.ToUniversalTime().Ticks -ne $OwnerStartedTicks) { break }
    }
    try {
        $ownershipPath = if ($StatusPath) { $StatusPath + '.owners.json' } else { $null }
        $result = Clear-ExcelDiagnosticLogs -Root $root -OwnershipPath $ownershipPath
        if ($StatusPath) {
            $pending = $StatusPath + '.pending'
            $result | ConvertTo-Json | Set-Content -LiteralPath $pending -Encoding UTF8
            Move-Item -LiteralPath $pending -Destination $StatusPath -Force
        }
        if ($Once) { $result | ConvertTo-Json; break }
    } catch {
        if ($Once) { throw }
        # Diagnose a failed pass without killing the corpus or spinning tightly.
        if ($StatusPath) { @{error=$_.Exception.Message;sampled_utc=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json | Set-Content -LiteralPath $StatusPath -Encoding UTF8 }
    }
    Start-Sleep -Seconds 30
}
