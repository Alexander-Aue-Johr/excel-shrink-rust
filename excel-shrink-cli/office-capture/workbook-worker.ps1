param(
    [Parameter(Mandatory=$true)][string]$ExcelExecutable,
    [Parameter(Mandatory=$true)][string]$Seed,
    [ValidateSet('immediate','owned')][string]$Startup = 'immediate'
)
$ErrorActionPreference = 'Stop'
$sources = @('CaptureProcesses.cs', 'CaptureDesktop.cs', 'CaptureWorkbookSession.cs', 'CaptureWorkbookWorker.cs') |
    ForEach-Object { Join-Path $PSScriptRoot $_ }
Add-Type -Path $sources -ReferencedAssemblies @('System', 'System.Core', 'System.Web.Extensions')
if ($Startup -eq 'owned' -and [Console]::ReadLine() -ne 'start') {
    throw 'Workbook worker did not receive its ownership gate.'
}
[CaptureWorkbookWorker]::Run($ExcelExecutable, $Seed)
