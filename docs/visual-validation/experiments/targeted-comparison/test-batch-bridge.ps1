<# .SYNOPSIS
Tests real file-notified dispatch with synthetic resident responses, without Excel.
#>
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../../../..')).Path
Import-Module (Join-Path $repository 'excel-shrink-cli/office-capture/TargetedResident.psm1') -Force -DisableNameChecking
$module = Get-Module TargetedResident
$root = Join-Path $repository ('.tmp/batch-bridge-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $root
$producer = Start-Job -ArgumentList $root -ScriptBlock {
    param($Root)
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not ((Test-Path "$Root/resident-O/commands/one/targeted-1.json") -and
                 (Test-Path "$Root/resident-SD/commands/one/targeted-1.json"))) {
        if ([DateTime]::UtcNow -gt $deadline) {
            $null = New-Item -ItemType Directory -Path "$Root/resident-O/responses/one" -Force
            [IO.File]::WriteAllText("$Root/resident-O/responses/one/targeted-1.json", '{"ok":false,"diagnostic_error":"Both jobs were not dispatched together"}')
            return
        }
        Start-Sleep -Milliseconds 10
    }
    # Complete in reverse order; the bridge must preserve request/result association.
    foreach ($lane in @('SD','O')) {
        $image = "$Root/resident-$lane/diagnosis-images/one/targeted-1.image"
        $null = New-Item -ItemType Directory -Path (Split-Path $image -Parent) -Force
        [IO.File]::WriteAllText($image, "synthetic-$lane")
        $response = "$Root/resident-$lane/responses/one/targeted-1.json"
        $null = New-Item -ItemType Directory -Path (Split-Path $response -Parent) -Force
        [IO.File]::WriteAllText($response, (@{ok=$true;image_path=$image;geometry=@{lane=$lane}} | ConvertTo-Json -Compress))
    }
}
try {
    & $module {
        param($Root)
        $jobs = @('O','SD') | ForEach-Object {
            $sessions[$_] = @{path="$Root/$_.xlsx";session="$Root/resident-$_";sequence=0;launcher=[pscustomobject]@{HasExited=$false}}
            @{kind='capture';lane=$_;sheet='Rows';range='A1:P128';output="$Root/$_.png"}
        }
        $result = Invoke-TargetedCaptureBatch @{jobs=$jobs}
        if ($result.images.Count -ne 2 -or $result.images[0].geometry.lane -ne 'O' -or $result.images[1].geometry.lane -ne 'SD') {
            throw 'Batch response identities were reordered.'
        }
        foreach ($lane in @('O','SD')) {
            if ([IO.File]::ReadAllText("$Root/$lane.png") -ne "synthetic-$lane") { throw 'Wrong image adopted.' }
        }
        $rejected = $false
        try { Invoke-TargetedCaptureBatch @{jobs=@($jobs[0],$jobs[0])} } catch { $rejected = $true }
        if (-not $rejected) { throw 'Duplicate lane was accepted.' }
    } $root
    Receive-Job $producer -Wait -ErrorAction Stop
    'Parallel submission, response association, owned images and duplicate-lane rejection verified.'
} finally {
    Stop-Job $producer -ErrorAction SilentlyContinue
    Remove-Job $producer -Force -ErrorAction SilentlyContinue
    & $module { if ($null -ne $script:responseWatcher) { $script:responseWatcher.Dispose() }; $sessions.Clear() }
    if ([IO.Path]::GetFullPath($root).StartsWith($repository + '\.tmp\', [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
