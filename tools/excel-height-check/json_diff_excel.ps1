# Managed by patch_excel_json_diff.py. Isolated Excel worker, supervised by json_diff.ps1.
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$RequestFile)
Set-StrictMode -Version 2.0
$ErrorActionPreference='Stop'
# Excel Lab v2: explicit method dispatch, LCID and non-enumerated COM results.
. (Join-Path $PSScriptRoot '../excel-lab/ExcelInterop.ps1')
$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
function Emit([string]$Message){[Console]::WriteLine($Message);[Console]::Out.Flush()}
function SaveJson($Value,[string]$Path){[IO.File]::WriteAllText($Path,(ConvertTo-Json -InputObject $Value -Depth 20),[Text.UTF8Encoding]::new($false))}
function Release($Object){
    if($null -ne $Object -and [Runtime.InteropServices.Marshal]::IsComObject($Object)){
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($Object)
    }
}
function Close-Book($Book){
    if($null -ne $Book){
        try{$Book.Close($false)}catch{Emit ('CLEANUP warning: '+$_.Exception.Message)}
        finally{Release $Book}
    }
}
function Open-ReadOnly([string]$Path){
    $book=Open-LabWorkbook -Excel $script:Excel -Books $script:Books -Path $Path
    return ,$book
}

function Test-Probe($HostBook,[switch]$SelfTest){
    $prefix="'$($HostBook.Name.Replace("'","''"))'!JsonDiffProbe."
    $version=[string]$script:Excel.Run(($prefix+'JsonDiffVersion'))
    if($version -cne $script:Request.probe_version){throw "VBA helper version mismatch: expected $($script:Request.probe_version), got $version. Rebuild/import the current JsonDiffProbe.bas."}
    if($SelfTest){
        Emit "BEGIN VBA self-test version=$version"
        $result=[string]$script:Excel.Run(($prefix+'SelfTest'))
        if($result -cne 'ok'){throw "VBA self-test returned: $result"}
        Emit 'DONE VBA self-test (mixed heights, hidden row, nonzero batch offset)'
    }
    return $prefix
}
function Open-Helper([string]$Path){
    $book=Open-LabWorkbook -Excel $script:Excel -Books $script:Books -Path $Path -Helper
    return ,$book
}

