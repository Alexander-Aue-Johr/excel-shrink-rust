param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$CaptureRoot,
    [Parameter(Mandatory=$true)][string]$InputWorkbook,
    [Parameter(Mandatory=$true)][string]$Output,
    [ValidateSet('FileSystem','Registry')][string]$Catalog = 'FileSystem',
    [switch]$WorkbookRelationship
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $Output) { throw 'Choose a new probe directory.' }
[void][IO.Directory]::CreateDirectory($Output)
$utf8 = [Text.UTF8Encoding]::new($false)
Add-Type -Path @((Join-Path $CaptureRoot 'CaptureProcesses.cs'), (Join-Path $CaptureRoot 'CaptureDesktop.cs'))
Add-Type -Path (Join-Path $CaptureRoot 'TargetedOpeningDialogs.cs')
Import-Module (Join-Path $CaptureRoot 'CaptureSetup.psm1') -DisableNameChecking

# Reuse only the probe's JSON, hash and HTTP startup helpers, without executing it.
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'office-ready.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Invalid probe helper script.' }
foreach ($function in $ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst]}, $false)) {
    if ($function.Name -in @('Write-ProbeJson','Get-ProbeHash','Start-WorkbookHost')) {
        Invoke-Expression $function.Extent.Text
    }
}
$owned = New-Object CaptureProcesses
$desktop = New-Object CaptureDesktop
$registered = $false
$process = $null
$id = [Guid]::NewGuid().ToString()
$sourceHash = Get-ProbeHash $InputWorkbook
try {
    $hostInfo = Start-WorkbookHost (Join-Path $Output 'host') 'hello.xlsx' -ProbeAccessLog
    $html = @'
<!doctype html>
<html><head><meta charset="utf-8"><title>Hello probe</title>
<script>
const origin = performance.timeOrigin;
const documentId = crypto.randomUUID();
const configPromise = fetch(`/config?client=one&document=${documentId}&run=${crypto.randomUUID()}&request=${crypto.randomUUID()}`)
  .then(response => response.json());
</script>
<script src="https://officeapis.public.onecdn.static.microsoft/1/office.js"></script>
</head><body>Hello World<script>
Office.onReady(async () => {
  const config = await configPromise;
  const workbook = await Excel.run(async context => {
    context.workbook.load('name');
    await context.sync();
    return context.workbook.name;
  });
  await fetch('/ready?client=one', {
    method: 'POST',
    headers: {'Content-Type': 'application/json', 'X-Client-Token': config.client_token},
    body: JSON.stringify({workbook, startup: {time_origin_ms: origin, hello_ms: performance.now()}})
  });
});
</script></body></html>
'@
    [IO.File]::WriteAllText((Join-Path $hostInfo.directory 'web/taskpane.html'), $html, $utf8)
    [xml]$manifest = [IO.File]::ReadAllText((Join-Path $CaptureRoot 'web/manifest.xml')).Replace('http://localhost:3000', $hostInfo.url)
    $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Id']").InnerText = $id
    $catalogPath = Join-Path $Output 'catalog'
    [void][IO.Directory]::CreateDirectory($catalogPath)
    $manifestPath = Join-Path $catalogPath 'manifest.xml'
    [IO.File]::WriteAllText($manifestPath, $manifest.OuterXml, $utf8)
    $store = $catalogPath
    if ($Catalog -eq 'Registry') {
        $previous = Register-AddIn $id $manifestPath
        $registered = $true
        $store = 'developer'
    }
    $workbook = Join-Path $Output 'hello.xlsx'
    $settings = Join-Path $Output 'inject.json'
    Write-ProbeJson $settings @{input=$InputWorkbook;output=$workbook;id=$id;version='1.0.0.0';store=$store;store_type=$Catalog;workbook_relationship=[bool]$WorkbookRelationship}
    $preparation = [Diagnostics.Stopwatch]::StartNew()
    & $Binary capture-host inject-addin-probe $settings
    if ($LASTEXITCODE -ne 0) { throw 'Rust injection failed.' }
    $prepareSeconds = $preparation.Elapsed.TotalSeconds
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $Output 'profile'
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $helloSeconds = $null
    $process = $desktop.Start((Find-ExcelExecutable), ('/x ' + (Quote-Argument $workbook)), $owned)
    $seen = $null
    $ready = $null
    while ($clock.Elapsed.TotalSeconds -lt 30) {
        if ($process.HasExited) { throw 'Probe Excel exited.' }
        $health = Invoke-RestMethod ($hostInfo.url + '/health') -TimeoutSec 2
        if ($health.clients.one) {
            $seen = $health.clients.one
            if ($seen.ready_at) {
                $ready = $seen
                $helloSeconds = $clock.Elapsed.TotalSeconds
                break
            }
        }
        Start-Sleep -Milliseconds 100
    }
    $result = @{catalog=$Catalog;registry_registration_performed=$registered;prepare_seconds=$prepareSeconds;hello_received=($null -ne $ready);client=$seen;excel_started_at=$process.StartTime.ToUniversalTime().ToString('o')}
    $result.native_dialogs = @([TargetedOpeningDialogs]::Inspect($process.Id, $desktop.Name))
    if ($ready) {
        # A monotonic timer avoids culture-dependent conversion of JSON timestamps.
        $result.hello_observed_seconds = $helloSeconds
    }
    Write-ProbeJson (Join-Path $Output 'measurement.json') $result
} finally {
    $owned.Dispose()
    if ($null -ne $process -and -not $process.WaitForExit(10000)) { throw 'Probe Excel exit unconfirmed.' }
    if ($registered) { Restore-AddIn $id $previous }
    $desktop.Dispose()
}
if ((Get-ProbeHash $InputWorkbook) -ne $sourceHash) { throw 'Probe input changed.' }
Write-ProbeJson (Join-Path $Output 'cleanup.json') @{owned_excel_exit_confirmed=$true;source_hash_unchanged=$true;registration_restored=$registered}
