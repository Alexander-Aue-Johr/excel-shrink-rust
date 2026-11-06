# Verify launch-environment types without starting processes or touching diagnostic files.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'Launch.psm1') -Force
$module = Get-Module Launch
foreach ($environment in @(@{TEMP='V:\excel-test-temp'}, [ordered]@{TEMP='V:\excel-test-temp'})) {
    $actual = & $module { param($value) Get-ExcelDiagnosticRoot $value } $environment
    if ($actual -ne 'V:\excel-test-temp\Diagnostics\EXCEL') {
        throw 'Excel TEMP was not resolved from the launch environment.'
    }
}
$fallback = & $module { Get-ExcelDiagnosticRoot @{} }
if ($fallback -ne (Join-Path $env:TEMP 'Diagnostics/EXCEL')) {
    throw 'Missing launch TEMP did not use the inherited process TEMP.'
}
'Hashtable, OrderedDictionary and inherited-TEMP diagnostic roots verified.'
