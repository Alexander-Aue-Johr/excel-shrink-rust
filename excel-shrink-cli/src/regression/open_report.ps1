param([Parameter(Mandatory=$true)][string]$ReportPath)

$ErrorActionPreference = 'Stop'
Start-Process -FilePath $ReportPath
