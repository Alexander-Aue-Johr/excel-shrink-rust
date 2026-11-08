param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [Parameter(Mandatory=$true)][string]$CaptureRoot,
    [Parameter(Mandatory=$true)][string]$Seed,
    [Parameter(Mandatory=$true)][string]$Original,
    [Parameter(Mandatory=$true)][string]$Shrunk,
    [Parameter(Mandatory=$true)][string]$Output,
    [string]$CaptureRange,
    [switch]$ExpectDifferentPixels
)
$ErrorActionPreference = 'Stop'
if ($ExpectDifferentPixels -and -not $CaptureRange) { throw 'Different-pixel control requires CaptureRange.' }
if (Test-Path -LiteralPath $Output) { throw 'Choose a new probe output directory.' }
[void][IO.Directory]::CreateDirectory($Output)
Add-Type -Path @((Join-Path $CaptureRoot 'CaptureProcesses.cs'), (Join-Path $CaptureRoot 'CaptureDesktop.cs'))
Add-Type -Path (Join-Path $CaptureRoot 'ExcelProcessId.cs')
Add-Type -Path (Join-Path $CaptureRoot 'CaptureWorkbook.cs') -ReferencedAssemblies @('System.IO.Compression', 'System.IO.Compression.FileSystem', 'System.Xml')
Import-Module (Join-Path $CaptureRoot 'CaptureSetup.psm1') -DisableNameChecking
$utf8 = [Text.UTF8Encoding]::new($false)

function Write-ProbeJson([string]$Path, $Value) {
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 12), $utf8)
}

function Get-ProbeHash([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hasher.ComputeHash($stream)) }
    finally { $hasher.Dispose(); $stream.Dispose() }
}

function Start-WorkbookHost([string]$Directory, [string]$Name, [switch]$ProbeAccessLog) {
    $probe = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $probe.Start()
    try { $port = $probe.LocalEndpoint.Port } finally { $probe.Stop() }
    $url = 'http://localhost:' + $port
    $session = Join-Path $Directory 'session'
    Write-ProbeJson (Join-Path $session 'clients/one.json') @{
        client_id='one'; workbook=$Name; sheets=@(@{index=1;name='Sheet1';bottom_row=1;base_right_column=1})
    }
    $web = Join-Path $Directory 'web'
    Copy-Item -LiteralPath (Join-Path $CaptureRoot 'web') -Destination $web -Recurse
    $pane = Join-Path $web 'taskpane.html'
    [IO.File]::WriteAllText($pane, [IO.File]::ReadAllText($pane).Replace('__CAPTURE_RUN_ID__', [Guid]::NewGuid().ToString('N')), $utf8)
    $config = Join-Path $Directory 'host.json'
    $token = [Guid]::NewGuid().ToString('N')
    Write-ProbeJson $config @{port=$port;web=$web;session=$session;output=(Join-Path $Directory 'images');launcher_token=$token;probe_access_log=[bool]$ProbeAccessLog}
    $hostProcess = Start-Process $Binary -ArgumentList ('capture-host resident ' + (Quote-Argument $config)) -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $Directory 'host.stdout.log') -RedirectStandardError (Join-Path $Directory 'host.stderr.log')
    $owned.Add($hostProcess)
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.Elapsed.TotalSeconds -lt 20) {
        if ($hostProcess.HasExited) { throw 'Probe HTTP host exited.' }
        try {
            $health = Invoke-RestMethod ($url + '/health') -TimeoutSec 2
            if ($health.ok) { return @{url=$url;process=$hostProcess;directory=$Directory;session=$session;headers=@{'X-Launcher-Token'=$token}} }
        } catch [Net.WebException] { }
        Start-Sleep -Milliseconds 100
    }
    throw 'Probe HTTP host did not become ready.'
}

function Prepare-ProbeWorkbook([string]$Source, [string]$Path, $HostInfo) {
    $guid = $script:probeAddinGuid
    [xml]$manifest = [IO.File]::ReadAllText((Join-Path $CaptureRoot 'web/manifest.xml')).Replace('http://localhost:3000', $HostInfo.url)
    $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Id']").InnerText = $guid
    foreach ($node in $manifest.SelectNodes('//*[@DefaultValue]')) {
        $url = $node.GetAttribute('DefaultValue')
        if ($url.StartsWith($HostInfo.url + '/')) { $node.SetAttribute('DefaultValue', ($url + '?client=one')) }
    }
    $manifestPath = Join-Path $HostInfo.directory 'manifest.xml'
    [IO.File]::WriteAllText($manifestPath, $manifest.OuterXml, $utf8)
    Copy-Item -LiteralPath $Source -Destination $Path
    $version = $manifest.SelectSingleNode("/*[local-name()='OfficeApp']/*[local-name()='Version']").InnerText.Trim()
    Add-AutoOpenTaskpane $Path $guid $version
    Set-InitialWorksheet $Path 1 $true $true
    if (-not $registrations.ContainsKey($guid)) {
        $registrations[$guid] = Register-AddIn $guid $manifestPath
    }
}

