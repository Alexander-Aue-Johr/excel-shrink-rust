[CmdletBinding()]
param(
    [string]$ConfigurationPath,
    [string]$ExcelShrinkExe,
    [string]$Corpus,
    [int]$Partitions = -1,
    [int]$Partition = -1,
    [ValidateSet('size-asc','size-desc')][string]$Order,
    [int]$Groups = -1,
    [int]$TallSheetThreshold = -1,
    [int]$TallSheetEdgeTiles = -1,
    [string]$OutputRoot,
    [string]$MachineLabel,
    [int]$ControlRoomPort = -1,
    [string[]]$ImportResults = @(),
    [string[]]$ApprovePreparedFrom = @(),
    [switch]$HideExcel,
    [switch]$SkipBuild,
    [switch]$Headless,
    [switch]$Preview,
    [switch]$MigrateOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Import-Module (Join-Path $PSScriptRoot 'corpus-launcher/Core.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'corpus-launcher/Launch.psm1') -Force
if(-not $ConfigurationPath){$ConfigurationPath=Join-Path $repo '.tmp/corpus-launcher/config.json'}
$ConfigurationPath=Resolve-StudyPath $repo $ConfigurationPath
if(-not $ExcelShrinkExe){$ExcelShrinkExe=Join-Path $repo 'target/release/excel-shrink-cli.exe'}
$configuration=Import-StudyConfiguration $repo $ConfigurationPath
if($Corpus){$configuration.settings.corpus=$Corpus}
if($PSBoundParameters.ContainsKey('Partitions')){$configuration.settings.partitions=$Partitions}
if($PSBoundParameters.ContainsKey('Partition')){$configuration.settings.partition=$Partition}
if($Order){$configuration.settings.order=$Order}
if($PSBoundParameters.ContainsKey('Groups')){$configuration.settings.comparison_groups=$Groups}
if($PSBoundParameters.ContainsKey('TallSheetThreshold')){$configuration.settings.tall_sheet_threshold=$TallSheetThreshold}
if($PSBoundParameters.ContainsKey('TallSheetEdgeTiles')){$configuration.settings.tall_sheet_edge_tiles=$TallSheetEdgeTiles}
if($OutputRoot){$configuration.settings.output_root=$OutputRoot}
if($MachineLabel){$configuration.settings.machine_label=$MachineLabel}
if($PSBoundParameters.ContainsKey('ControlRoomPort')){$configuration.settings.control_room_port=$ControlRoomPort}
if($PSBoundParameters.ContainsKey('HideExcel')){$configuration.settings.hide_excel=[bool]$HideExcel}
if($SkipBuild){$configuration.settings.build_release=$false}
if($ApprovePreparedFrom.Count){$configuration.settings.reuse_prepared_from=@(@($configuration.settings.reuse_prepared_from)+$ApprovePreparedFrom | Select-Object -Unique)}
Assert-StudySettings $configuration.settings

if($Preview -or $MigrateOnly -or $Headless){
    $stamp=(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8)
    $directory=Join-Path ([IO.Path]::GetDirectoryName($ConfigurationPath)) ('runs/'+$stamp)
    $plan=New-StudyLaunchPlan $configuration $repo $ExcelShrinkExe $directory $ImportResults
    if($Preview){Get-StudyLaunchSummary $plan $configuration | ConvertTo-Json -Depth 8;exit 0}
    if($MigrateOnly){Save-StudyJson $ConfigurationPath $configuration;Get-StudyLaunchSummary $plan $configuration | ConvertTo-Json -Depth 8;exit 0}
    Start-StudyLaunch $configuration $ConfigurationPath $repo $plan | ConvertTo-Json -Depth 12
    exit 0
}
. (Join-Path $PSScriptRoot 'corpus-launcher/Dialog.ps1')
$result=Show-CorpusLaunchDialog $configuration $ConfigurationPath $repo $ExcelShrinkExe $ImportResults
if($result){$result | ConvertTo-Json -Depth 12}
