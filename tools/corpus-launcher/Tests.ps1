[CmdletBinding()]
param([switch]$NativeArgumentTest, [switch]$DialogTest)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot 'Core.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'Launch.psm1') -Force
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$root=Join-Path $repo ('.tmp/corpus-launcher-tests/'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.IO.Compression
$script:checks=0
function Assert($Condition,[string]$Message){if(-not $Condition){throw $Message};$script:checks++}
function Reject([scriptblock]$Action,[string]$Message){$failed=$false;try{& $Action}catch{$failed=$true};Assert $failed $Message}
function Write-Archive([string]$Path,[byte]$Content=1){
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))
    $zip=[IO.Compression.ZipFile]::Open($Path,[IO.Compression.ZipArchiveMode]::Create)
    try{$entry=$zip.CreateEntry('directory/workbook A.xlsx');$stream=$entry.Open();try{$stream.WriteByte($Content)}finally{$stream.Dispose()}}
    finally{$zip.Dispose()}
}
function Arg-Value($Plan,[string]$Name){$at=[Array]::IndexOf($Plan.Arguments,$Name);if($at -lt 0){return $null};return $Plan.Arguments[$at+1]}

$catalog=[pscustomobject]@{version=1;corpora=@()};$original=[pscustomobject]@{corpora=@();access_url='https://example.invalid/private-access'};$portable=[pscustomobject]@{corpora=@()}
foreach($name in @('Enron','Destatis','Data.gov')){
    $url='https://example.invalid/'+$name+'.zip?access=local-test-only';$id=Get-TextHash $url
    $path=Join-Path $root ('.tmp/archive-regression/downloads/'+$id.Substring(0,20)+'.zip');Write-Archive $path
    $catalog.corpora += [pscustomobject]@{name=$name;id=$id;url=$null;cache_file=($id.Substring(0,20)+'.zip');total_workbooks=1}
    $original.corpora += [pscustomobject]@{name=$name;url=$url}
    $portable.corpora += [pscustomobject]@{name=$name;id=$id;url=$path;total_workbooks=1}
}
$catalogPath=Join-Path $root 'catalog.json';Save-StudyJson $catalogPath $catalog
Save-StudyJson (Join-Path $root '.tmp/archive-regression/study-request.json') $original
Save-StudyJson (Join-Path $root '.tmp/distributed-study-request.json') $portable
$approved='a'*64;$previous='b'*64
Save-StudyJson (Join-Path $root '.tmp/archive-regression/tower-active-run.json') ([pscustomobject]@{MachineLabel=[Environment]::MachineName;ComparisonWorkers=3;WorkerMemoryMiB=8192;ComMemoryMiB=4096;ResidentMemoryMiB=32768;ReusePreparedFrom=@($approved);BinarySha256=$previous;Arguments=@('regression','study');OutputRoot='.tmp/existing output';RunCorpus='Data.gov';ImportReport='old-stale-field.json'})
$configPath=Join-Path $root '.tmp/corpus-launcher/config.json'
$output=Join-Path $root '.tmp/existing output'
[void][IO.Directory]::CreateDirectory((Join-Path $output 'records'));[void][IO.Directory]::CreateDirectory((Join-Path $output 'prepared'))
[IO.File]::WriteAllText((Join-Path $output 'records/keep.json'),'existing scientific record')
[IO.File]::WriteAllText((Join-Path $output 'prepared/keep.zip'),'existing retained bundle')
$before=@{};Get-ChildItem -LiteralPath $root -File -Recurse | ForEach-Object {$before[$_.FullName]=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}
$config=Import-StudyConfiguration $root $configPath $catalogPath
Assert ($config.corpora.Count -eq 3) 'Migration lost corpus definitions.'
Assert ($config.settings.worker_memory_mib -eq 8192 -and $config.settings.resident_memory_mib -eq 32768) 'Machine-specific caps changed.'
Assert ($config.settings.reuse_prepared_from.Count -eq 1 -and $config.settings.reuse_prepared_from[0] -eq $approved) 'Compatibility allowlist was not preserved.'
Assert ($config.settings.reuse_prepared_from -notcontains $previous) 'Previous executable was implicitly approved.'
Assert ($null -eq $config.settings.import_report) 'A stale metadata field added an unwanted import.'
Assert ($config.settings.tall_sheet_threshold -eq 40 -and $config.settings.tall_sheet_edge_tiles -eq 2) 'Tall-sheet defaults are missing.'
$plan=New-StudyLaunchPlan $config $root 'target/release/excel-shrink-cli.exe' (Join-Path $root '.tmp/runs/example with spaces')
Assert ($plan.Request.corpora.Count -eq 3) 'Selecting one corpus removed other import identities.'
foreach($entry in $portable.corpora){$mapped=@($plan.Request.corpora | Where-Object id -eq $entry.id);Assert ($mapped.Count -eq 1 -and $mapped[0].url -eq $entry.url) 'Existing ZIP binding or stable identity changed.'}
Assert ((Arg-Value $plan '--run-corpus') -eq 'Data.gov') 'Wrong execution corpus.'
Assert ($null -eq (Arg-Value $plan '--size-part')) 'Whole-corpus mode added a partition.'
Assert ((Arg-Value $plan '--output-root') -eq $output) 'Existing output directory was changed.'
Assert ($plan.ExportResults.StartsWith((Join-Path $output 'exports'))) 'Result export was not retained.'
Assert ($plan.Environment.EXCEL_SHRINK_TALL_SHEET_THRESHOLD -eq '40' -and $plan.Environment.EXCEL_SHRINK_TALL_SHEET_EDGE_TILES -eq '2') 'Tall-sheet settings do not reach the runner.'
foreach($path in $before.Keys){Assert ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq $before[$path]) 'Migration rewrote an existing artifact.'}
Assert ((@(Get-ChildItem -LiteralPath $root -File -Recurse)).Count -eq $before.Count) 'Read-only planning wrote files.'
Assert (-not [IO.File]::Exists($configPath)) 'Preview saved configuration unexpectedly.'
$metadataPath=Join-Path $root '.tmp/archive-regression/tower-active-run.json'
$metadata=Read-StudyJson $metadataPath
$metadata.Arguments=@('regression','study','--size-part','3/4','--order','size-asc','--run-corpus','Enron','--output-root','.tmp/actual selected output')
Save-StudyJson $metadataPath $metadata
$partitioned=Import-StudyConfiguration $root $configPath $catalogPath
Assert ($partitioned.settings.partition -eq 3 -and $partitioned.settings.partitions -eq 4) 'A distributed machine lost its previous size partition.'
Assert ($partitioned.settings.corpus -eq 'Enron' -and $partitioned.settings.order -eq 'size-asc' -and $partitioned.settings.output_root -eq '.tmp/actual selected output') 'Previous CLI arguments were overridden by stale metadata fields.'
$binding=$config.corpora[0];$bindingId=$binding.id;$originalPath=$binding.local_archive
Set-StudyArchiveBinding $binding $originalPath $root
Assert ($null -eq (Get-Value $binding 'archive_sha256')) 'Unchanged archive binding was unnecessarily hashed.'
$sameArchive=Join-Path $root 'same archive elsewhere.zip';[IO.File]::Copy($originalPath,$sameArchive)
Set-StudyArchiveBinding $binding $sameArchive $root
Assert ($binding.local_archive -eq $sameArchive -and $binding.id -eq $bindingId) 'Rebinding the same archive changed corpus identity.'
$otherArchive=Join-Path $root 'different archive.zip';Write-Archive $otherArchive 2
Reject {Set-StudyArchiveBinding $binding $otherArchive $root} 'A different archive with the same workbook count reused an existing ID.'
Assert ($binding.local_archive -eq $sameArchive) 'Rejected archive rebinding changed the stored locator.'
$unverifiable=[pscustomobject]@{name='Old collection';id=$bindingId;local_archive=(Join-Path $root 'missing original.zip');total_workbooks=1}
Reject {Set-StudyArchiveBinding $unverifiable $otherArchive $root $output} 'Missing original archive allowed an unverifiable rebind over historical results.'
Assert ($unverifiable.local_archive.EndsWith('missing original.zip')) 'Unverifiable rebinding changed the stored locator.'
Set-StudyArchiveBinding $unverifiable $sameArchive $root (Join-Path $root 'fresh output')
Assert ($unverifiable.archive_sha256.Length -eq 64) 'A fresh machine did not establish an archive hash.'

