param(
    [string]$OutputDirectory = (Join-Path $env:TEMP 'excel-shrink-row-top-change')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

function Save-RangeImage {
    <#
    .SYNOPSIS
    Saves Excel's rendered image of the changed row and its neighbors.
    .DESCRIPTION
    CopyPicture uses the Windows clipboard. The workbook must be visible so
    Excel can render the selected range in the active desktop session.
    #>
    param($Workbook, [string]$ImagePath)

    $sheet = $Workbook.Worksheets.Item('Rows')
    $sheet.Activate()
    $null = $sheet.Range('A499999:B500001').CopyPicture(1, 2)
    Start-Sleep -Milliseconds 250
    $bitmap = [System.Windows.Forms.Clipboard]::GetImage()
    if ($null -eq $bitmap) {
        throw 'Excel did not place a bitmap on the clipboard'
    }
    try {
        $bitmap.Save($ImagePath, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        $bitmap.Dispose()
    }
}

function Invoke-ImageComparison {
    <#
    .SYNOPSIS
    Compares one live Excel image before Save, after Save, and after reopening.
    .DESCRIPTION
    Adds visible text immediately below the removed font cell so the capture
    contains real content. All changes are confined to a temporary copy.
    Writes three PNGs and a JSON result, then closes its private Excel instance.
    #>
    param([string]$Directory)

    $imageDirectory = Join-Path $Directory 'save-image-control'
    New-Item -ItemType Directory -Force -Path $imageDirectory | Out-Null
    $inputPath = Join-Path $Directory 'removed-empty-cell.xlsx'
    $workingPath = Join-Path $imageDirectory 'saved.xlsx'
    Copy-Item -LiteralPath $inputPath -Destination $workingPath -Force
    $images = @{
        before = Join-Path $imageDirectory 'before.png'
        after_save = Join-Path $imageDirectory 'after-save.png'
        reopened = Join-Path $imageDirectory 'reopened.png'
    }

    $excel = New-Object -ComObject Excel.Application
    $excel.Visible = $true
    $excel.DisplayAlerts = $false
    $workbook = $null
    try {
        $workbook = $excel.Workbooks.Open($workingPath, 0, $false)
        $workbook.Worksheets.Item('Rows').Range('A500001').Value2 = 'Below removed row'
        Save-RangeImage $workbook $images.before
        $workbook.Save()
        Save-RangeImage $workbook $images.after_save
        $workbook.Close($false)
        $workbook = $null

        $workbook = $excel.Workbooks.Open($workingPath, 0, $true)
        Save-RangeImage $workbook $images.reopened
    }
    finally {
        if ($null -ne $workbook) { $workbook.Close($false) }
        $excel.Quit()
    }

    $hashes = @{}
    foreach ($name in $images.Keys) {
        $hashes[$name] = (Get-FileHash -LiteralPath $images[$name] -Algorithm SHA256).Hash
    }
    $result = [pscustomobject]@{
        images = $images
        png_sha256 = $hashes
        all_equal = ($hashes.Values | Select-Object -Unique).Count -eq 1
    }
    $result | ConvertTo-Json -Depth 4 |
        Set-Content -Encoding UTF8 (Join-Path $imageDirectory 'result.json')
    $result | ConvertTo-Json -Depth 4 -Compress
}

Invoke-ImageComparison -Directory $OutputDirectory
