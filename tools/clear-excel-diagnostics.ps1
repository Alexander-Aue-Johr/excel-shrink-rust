param(
    [int]$OwnerPid,
    [long]$OwnerStartedTicks,
    [string]$StatusPath,
    [string]$Root = (Join-Path $env:LOCALAPPDATA 'Temp\Diagnostics\EXCEL'),
    [ValidateRange(0, 4096)][int]$MaximumCompressionFiles = 32,
    [switch]$Once
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'ExcelDiagnostics.psm1') -Force
if (-not $Once -and ($OwnerPid -le 0 -or $OwnerStartedTicks -le 0)) { throw 'Watch mode requires an owner process identity.' }
$lastGraphicsPass = [DateTime]::MinValue
$graphicsResult = $null
while ($true) {
    $ownerEnded = $false
    if (-not $Once) {
        $owner = Get-Process -Id $OwnerPid -ErrorAction SilentlyContinue
        if (-not $owner -or $owner.StartTime.ToUniversalTime().Ticks -ne $OwnerStartedTicks) {
            $ownerEnded = $true
        }
    }
    try {
        $ownershipPath = if ($StatusPath) { $StatusPath + '.owners.json' } else { $null }
        $result = Clear-ExcelDiagnosticLogs -Root $root -OwnershipPath $ownershipPath -MaximumCompressionFiles $MaximumCompressionFiles
        if ($ownerEnded -or ([DateTime]::UtcNow - $lastGraphicsPass).TotalMinutes -ge 5) {
            $graphicsResult = Clear-OfficeGraphicsCache
            $lastGraphicsPass = [DateTime]::UtcNow
        }
        $result | Add-Member -NotePropertyName graphics_cache -NotePropertyValue $graphicsResult
        if ($StatusPath) {
            $pending = $StatusPath + '.pending'
            $result | ConvertTo-Json | Set-Content -LiteralPath $pending -Encoding UTF8
            Move-Item -LiteralPath $pending -Destination $StatusPath -Force
        }
        if ($Once) { $result | ConvertTo-Json; break }
        if ($ownerEnded) { break }
    } catch {
        if ($Once) { throw }
        if ($ownerEnded) { throw }
        # Diagnose a failed pass without killing the corpus or spinning tightly.
        if ($StatusPath) { @{error=$_.Exception.Message;sampled_utc=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json | Set-Content -LiteralPath $StatusPath -Encoding UTF8 }
    }
    Start-Sleep -Seconds 30
}
