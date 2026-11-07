<#
.SYNOPSIS
Compare production COM conversion on a shared desktop and separate private desktops.
#>
param(
    [Parameter(Mandatory=$true)][string]$Output,
    [Parameter(Mandatory=$true)][string]$Fixture,
    [ValidateRange(1,8)][int]$Parallel = 4
)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$capture=Join-Path $repo 'excel-shrink-cli/office-capture'
$outputPath=[IO.Path]::GetFullPath($Output)
if(Test-Path -LiteralPath $outputPath){throw 'Choose a new output directory.'}
[void][IO.Directory]::CreateDirectory($outputPath)
$env:TEMP=$outputPath; $env:TMP=$outputPath
Add-Type -Path @((Join-Path $capture 'CaptureProcesses.cs'),(Join-Path $capture 'CaptureDesktop.cs'))
Add-Type -Path (Join-Path $PSScriptRoot '../check-performance-trigger/DesktopIdentity.cs')
$inputDesktop=[DesktopIdentity]::InputName()
$sharedDesktop=[DesktopIdentity]::CurrentName()
$worker=Join-Path $outputPath 'worker'
[void][IO.Directory]::CreateDirectory($worker)
foreach($file in @('archive_processes.ps1','ArchiveProcesses.cs','ExcelConversionErrors.psm1')){
    Copy-Item -LiteralPath (Join-Path $repo ('excel-shrink-cli/src/regression/'+$file)) -Destination $worker
}
foreach($file in @('CaptureProcesses.cs','ExcelProcessSlots.ps1')){
    Copy-Item -LiteralPath (Join-Path $capture $file) -Destination $worker
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../check-performance-trigger/DesktopIdentity.cs') -Destination $worker
$source=Get-Content (Join-Path $repo 'excel-shrink-cli/src/regression/excel_open.ps1') -Raw
$source=$source.Replace('Add-Type -Path (Join-Path $PSScriptRoot ''CaptureProcesses.cs'')',
    'Add-Type -Path (Join-Path $PSScriptRoot ''DesktopIdentity.cs'')'+[Environment]::NewLine+
    'Add-Type -Path (Join-Path $PSScriptRoot ''CaptureProcesses.cs'')')
$source=$source.Replace('    $excel.Visible = $false',
    '    $desktopEvidence=@{helper=[DesktopIdentity]::CurrentName();excel=[DesktopIdentity]::WindowName([IntPtr]$excel.Hwnd)}'+[Environment]::NewLine+
    '    [IO.File]::WriteAllText(($ProgressPath+''.desktop.json''),($desktopEvidence | ConvertTo-Json),[Text.UTF8Encoding]::new($false))'+[Environment]::NewLine+
    '    $excel.Visible = $false')
[IO.File]::WriteAllText((Join-Path $worker 'open.ps1'),$source,[Text.UTF8Encoding]::new($false))
$results=@()
$trial=0
foreach($mode in @('shared','private','private','shared')){
    $trial++
    $owned=New-Object CaptureProcesses
    $desktops=@(); $children=@()
    $directory=Join-Path $outputPath ($trial.ToString()+'-'+$mode)
    [void][IO.Directory]::CreateDirectory($directory)
    $clock=[Diagnostics.Stopwatch]::StartNew()
    try {
        for($index=0;$index -lt $Parallel;$index++){
            $case=Join-Path $directory ([string]$index)
            [void][IO.Directory]::CreateDirectory($case)
            $desktopName=$sharedDesktop
            if($mode -eq 'private'){
                $desktop=New-Object CaptureDesktop
                $desktops+=,$desktop
                $desktopName=$desktop.Name
            }
            $arguments='-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -InputPath "{1}" -MetricsPath "{2}" -ProgressPath "{3}" -OutputPath "{4}"' -f
                (Join-Path $worker 'open.ps1'),([IO.Path]::GetFullPath($Fixture)),(Join-Path $case 'metrics.json'),
                (Join-Path $case 'progress.log'),(Join-Path $case 'converted.xlsx')
            $child=[CaptureDesktop]::StartOnDesktop($desktopName,(Join-Path $PSHOME 'powershell.exe'),$arguments,$owned)
            $children+=,$child
        }
        foreach($child in $children){
            $remaining=[Math]::Max(1,90000-[int]$clock.Elapsed.TotalMilliseconds)
            if(-not $child.WaitForExit($remaining)){throw 'Conversion desktop trial timed out.'}
            if($child.ExitCode -ne 0){throw "Conversion desktop trial exited with $($child.ExitCode)."}
        }
        $wall=$clock.Elapsed.TotalSeconds
        $cases=@(0..($Parallel-1) | ForEach-Object {
            $case=Join-Path $directory ([string]$_)
            @{metrics=([IO.File]::ReadAllText((Join-Path $case 'metrics.json')) | ConvertFrom-Json)
              desktop=([IO.File]::ReadAllText((Join-Path $case 'progress.log.desktop.json')) | ConvertFrom-Json)}
        })
        $result=@{trial=$trial;mode=$mode;parallel=$Parallel;wall_seconds=$wall;cases=$cases}
        $results+=,$result
        $result | ConvertTo-Json -Depth 6 -Compress | Write-Output
        [IO.File]::WriteAllText((Join-Path $outputPath 'results.json'),
            (@{results=$results;input_desktop_unchanged=($inputDesktop -eq [DesktopIdentity]::InputName())} | ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    } finally {
        $owned.Dispose()
        foreach($child in $children){$child.Dispose()}
        foreach($desktop in $desktops){$desktop.Dispose()}
    }
}
