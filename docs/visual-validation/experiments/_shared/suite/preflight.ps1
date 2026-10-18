param([string]$RunRoot)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '..\reporting.ps1')
Add-Type -AssemblyName System.Drawing -ErrorAction Stop
if($null -eq ('VVStripDigest' -as [type])){Add-Type -Path (Join-Path $PSScriptRoot 'StripDigest.cs') -ReferencedAssemblies @('System.dll','System.Drawing.dll') -ErrorAction Stop}
if($null -eq ('VVSuiteJob' -as [type])){Add-Type -Path (Join-Path $PSScriptRoot 'SuiteRuntime.cs') -ReferencedAssemblies @('System.dll') -ErrorAction Stop}
$dir=Join-Path $env:TEMP ('vv-decision-preflight-'+[Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($dir)|Out-Null
$full=$null;$first=$null;$last=$null;$bad=$null
$checks=[Collections.Generic.List[string]]::new()
function New-PreflightStrip([string]$Path,[int]$Y,[int]$Width,[int]$Height){
    $s=[VVStripSpec]::new();$s.Path=$Path;$s.X=0;$s.Y=$Y;$s.Width=$Width;$s.Height=$Height
    $s.Sha256=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    return $s
}
function Must-Reject([scriptblock]$Action,[string]$Name){
    $rejected=$false
    try{& $Action|Out-Null}catch{$rejected=$true}
    if(-not $rejected){throw ('Pixel preflight did not reject '+$Name)}
    $checks.Add($Name)
}
try {
    $full=[Drawing.Bitmap]::new(3,5,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    for($y=0;$y -lt 5;$y++){for($x=0;$x -lt 3;$x++){$full.SetPixel($x,$y,[Drawing.Color]::FromArgb(255,20+$x*30,10+$y*30,50))}}
    $first=$full.Clone([Drawing.Rectangle]::new(0,0,3,3),[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $last=$full.Clone([Drawing.Rectangle]::new(0,2,3,3),[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $fp=Join-Path $dir 'full.png';$ap=Join-Path $dir 'a.png';$bp=Join-Path $dir 'b.png'
    $full.Save($fp,[Drawing.Imaging.ImageFormat]::Png);$first.Save($ap,[Drawing.Imaging.ImageFormat]::Png);$last.Save($bp,[Drawing.Imaging.ImageFormat]::Png)
    $a=New-PreflightStrip $ap 0 3 3;$b=New-PreflightStrip $bp 2 3 3;$f=New-PreflightStrip $fp 0 3 5
    $single=[VVStripDigest]::Read([VVStripSpec[]]@($f),100)
    $tiled=[VVStripDigest]::Read([VVStripSpec[]]@($a,$b),100)
    if($single.Sha256 -cne $tiled.Sha256 -or $tiled.Height -ne 5 -or $tiled.OverlapRowsChecked -ne 1){throw 'Canonical strip preflight failed: different tile layouts disagree'}
    $checks.Add('same pixels across different strip layouts')
    $b.Y=4;Must-Reject { [VVStripDigest]::Read([VVStripSpec[]]@($a,$b),100) } 'gap';$b.Y=2
    $f.Sha256=('0'*64);Must-Reject { [VVStripDigest]::Read([VVStripSpec[]]@($f),100) } 'changed receipt';$f=New-PreflightStrip $fp 0 3 5
    Must-Reject { [VVStripDigest]::Read([VVStripSpec[]]@($f),10) } 'decode budget'
    $last.SetPixel(0,0,[Drawing.Color]::Red);$badPath=Join-Path $dir 'bad-seam.png';$last.Save($badPath,[Drawing.Imaging.ImageFormat]::Png)
    $bad=New-PreflightStrip $badPath 2 3 3
    Must-Reject { [VVStripDigest]::Read([VVStripSpec[]]@($a,$bad),100) } 'inconsistent seam'
    $f.X=1;Must-Reject { [VVStripDigest]::Read([VVStripSpec[]]@($f),100) } 'unsupported horizontal layout'
    if($RunRoot){Write-VVJson (Join-Path $RunRoot 'preflight.json') @{schema='vv-decision-preflight-v1';status='complete';checks=@($checks);powershell_version=$PSVersionTable.PSVersion.ToString();drawing_assembly=[Drawing.Bitmap].Assembly.Location;timestamp_utc=[DateTime]::UtcNow.ToString('o')}}
    Write-Host ('PREFLIGHT: '+$checks.Count+' local pixel checks passed; no Excel instance was opened.')
} finally {
    if($null -ne $last){$last.Dispose()};if($null -ne $first){$first.Dispose()};if($null -ne $full){$full.Dispose()}
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
