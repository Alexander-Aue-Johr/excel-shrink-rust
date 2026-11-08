$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
$openingPath = Join-Path $captureRoot 'resident/Opening.ps1'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $captureRoot 'resident.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Resident launcher has syntax errors.' }
[void][Management.Automation.Language.Parser]::ParseFile($openingPath, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Opening steps have syntax errors.' }
$function = $ast.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Open-ResidentClient'
}, $false)
Invoke-Expression $function.Extent.Text

function Assert($condition, [string]$message) {
    if (-not $condition) { throw $message }
}

$events = New-Object 'Collections.Generic.List[string]'
$script:openingSeconds = 0
$otherOpeningSeconds = 0
$script:failurePhase = ''
$script:existing = $false
function Initialize-ResidentClientRequest($Spec, $VisibilityOverride) {
    $events.Add('initialize')
    if (-not $existing) { return @{id='one';source='fixture.xlsx'} }
}
function Prepare-ResidentClientWorkbook($Spec, $Request) {
    $events.Add('prepare')
    return @{path='staged.xlsx';registration='fixture'}
}
function Start-ResidentClientExcel($Id, $Workbook, $VisibilityOverride) {
    $events.Add('start')
    Assert ($Id -eq 'one' -and $Workbook.path -eq 'staged.xlsx') 'Prepared identity was lost.'
    if ($failurePhase -eq 'start') { throw 'fixture failure' }
}
function Wait-ResidentClientAddIn($Id) {
    $events.Add('wait')
    if ($failurePhase -eq 'wait') { throw 'fixture failure' }
}
function Update-OtherGroupPause { $events.Add('account') }
function Get-OwnOpeningSeconds { return 2 }

Open-ResidentClient @{}
Assert (($events -join ',') -eq 'initialize,prepare,start,wait,account') 'Opening order changed.'
Assert ($openingSeconds -eq 2) 'Opening duration was not accumulated.'
foreach ($phase in @('start', 'wait')) {
    $events.Clear()
    $script:failurePhase = $phase
    $failed = $false
    try { Open-ResidentClient @{} } catch {
        Assert ($_.Exception.Message -eq 'fixture failure') 'Failure context changed.'
        $failed = $true
    }
    Assert $failed 'A startup failure was swallowed.'
    Assert ($events[$events.Count - 1] -eq 'account') 'Failure bypassed opening-time accounting.'
}
$events.Clear()
$script:existing = $true
Open-ResidentClient @{}
Assert (($events -join ',') -eq 'initialize') 'An existing client was opened again.'

# Exercise the extracted wait against a controlled add-in registration.
. $openingPath
$script:resourceUpdates = 0
$script:sleeps = 0
$script:registerAfterUpdate = $true
$clients = @{one=@{seen=$false;open_clock=[Diagnostics.Stopwatch]::StartNew()}}
$metrics = @{clients=@{one=@{status='opening';stage='waiting_for_addin'}}}
$session = Join-Path ([IO.Path]::GetTempPath()) ('absent-session-' + [Guid]::NewGuid().ToString('N'))
function Update-Resources {
    $script:resourceUpdates++
    if ($registerAfterUpdate -and $resourceUpdates -eq 2) { $clients.one.seen = $true }
}
function Read-HostHealth { return @{} }
function Test-ResidentDeadline { return -not $registerAfterUpdate }
function Start-Sleep { param([int]$Milliseconds) $script:sleeps++ }
function Write-ResourceMetrics { }
Wait-ResidentClientAddIn -Id one
Assert ($resourceUpdates -eq 2 -and $sleeps -eq 1) 'Add-in readiness polling changed.'
Assert ($metrics.clients.one.status -eq 'ready') 'Registered add-in did not reach ready.'
$clients.one.seen = $false
$script:registerAfterUpdate = $false
$failed = $false
try { Wait-ResidentClientAddIn -Id one } catch {
    Assert ($_.Exception.Message -like 'Resident Office add-in startup timeout:*') 'Timeout context changed.'
    $failed = $true
}
Assert $failed 'A missing add-in bypassed the opening deadline.'
Write-Output 'PASS resident opening order, failures, duplicate clients and add-in readiness'
