param([Parameter(Mandatory=$true)][string]$RunRoot)
$ErrorActionPreference='Stop'
if([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA'){throw 'Requires powershell.exe -STA'}
$manifest=Get-Content -LiteralPath "$RunRoot\manifest.json" -Raw -Encoding UTF8 | ConvertFrom-Json
$results=[Collections.Generic.List[object]]::new()
$measured=[Collections.Generic.List[object]]::new()
$timings=[Collections.Generic.List[object]]::new()
function Release($o){if($null -ne $o -and [Runtime.InteropServices.Marshal]::IsComObject($o)){[void][Runtime.InteropServices.Marshal]::ReleaseComObject($o)}}
function SaveJson($value,$path){[IO.File]::WriteAllText($path,(ConvertTo-Json -InputObject $value -Depth 10),[Text.UTF8Encoding]::new($false))}
$excel=$null;$books=$null;$hostBook=$null;$original=$null;$shrunk=$null
$temporary=Join-Path ([IO.Path]::GetTempPath()) ('excel-height-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temporary)
$helper=Join-Path $temporary 'HeightProbeHost.xlsm'
Copy-Item -LiteralPath "$PSScriptRoot\HeightProbeHost.xlsm" -Destination $helper
try {
 $excel=New-Object -ComObject Excel.Application
 $excel.Visible=$false;$excel.DisplayAlerts=$false;$excel.EnableEvents=$false;$excel.ScreenUpdating=$false;$excel.AskToUpdateLinks=$false
 $books=$excel.Workbooks;$excel.AutomationSecurity=1
 try{$hostBook=$books.Open($helper,0,$true)}finally{$excel.AutomationSecurity=3}
 $excel.Calculation=-4135
 $macro="'$($hostBook.Name)'!HeightProbe.Collect"
 $copies=Join-Path $RunRoot 'reference-copies';[void][IO.Directory]::CreateDirectory($copies)
 foreach($source in $manifest.files){
  $referenceOutput=Join-Path $copies ('original-'+$source.name)
  $copy=Join-Path $temporary 'original.xlsx'
  if(Test-Path -LiteralPath $referenceOutput){throw "Reference already exists: $referenceOutput"}
  Copy-Item -LiteralPath $source.source -Destination $copy
  if((Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash -ne $source.sha256){throw 'Source changed since preparation'}
  Write-Output "ORIGINAL $($source.name)"
  $watch=[Diagnostics.Stopwatch]::StartNew()
  try {
   if($manifest.resave_originals){
    $original=$books.Open($copy,0,$false)
    if($original.ReadOnly){throw 'Reference copy opened read-only'}
    $original.Save();$original.Close($false);Release $original;$original=$null
   }
   Copy-Item -LiteralPath $copy -Destination $referenceOutput
   $original=$books.Open($copy,0,$true)
   $openMs=$watch.Elapsed.TotalMilliseconds
   $sheetCount=$null
   foreach($profile in $manifest.profiles.PSObject.Properties.Name){
    $watch.Restart()
    try {
     $profileFolder=$manifest.profile_directories.$profile
     $shortShrunk=Join-Path $temporary 'shrunk.xlsx'
     Copy-Item -LiteralPath (Join-Path "$RunRoot\$profileFolder\shrunk" $source.name) -Destination $shortShrunk
     $shrunk=$books.Open($shortShrunk,0,$true)
     $a=$excel.Run($macro,[string]$shrunk.Name);$n=$a.GetUpperBound(0)
     if($null -ne $sheetCount -and $sheetCount -ne $n){throw 'Profile sheet count mismatch'}
     $sheetCount=$n
     $addresses=@(for($i=0;$i -lt $n;$i++){[string]$a[$i,3]})
     $b=$excel.Run($macro,[string]$original.Name,($addresses -join "`n"))
     if($b.GetUpperBound(0) -ne $n){throw 'Original/shrunk sheet count mismatch'}
     for($i=0;$i -lt $n;$i++){
      if($a[$i,4] -ne $b[$i,4] -or $a[$i,3] -ne $b[$i,3]){throw 'Sheet name or address mismatch'}
      $delta=[double]$a[$i,0]-[double]$b[$i,0]
      $results.Add([ordered]@{profile=$profile;file=$source.name;sheet_index=($i+1);sheet=($a[$i,4]);range=($a[$i,3]);row_count=($a[$i,5]);original_points=($b[$i,0]);shrunk_points=($a[$i,0]);delta_points=$delta;delta_pixels_96dpi=($delta*4/3)})
     }
     SaveJson $results.ToArray() "$RunRoot\comparison.json"
     $timings.Add([ordered]@{file=$source.name;profile=$profile;open_and_measure_ms=$watch.Elapsed.TotalMilliseconds;shrunk_vba_ms=($a[$n,0]);original_vba_ms=($b[$n,0])})
     SaveJson $timings.ToArray() "$RunRoot\excel-timings.json"
     Write-Output "DONE profile=$profile sheets=$n open_and_measure_ms=$($watch.ElapsedMilliseconds)"
    }finally{if($shrunk){$shrunk.Close($false);Release $shrunk;$shrunk=$null}}
   }
   $measured.Add([ordered]@{file=$source.name;sheet_count=$sheetCount;reference_prepare_ms=$openMs})
   SaveJson $measured.ToArray() "$RunRoot\measured-files.json"
   if((Get-FileHash -LiteralPath $source.source -Algorithm SHA256).Hash -ne $source.sha256){throw 'Original file changed during measurement'}
  }finally{if($original){$original.Close($false);Release $original;$original=$null}}
 }
}finally{
 if($hostBook){$hostBook.Close($false)}
 if($excel){$excel.Quit()}
 Release $hostBook;Release $books;Release $excel
 # Delete only the exact helper copy we created, never recursive computed paths.
 foreach($name in @('HeightProbeHost.xlsm','original.xlsx','shrunk.xlsx')){
  Remove-Item -LiteralPath (Join-Path $temporary $name) -ErrorAction SilentlyContinue
 }
}
