# Inserted only into the experiment's disposable resident launcher copy.
[IO.File]::WriteAllText($env:EXCEL_SHRINK_EXPERIMENT_PREPARED, '{"prepared":true}')
$barrierWait = [Diagnostics.Stopwatch]::StartNew()
while (-not [IO.File]::Exists($env:EXCEL_SHRINK_EXPERIMENT_GATE)) {
    if (Test-Path -LiteralPath (Join-Path $session 'finish.json')) { throw 'Experiment start cancelled.' }
    if ($barrierWait.Elapsed.TotalSeconds -gt 120) { throw 'Experiment start gate timed out.' }
    Start-Sleep -Milliseconds 10
}
$gate = [IO.File]::ReadAllText($env:EXCEL_SHRINK_EXPERIMENT_GATE) | ConvertFrom-Json
$targetStart = [DateTimeOffset]::Parse($gate.start_at).UtcDateTime
while ([DateTime]::UtcNow -lt $targetStart) { Start-Sleep -Milliseconds 1 }
$experimentRequestedAt = [DateTime]::UtcNow.ToString('o')
