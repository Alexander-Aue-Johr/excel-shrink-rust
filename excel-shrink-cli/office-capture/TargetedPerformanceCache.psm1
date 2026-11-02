$ErrorActionPreference = 'Stop'

function Get-TargetedCacheEnvironment {
    <# .SYNOPSIS
    Identifies Excel and the generation recipe without another Excel process.
    #>
    Import-Module (Join-Path $PSScriptRoot 'CaptureSetup.psm1') -DisableNameChecking
    $excel = Get-Item -LiteralPath (Find-ExcelExecutable)
    $recipe = @('check-performance-native-v1')
    $files = @(Get-ChildItem -LiteralPath $PSScriptRoot -File | Where-Object Extension -in @('.ps1', '.psm1', '.cs'))
    $files += @(Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'web') -File -Recurse | Where-Object Extension -eq '.js')
    foreach ($file in ($files | Sort-Object FullName)) {
        $recipe += $file.FullName.Substring($PSScriptRoot.Length) + ':' + (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($recipe -join "`n")))
        $recipeHash = [BitConverter]::ToString($digest).Replace('-', '').ToLowerInvariant()
    } finally { $sha.Dispose() }
    return @{
        excel_sha256=(Get-FileHash -LiteralPath $excel.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        excel_version=$excel.VersionInfo.FileVersion
        excel_path=$excel.FullName
        machine=$env:COMPUTERNAME
        user=$env:USERNAME
        culture=[Globalization.CultureInfo]::CurrentCulture.Name
        ui_culture=[Globalization.CultureInfo]::CurrentUICulture.Name
        os_version=[Environment]::OSVersion.VersionString
        generation_recipe=$recipeHash
        calculation='manual; explicit full rebuild only'
        window_mode='owned-private-desktop'
    }
}

function Get-TargetedWorkbookMeasurements {
    <# .SYNOPSIS
    Retains compact existing launcher observations, without an additional open.
    Peaks are observed through this sample, not a full-lifetime maximum.
    #>
    param([string]$Session)
    $path = Join-Path $Session 'metrics.json'
    $metrics = [IO.File]::ReadAllText($path) | ConvertFrom-Json
    if ($null -eq $metrics.clients.one) { throw 'Workbook measurements are unavailable.' }
    $client = $metrics.clients.one
    $sample = @{
        sampled_at=[DateTime]::UtcNow.ToString('o')
        machine=$env:COMPUTERNAME
        method='existing-resident-launcher-snapshot-v1'
        peak_scope='observed through sample; not a full-lifetime peak'
    }
    foreach ($name in @('pid','process_started_unix_ms','requested_at','opened_at','ready_at',
        'startup_seconds','startup_measurement_method','startup_sampled_unix_ms',
        'startup_memory_private_bytes','startup_memory_working_set_bytes',
        'working_set_bytes','private_bytes','peak_private_bytes','peak_working_set_bytes')) {
        $sample[$name] = $client.$name
    }
    return $sample
}

Export-ModuleMember -Function Get-TargetedCacheEnvironment, Get-TargetedWorkbookMeasurements