function Wait-ProbeOfficeReady($HostInfo) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.Elapsed.TotalSeconds -lt 60) {
        if ($process.HasExited) { throw 'Owned Excel exited during add-in initialization.' }
        $health = Invoke-RestMethod ($HostInfo.url + '/health') -TimeoutSec 3
        if (@($health.errors).Count) { throw ($health.errors | ConvertTo-Json -Compress) }
        if ($health.clients.one.ready_at) { return $health.clients.one }
        Start-Sleep -Milliseconds 100
    }
    throw 'Office.js readiness timed out after workbook switch.'
}

function Set-PixelProbePage($HostInfo, [string]$Range) {
    $rangeJson = ConvertTo-Json -InputObject $Range -Compress
    $page = @'
<!doctype html>
<html><head><meta charset="utf-8"><title>Workbook switch pixel probe</title>
<script src="https://officeapis.public.onecdn.static.microsoft/1/office.js"></script>
</head><body>Workbook switch pixel probe<script>
Office.onReady(async () => {
  const documentId = crypto.randomUUID();
  const config = await fetch(`/config?client=one&document=${documentId}&run=${crypto.randomUUID()}&request=${crypto.randomUUID()}`)
    .then(response => response.json());
  const captured = await Excel.run(async context => {
    context.workbook.load('name');
    const picture = context.workbook.worksheets.getFirst().getRange(__RANGE__).getImage();
    await context.sync();
    return {workbook: context.workbook.name, png: picture.value};
  });
  const image = new Image();
  image.src = `data:image/png;base64,${captured.png}`;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = image.naturalWidth;
  canvas.height = image.naturalHeight;
  const drawing = canvas.getContext('2d');
  drawing.drawImage(image, 0, 0);
  const pixels = drawing.getImageData(0, 0, canvas.width, canvas.height).data;
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', pixels));
  const rgbaSha256 = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
  const response = await fetch('/ready?client=one', {
    method: 'POST',
    headers: {'Content-Type': 'application/json', 'X-Client-Token': config.client_token},
    body: JSON.stringify({workbook: captured.workbook, startup: {
      pixel_probe: {range: __RANGE__, width: canvas.width, height: canvas.height, rgba_sha256: rgbaSha256}
    }})
  });
  if (!response.ok) throw new Error(`Ready failed with HTTP ${response.status}`);
});
</script></body></html>
'@
    $page = $page.Replace('__RANGE__', $rangeJson)
    [IO.File]::WriteAllText((Join-Path $HostInfo.directory 'web/taskpane.html'), $page, $utf8)
}

