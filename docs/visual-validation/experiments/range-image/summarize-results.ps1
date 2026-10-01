param(
    [Parameter(Mandatory=$true)]
    [string]$VbaResultPath,

    [Parameter(Mandatory=$true)]
    [string]$OfficeResultPath,

    [Parameter(Mandatory=$true)]
    [string]$VbaImageRoot,

    [Parameter(Mandatory=$true)]
    [string]$OfficeImageRoot,

    [Parameter(Mandatory=$true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$StrictUtf8 = New-Object System.Text.UTF8Encoding($false, $true)
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Read-Utf8Json([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        throw "Result file does not exist: $Path"
    }

    $text = [IO.File]::ReadAllText($Path, $StrictUtf8)
    return ($text | ConvertFrom-Json)
}

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    $parent = Split-Path -Parent $Path

    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }

    [IO.File]::WriteAllText($Path, $Text, $Utf8NoBom)
}

function Read-PngDimensions([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) {
        return $null
    }

    $bytes = [IO.File]::ReadAllBytes($Path)

    if ($bytes.Length -lt 24) {
        throw "PNG is too short: $Path"
    }

    $signature = [byte[]]@(137, 80, 78, 71, 13, 10, 26, 10)

    for ($index = 0; $index -lt 8; $index++) {
        if ($bytes[$index] -ne $signature[$index]) {
            throw "File is not a PNG: $Path"
        }
    }

    $width =
        ([int]$bytes[16] -shl 24) -bor
        ([int]$bytes[17] -shl 16) -bor
        ([int]$bytes[18] -shl 8) -bor
        [int]$bytes[19]

    $height =
        ([int]$bytes[20] -shl 24) -bor
        ([int]$bytes[21] -shl 16) -bor
        ([int]$bytes[22] -shl 8) -bor
        [int]$bytes[23]

    return [pscustomobject]@{
        width = $width
        height = $height
    }
}

function Get-CaptureMap($Document) {
    $map = @{}

    foreach ($capture in @($Document.captures)) {
        if ($null -eq $capture) {
            continue
        }

        $id = [string]$capture.id

        if ([string]::IsNullOrWhiteSpace($id)) {
            throw 'Capture result contains an empty id.'
        }

        if ($map.ContainsKey($id)) {
            throw "Duplicate capture id: $id"
        }

        $map[$id] = $capture
    }

    return $map
}

function Get-NullableDouble($Object, [string]$Name) {
    if ($null -eq $Object) {
        return $null
    }

    $property = $Object.PSObject.Properties[$Name]

    if ($null -eq $property -or $null -eq $property.Value) {
        return $null
    }

    return [double]$property.Value
}

$VbaResultPath = [IO.Path]::GetFullPath($VbaResultPath)
$OfficeResultPath = [IO.Path]::GetFullPath($OfficeResultPath)
$VbaImageRoot = [IO.Path]::GetFullPath($VbaImageRoot)
$OfficeImageRoot = [IO.Path]::GetFullPath($OfficeImageRoot)
$OutputPath = [IO.Path]::GetFullPath($OutputPath)

$vba = Read-Utf8Json $VbaResultPath
$office = Read-Utf8Json $OfficeResultPath
$vbaMap = Get-CaptureMap $vba
$officeMap = Get-CaptureMap $office

$ids = @((@($vbaMap.Keys) + @($officeMap.Keys)) | Sort-Object -Unique)
$comparisons = New-Object 'System.Collections.Generic.List[object]'

$paired = 0
$dimensionMatches = 0
$dimensionMismatches = 0
$byteIdentical = 0
$missingVba = 0
$missingOffice = 0
$vbaErrors = 0
$officeErrors = 0

foreach ($id in $ids) {
    $vbaCapture = if ($vbaMap.ContainsKey($id)) { $vbaMap[$id] } else { $null }
    $officeCapture = if ($officeMap.ContainsKey($id)) { $officeMap[$id] } else { $null }

    if ($null -eq $vbaCapture) {
        $missingVba++
    }

    if ($null -eq $officeCapture) {
        $missingOffice++
    }

    if (
        $null -ne $vbaCapture -and
        $null -ne $vbaCapture.PSObject.Properties['error'] -and
        $null -ne $vbaCapture.error
    ) {
        $vbaErrors++
    }

    if (
        $null -ne $officeCapture -and
        $null -ne $officeCapture.PSObject.Properties['error'] -and
        $null -ne $officeCapture.error
    ) {
        $officeErrors++
    }

    $relativePng = if ($null -ne $vbaCapture) {
        [string]$vbaCapture.relative_png
    }
    elseif ($null -ne $officeCapture) {
        [string]$officeCapture.relative_png
    }
    else {
        ''
    }

    $vbaPath = if (-not [string]::IsNullOrWhiteSpace($relativePng)) {
        Join-Path $VbaImageRoot $relativePng
    }
    else {
        ''
    }

    $officePath = if (-not [string]::IsNullOrWhiteSpace($relativePng)) {
        Join-Path $OfficeImageRoot $relativePng
    }
    else {
        ''
    }

    $vbaDimensions = if (-not [string]::IsNullOrWhiteSpace($vbaPath)) {
        Read-PngDimensions $vbaPath
    }
    else {
        $null
    }

    $officeDimensions = if (-not [string]::IsNullOrWhiteSpace($officePath)) {
        Read-PngDimensions $officePath
    }
    else {
        $null
    }

    $dimensionMatch = $null
    $shaMatch = $null

    if (
        $null -ne $vbaCapture -and
        $null -ne $officeCapture -and
        $null -ne $vbaDimensions -and
        $null -ne $officeDimensions
    ) {
        $paired++
        $dimensionMatch = (
            $vbaDimensions.width -eq $officeDimensions.width -and
            $vbaDimensions.height -eq $officeDimensions.height
        )

        if ($dimensionMatch) {
            $dimensionMatches++
        }
        else {
            $dimensionMismatches++
        }

        $vbaHash = (
            Get-FileHash -LiteralPath $vbaPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        $officeHash = (
            Get-FileHash -LiteralPath $officePath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        $shaMatch = $vbaHash -eq $officeHash

        if ($shaMatch) {
            $byteIdentical++
        }
    }

    $comparisons.Add(
        [pscustomobject][ordered]@{
            id = $id
            workbook = if ($null -ne $vbaCapture) {
                [string]$vbaCapture.workbook
            }
            elseif ($null -ne $officeCapture) {
                [string]$officeCapture.workbook
            }
            else { $null }
            sheet = if ($null -ne $vbaCapture) {
                [string]$vbaCapture.sheet
            }
            elseif ($null -ne $officeCapture) {
                [string]$officeCapture.sheet
            }
            else { $null }
            address = if ($null -ne $vbaCapture) {
                [string]$vbaCapture.address
            }
            elseif ($null -ne $officeCapture) {
                [string]$officeCapture.address
            }
            else { $null }
            relative_png = $relativePng
            vba_duration_ms = Get-NullableDouble $vbaCapture 'duration_ms'
            office_render_ms = Get-NullableDouble $officeCapture 'render_ms'
            office_persist_ms = Get-NullableDouble $officeCapture 'persist_ms'
            office_capture_ms = Get-NullableDouble $officeCapture 'capture_ms'
            vba_png = if ($null -ne $vbaDimensions) {
                [ordered]@{
                    width = $vbaDimensions.width
                    height = $vbaDimensions.height
                    bytes = (Get-Item -LiteralPath $vbaPath).Length
                }
            }
            else { $null }
            office_png = if ($null -ne $officeDimensions) {
                [ordered]@{
                    width = $officeDimensions.width
                    height = $officeDimensions.height
                    bytes = (Get-Item -LiteralPath $officePath).Length
                }
            }
            else { $null }
            dimensions_match = $dimensionMatch
            byte_identical = $shaMatch
        }
    )
}

$vbaTotal = Get-NullableDouble $vba._meta 'capture_total_ms'
$officeTotal = Get-NullableDouble $office._meta 'capture_total_ms'
$officeRenderTotal = Get-NullableDouble $office._meta 'render_total_ms'
$officePersistTotal = Get-NullableDouble $office._meta 'persist_total_ms'

$speedRatio = if (
    $null -ne $vbaTotal -and
    $null -ne $officeTotal -and
    $officeTotal -gt 0
) {
    $vbaTotal / $officeTotal
}
else {
    $null
}

$summary = [ordered]@{
    comparison = 'VBA Range.CopyPicture/Chart.Export vs Office.js Range.getImage'
    timing = [ordered]@{
        vba_capture_total_ms = $vbaTotal
        officejs_capture_total_ms = $officeTotal
        officejs_render_total_ms = $officeRenderTotal
        officejs_persist_total_ms = $officePersistTotal
        vba_over_officejs_ratio = $speedRatio
        note = (
            'The timing scopes intentionally include the operations needed to make PNG ' +
            'files available locally, but exclude workbook open/close and experiment setup.'
        )
    }
    correctness = [ordered]@{
        paired_pngs = $paired
        dimension_matches = $dimensionMatches
        dimension_mismatches = $dimensionMismatches
        byte_identical_pngs = $byteIdentical
        missing_vba_captures = $missingVba
        missing_officejs_captures = $missingOffice
        vba_capture_errors = $vbaErrors
        officejs_capture_errors = $officeErrors
        note = (
            'Equal dimensions and byte hashes are diagnostics, not proof of pixel equality. ' +
            'Different Excel rendering paths may encode or render PNGs differently; inspect ' +
            'representative images or add a pixel decoder before using this as a visual-correctness gate.'
        )
    }
    captures = $comparisons.ToArray()
}

Write-Utf8NoBom `
    $OutputPath `
    (($summary | ConvertTo-Json -Depth 10) + "`n")

Write-Host ''
Write-Host 'Range-image timing summary:'
Write-Host ("  VBA CopyPicture/Chart.Export: {0:N3} ms" -f $vbaTotal)
Write-Host ("  Office.js Range.getImage:      {0:N3} ms" -f $officeTotal)
Write-Host ("    render:                      {0:N3} ms" -f $officeRenderTotal)
Write-Host ("    transfer/persist:            {0:N3} ms" -f $officePersistTotal)
Write-Host ("  Paired PNGs:                   {0}" -f $paired)
Write-Host ("  Equal dimensions:              {0}" -f $dimensionMatches)
Write-Host ("  Byte-identical PNGs:           {0}" -f $byteIdentical)
Write-Host "  JSON: $OutputPath"
