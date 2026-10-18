
param(
    [Parameter(Mandatory=$true)][string]$InputPath,
    [Parameter(Mandatory=$true)][string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$excel = $null
$workbook = $null

try {
    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $false
    $excel.DisplayAlerts = $false
    $excel.AskToUpdateLinks = $false
    $excel.AutomationSecurity = 3

    $workbook = $excel.Workbooks.Open($InputPath, 0, $false)
    $workbook.ForceFullCalculation = $true
    $excel.CalculateFullRebuild()
    $workbook.SaveAs($OutputPath, 51)
}
finally {
    if ($workbook -ne $null) {
        $workbook.Close($false)
    }
    if ($excel -ne $null) {
        $excel.Quit()
    }
}