$config.settings.partitions=4;$config.settings.partition=4;$config.settings.order='size-asc';$config.settings.comparison_groups=0
$config.settings.machine_label='Laptop & "Paper" $(do-not-execute) `literal`'
$plan=New-StudyLaunchPlan $config $root 'target/release/excel-shrink-cli.exe' (Join-Path $root '.tmp/runs/second') @('result archive with spaces.zip')
Assert ((Arg-Value $plan '--size-part') -eq '4/4') 'Wrong selected partition.'
Assert ((Arg-Value $plan '--order') -eq 'size-asc') 'Partition changed the independent processing order.'
Assert ((Arg-Value $plan '--comparison-workers') -eq '0') 'Paused start was not represented.'
Assert ((Arg-Value $plan '--machine-label') -eq $config.settings.machine_label) 'Machine label was shell-interpreted.'
Assert ((Arg-Value $plan '--import-results').EndsWith('result archive with spaces.zip')) 'Portable result import was lost.'
$config.settings.partition=5;Reject {Assert-StudySettings $config.settings} 'Out-of-range partition accepted.';$config.settings.partition=4
$config.settings.comparison_groups=-1;Reject {Assert-StudySettings $config.settings} 'Negative group count accepted.';$config.settings.comparison_groups=0
foreach($name in @('tall_sheet_threshold','tall_sheet_edge_tiles')){
    $old=$config.settings.$name
    foreach($invalid in @(0,-1,1.5,2147483648)){$config.settings.$name=$invalid;Reject {Assert-StudySettings $config.settings} "Invalid $name accepted: $invalid"}
    $config.settings.$name=$old
}
$reserve=$config.settings.system_reserve_mib;$config.settings.system_reserve_mib=512;Reject {Assert-StudySettings $config.settings} 'A system reserve below the resident runner minimum was accepted.';$config.settings.system_reserve_mib=$reserve
Reject {ConvertTo-WindowsArgument "line`nbreak"} 'Control character in argv accepted.'
Reject {Add-StudyCorpus $config 'Enron' 'https://example.invalid/new.zip' $root} 'Duplicate corpus name accepted.'
Reject {Add-StudyCorpus $config 'Bad' 'ftp://example.invalid/new.zip' $root} 'Unsupported archive source accepted.'
Reject {Add-StudyCorpus $config 'Credentials' 'https://user:secret@example.invalid/new.zip' $root} 'Embedded URL credentials accepted.'
$added=Add-StudyCorpus $config 'New data' 'https://example.invalid/download?id=42' $root
Assert ($added.id -eq (Get-TextHash $added.url)) 'New remote corpus ID is not deterministic.'
$localPath=Join-Path $root 'local collection.zip';Write-Archive $localPath
$local=Add-StudyCorpus $config 'Own archive' $localPath $root
Assert ($local.id -match '\A[0-9a-f]{64}\z' -and $local.local_archive -eq $localPath) 'Local corpus registration failed.'
Save-StudyJson $configPath $config
$loaded=Import-StudyConfiguration $root $configPath $catalogPath
Assert ($loaded.corpora.Count -eq 5 -and $loaded.corpora[-1].id -eq $local.id) 'Additional corpora did not persist.'
$loaded.settings.PSObject.Properties.Remove('tall_sheet_threshold');$loaded.settings.PSObject.Properties.Remove('tall_sheet_edge_tiles')
Save-StudyJson $configPath $loaded
$legacyBytes=[IO.File]::ReadAllText($configPath)
$loaded=Import-StudyConfiguration $root $configPath $catalogPath
Assert ($loaded.settings.tall_sheet_threshold -eq 40 -and $loaded.settings.tall_sheet_edge_tiles -eq 2) 'Existing launcher configurations did not acquire tall-sheet defaults.'
Assert ([IO.File]::ReadAllText($configPath) -ceq $legacyBytes) 'Loading legacy configuration changed it on disk.'
$loaded.settings.machine_name='DIFFERENT-COMPUTER';$loaded.settings.worker_memory_mib=65536;Save-StudyJson $configPath $loaded
$rebound=Import-StudyConfiguration $root $configPath $catalogPath
Assert ($rebound.settings.worker_memory_mib -ne 65536 -and $rebound.settings.comparison_groups -eq 1) 'A copied machine config imposed foreign memory settings.'
Assert (-not (Test-StudyOutputBusy $output)) 'An idle output was marked busy.'
$heldLock=[IO.File]::Open((Join-Path $output '.run.lock'),[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::ReadWrite)
try {
    $heldLock.Lock(0,1)
    Assert (Test-StudyOutputBusy $output) 'An output held by another writer was marked idle.'
    Reject {Start-StudyLaunch $config $configPath $root $plan} 'A second writer started for an active output.'
    Assert (-not [IO.Directory]::Exists($plan.RunDirectory)) 'A rejected second start wrote run files or launched a build.'
} finally { $heldLock.Unlock(0,1);$heldLock.Dispose() }

if($DialogTest){
    . (Join-Path $PSScriptRoot 'Dialog.ps1')
    $dialog=New-CorpusLaunchDialog $config $configPath $root 'target/release/excel-shrink-cli.exe' @('requested import.zip')
    try {
        Assert ($dialog.Controls.Corpus.Items.Count -eq 5) 'WPF catalog did not retain added corpora.'
        Assert ($dialog.Controls.PartitionButtons.Children.Count -eq 5) 'WPF partition choices are missing.'
        Assert ($dialog.State.imports.Count -eq 1 -and $dialog.State.imports[0] -eq 'requested import.zip') 'GUI silently discarded command-line result imports.'
        $click=[Windows.RoutedEventArgs]::new([Windows.Controls.Button]::ClickEvent)
        $dialog.Controls.Parts.Text='8';$dialog.Controls.Save.RaiseEvent($click)
        Assert ($dialog.Controls.Status.Text -match 'Aufteilen' -and (Read-StudyJson $configPath).settings.partitions -eq 4) 'An unapplied partition edit silently changed the selected fraction.'
        $dialog.Controls.Parts.Text='3';$dialog.Controls.ApplyParts.RaiseEvent($click)
        Assert ($dialog.State.partition -eq 0 -and $dialog.Controls.PartitionButtons.Children.Count -eq 4) 'WPF repartitioning retained an invalid selection.'
        $dialog.Controls.PartitionButtons.Children[2].IsChecked=$true
        $dialog.Controls.Groups.Text='0';$dialog.Controls.LargeFirst.IsChecked=$true
        $dialog.Controls.TallThreshold.Text='75';$dialog.Controls.TallEdges.Text='3'
        $dialog.Controls.Save.RaiseEvent($click)
        $fromDialog=Read-StudyJson $configPath
        Assert ($fromDialog.settings.partition -eq 2 -and $fromDialog.settings.partitions -eq 3) 'WPF selected partition did not persist.'
        Assert ($fromDialog.settings.comparison_groups -eq 0 -and $fromDialog.settings.order -eq 'size-desc') 'WPF pause/order settings did not persist.'
        Assert ($fromDialog.settings.tall_sheet_threshold -eq 75 -and $fromDialog.settings.tall_sheet_edge_tiles -eq 3) 'WPF tall-sheet settings did not persist.'
        Assert ($dialog.Controls.Status.Text -eq 'Konfiguration gespeichert. Kein Lauf gestartet.') 'WPF save handler failed.'
        $dialog.Controls.Groups.Text='-1';$dialog.Controls.Save.RaiseEvent($click)
        Assert ($dialog.Controls.Status.Text -match 'comparison_groups') 'WPF accepted a negative group count.'
        Assert ((Read-StudyJson $configPath).settings.comparison_groups -eq 0) 'WPF invalid edit overwrote the valid configuration.'
        $dialog.Controls.Groups.Text='0';$dialog.Controls.TallEdges.Text='0';$dialog.Controls.Save.RaiseEvent($click)
        Assert ($dialog.Controls.Status.Text -match 'tall_sheet_edge_tiles' -and (Read-StudyJson $configPath).settings.tall_sheet_edge_tiles -eq 3) 'WPF accepted zero edge captures or overwrote valid settings.'
        $dialog.Controls.TallEdges.Text='3';$dialog.Controls.Save.RaiseEvent($click)
    } finally { $dialog.Window.Close() }
}

if($NativeArgumentTest){
    $helper=Join-Path $root 'argument helper.exe'
    Add-Type -TypeDefinition @'
using System;
using System.IO;
public static class CorpusArgumentProbe {
 public static void Main(string[] args) {
  if(args.Length > 1 && args[0] == "regression") {
   string output = args[Array.IndexOf(args,"--output-root") + 1];
   Directory.CreateDirectory(output);
   using(var held = new FileStream(Path.Combine(output,".run.lock"),FileMode.OpenOrCreate,FileAccess.ReadWrite,FileShare.ReadWrite)) {
    held.Lock(0,1);
    File.WriteAllText(Path.Combine(output,"child-memory.txt"),Environment.GetEnvironmentVariable("EXCEL_SHRINK_WORKER_MEMORY_MIB"));
    Console.WriteLine("Synthetic runner: no Excel and no corpus work.");
    System.Threading.Thread.Sleep(4000);
    held.Unlock(0,1);
   }
   return;
  }
  using(var writer=new BinaryWriter(File.Create(args[0]))) {
   writer.Write(args.Length-1); for(int i=1;i<args.Length;i++) writer.Write(args[i]);
  }
 }
}
'@ -OutputAssembly $helper -OutputType ConsoleApplication
    $record=Join-Path $root 'arguments.bin'
    $values=@('', 'a b', 'a"b', 'C:\trailing directory\', 'backslash\"quote', '$() & `literal`', 'äö ü')
    $commandLine=(@($record)+$values | ForEach-Object {ConvertTo-WindowsArgument $_}) -join ' '
    $child=Start-Process -FilePath $helper -ArgumentList $commandLine -WindowStyle Hidden -Wait -PassThru
    Assert ($child.ExitCode -eq 0) 'Native argument helper failed.'
    $reader=[IO.BinaryReader]::new([IO.File]::OpenRead($record))
    try{Assert ($reader.ReadInt32() -eq $values.Count) 'Windows command-line quoting changed argument count.';foreach($value in $values){Assert ($reader.ReadString() -ceq $value) 'Windows command-line quoting changed an argument.'}}
    finally{$reader.Dispose()}
    $config.settings.comparison_groups=0;$config.settings.build_release=$false
    $config.settings.output_root=Join-Path $root 'synthetic detached output'
    $listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$listener.Start()
    try{$config.settings.control_room_port=$listener.LocalEndpoint.Port}finally{$listener.Stop()}
    $fakePlan=New-StudyLaunchPlan $config $root $helper (Join-Path $root 'synthetic detached launch')
    $parentMemory=[Environment]::GetEnvironmentVariable('EXCEL_SHRINK_WORKER_MEMORY_MIB','Process')
    $started=Start-StudyLaunch $config $configPath $root $fakePlan
    Assert ($started.Pid -gt 0 -and $started.BinarySha256.Length -eq 64) 'Detached launch lacks process/build provenance.'
    Assert ([IO.File]::Exists($started.RunMetadata) -and [IO.File]::Exists($started.RequestFile)) 'Detached launch did not persist metadata/request.'
    Assert ((Read-StudyJson $started.RequestFile).corpora.Count -eq 5) 'Detached launch dropped unselected corpus identities.'
    Assert ([Environment]::GetEnvironmentVariable('EXCEL_SHRINK_WORKER_MEMORY_MIB','Process') -ceq $parentMemory) 'Detached launch changed the parent memory environment.'
    Assert ([IO.File]::ReadAllText((Join-Path $fakePlan.OutputRoot 'child-memory.txt')) -eq [string]$config.settings.worker_memory_mib) 'Detached child did not receive its own memory settings.'
    Assert (Test-StudyOutputBusy $fakePlan.OutputRoot) 'Detached fake runner is not holding its output lock.'
    $fakeProcess=Get-Process -Id $started.Pid
    Assert ($fakeProcess.WaitForExit(6000)) 'Synthetic detached runner failed to exit.'
    Assert ([IO.File]::ReadAllText($started.Stdout).Contains('Synthetic runner: no Excel and no corpus work.')) 'Detached stdout was not retained.'
}

$receipt=[pscustomobject]@{passed=$true;checks=$script:checks;native_argument_test=[bool]$NativeArgumentTest;dialog_test=[bool]$DialogTest;root=$root;production_untouched=$true;network_downloads=0}
Save-StudyJson (Join-Path $root 'receipt.json') $receipt
$receipt | ConvertTo-Json
