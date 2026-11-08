param(
    [Parameter(Mandatory=$true)][string]$ExcelExecutable,
    [Parameter(Mandatory=$true)][string]$Seed
)
$ErrorActionPreference = 'Stop'
$sources = @('CaptureProcesses.cs', 'CaptureDesktop.cs', 'CaptureWorkbookSession.cs', 'CaptureWorkbookWorker.cs') |
    ForEach-Object { Join-Path $PSScriptRoot $_ }
Add-Type -Path $sources -ReferencedAssemblies @('System', 'System.Core', 'System.Web.Extensions')
[CaptureWorkbookWorker]::Run($ExcelExecutable, $Seed)
