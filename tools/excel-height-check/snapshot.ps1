param([Parameter(Mandatory=$true)][string]$RunRoot)
$ErrorActionPreference='Stop'
$manifest=Get-Content "$RunRoot\manifest.json" -Raw -Encoding UTF8|ConvertFrom-Json
$old=[Collections.Generic.List[object]]::new();$new=[Collections.Generic.List[object]]::new();$timings=[Collections.Generic.List[object]]::new()
function Release($o){if($null -ne $o -and [Runtime.InteropServices.Marshal]::IsComObject($o)){[void][Runtime.InteropServices.Marshal]::ReleaseComObject($o)}}
function SaveJson($o,$path){[IO.File]::WriteAllText($path,(ConvertTo-Json -InputObject $o -Depth 12),[Text.UTF8Encoding]::new($false))}
function SheetData($raw,$matched){
 for($i=0;$i -lt $raw.GetUpperBound(0);$i++){
  [ordered]@{index=($i+1);name=($raw[$i,4]);visibility=($raw[$i,10]);
   used_range=[ordered]@{address=($raw[$i,3]);first_row=($raw[$i,7]);last_row=($raw[$i,7]+$raw[$i,5]-1);first_column=($raw[$i,8]);last_column=($raw[$i,8]+$raw[$i,6]-1);rows=($raw[$i,5]);columns=($raw[$i,6]);height_points=($raw[$i,0]);width_points=($raw[$i,9])};
   shrunk_range_comparison=[ordered]@{address=($matched[$i,3]);height_points=($matched[$i,0]);width_points=($matched[$i,9])};
   standard_height_points=($raw[$i,11]);standard_column_width=($raw[$i,12])}
 }
}
$temp=Join-Path ([IO.Path]::GetTempPath()) ('excel-json-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$e=$null;$books=$null;$hostBook=$null;$a=$null;$b=$null
try{
 Copy-Item "$PSScriptRoot\HeightProbeHost.xlsm" "$temp\HeightProbeHost.xlsm"
 $e=New-Object -ComObject Excel.Application
 $e.Visible=$false;$e.DisplayAlerts=$false;$e.EnableEvents=$false;$e.ScreenUpdating=$false;$e.AskToUpdateLinks=$false
 $books=$e.Workbooks;$e.AutomationSecurity=1
 try{$hostBook=$books.Open("$temp\HeightProbeHost.xlsm",0,$true)}finally{$e.AutomationSecurity=3}
 $e.Calculation=-4135;$macro="'$($hostBook.Name)'!HeightProbe.Collect"
 foreach($file in $manifest.files){
  Write-Output "Collect: $($file.name)"
  $watch=[Diagnostics.Stopwatch]::StartNew()
  try{
   Copy-Item -LiteralPath "$RunRoot\input\$($file.id).xlsx" -Destination "$temp\original.xlsx"
   Copy-Item -LiteralPath "$RunRoot\shrunk\$($file.id).xlsx" -Destination "$temp\shrunk.xlsx"
   $a=$books.Open("$temp\original.xlsx",0,$true);$b=$books.Open("$temp\shrunk.xlsx",0,$true)
   $orig=$e.Run($macro,[string]$a.Name);$shrunk=$e.Run($macro,[string]$b.Name)
   $n=$shrunk.GetUpperBound(0)
   if($orig.GetUpperBound(0) -ne $n){throw 'Sheet count mismatch'}
   $addresses=@(for($i=0;$i -lt $n;$i++){if($orig[$i,4] -ne $shrunk[$i,4]){throw 'Sheet order/name mismatch'};[string]$shrunk[$i,3]})
   $matched=$e.Run($macro,[string]$a.Name,($addresses -join "`n"))
   $old.Add([ordered]@{file=$file.name;sheets=@(SheetData $orig $matched)})
   $new.Add([ordered]@{file=$file.name;sheets=@(SheetData $shrunk $shrunk)})
   $timings.Add([ordered]@{file=$file.name;open_and_collect_ms=$watch.Elapsed.TotalMilliseconds;original_vba_ms=($orig[$n,0]);shrunk_vba_ms=($shrunk[$n,0]);matched_vba_ms=($matched[$n,0])})
  }finally{
   if($b){$b.Close($false);Release $b;$b=$null};if($a){$a.Close($false);Release $a;$a=$null}
  }
 }
 SaveJson $old.ToArray() "$RunRoot\original.json"
 SaveJson $new.ToArray() "$RunRoot\shrunk.json"
 SaveJson $timings.ToArray() "$RunRoot\excel-timings.json"
}finally{
 if($hostBook){$hostBook.Close($false)};if($e){$e.Quit()}
 Release $hostBook;Release $books;Release $e
 foreach($name in @('HeightProbeHost.xlsm','original.xlsx','shrunk.xlsx')){Remove-Item -LiteralPath (Join-Path $temp $name) -ErrorAction SilentlyContinue}
}
