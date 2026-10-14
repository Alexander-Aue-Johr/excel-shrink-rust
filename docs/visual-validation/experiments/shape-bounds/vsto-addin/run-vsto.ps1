param(
    [Parameter(Mandatory=$true)]
    [string]$FileListPath,

    [Parameter(Mandatory=$true)]
    [string]$OutputPath,

    [switch]$SkipIfUnavailable
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '..\..\_shared\reporting.ps1')
$vvReportOwner = Start-VVReport -Start $PSScriptRoot -Experiment 'shape-bounds-vsto' -Arguments $PSBoundParameters
try {
$ErrorActionPreference = 'Stop'
if (-not $OutputPath.StartsWith($env:VV_REPORT_DIR,[StringComparison]::OrdinalIgnoreCase)) {
    $step=New-VVStep 'vsto'; $OutputPath=Join-Path $step 'result.json'
}
New-Item -ItemType Directory -Path (Split-Path -Parent $OutputPath) -Force | Out-Null
$ProgId = 'ExcelShrink.DrawingBoundsVstoExperiment' 
$StatusPath = Join-Path (Split-Path -Parent $OutputPath) 'vsto-status.json'
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Write-Status([string]$Status, [string]$Message) {
    $data = [ordered]@{ status=$Status; message=$Message }
    [IO.File]::WriteAllText($StatusPath, (($data | ConvertTo-Json -Depth 4) + "`n"), $Utf8NoBom)
}

function Find-MsBuildWithVsto {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { return $null }

    foreach ($component in @(
        'Microsoft.VisualStudio.Component.TeamOffice.BuildTools',
        'Microsoft.VisualStudio.Component.TeamOffice'
    )) {
        $candidate = @(& $vswhere -latest -products * -requires $component -find 'MSBuild\**\Bin\MSBuild.exe' 2>$null) | Select-Object -First 1
        if ($candidate) { return [string]$candidate }
    }

    return $null
}

$msbuild = Find-MsBuildWithVsto
if ($null -eq $msbuild) {
    $message = 'VSTO build tools are not installed. Install the Visual Studio Office/SharePoint development workload or the Office/SharePoint Build Tools workload with the VSTO component.'
    Write-Status 'skipped' $message
    if ($SkipIfUnavailable) {
        Write-Host "C# VSTO: SKIPPED - $message"
        return
    }
    throw $message
}

$projectRoot = $PSScriptRoot
$tempRoot = Join-Path $env:TEMP ('drawing-bounds-vsto-' + [Guid]::NewGuid().ToString('N'))
$stage = Join-Path $tempRoot 'src'
$registryPath = "HKCU:\Software\Microsoft\Office\Excel\Addins\$ProgId"
$excel = $null
$addin = $null

New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item -Path (Join-Path $projectRoot '*') -Destination $stage -Recurse -Force

try {
    $project = Join-Path $stage 'DrawingBoundsVsto.csproj'
    & $msbuild $project /t:Rebuild /p:Configuration=Release /p:Platform=AnyCPU /nologo /verbosity:minimal
    if ($LASTEXITCODE -ne 0) {
        throw "VSTO build failed with exit code $LASTEXITCODE"
    }

    $manifest = Get-ChildItem -LiteralPath $stage -Recurse -File -Filter '*.vsto' | Select-Object -First 1
    if ($null -eq $manifest) {
        throw "VSTO build completed but no .vsto deployment manifest was found below: $stage"
    }

    if (Test-Path -LiteralPath $registryPath) {
        throw "Refusing to overwrite an existing VSTO registration: $registryPath"
    }

    New-Item -Path $registryPath -Force | Out-Null
    New-ItemProperty -Path $registryPath -Name Description -Value 'Excel Shrink drawing bounds VSTO experiment' -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $registryPath -Name FriendlyName -Value 'Excel Shrink Drawing Bounds VSTO Experiment' -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $registryPath -Name LoadBehavior -Value 3 -PropertyType DWord -Force | Out-Null
    $manifestUri = ([Uri]$manifest.FullName).AbsoluteUri + '|vstolocal'
    New-ItemProperty -Path $registryPath -Name Manifest -Value $manifestUri -PropertyType String -Force | Out-Null

    $excel = New-Object -ComObject Excel.Application

    if(Get-Command Register-VVDecisionExcel -ErrorAction SilentlyContinue){Register-VVDecisionExcel $excel}
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $excel.EnableEvents = $false

    $addin = $excel.COMAddIns.Item($ProgId)
    $addin.Connect = $true

    $automation = $null
    for ($attempt = 0; $attempt -lt 50 -and $null -eq $automation; $attempt++) {
        Start-Sleep -Milliseconds 100
        $automation = $addin.Object
    }

    if ($null -eq $automation) {
        throw 'VSTO add-in loaded but did not expose its COM automation service.'
    }

    $result = $automation.RunDrawingBoundsBatch($FileListPath, $OutputPath)
    if ("$result" -ne 'OK') {
        throw "VSTO experiment failed: $result"
    }

    if (-not (Test-Path -LiteralPath $OutputPath)) {
        throw "VSTO experiment completed without producing: $OutputPath"
    }

    Write-Status 'ok' 'VSTO experiment completed.'
    Write-Host 'C# VSTO result:'
    Write-Host "  $OutputPath"
}
catch {
    Write-Status 'error' $_.Exception.Message
    throw
}
finally {
    if ($null -ne $addin) {
        try { $addin.Connect = $false } catch {}
    }
    if ($null -ne $excel) {
        try { $excel.Quit() } catch {}
    }
    if (Test-Path -LiteralPath $registryPath) {
        Remove-Item -LiteralPath $registryPath -Recurse -Force -ErrorAction SilentlyContinue
    }
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}

} catch {
    $vvCaughtError = $_
    try { Write-VVFailure $vvCaughtError $PSCommandPath } catch { Write-Warning ('Could not persist error: ' + $_.Exception.Message) }
    throw $vvCaughtError
} finally {
    Stop-VVReport $vvReportOwner
}
