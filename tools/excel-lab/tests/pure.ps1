[CmdletBinding()]
param([string]$ToolRoot)
$ErrorActionPreference='Stop'
Set-StrictMode -Version 2.0
if (-not $ToolRoot) { $ToolRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')) }
$labRoot=[IO.Path]::GetFullPath($ToolRoot)
$repo=[IO.Path]::GetFullPath((Join-Path $labRoot '../..'))
$script:LabPassed=0
function Assert-Lab([bool]$Condition,[string]$Label) {
    if (-not $Condition) { throw "TEST FAILED: $Label" }
    $script:LabPassed++; [Console]::WriteLine('PASS '+$Label)
}
function Reject-Lab([scriptblock]$Operation,[string]$Label) {
    $caught=$false
    try { & $Operation | Out-Null } catch { $caught=$true }
    Assert-Lab $caught $Label
}
foreach ($file in (Get-ChildItem -LiteralPath $labRoot -Recurse -Filter '*.ps1' -File)) {
    $tokens=$null; $errors=$null
    [void][Management.Automation.Language.Parser]::ParseFile($file.FullName,[ref]$tokens,[ref]$errors)
    if ($errors.Count -gt 0) { throw ($file.FullName+': '+(($errors | ForEach-Object {$_.ToString()}) -join "`n")) }
    Assert-Lab $true ('PowerShell syntax: '+$file.Name)
}
. (Join-Path $repo 'tools/excel-height-check/json_diff.ps1') -FunctionsOnly
. (Join-Path $labRoot 'Plan.ps1')
$temp=Join-Path ([IO.Path]::GetTempPath()) ('excel-lab-pure-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
try {
    # This parses/constructs WPF but never shows a dialog or starts Excel.
    Add-Type -AssemblyName PresentationFramework,PresentationCore,WindowsBase
    Add-Type -Path (Join-Path $labRoot 'FolderPicker.cs')
    $xaml=[xml][IO.File]::ReadAllText((Join-Path $labRoot 'Options.xaml'))
    $reader=[Xml.XmlNodeReader]::new($xaml)
    try { $window=[Windows.Markup.XamlReader]::Load($reader) } finally { $reader.Close() }
    try {
        Assert-Lab ($null -ne $window.FindName('RunButton')) 'WPF XAML loads without showing a window'
        Assert-Lab ($window.FindName('RowsBox').SelectedItem.Tag -ceq 'none') 'fast range metadata is the default; exhaustive rows are opt-in'
        Assert-Lab ($window.FindName('ModeBox').SelectedItem.Tag -ceq 'shrink') 'default workflow is shrink-and-compare'
    } finally { $window.Close() }
    # A CLR IEnumerable stand-in tests the non-enumerating return contract used
    # for Excel COM. Actual IDispatch/Excel is tested by the VBA fixture task.
    Add-Type @'
using System;
using System.Collections;
public class LabTestEnumerable : IEnumerable {
    public string Name { get { return "one workbook"; } }
    public IEnumerator GetEnumerator() { return new string[] {"sheet1","sheet2"}.GetEnumerator(); }
}
public class LabTestFactory {
    public LabTestEnumerable Open(string name, int update, bool readOnly) {
        if(name != "test" || update != 0 || !readOnly) throw new ArgumentException("wrong explicit arguments");
        return new LabTestEnumerable();
    }
}
'@
    $value=Invoke-LabComMethod -Target ([LabTestFactory]::new()) -Method Open -Arguments ([object[]]@('test',[int]0,[bool]$true))
    Assert-Lab ($value -is [LabTestEnumerable] -and $value.Name -ceq 'one workbook') 'method dispatch returns one enumerable object, not its children'
    foreach ($address in @('A1','$A$1:$XFD$1048576','b2:D99','Z12')) { Test-LabAddress $address; Assert-Lab $true ('valid plan address '+$address) }
    foreach ($address in @('A0','A1048577','XFE1','B2:A1','Sheet1!A1','[book]A1','=A1','A1,B2','1:10','A:A','A1:Z9999999999999999999999')) { Reject-Lab { Test-LabAddress $address } ('reject unsafe plan address '+$address) }
    $planPath=Join-Path $temp 'plan.json'
    [IO.File]::WriteAllText($planPath,'{"captures":[{"sheet":"Sheet \"quoted\"","range":"A1:C8","format":-4147,"transparent":true}],"overflow":[]}')
    $external=Get-LabExternalPlan $planPath
    Assert-Lab ($external.captures.Count -eq 1 -and $external.captures[0].pair -ceq '000000') 'one-entry JSON plan remains an array and gets explicit default pair'
    foreach ($bad in @('{"captures":{}}','{"captures":[{"sheet":"S","range":"A1","transparent":true}]}','{"captures":[{"sheet":"S","range":"A1","format":9}]}','{"overflow":[{"sheet":"S","capture":"A1:C8","probe":"B1:C8"}]}','{"unknown":true}')) {
        [IO.File]::WriteAllText($planPath,$bad); Reject-Lab { Get-LabExternalPlan $planPath } 'reject malformed or ambiguous plan'
    }
    $config=@{shapes=$true;hash_values=$false;compare_values=$false;row_scope='none';screenshots=$true;storage='png';copy_matrix=$false;transparent_emf=$false;compare_visual_vba=$false;compare_visual_rust=$false;block_cells=65536;max_cells=20000000;tile_points=1200;max_tiles=1024;max_differences=1000}
    $pairs=@(@{id='000000';name='a & b.xlsx';original='C:\path & "quoted"\a.xlsx';shrunk=''})
    $xml=New-LabPlan $config $pairs $external
    Assert-Lab ($xml.DocumentElement.SelectSingleNode('pairs/pair').GetAttribute('original') -ceq $pairs[0].original) 'XML plan preserves literal paths and escapes ampersands/quotes'
    $external.captures[0].pair='999999'
    Reject-Lab { New-LabPlan $config $pairs $external } 'unknown pair rejected before shrink'
    $cursor=New-LabProgressCursor (Join-Path $temp 'progress.log')
    Assert-Lab (-not (Read-LabProgress $cursor).Advanced) 'missing progress file is harmless'
    [IO.File]::WriteAllText($cursor.Path,"one`r`npar",[Text.Encoding]::ASCII)
    $r=Read-LabProgress $cursor
    Assert-Lab ($r.Advanced -and $r.Lines.Length -eq 1 -and $r.Lines[0] -ceq 'one') 'progress tail delivers completed line only'
    [IO.File]::AppendAllText($cursor.Path,"tial`n",[Text.Encoding]::ASCII)
    $r=Read-LabProgress $cursor
    Assert-Lab ($r.Lines.Length -eq 1 -and $r.Lines[0] -ceq 'partial') 'progress tail merges a split line'
    Assert-Lab (-not (Read-LabProgress $cursor).Advanced) 'unchanged progress does not reset timeout'
    [IO.File]::WriteAllText($cursor.Path,"x`n",[Text.Encoding]::ASCII)
    $r=Read-LabProgress $cursor
    Assert-Lab ($r.Lines.Length -eq 1 -and $r.Lines[0] -ceq 'x') 'truncated progress file restarts safely'
    [IO.File]::AppendAllText($cursor.Path,'tail',[Text.Encoding]::ASCII)
    [void](Read-LabProgress $cursor)
    $r=Read-LabProgress $cursor -FlushPending
    Assert-Lab ($r.Lines.Length -eq 1 -and $r.Lines[0] -ceq 'tail') 'trailing progress fragment is not lost at exit'
    $sources=@(Get-LabSources $labRoot)
    Assert-Lab ($sources.Count -ge 13 -and @($sources | Where-Object {$_.name -ceq 'HeightProbe'}).Count -eq 1) 'shared host includes original HeightProbe and all lab modules'
    $signature=Get-LabSignature $sources
    Assert-Lab ($signature -match '^[a-f0-9]{64}$' -and $signature -ceq (Get-LabSignature $sources)) 'source manifest signature is deterministic'
    # Build fake OOXML packages only to test ZIP/entry preflight, not to pretend
    # that an Excel workbook or compiled VBA project was created.
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $fake=Join-Path $temp 'empty.zip'
    $archive=[IO.Compression.ZipFile]::Open($fake,[IO.Compression.ZipArchiveMode]::Create)
    $archive.Dispose()
    Reject-Lab { Test-LabWorkbookPackage $fake } 'invalid workbook package rejected before COM'
    $fakeVba=Join-Path $temp 'fake.xlsm'
    $archive=[IO.Compression.ZipFile]::Open($fakeVba,[IO.Compression.ZipArchiveMode]::Create)
    foreach ($entry in @('[Content_Types].xml','xl/workbook.xml')) { [void]$archive.CreateEntry($entry) }; $archive.Dispose()
    Reject-Lab { Test-LabWorkbookPackage $fakeVba -RequireVba } 'helper without vbaProject.bin rejected'
    # A token mismatch must never reach Process.GetProcessById/Kill.
    $statePath=Join-Path $temp 'state.json'
    Write-LabJson @{owner_token='other';pid=$PID;start_ticks='0'} $statePath
    Stop-LabOwnedExcel -StateFile $statePath -OwnerToken 'mine' -GraceMilliseconds 0
    Assert-Lab (-not (Get-Process -Id $PID).HasExited) 'cleanup refuses a different owner token'
    Write-LabJson @{owner_token='mine';pid=$PID;start_ticks=(Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks.ToString()} $statePath
    Stop-LabOwnedExcel -StateFile $statePath -OwnerToken 'mine' -GraceMilliseconds 0
    Assert-Lab (-not (Get-Process -Id $PID).HasExited) 'cleanup refuses non-Excel processes'
    # Heartbeats are independent of stdout: a silent child can make genuine
    # progress through the VBA log without causing a false stall timeout.
    $child=Join-Path $temp 'progress-child.ps1'
    [IO.File]::WriteAllText($child,@'
param([string]$Path)
for($i=0;$i -lt 14;$i++) {
    [IO.File]::AppendAllText($Path,"batch $i`n",[Text.Encoding]::ASCII)
    Start-Sleep -Milliseconds 450
}
'@,[Text.Encoding]::ASCII)
    $progress=Join-Path $temp 'child-progress.log'
    $shell=(Get-Process -Id $PID).Path
    $result=Invoke-LoggedProcess -Executable $shell -Arguments @('-NoProfile','-File',$child,'-Path',$progress) -Stage 'test-vba-progress' -TimeoutSeconds 5 -ProgressPath $progress
    Assert-Lab ($result.exit_code -eq 0) 'real progress from a silent child prevents false timeout'
    [Console]::WriteLine("EXCEL LAB PURE TESTS PASSED: $script:LabPassed (no Excel started)")
} finally { [IO.Directory]::Delete($temp,$true) }
