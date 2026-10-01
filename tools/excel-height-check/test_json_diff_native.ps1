# Managed by patch_excel_json_diff.py. Pure tests: no Excel/COM activation.
[CmdletBinding()]
param([string]$ToolRoot,[switch]$NoProcessTests)
$ErrorActionPreference='Stop'
Set-StrictMode -Version 2.0
if([string]::IsNullOrWhiteSpace($ToolRoot)){
    $ToolRoot=$PSScriptRoot
}
if([string]::IsNullOrWhiteSpace($ToolRoot) -and $MyInvocation.MyCommand.Path){
    $ToolRoot=Split-Path -Parent $MyInvocation.MyCommand.Path
}
if([string]::IsNullOrWhiteSpace($ToolRoot) -or -not [IO.Directory]::Exists($ToolRoot)){
    throw "Cannot determine PowerShell test tool root. ToolRoot='$ToolRoot' ScriptPath='$($MyInvocation.MyCommand.Path)'"
}
$script:Passed=0
function Check([bool]$Condition,[string]$Label){
    if(-not $Condition){throw "TEST FAILED: $Label"}
    $script:Passed++;[Console]::WriteLine("PASS $Label")
}
function Expect-Failure([scriptblock]$Operation,[string]$Label,[string]$Pattern=''){
    $caught=$null
    try{& $Operation | Out-Null}catch{$caught=$_}
    Check ($null -ne $caught) $Label
    if($Pattern){Check ($caught.Exception.Message -match $Pattern) ($Label+' (message)')}
}
foreach($file in Get-ChildItem -LiteralPath $ToolRoot -Filter '*.ps1' -File){
    $tokens=$null;$errors=$null
    [void][Management.Automation.Language.Parser]::ParseFile($file.FullName,[ref]$tokens,[ref]$errors)
    if($errors.Count -gt 0){throw ($file.Name+': '+(($errors | ForEach-Object {$_.ToString()}) -join "`n"))}
    Check $true ('PowerShell syntax: '+$file.Name)
}
. (Join-Path $ToolRoot 'json_diff.ps1') -FunctionsOnly
Check ((Quote-NativeArgument '') -ceq '""') 'quote empty argument'
Check ((Quote-NativeArgument 'two words') -ceq '"two words"') 'quote spaces'
Check ((Quote-NativeArgument 'C:\dir\') -ceq '"C:\dir\\"') 'quote trailing backslash'
Check ((Quote-NativeArgument 'a"b') -ceq '"a\"b"') 'quote double quote'
Check ((Quote-NativeArgument 'a\"b') -ceq '"a\\\"b"') 'quote backslash before quote'
Check ((Quote-WslArgument '-d') -ceq '-d') 'WSL option remains unquoted'
Check ((Quote-WslArgument 'Ubuntu-26.04') -ceq 'Ubuntu-26.04') 'WSL distro remains unquoted'
Check ((Quote-WslArgument '/tmp/a b') -ceq '"/tmp/a b"') 'WSL path with spaces is quoted'
$jsonArrayProbe='[{"n":1},{"n":2}]' | ConvertFrom-Json
Check ($jsonArrayProbe.Count -eq 2 -and $jsonArrayProbe[0].n -eq 1 -and $jsonArrayProbe[1].n -eq 2) 'PowerShell JSON array assignment preserves elements'
$temp=Join-Path ([IO.Path]::GetTempPath()) ('json-diff-tests-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
try{
    $rows=Join-Path $temp 'rows.tsv'
    [IO.File]::WriteAllText($rows,"1`t2`t15`t0`n3`t8`t15`t0`n9`t9`t0`t1`n10`t20`t30`t0`n")
    $writer=[IO.StringWriter]::new()
    $n=Write-RowRuns $writer $rows 1 20
    $actual=('['+$writer.ToString()+']') | ConvertFrom-Json
    Check ($n -eq 3 -and $actual.Count -eq 3) ("merge identical runs across batch boundary (runCount=$n parsedCount=$($actual.Count) json=$($writer.ToString().Trim()))")
    Check ($actual[0].from -eq 1 -and $actual[0].to -eq 8) 'merged inclusive endpoints'
    Check ($actual[1].hidden -and $actual[1].height_points -eq 0) 'hidden row retained'
    Check ($actual[2].height_points -eq 30) 'height change retained'
    $writer.Dispose()
    [IO.File]::WriteAllText($rows,"1`t8192`t15`t0`n8193`t1048576`t15`t0`n")
    $writer=[IO.StringWriter]::new();$n=Write-RowRuns $writer $rows 1 1048576
    $actual=('['+$writer.ToString()+']') | ConvertFrom-Json
    Check ($n -eq 1 -and $actual[0].to -eq 1048576) 'full worksheet coverage without one object per row'
    $writer.Dispose()
    [IO.File]::WriteAllText($rows,"5`t7`t0.75`t0`n")
    $writer=[IO.StringWriter]::new();$n=Write-RowRuns $writer $rows 5 7
    Check ($n -eq 1 -and $writer.ToString().Contains('0.75')) 'nonzero scan start and decimal points'
    $writer.Dispose()
    foreach($sample in @(
        @{name='gap';text="1`t2`t15`t0`n4`t5`t15`t0`n"},
        @{name='overlap';text="1`t3`t15`t0`n3`t5`t15`t0`n"},
        @{name='missing tail';text="1`t4`t15`t0`n"},
        @{name='past end';text="1`t6`t15`t0`n"},
        @{name='negative height';text="1`t5`t-1`t0`n"},
        @{name='NaN height';text="1`t5`tNaN`t0`n"},
        @{name='hidden flag';text="1`t5`t15`t2`n"},
        @{name='malformed';text="1`t5`t15`n"},
        @{name='empty';text=''}
    )){
        [IO.File]::WriteAllText($rows,[string]$sample.text)
        $writer=[IO.StringWriter]::new()
        try{Expect-Failure {Write-RowRuns $writer $rows 1 5} ('reject '+$sample.name)}finally{$writer.Dispose()}
    }
    # Independent randomized round-trip: original row vectors -> arbitrary batch boundaries -> runs.
    $random=[Random]::new(173)
    for($trial=0;$trial -lt 12;$trial++){
        $lines=[Collections.Generic.List[string]]::new();$heights=@();$hidden=@()
        for($row=1;$row -le 80;$row++){
            $h=[double]($random.Next(0,4)*7.5);$flag=$random.Next(0,2)
            $heights+=,$h;$hidden+=,([bool]$flag)
            $lines.Add("$row`t$row`t$($h.ToString([Globalization.CultureInfo]::InvariantCulture))`t$flag")
        }
        [IO.File]::WriteAllText($rows,($lines.ToArray() -join "`n")+"`n")
        $writer=[IO.StringWriter]::new();[void](Write-RowRuns $writer $rows 1 80)
        $actual=('['+$writer.ToString()+']') | ConvertFrom-Json
        foreach($run in $actual){for($r=$run.from;$r -le $run.to;$r++){
            if($run.height_points -ne $heights[$r-1] -or $run.hidden -ne $hidden[$r-1]){throw 'Randomized RLE round-trip failed.'}
        }}
        $writer.Dispose()
    }
    Check $true '12 randomized exact RLE round-trips'
    $id='000000';$rowDir=Join-Path $temp ('rows\'+$id)
    [void][IO.Directory]::CreateDirectory($rowDir)
    [IO.File]::WriteAllText((Join-Path $rowDir 'original-s1.tsv'),"1`t2`t15`t0`n3`t4`t15`t0`n")
    $sheet=[ordered]@{index=1;name=('Sheet '+[char]0x00E4+' "quoted"');visibility=-1;used_range=@{address='$A$1:$C$4'};row_scan=@{first_row=1;last_row=4;measured_rows=4;scope='all'}}
    $book=[ordered]@{id=$id;file=('book-'+[char]0x00E4+'.xlsx');sheets=@($sheet)}
    Write-JsonFile @($book) (Join-Path $temp 'original.raw.json')
    $manifest=@{files=@(@{id=$id;name=$book.file})}
    $result=Write-ComparisonJson 'original' $temp $manifest
    $parsed=[IO.File]::ReadAllText($result) | ConvertFrom-Json
    Check ($parsed.Count -eq 1 -and $parsed[0].sheets.Count -eq 1) 'JSON retains one-element arrays'
    Check ($parsed[0].file -ceq $book.file -and $parsed[0].sheets[0].name -ceq $sheet.name) 'JSON escapes Unicode and quotes'
    Check ($parsed[0].sheets[0].row_heights.Count -eq 1 -and $parsed[0].sheets[0].row_heights[0].to -eq 4) 'JSON contains complete canonical runs'
    if(-not $NoProcessTests){
        $fixture=Join-Path $temp 'child.ps1'
        [IO.File]::WriteAllText($fixture,@'
param([string]$Mode,[string]$Value)
$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
switch($Mode){
 'echo' {[Console]::WriteLine((ConvertTo-Json -InputObject $Value -Compress));exit 0}
 'streams' {
    [Console]::WriteLine('FIRST');[Console]::Out.Flush()
    [Console]::Error.WriteLine('STDERR');[Console]::Error.Flush()
    Start-Sleep -Milliseconds 600
    [Console]::WriteLine('LAST');exit 0
 }
 'fail' {[Console]::Error.WriteLine('EXPECTED FAILURE');exit 7}
 'stall' {Start-Sleep -Seconds 10;exit 0}
}
'@,[Text.UTF8Encoding]::new($false))
        $shell=(Get-Process -Id $PID).Path
        foreach($arg in @('','two words','C:\dir\','a"b','a\"b',('Umlaut '+[char]0x00E4+' $ % & end\'))){
            $r=Invoke-LoggedProcess -Executable $shell -Arguments @('-NoProfile','-File',$fixture,'-Mode','echo','-Value',$arg) -Stage 'test-quote' -Capture -TimeoutSeconds 15
            $echo=ConvertFrom-Json -InputObject $r.stdout
            Check ($echo -ceq $arg) 'native argument round-trip'
        }
        $events=[Collections.Generic.List[object]]::new()
        $originalLog=(Get-Item Function:\Write-RunLog).ScriptBlock
        function Write-RunLog([string]$Stage,[string]$Message){$events.Add(@{message=$Message;time=[DateTime]::UtcNow});& $originalLog $Stage $Message}
        try{
            $r=Invoke-LoggedProcess -Executable $shell -Arguments @('-NoProfile','-File',$fixture,'-Mode','streams') -Stage 'test-streams' -Capture -TimeoutSeconds 15
            $first=@($events | Where-Object {$_.message -ceq 'FIRST'})[0]
            $last=@($events | Where-Object {$_.message -ceq 'LAST'})[0]
            Check (($last.time-$first.time).TotalMilliseconds -ge 250) 'stdout is delivered before process completion'
            Check (@($events | Where-Object {$_.message -ceq 'STDERR'}).Count -eq 1) 'stderr is drained concurrently'
        }finally{Set-Item Function:\Write-RunLog $originalLog}
        Expect-Failure {Invoke-LoggedProcess -Executable $shell -Arguments @('-NoProfile','-File',$fixture,'-Mode','fail') -Stage 'test-exit' -TimeoutSeconds 15} 'child nonzero exit rejected' 'exit 7'
        Expect-Failure {Invoke-LoggedProcess -Executable $shell -Arguments @('-NoProfile','-File',$fixture,'-Mode','stall') -Stage 'test-timeout' -TimeoutSeconds 1} 'child stall rejected and terminated' 'no progress for 1'
    }
    [Console]::WriteLine("ALL PURE TESTS PASSED: $script:Passed (Excel/COM was not started)")
}finally{
    # This directory was created above with a fresh GUID and contains only test fixtures.
    if([IO.Directory]::Exists($temp)){[IO.Directory]::Delete($temp,$true)}
}
