param(
    [Parameter(Mandatory=$true)][string]$Output,
    [switch]$Child
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$capture = Join-Path $repository 'excel-shrink-cli/office-capture'
$directory = [IO.Path]::GetFullPath($Output)
if ($Child) {
    Add-Type -Path (Join-Path $capture 'CaptureProcesses.cs')
} else {
    Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'), (Join-Path $capture 'CaptureDesktop.cs'))
}

if (-not $Child) {
    if (Test-Path -LiteralPath $directory) { throw 'Use a new output directory.' }
    $null = New-Item -ItemType Directory -Path $directory
    & python (Join-Path $PSScriptRoot 'make_fixture.py') (Join-Path $directory 'input.xlsx')
    if ($LASTEXITCODE -ne 0) { throw 'Fixture generation failed.' }
    $desktop = New-Object CaptureDesktop
    $owned = New-Object CaptureProcesses
    $previous = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
    try {
        $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $desktop.Name
        $arguments = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -Output "{1}" -Child' -f $PSCommandPath, $directory
        $process = $desktop.Start('C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe', $arguments, $owned)
        if (-not $process.WaitForExit(150000)) { throw 'Experiment exceeded 150 seconds.' }
        if ($process.ExitCode -ne 0) { throw 'Experiment failed; inspect result.json.' }
        & python (Join-Path $PSScriptRoot 'verify.py') $directory
        if ($LASTEXITCODE -ne 0) { throw 'Pixel verification failed.' }
    } finally {
        $owned.Dispose()
        $desktop.Dispose()
        $env:EXCEL_SHRINK_EXPECTED_DESKTOP = $previous
    }
    exit
}

Import-Module (Join-Path $capture 'TargetedResident.psm1') -Force -DisableNameChecking
$owned = New-Object CaptureProcesses
$result = [ordered]@{}
$scope = $null
$excel = $null
$book = $null
$charts = $null
$opened = $false
$source = Join-Path $directory 'input.xlsx'
$before = (Get-FileHash -LiteralPath $source).Hash
try {
    $open = Open-TargetedWorkbook @{
        lane='O'; path=$source
        host_executable=(Join-Path $repository 'target/debug/excel-shrink-cli.exe')
        sheets=@(@{index=1; name='Data'; bottom_row=2; base_right_column=8; right_column=8;
            horizontal_overflow_required=$false; drawing_fallback_required=$false})
    } $owned
    $opened = $true
    $result.pid = $open.pid
    $result.office_js_charts = Invoke-TargetedJob @{kind='charts'; lane='O'}
    try {
        $result.office_js_chart_sheet = Invoke-TargetedJob @{
            kind='measure'; lane='O'; sheet='Standalone'; range='A1'
        }
    } catch { $result.office_js_chart_sheet_error = $_.Exception.Message }

    $process = [Diagnostics.Process]::GetProcessById($open.pid)
    $scope = [CaptureErrorChecking]::TryDisable($process, $env:EXCEL_SHRINK_EXPECTED_DESKTOP)
    if ($null -eq $scope) { throw 'Cannot attach to the owned Excel window.' }
    $excel = $scope.GetOwnedApplication()
    $book = $excel.ActiveWorkbook
    $charts = $book.Charts
    $result.standalone_count = $charts.Count
    $result.exports = @()
    foreach ($name in @('Standalone', 'Standalone', 'Different')) {
        $chart = $null
        try {
            $chart = $charts.Item($name)
            $chart.Activate()
            $chart.Refresh()
            Start-Sleep -Milliseconds 200
            $path = Join-Path $directory ('chart-' + $result.exports.Count + '.png')
            $exported = $chart.Export($path, 'PNG', $false)
            if (-not $exported -or -not (Test-Path -LiteralPath $path) -or
                (Get-Item -LiteralPath $path).Length -eq 0) { throw 'Chart export failed or produced an empty file.' }
            $result.exports += @{name=$name; path=$path; bytes=(Get-Item -LiteralPath $path).Length}
        } finally {
            if ($null -ne $chart) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($chart) }
        }
    }
    $result.pid_after_export = $process.Id
    $result.office_js_after_export = Invoke-TargetedJob @{kind='charts'; lane='O'}
} catch {
    $result.error = $_.Exception.Message
} finally {
    foreach ($reference in @($charts, $book, $excel)) {
        if ($null -ne $reference -and [Runtime.InteropServices.Marshal]::IsComObject($reference)) {
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($reference)
        }
    }
    if ($null -ne $scope) { $scope.Dispose() }
    if ($opened) { $result.close = Close-TargetedWorkbook 'O' }
    $owned.Dispose()
    $result.source_unchanged = $before -eq (Get-FileHash -LiteralPath $source).Hash
    $result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $directory 'result.json') -Encoding UTF8
}
if ($result.error) { exit 1 }
