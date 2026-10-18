param(
 [Parameter(Mandatory=$true)][string]$InputPath,
 [Parameter(Mandatory=$true)][string]$OutputPath,
 [Parameter(Mandatory=$true)][string]$ProgressPath
)
$ErrorActionPreference='Stop'
function Progress([string]$message) { Add-Content -LiteralPath $ProgressPath -Encoding UTF8 -Value "$(Get-Date -Format o) $message" }
function Timed([string]$phase,[scriptblock]$action) {
 Progress "dimension $phase start"
 $watch=[Diagnostics.Stopwatch]::StartNew()
 try { & $action } finally { Progress "visual step phase=excel.dimension_$phase duration_ms=$($watch.ElapsedMilliseconds)" }
}
Set-Content -LiteralPath $ProgressPath -Value '' -Encoding UTF8
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class DimensionSavePid {
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
}
'@
$stage=Join-Path ([IO.Path]::GetTempPath()) ('es-dim-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($stage)
$excel=$null;$book=$null;$settings=$null
try {
 $local=Join-Path $stage 'shrunk.xlsx'
 Copy-Item -LiteralPath $InputPath -Destination $local
 $excel=New-Object -ComObject Excel.Application
 $excelPid=[uint32]0
 [void][DimensionSavePid]::GetWindowThreadProcessId([IntPtr]$excel.Hwnd,[ref]$excelPid)
 Progress "excel pid=$excelPid"
 $excel.Visible=$false;$excel.DisplayAlerts=$false;$excel.EnableEvents=$false
 $excel.AskToUpdateLinks=$false;$excel.AutomationSecurity=3
 $settings=Timed 'settings' { $excel.Workbooks.Add() }
 $excel.Calculation=-4135;$excel.CalculateBeforeSave=$false
 $book=Timed 'open' { $excel.Workbooks.Open($local,0,$false) }
 if($book.ReadOnly){throw 'Dimension-save workbook opened read-only'}
 if([int]$excel.Calculation -ne -4135){throw 'Excel changed manual calculation mode'}
 Timed 'save' { $book.Save() }
 Timed 'close' { $book.Close($false) }
 [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($book);$book=$null
 Copy-Item -LiteralPath $local -Destination $OutputPath -Force
 Progress 'dimension save finished'
} finally {
 if($null -ne $book){try{$book.Close($false)}finally{[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($book)}}
 if($null -ne $settings){try{$settings.Close($false)}finally{[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($settings)}}
 if($null -ne $excel){try{$excel.Quit()}finally{[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel)}}
 $resolved=[IO.Path]::GetFullPath($stage)
 $temp=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')+'\'
 if(-not $resolved.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase)){throw 'Invalid staging directory'}
 Remove-Item -LiteralPath $resolved -Recurse -Force
}
