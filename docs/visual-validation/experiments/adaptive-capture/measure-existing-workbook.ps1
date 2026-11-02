<#
.SYNOPSIS
Read-only geometry probe for the Rust layout test, using a separate inspection copy.
.DESCRIPTION
The test supplies CAPTURE_GEOMETRY_WORKBOOK. This is an experiment transport,
not the corpus launcher: production geometry comes from the existing Office.js
workers. No cells, visibility, calculation settings or window state are changed.
#>
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName Microsoft.VisualBasic
$workbook = [Microsoft.VisualBasic.Interaction]::GetObject($env:CAPTURE_GEOMETRY_WORKBOOK)
$sheet = $workbook.Worksheets.Item(1)
try {
    while ($null -ne ($line = [Console]::ReadLine())) {
        $command = $line | ConvertFrom-Json
        $range = $sheet.Range($command.range)
        try {
            [Console]::WriteLine((@{
                top = [double]$range.Top
                left = [double]$range.Left
                width = [double]$range.Width
                height = [double]$range.Height
            } | ConvertTo-Json -Compress))
        } finally {
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($range)
        }
    }
} finally {
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($sheet)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($workbook)
}
