[CmdletBinding()]
param([switch]$Run)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$captureRoot = Split-Path -Parent $PSScriptRoot
Add-Type -Path @((Join-Path $captureRoot 'CaptureProcesses.cs'), (Join-Path $captureRoot 'CaptureDesktop.cs'))
Import-Module (Join-Path $captureRoot 'CaptureSetup.psm1') -Force -DisableNameChecking

function Assert([bool]$condition, [string]$message) {
    if (-not $condition) { throw $message }
}

$rejected = $false
try { $null = [CaptureErrorChecking]::TryDisable([Diagnostics.Process]::GetCurrentProcess()) }
catch { $rejected = $_.Exception.Message -like '*owned Excel processes*' }
Assert $rejected 'A non-Excel process must never be attached to.'

if (-not $Run) {
    'PASS: owned-process guard. Pass -Run for two private Excel instances and error-cell preservation.'
    return
}

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root = Join-Path ([IO.Path]::GetTempPath()) ('capture-indicators-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$owned = [CaptureProcesses]::new()
$desktop = [CaptureDesktop]::new()
$processes = @()
$scopes = @()
$applications = @()
$snapshots = @()
try {
    $parts = @{
        '[Content_Types].xml' = '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>'
        '_rels/.rels' = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>'
        'xl/workbook.xml' = '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView activeTab="0"/></bookViews><sheets><sheet name="Indicators" sheetId="1" r:id="rId1"/></sheets><calcPr calcMode="manual"/></workbook>'
        'xl/_rels/workbook.xml.rels' = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>'
        'xl/worksheets/sheet1.xml' = '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:B2"/><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>123</t></is></c><c r="B1" t="e"><f>#REF!</f><v>#REF!</v></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2" t="e"><f>1/0</f><v>#DIV/0!</v></c></row></sheetData></worksheet>'
    }
    foreach ($index in 1, 2) {
        $source = Join-Path $root ('fixture-' + $index + '.xlsx')
        $zip = [IO.Compression.ZipFile]::Open($source, [IO.Compression.ZipArchiveMode]::Create)
        try {
            foreach ($name in $parts.Keys) {
                $entry = $zip.CreateEntry($name)
                $writer = [IO.StreamWriter]::new($entry.Open(), [Text.UTF8Encoding]::new($false))
                try { $writer.Write($parts[$name]) } finally { $writer.Dispose() }
            }
        } finally { $zip.Dispose() }
        $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
        $process = $desktop.Start((Find-ExcelExecutable), ('/x ' + (Quote-Argument $source)), $owned)
        $processes += @($process)
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $scope = $null
        while ($null -eq $scope -and $clock.Elapsed.TotalSeconds -lt 60) {
            $scope = [CaptureErrorChecking]::TryDisable($process, $desktop.Name)
            if ($null -eq $scope) { Start-Sleep -Milliseconds 100 }
        }
        Assert ($null -ne $scope) 'The owned private Excel instance never exposed its native window.'
        $scopes += @($scope)
        Assert ($scope.ProcessId -eq $process.Id -and $scope.IsDisabled) 'The wrong Excel process was configured.'

        # Access the scoped COM reference only for behavioral assertions. There is
        # deliberately no public global-Excel lookup in the production helper.
        $field = [CaptureErrorChecking].GetField('options', [Reflection.BindingFlags]'NonPublic,Instance')
        $application = $field.GetValue($scope).Application
        $applications += @($application)
        $sheet = $application.Workbooks.Item(1).Worksheets.Item('Indicators')
        $ref = $sheet.Range('B1')
        $division = $sheet.Range('B2')
        try {
            Assert ($ref.Formula -eq '=#REF!') 'Suppressing an indicator must not repair a #REF formula.'
            Assert ($division.Formula -eq '=1/0') 'Suppressing an indicator must not change a division error.'
            Assert ($ref.Text.StartsWith('#') -and $division.Text.StartsWith('#')) 'Real error text must remain visible.'
            $snapshots += @(@{ source=$source; hash=$sourceHash; ref=$ref.Text; division=$division.Text })
        } finally {
            foreach ($item in @($ref, $division, $sheet)) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($item) }
        }
    }
    $scopes[0].Dispose()
    Assert ($applications[0].ErrorCheckingOptions.BackgroundChecking -eq $scopes[0].PreviousBackgroundChecking) 'First instance preference was not restored.'
    Assert (-not $applications[1].ErrorCheckingOptions.BackgroundChecking) 'Restoring one Excel instance changed another.'
    $scopes[1].Dispose()
    Assert ($applications[1].ErrorCheckingOptions.BackgroundChecking -eq $scopes[1].PreviousBackgroundChecking) 'Second instance preference was not restored.'
    foreach ($index in 0, 1) {
        $sheet = $applications[$index].Workbooks.Item(1).Worksheets.Item('Indicators')
        Assert ($sheet.Range('B1').Text -eq $snapshots[$index].ref) 'Restoring the preference changed #REF error text.'
        Assert ($sheet.Range('B2').Text -eq $snapshots[$index].division) 'Restoring the preference changed division error text.'
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($sheet)

    }
    foreach ($process in $processes) { [void][CaptureProcesses]::Stop($process) }
    foreach ($snapshot in $snapshots) {
        Assert ((Get-FileHash -LiteralPath $snapshot.source -Algorithm SHA256).Hash -eq $snapshot.hash) 'Capture configuration modified the input file.'
    }
    'PASS: two owned private Excel instances; independent suppression/restoration; real error text and inputs preserved.'
} finally {
    foreach ($scope in $scopes) { try { $scope.Dispose() } catch { Write-Warning $_.Exception.Message } }
    foreach ($application in $applications) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($application) }
    foreach ($process in $processes) { [void][CaptureProcesses]::Stop($process) }
    $owned.Dispose()
    $desktop.Dispose()
    $resolved = [IO.Path]::GetFullPath($root)
    if ([IO.Path]::GetDirectoryName($resolved).TrimEnd('\') -ne [IO.Path]::GetTempPath().TrimEnd('\') -or
        [IO.Path]::GetFileName($resolved) -notlike 'capture-indicators-*') { throw 'Unsafe fixture cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
}
