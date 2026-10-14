param([Parameter(Mandatory=$true)][string]$PngPath)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '..\_shared\report-io.ps1')
Initialize-VVReportIO
if(-not [VVReportIO]::FileExists($PngPath)){throw 'The chosen PNG is genuinely absent.'}
$full=[VVReportIO]::FullPath($PngPath)
$entries=@(Get-VVChildItem -LiteralPath (Split-Path -Parent $full) -File)
if(@($entries|Where-Object{$_.FullName -eq $full}).Count -ne 1){throw 'Native enumeration did not find the chosen file.'}
Add-Type -AssemblyName System.Drawing
Add-Type -Path (Join-Path $PSScriptRoot '..\_shared\suite\SuitePixels.cs') -ReferencedAssemblies @('System.dll','System.Drawing.dll')
$stream=[VVReportIO]::OpenRead($full)
try{
    $before=[VVReportIO]::HashStream($stream);$stream.Position=0
    $digest=[VVSuitePixels]::ReadStream($stream,64000000)
    $stream.Position=0;$after=[VVReportIO]::HashStream($stream)
    if($before -ne $after){throw 'Unexpected source modification.'}
    if($digest.Width -lt 1 -or $digest.Height -lt 1){throw 'Invalid decoded dimensions.'}
    Write-Host ('PASS: native enumeration and pixel decode; path length '+$full.Length)
    $digest|Format-List
}finally{$stream.Dispose()}