$owned = New-Object CaptureProcesses
$desktop = New-Object CaptureDesktop
$registrations = @{}
$checking = $null
$excel = $null
$workbook = $null
$process = $null
$samples = @()
$script:probeAddinGuid = [Guid]::NewGuid().ToString()
$hashes = @((Get-ProbeHash $Original), (Get-ProbeHash $Shrunk))
try {
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $Output 'profile'
    $seedCopy = Join-Path $Output 'seed.xlsx'
    Copy-Item -LiteralPath $Seed -Destination $seedCopy
    $startup = [Diagnostics.Stopwatch]::StartNew()
    $process = $desktop.Start((Find-ExcelExecutable), ('/x ' + (Quote-Argument $seedCopy)), $owned)
    while ($null -eq $checking -and $startup.Elapsed.TotalSeconds -lt 60) {
        $checking = [CaptureErrorChecking]::TryDisable($process, $desktop.Name)
        if ($null -eq $checking) { Start-Sleep -Milliseconds 100 }
    }
    if ($null -eq $checking) { throw 'Owned Excel did not expose its automation window.' }
    $excel = $checking.GetOwnedApplication()
    $excel.Visible = $true
    $excel.DisplayAlerts = $false
    $excel.AutomationSecurity = 3
    $excel.EnableEvents = $false
    $excel.AutoRecover.Enabled = $false
    $excel.Calculation = -4135
    $startupSeconds = $startup.Elapsed.TotalSeconds
    $index = 0
    $hostInfo = Start-WorkbookHost (Join-Path $Output 'host') 'case-0.xlsx'
    if ($CaptureRange) { Set-PixelProbePage $hostInfo $CaptureRange }
    foreach ($source in @($Original, $Shrunk, $Original, $Shrunk)) {
        $directory = Join-Path $Output ('request-' + $index)
        [void][IO.Directory]::CreateDirectory($directory)
        $name = 'case-' + $index + '.xlsx'
        if ($index -gt 0) {
            # This readiness-only probe dispatches no data jobs. Existing reset
            # rules reject this operation after comparison work has started.
            Invoke-RestMethod -Method Post ($hostInfo.url + '/launcher/reset/one') -Headers $hostInfo.headers -ContentType 'application/json' -Body '{"stage":"prepare"}' -TimeoutSec 5 | Out-Null
            Write-ProbeJson (Join-Path $hostInfo.session 'clients/one.json') @{
                client_id='one';workbook=$name;sheets=@(@{index=1;name='Sheet1';bottom_row=1;base_right_column=1})
            }
            Invoke-RestMethod -Method Post ($hostInfo.url + '/launcher/reset/one') -Headers $hostInfo.headers -ContentType 'application/json' -Body '{"stage":"ready"}' -TimeoutSec 5 | Out-Null
        }
        $book = Join-Path $directory $name
        Prepare-ProbeWorkbook $source $book $hostInfo
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $workbook = $excel.Workbooks.Open($book, 0, $true, [Type]::Missing, '', '', $true)
        $openSeconds = $clock.Elapsed.TotalSeconds
        [uint32]$ownerId = 0
        [void][ExcelProcessId]::GetWindowThreadProcessId([IntPtr]$excel.Hwnd, [ref]$ownerId)
        if ($ownerId -ne $process.Id) { throw 'Workbook switched to another Excel process.' }
        $ready = Wait-ProbeOfficeReady $hostInfo
        $samples += @{index=$index;pid=$process.Id;com_open_seconds=$openSeconds;ready_seconds=$clock.Elapsed.TotalSeconds;observation=$ready}
        Write-ProbeJson (Join-Path $Output 'samples.json') @{startup_seconds=$startupSeconds;samples=$samples}
        $workbook.Close($false)
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($workbook)
        $workbook = $null
        if ($excel.Workbooks.Count -ne 1) { throw 'Unexpected workbook remained after switch.' }
        $index++
    }
} finally {
    if ($null -ne $workbook) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($workbook) }
    if ($null -ne $checking) { $checking.Dispose() }
    if ($null -ne $excel) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($excel) }
    $owned.Dispose()
    if ($null -ne $process -and -not $process.WaitForExit(10000)) { throw 'Owned Excel exit unconfirmed.' }
    foreach ($guid in $registrations.Keys) { Restore-AddIn $guid $registrations[$guid] }
    $desktop.Dispose()
}
if (-not $process.WaitForExit(10000)) { throw 'Owned Excel exit unconfirmed.' }
if ((Get-ProbeHash $Original) -ne $hashes[0] -or
    (Get-ProbeHash $Shrunk) -ne $hashes[1]) { throw 'Probe source changed.' }
if (@($samples | ForEach-Object { $_.observation.runtime.document_id } | Select-Object -Unique).Count -ne 4) {
    throw 'Each opened workbook must have a distinct Office.js document identity.'
}
if ($CaptureRange) {
    foreach ($sample in $samples) {
        $pixels = $sample.observation.startup_diagnostics.pixel_probe
        if ($pixels.range -ne $CaptureRange -or $pixels.width -le 0 -or $pixels.height -le 0 -or
            $pixels.rgba_sha256 -notmatch '^[a-f0-9]{64}$') {
            throw 'Missing or invalid workbook-switch pixel observation.'
        }
    }
    foreach ($pair in @(@(0,2), @(1,3))) {
        $before = $samples[$pair[0]].observation.startup_diagnostics.pixel_probe
        $after = $samples[$pair[1]].observation.startup_diagnostics.pixel_probe
        if ($before.width -ne $after.width -or $before.height -ne $after.height -or
            $before.rgba_sha256 -ne $after.rgba_sha256) {
            throw 'Repeated capture pixels differed for the same source workbook.'
        }
    }
    if ($ExpectDifferentPixels -and
        $samples[0].observation.startup_diagnostics.pixel_probe.rgba_sha256 -eq
        $samples[1].observation.startup_diagnostics.pixel_probe.rgba_sha256) {
        throw 'The intentionally different workbook rendered the same pixels.'
    }
}
Write-ProbeJson (Join-Path $Output 'result.json') @{samples=$samples;startup_seconds=$startupSeconds;owned_excel_exit_confirmed=$true;source_hashes_unchanged=$true}
