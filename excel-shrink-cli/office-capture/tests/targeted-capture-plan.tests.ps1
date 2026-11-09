$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../TargetedCapturePlan.psm1') -Force
$root = Join-Path ([IO.Path]::GetTempPath()) ('capture-plan-test-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$path = Join-Path $root 'plan.jsonl'
try {
    foreach ($range in @('A1:P1048576', 'A1:T617', 'A1:XFD1048576')) {
        $lines = @(
            (@{version=1;stage='before_geometry_partition';total=1}|ConvertTo-Json -Compress),
            (@{sheet='Data';range=$range}|ConvertTo-Json -Compress)
        )
        [IO.File]::WriteAllLines($path, $lines)
        $failure = Get-TargetedFullGridPlanFailure $path
        if ($range -eq 'A1:XFD1048576') {
            if ($failure.kind -ne 'unbounded_capture_plan' -or $failure.coverage -ne 'unverified') {
                throw 'Full-grid planning must retain an explicit unverified failure.'
            }
        } elseif ($null -ne $failure) { throw 'A bounded-column or finite plan was rejected.' }
    }
    [IO.File]::WriteAllText($path, '{"version":1,"stage":"after_geometry_partition","total":0}')
    if ($null -ne (Get-TargetedFullGridPlanFailure $path)) { throw 'An unrelated plan stage was rejected.' }
    'Targeted full-grid plan tests passed.'
} finally {
    $resolved = [IO.Path]::GetFullPath($root)
    if (-not $resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolved) -notlike 'capture-plan-test-*') { throw 'Unsafe test cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
