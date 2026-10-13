$script:VVHistoryAssetRoot = $PSScriptRoot
# Offline, read-only run catalog. Run deletion remains an explicit launcher action.
function Write-VVHistory([string]$ReportRoot, $Items) {
    [IO.Directory]::CreateDirectory($ReportRoot) | Out-Null
    $runs = @()
    foreach ($item in @($Items)) {
        $summary = $null
        $warning = $null
        try { $summary = Read-VVJson (Join-Path $item.Path 'result.json') }
        catch { $warning = 'Result summary is temporarily unavailable.' }
        $status = [string]$item.Meta.status
        if ($status -eq 'running' -and -not $item.Active) { $status = 'interrupted' }
        $runs += [ordered]@{
            id = $item.Name; experiment = [string]$item.Meta.experiment; status = $status
            started_utc = $item.Meta.started_utc; finished_utc = $item.Meta.completed_utc
            arguments = $item.Meta.arguments; rollup = $summary.capture_summary
            errors = @($summary.errors | Select-Object -First 3); warning = $warning
            report = ('runs/' + $item.Name + '/report.html')
            bundle = $(if (Test-Path -LiteralPath (Join-Path $item.Path 'review-metadata.zip') -PathType Leaf) { 'runs/' + $item.Name + '/review-metadata.zip' } elseif (Test-Path -LiteralPath (Join-Path $item.Path 'review.zip') -PathType Leaf) { 'runs/' + $item.Name + '/review.zip' } else { $null })
        }
    }
    $data = [ordered]@{schema='vv-history-v2'; generated_utc=[DateTime]::UtcNow.ToString('o');runs=$runs}
    $json = ($data | ConvertTo-Json -Depth 30 -Compress).Replace('<','\u003c').Replace('>','\u003e').Replace('&','\u0026')
    $assets = Join-Path $ReportRoot 'report-assets'
    [IO.Directory]::CreateDirectory($assets) | Out-Null
    foreach ($name in @('inspector.css','history.css','history.js')) {
        Copy-Item -LiteralPath (Join-Path $script:VVHistoryAssetRoot $name) -Destination (Join-Path $assets $name) -Force
    }
    $html = @"
<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Run history · Excel Shrink</title><link rel="stylesheet" href="report-assets/inspector.css"><link rel="stylesheet" href="report-assets/history.css"></head>
<body><div id="vv-history"></div><noscript>Enable JavaScript to filter the offline run history. Reports remain under runs/.</noscript>
<script type="application/json" id="vv-history-data">$json</script><script src="report-assets/history.js"></script></body></html>
"@
    $path = Join-Path $ReportRoot 'index.html'
    [IO.File]::WriteAllText($path + '.new', $html, [Text.UTF8Encoding]::new($false))
    # A generated view only; source run metadata and artifacts are never rewritten here.
    [IO.File]::Copy($path + '.new', $path, $true)
    [IO.File]::Delete($path + '.new')
}