function Copy-Verified([string]$Source,[string]$Destination,[string]$Expected){
    [IO.File]::Copy($Source,$Destination,$false)
    $actual=(Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash
    if($actual -ine $Expected){throw "Workbook hash changed: $Source"}
}

$script:Excel=$null;$script:Books=$null;$hostBook=$null;$a=$null;$b=$null
$ownsExcel=$false;$temp='';$exitCode=0
try{
    if([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT){throw 'Requires Windows Excel and Windows PowerShell.'}
    if([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA'){throw 'Requires powershell.exe -STA.'}
    $script:Request=[IO.File]::ReadAllText($RequestFile) | ConvertFrom-Json
    $request=$script:Request
    $run=[string]$request.run_root
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('excel-json-'+[guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($temp)
    Emit "Worker action=$($request.action); local Excel workdir=$temp"
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class JsonDiffWindowProcess {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
}
'@
    $before=@([Diagnostics.Process]::GetProcessesByName('EXCEL') | ForEach-Object {try{$_.Id}finally{$_.Dispose()}})
    Emit 'BEGIN Excel.Application COM activation'
    $script:Excel=New-Object -ComObject Excel.Application
    [uint32]$excelPid=0
    [void][JsonDiffWindowProcess]::GetWindowThreadProcessId([IntPtr]$script:Excel.Hwnd,[ref]$excelPid)
    if($excelPid -eq 0 -or $before -contains [int]$excelPid){throw 'Could not establish an isolated new Excel process. Refusing to change or close a pre-existing Excel instance.'}
    $ownsExcel=$true
    $process=[Diagnostics.Process]::GetProcessById([int]$excelPid)
    try{$ticks=$process.StartTime.ToUniversalTime().Ticks.ToString()}finally{$process.Dispose()}
    SaveJson ([ordered]@{owner_token=$request.owner_token;pid=$excelPid;start_ticks=$ticks}) ([string]$request.state_file)
    Emit "DONE Excel COM activation excel_pid=$excelPid"
    $script:Excel.Visible=$false;$script:Excel.DisplayAlerts=$false
    $script:Excel.EnableEvents=$false;$script:Excel.ScreenUpdating=$false
    $script:Excel.AskToUpdateLinks=$false;$script:Excel.AutomationSecurity=3
    $script:Books=$script:Excel.Workbooks
    $localHost=Join-Path $temp 'ProbeHost.xlsm'
    if($request.action -ceq 'prepare'){
        $candidate=[string]$request.helper_workbook
        if(-not $candidate -and [IO.File]::Exists([string]$request.cache_path)){$candidate=[string]$request.cache_path}
        if($candidate){
            Emit "Using existing probe helper: $candidate"
            [IO.File]::Copy($candidate,$localHost,$false)
            $hostBook=Open-Helper $localHost
            [void](Test-Probe $hostBook -SelfTest)
        }else{
            Emit 'BEGIN building current VBA helper in an otherwise empty workbook'
            $script:Excel.AutomationSecurity=1
            try{$hostBook=$script:Books.Add(-4167)}finally{$script:Excel.AutomationSecurity=3}
            $project=$null;$components=$null;$component=$null
            try{
                try{
                    $project=$hostBook.VBProject
                    if($null -eq $project){throw 'Workbook.VBProject returned null.'}
                    $components=$project.VBComponents
                    if($null -eq $components){throw 'VBProject.VBComponents returned null.'}
                }catch{
                    throw @"
Cannot access the VBA project. Trust Center or Office policy is one possible cause, not proven by this generic error. No security settings were changed.
Option A (no permanent security change): in Excel create a blank workbook, press Alt+F11,
import the reviewed file $($request.bas_path), then save as
$PSScriptRoot\JsonDiffProbeHost.xlsm (macro-enabled workbook). Rerun this task.
Option B: for building the helper, explicitly enable Trust Center > Macro Settings >
Trust access to the VBA project object model; rerun once, then disable that option again.
The source-hash-specific cached helper can subsequently run without VBA-project access.
The old HeightProbeHost.xlsm is NOT compatible with this separate row probe.
Underlying error: $(Format-LabException $_)
"@
                }
                $localBas=Join-Path $temp 'JsonDiffProbe.bas'
                [IO.File]::Copy([string]$request.bas_path,$localBas,$false)
                $component=$components.Import($localBas)
                if([string]$component.Name -cne 'JsonDiffProbe'){throw 'Unexpected imported VBA module name.'}
            }finally{Release $component;Release $components;Release $project}
            [void](Test-Probe $hostBook -SelfTest)
            $hostBook.SaveAs($localHost,52)
            Emit 'DONE VBA helper build; only the disposable helper workbook was saved'
        }
        Close-Book $hostBook;$hostBook=$null
        # Validate the persisted helper too, not just the in-memory VBA module.
        $hostBook=Open-Helper $localHost
        [void](Test-Probe $hostBook)
        Close-Book $hostBook;$hostBook=$null
        $destination=Join-Path $run 'probe-host.xlsm'
        [IO.File]::Copy($localHost,$destination,$false)
        if(-not $candidate){
            [void][IO.Directory]::CreateDirectory((Split-Path ([string]$request.cache_path) -Parent))
            if(-not [IO.File]::Exists([string]$request.cache_path)){
                [IO.File]::Copy($localHost,[string]$request.cache_path,$false)
            }
        }
        SaveJson ([ordered]@{version=$request.probe_version;helper_sha256=(Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant();source_sha256=(Get-FileHash -LiteralPath ([string]$request.bas_path) -Algorithm SHA256).Hash.ToLowerInvariant();excel_version=[string]$script:Excel.Version;excel_build=[string]$script:Excel.Build}) (Join-Path $run 'probe-info.json')
        Emit 'DONE preflight: Excel, compiled VBA helper and row-batch self-test succeeded'
    }elseif($request.action -ceq 'measure'){
        $manifest=[IO.File]::ReadAllText((Join-Path $run 'manifest.json')) | ConvertFrom-Json
        Copy-Verified (Join-Path $run 'probe-host.xlsm') $localHost ([string]$manifest.probe.helper_sha256)
        $hostBook=Open-Helper $localHost
        $macro=Test-Probe $hostBook
        $script:Excel.Calculation=-4135
        $old=[Collections.Generic.List[object]]::new();$new=[Collections.Generic.List[object]]::new()
        $timings=[Collections.Generic.List[object]]::new()
        $number=0
        foreach($file in $manifest.files){
            $number++
            $watch=[Diagnostics.Stopwatch]::StartNew()
            $localA=Join-Path $temp 'original.xlsx';$localB=Join-Path $temp 'shrunk.xlsx'
            try{
                Emit "BEGIN workbook $number/$($manifest.files.Count): $($file.name)"
                Copy-Verified (Join-Path $run ('input\'+$file.id+'.xlsx')) $localA ([string]$file.sha256)
                Copy-Verified (Join-Path $run ('shrunk\'+$file.id+'.xlsx')) $localB ([string]$file.shrunk_sha256)
                Emit "BEGIN Excel.Open original: $localA"
                $a=Open-ReadOnly $localA
                Emit 'DONE Excel.Open original'
                Emit "BEGIN Excel.Open shrunk: $localB"
                $b=Open-ReadOnly $localB
                Emit 'DONE Excel.Open shrunk'
                Emit 'BEGIN VBA UsedRange metadata for both workbooks (before row scanning)'
                $orig=([string]$script:Excel.Run(($macro+'Describe'),[string]$a.Name)) | ConvertFrom-Json
                $shrunk=([string]$script:Excel.Run(($macro+'Describe'),[string]$b.Name)) | ConvertFrom-Json
                if($orig.Count -eq 0 -or $orig.Count -ne $shrunk.Count){throw 'Worksheet count mismatch or no worksheets.'}
                for($i=0;$i -lt $orig.Count;$i++){
                    if($orig[$i].name -cne $shrunk[$i].name -or $orig[$i].index -ne $shrunk[$i].index){throw 'Worksheet name/order mismatch.'}
                    if($orig[$i].worksheet_row_count -ne $shrunk[$i].worksheet_row_count){throw 'Worksheet row capacity mismatch.'}
                }
                Emit "DONE UsedRange metadata; worksheets=$($orig.Count)"
                for($i=0;$i -lt $orig.Count;$i++){
                    $sheetIndex=$i+1
                    $address=[string]$shrunk[$i].used_range.address
                    $matched=([string]$script:Excel.Run(($macro+'MatchedRange'),[string]$a.Name,[int]$sheetIndex,$address)) | ConvertFrom-Json
                    Add-Member -InputObject $orig[$i] -NotePropertyName shrunk_range_comparison -NotePropertyValue $matched
                    Add-Member -InputObject $shrunk[$i] -NotePropertyName shrunk_range_comparison -NotePropertyValue $shrunk[$i].used_range
                    $last=if($request.row_scope -ceq 'all'){[int]$orig[$i].worksheet_row_count}else{[Math]::Max([int]$orig[$i].used_range.last_row,[int]$shrunk[$i].used_range.last_row)}
                    $rowDir=Join-Path $run ('rows\'+$file.id)
                    [void][IO.Directory]::CreateDirectory($rowDir)
                    foreach($variant in @('original','shrunk')){
                        if($variant -ceq 'original'){$book=$a;$sheet=$orig[$i]}else{$book=$b;$sheet=$shrunk[$i]}
                        $rowsPath=Join-Path $rowDir ($variant+'-s'+$sheetIndex+'.tsv')
                        $writer=[IO.StreamWriter]::new(($rowsPath+'.partial'),$false,[Text.UTF8Encoding]::new($false))
                        $measured=0
                        try{
                            for($first=1;$first -le $last;$first+=[int]$request.batch_rows){
                                $count=[Math]::Min([int]$request.batch_rows,$last-$first+1)
                                $end=$first+$count-1
                                Emit "BEGIN rows: file=$($file.name) variant=$variant sheet=$sheetIndex/$($orig.Count) '$($sheet.name)' rows=$first..$end/$last"
                                $batchWatch=[Diagnostics.Stopwatch]::StartNew()
                                $batch=[string]$script:Excel.Run(($macro+'RowsBatch'),[string]$book.Name,[int]$sheetIndex,[int]$first,[int]$count)
                                if([string]::IsNullOrEmpty($batch)){throw 'VBA returned an empty row batch.'}
                                $writer.Write($batch);$writer.Flush();$measured+=$count
                                Emit ('DONE rows: variant={0} sheet={1} through={2}/{3} percent={4:N1} batch_ms={5}' -f $variant,$sheetIndex,$end,$last,(100.0*$end/$last),$batchWatch.ElapsedMilliseconds)
                            }
                        }finally{$writer.Dispose()}
                        [IO.File]::Move(($rowsPath+'.partial'),$rowsPath)
                        Add-Member -InputObject $sheet -NotePropertyName row_scan -NotePropertyValue ([ordered]@{scope=$request.row_scope;first_row=1;last_row=$last;measured_rows=$measured;measurement='Rows(r).Height in points; every row read in VBA; exact RLE'})
                        Emit "DONE worksheet rows: $variant '$($sheet.name)' measured_rows=$measured"
                    }
                }
                $old.Add([ordered]@{id=$file.id;file=$file.name;sheets=$orig})
                $new.Add([ordered]@{id=$file.id;file=$file.name;sheets=$shrunk})
                $timings.Add([ordered]@{file=$file.name;worksheets=$orig.Count;open_and_measure_ms=$watch.Elapsed.TotalMilliseconds})
                SaveJson ($old.ToArray()) (Join-Path $run 'original.raw.json')
                SaveJson ($new.ToArray()) (Join-Path $run 'shrunk.raw.json')
                SaveJson ($timings.ToArray()) (Join-Path $run 'excel-timings.json')
                Emit "DONE workbook: $($file.name); elapsed_ms=$($watch.ElapsedMilliseconds)"
            }finally{
                Close-Book $b;$b=$null;Close-Book $a;$a=$null
                foreach($path in @($localA,$localB)){if([IO.File]::Exists($path)){[IO.File]::Delete($path)}}
            }
        }
        SaveJson ([ordered]@{complete=$true;files=$number}) (Join-Path $run 'measurement-complete.json')
        Emit "DONE all Excel measurements: files=$number"
    }else{throw "Unknown worker action: $($request.action)"}
}catch{
    $exitCode=1
    [Console]::Error.WriteLine('EXCEL WORKER ERROR: '+(Format-LabException $_))
    if($_.ScriptStackTrace){[Console]::Error.WriteLine($_.ScriptStackTrace)}
}finally{
    Close-Book $b;Close-Book $a;Close-Book $hostBook
    if($ownsExcel -and $null -ne $script:Excel){
        Emit 'BEGIN Excel.Quit (own instance only)'
        try{$script:Excel.Quit();Emit 'DONE Excel.Quit'}catch{[Console]::Error.WriteLine('Excel.Quit warning: '+$_.Exception.Message)}
    }
    Release $script:Books;Release $script:Excel
    $script:Books=$null;$script:Excel=$null
    [GC]::Collect();[GC]::WaitForPendingFinalizers()
    # Only delete the exact short-lived files created by this worker, never recursive user paths.
    if($temp){
        foreach($name in @('original.xlsx','shrunk.xlsx','ProbeHost.xlsm','JsonDiffProbe.bas')){
            $path=Join-Path $temp $name
            try{if([IO.File]::Exists($path)){[IO.File]::Delete($path)}}catch{}
        }
        try{[IO.Directory]::Delete($temp,$false)}catch{}
    }
}
exit $exitCode
