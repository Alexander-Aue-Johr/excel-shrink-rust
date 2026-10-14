# Pure PowerShell checks; no Excel, WSL, native host or report mutation required.
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '..\_shared\suite\plan.ps1')
$config=Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\suite.json') -Raw | ConvertFrom-Json
function Assert($ok,$message){if(-not $ok){throw $message}}
$p=Get-VVSuitePlan $config recommended
Assert ($p.jobs.Count -eq 16) 'Recommended suite must have four warm-ups and twelve measured cases.'
Assert (@($p.jobs|Where-Object{$_.phase -eq 'warmup'}).Count -eq 4) 'Warm-up count mismatch.'
Assert (@($p.jobs.id|Select-Object -Unique).Count -eq 16) 'Case IDs must be unique.'
Assert ($p.jobs[4].candidate_id -ne $p.jobs[8].candidate_id) 'Measured order must rotate.'
$i=Get-VVSuitePlan $config inventory
Assert ($i.jobs.Count -eq 18) 'Inventory should invoke eighteen explicit cases, not a Cartesian product.'
Assert (@($i.jobs|Where-Object{$_.candidate.kind -eq 'shape-native'}).Count -eq 1) 'Native base group must not run four times.'
Assert (@($i.jobs|Where-Object{$_.candidate.kind -eq 'diagnostic'}).Count -eq 4) 'Diagnostic coverage mismatch.'
$d=Get-VVSuitePlan $config diagnostics
Assert ($d.jobs.Count -eq 4) 'Diagnostics-only profile mismatch.'
$config.capture.batch_size=64
$rejected=$false;try{Get-VVSuitePlan $config recommended|Out-Null}catch{$rejected=$true}
Assert $rejected 'A logical batch sweep must not masquerade as concurrent image requests.'
Write-Host '9 PowerShell plan assertions passed; native worker execution is not tested.'
