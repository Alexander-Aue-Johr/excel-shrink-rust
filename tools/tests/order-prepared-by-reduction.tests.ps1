$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root=Join-Path $repo ('.tmp/reduction-order-tests/'+[Guid]::NewGuid().ToString('N'))
$inputs=Join-Path $root 'prepared'
New-Item -ItemType Directory -Force $inputs|Out-Null
foreach($case in @(@('a',100,10),@('b',1000,500),@('c',2000,1000),@('d',100,110))){
    $zip=[IO.Compression.ZipFile]::Open((Join-Path $inputs ($case[0]+'.zip')),[IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach($file in @(@('original.xlsx',$case[1]),@('default.xlsx',$case[2]))){
            $stream=$zip.CreateEntry($file[0]).Open()
            try {$bytes=[byte[]]::new($file[1]);$stream.Write($bytes,0,$bytes.Length)} finally {$stream.Dispose()}
        }
        $entry=$zip.CreateEntry('preparation.json')
        $writer=[IO.StreamWriter]::new($entry.Open())
        $meta=@{identity=$case[0];name=($case[0]+'.xls');variants=@(@{profile='default';member='default.xlsx'})}
        try {$writer.Write(($meta|ConvertTo-Json -Depth 4))}finally{$writer.Dispose()}
    } finally {$zip.Dispose()}
}
powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo 'tools/order-prepared-by-reduction.ps1') -PreparedDirectory $inputs -OutputDirectory (Join-Path $root 'output')
if($LASTEXITCODE -ne 0){throw 'Ranking command failed.'}
$list=Get-Content (Join-Path $root 'output/workbook-list.json') -Raw|ConvertFrom-Json
if(($list -join ',') -ne 'a,c,b,d'){throw 'Relative ratio, savings tie-break or growth order is incorrect.'}
'Prepared reduction ordering passed.'