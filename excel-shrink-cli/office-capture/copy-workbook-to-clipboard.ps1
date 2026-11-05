<#
.SYNOPSIS
Copies an inspection file to the Windows file clipboard for Explorer paste.
.DESCRIPTION
Uses Windows PowerShell's built-in Windows Forms clipboard support. No SDK or
additional runtime installation is required. The caller supplies a disposable
inspection copy, never a corpus source or a workbook owned by a capture worker.
#>
param([Parameter(Mandatory=$true)][string]$Path)
$ErrorActionPreference = 'Stop'
if (-not [IO.File]::Exists($Path)) { throw 'The inspection copy does not exist.' }
Add-Type -AssemblyName System.Windows.Forms
$files = New-Object Collections.Specialized.StringCollection
[void]$files.Add([IO.Path]::GetFullPath($Path))
[Windows.Forms.Clipboard]::SetFileDropList($files)
