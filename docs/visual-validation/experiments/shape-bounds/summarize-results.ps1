param(
    [string]$ResultsDir = (Join-Path $PSScriptRoot 'results'),
    [switch]$FailOnMismatch,
    [switch]$BaseOnly
)

$ErrorActionPreference = 'Stop'

$StrictUtf8 = New-Object System.Text.UTF8Encoding($false, $true)
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Read-Utf8Json([string]$Path) {
    $text = [IO.File]::ReadAllText($Path, $StrictUtf8)
    return ($text | ConvertFrom-Json)
}

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, $Utf8NoBom)
}

function Get-SortedPropertyNames($Object, [switch]$IgnoreMeta) {
    $names = New-Object 'System.Collections.Generic.List[string]'

    foreach ($property in $Object.PSObject.Properties) {
        if ($IgnoreMeta -and $property.Name.StartsWith('_')) {
            continue
        }

        $names.Add($property.Name)
    }

    $array = $names.ToArray()
    [Array]::Sort($array, [StringComparer]::Ordinal)
    return $array
}

function New-CanonicalGeometry($Document) {
    $canonical = [ordered]@{}

    foreach ($workbookName in (Get-SortedPropertyNames $Document -IgnoreMeta)) {
        $workbook = $Document.PSObject.Properties[$workbookName].Value
        $worksheets = [ordered]@{}

        foreach ($sheetName in (Get-SortedPropertyNames $workbook)) {
            $sheet = $workbook.PSObject.Properties[$sheetName].Value

            $worksheets[$sheetName] = [ordered]@{
                right = [double]$sheet.right
                bottom = [double]$sheet.bottom
            }
        }

        $canonical[$workbookName] = $worksheets
    }

    return $canonical
}

function ConvertTo-CanonicalGeometryJson($Document) {
    $json = (New-CanonicalGeometry $Document) | ConvertTo-Json -Depth 10

    if ($json -match '"duration_ms"') {
        throw 'Internal error: canonical geometry JSON contains duration_ms.'
    }

    return $json + "`n"
}

function Get-TimingEntry([string]$Path, [string]$DisplayName) {
    $document = Read-Utf8Json $Path
    $sheetSum = 0.0
    $sheetCount = 0
    $timedSheetCount = 0
    $workbookCount = 0

    foreach ($workbookName in (Get-SortedPropertyNames $document -IgnoreMeta)) {
        $workbookCount++
        $workbook = $document.PSObject.Properties[$workbookName].Value

        foreach ($sheetName in (Get-SortedPropertyNames $workbook)) {
            $sheetCount++
            $sheet = $workbook.PSObject.Properties[$sheetName].Value
            $durationProperty = $sheet.PSObject.Properties['duration_ms']

            if (
                $null -ne $durationProperty -and
                $null -ne $durationProperty.Value
            ) {
                $sheetSum += [double]$durationProperty.Value
                $timedSheetCount++
            }
        }
    }

    $geometryTotal = $sheetSum
    $timingSource = 'sum_sheet_duration_ms'
    $timingScope = 'Sum of producer per-sheet drawing-bound timers.'
    $metaProperty = $document.PSObject.Properties['_meta']

    if ($null -ne $metaProperty) {
        $geometryProperty = $metaProperty.Value.PSObject.Properties['geometry_total_ms']
        if ($null -ne $geometryProperty -and $null -ne $geometryProperty.Value) {
            $geometryTotal = [double]$geometryProperty.Value
            $timingSource = 'meta.geometry_total_ms'
        }

        $scopeProperty = $metaProperty.Value.PSObject.Properties['timing_scope']
        if ($null -ne $scopeProperty -and $null -ne $scopeProperty.Value) {
            $timingScope = [string]$scopeProperty.Value
        }
    }

    return [pscustomobject][ordered]@{
        variant = $DisplayName
        geometry_total_ms = $geometryTotal
        timing_source = $timingSource
        timing_scope = $timingScope
        sum_sheet_duration_ms = $sheetSum
        workbook_count = $workbookCount
        sheet_count = $sheetCount
        timed_sheet_count = $timedSheetCount
    }
}

function Get-CodeCommand {
    foreach ($candidate in @('code.cmd', 'code.exe', 'code')) {
        $command = Get-Command $candidate -ErrorAction SilentlyContinue
        if ($null -ne $command) {
            return $command
        }
    }

    return $null
}

function Open-CodeDiff(
    [string]$Left,
    [string]$Right,
    [string]$Label
) {
    $code = Get-CodeCommand

    if ($null -eq $code) {
        Write-Warning 'VS Code command "code" was not found on PATH.'
        return
    }

    Write-Host "Opening VS Code diff: $Label"

    Push-Location -LiteralPath $env:TEMP

    try {
        & $code.Source `
            --reuse-window `
            --diff `
            $Left `
            $Right
    }
    finally {
        Pop-Location
    }
}

$ResultsDir = [IO.Path]::GetFullPath($ResultsDir)
$legacyCompareDir = Join-Path $ResultsDir 'compare'
$mismatchRoot = Join-Path $ResultsDir 'geometry-mismatches'

# compare/ used to persist one timing-free copy per producer. Identical files there
# were expected when geometry matched, but the duplication was easy to misread as
# duplicated timing results. Canonical geometry is now compared in memory and only
# persisted when a mismatch needs inspection.
Remove-Item `
    -LiteralPath $legacyCompareDir `
    -Recurse `
    -Force `
    -ErrorAction SilentlyContinue

Remove-Item `
    -LiteralPath $mismatchRoot `
    -Recurse `
    -Force `
    -ErrorAction SilentlyContinue

$variants = @(
    [pscustomobject]@{
        Key='vba'
        File='vba.json'
        Display='VBA'
        Caveat=$null
        Base=$true
    },
    [pscustomobject]@{
        Key='csharp_addin'
        File='csharp-addin.json'
        Display='C# Excel-DNA'
        Caveat=$null
        Base=$true
    },
    [pscustomobject]@{
        Key='powershell_com'
        File='powershell-com.json'
        Display='PowerShell COM'
        Caveat=$null
        Base=$true
    },
    [pscustomobject]@{
        Key='rust_com'
        File='rust-com.json'
        Display='Rust COM'
        Caveat='Shape.Visible filtering remains disabled until the excel-com dependency accepts Excel MsoTriState/VT_I4.'
        Base=$true
    },
    [pscustomobject]@{
        Key='vsto'
        File='vsto.json'
        Display='C# VSTO'
        Caveat=$null
        Base=$false
    },
    [pscustomobject]@{
        Key='office_addin'
        File='office-addin.json'
        Display='Office.js add-in'
        Caveat='Office.js batches ShapeCollection property loading. Its geometry_total_ms is a whole-batch timer rather than a sum of per-sheet timers.'
        Base=$false
    }
)

if ($BaseOnly) {
    $variants = @($variants | Where-Object { $_.Base })
}

$available = New-Object 'System.Collections.Generic.List[object]'

foreach ($variant in $variants) {
    $source = Join-Path $ResultsDir $variant.File

    if (-not (Test-Path -LiteralPath $source)) {
        continue
    }

    $document = Read-Utf8Json $source
    $canonicalJson = ConvertTo-CanonicalGeometryJson $document

    $available.Add([pscustomobject]@{
        Variant = $variant
        Source = $source
        CanonicalJson = $canonicalJson
    })
}

$baseline = $available |
    Where-Object { $_.Variant.Key -eq 'vba' } |
    Select-Object -First 1

if ($null -eq $baseline) {
    throw "VBA baseline result is missing below: $ResultsDir"
}

$validation = [ordered]@{
    baseline = 'vba'
    comparison = (
        'Strict UTF-8 decode; sorted canonical right/bottom geometry compared ' +
        'in memory. Timing is excluded. Geometry files are persisted only for ' +
        'actual mismatches.'
    )
    variants = [ordered]@{}
}

$hasMismatch = $false

foreach ($entry in $available) {
    $isBaseline = $entry.Variant.Key -eq 'vba'
    $matches = $isBaseline -or (
        $entry.CanonicalJson -ceq $baseline.CanonicalJson
    )

    if (-not $matches) {
        $hasMismatch = $true

        $variantDir = Join-Path $mismatchRoot $entry.Variant.Key
        New-Item -ItemType Directory -Path $variantDir -Force | Out-Null

        $baselinePath = Join-Path $variantDir 'vba.json'
        $variantPath = Join-Path $variantDir ($entry.Variant.Key + '.json')

        Write-Utf8NoBom $baselinePath $baseline.CanonicalJson
        Write-Utf8NoBom $variantPath $entry.CanonicalJson

        Open-CodeDiff `
            $baselinePath `
            $variantPath `
            ("VBA vs " + $entry.Variant.Display)
    }

    $validation.variants[$entry.Variant.Key] = [ordered]@{
        available = $true
        matches_vba = $matches
        semantic_caveat = $entry.Variant.Caveat
    }
}

foreach ($variant in $variants) {
    if (-not $validation.variants.Contains($variant.Key)) {
        $validation.variants[$variant.Key] = [ordered]@{
            available = $false
            matches_vba = $null
            semantic_caveat = $variant.Caveat
        }
    }
}

$validationPath = Join-Path $ResultsDir 'geometry-validation.json'

Write-Utf8NoBom `
    $validationPath `
    (($validation | ConvertTo-Json -Depth 8) + "`n")

$timings = @()

foreach ($entry in $available) {
    $timings += Get-TimingEntry `
        $entry.Source `
        $entry.Variant.Display
}

$timingPath = Join-Path $ResultsDir 'timing-summary.json'

Write-Utf8NoBom `
    $timingPath `
    (($timings | ConvertTo-Json -Depth 5) + "`n")

$csv = New-Object Text.StringBuilder

[void]$csv.AppendLine(
    'variant,geometry_total_ms,timing_source,sum_sheet_duration_ms,' +
    'workbook_count,sheet_count,timed_sheet_count'
)

foreach ($timing in $timings) {
    $line = @(
        '"' + ($timing.variant -replace '"','""') + '"',
        ([double]$timing.geometry_total_ms).ToString(
            '0.000000',
            [Globalization.CultureInfo]::InvariantCulture
        ),
        $timing.timing_source,
        ([double]$timing.sum_sheet_duration_ms).ToString(
            '0.000000',
            [Globalization.CultureInfo]::InvariantCulture
        ),
        $timing.workbook_count,
        $timing.sheet_count,
        $timing.timed_sheet_count
    ) -join ','

    [void]$csv.AppendLine($line)
}

Write-Utf8NoBom `
    (Join-Path $ResultsDir 'timing-summary.csv') `
    $csv.ToString()

Write-Host
Write-Host 'Canonical geometry validation:'

foreach ($entry in $available) {
    if ($entry.Variant.Key -eq 'vba') {
        Write-Host '  VBA: baseline'
        continue
    }

    $status = if (
        $validation.variants[$entry.Variant.Key].matches_vba
    ) {
        'MATCH'
    }
    else {
        'MISMATCH'
    }

    Write-Host (
        '  {0}: {1}' -f
        $entry.Variant.Display,
        $status
    )
}

Write-Host "  Validation: $validationPath"

if ($hasMismatch) {
    Write-Host "  Mismatch artifacts: $mismatchRoot"
}
else {
    Write-Host '  Mismatch artifacts: none'
}

Write-Host
Write-Host 'Timing summary (drawing-bound collection; lower is faster):'

$rank = 1

foreach ($timing in ($timings | Sort-Object geometry_total_ms)) {
    Write-Host (
        '  {0}. {1}: {2} ms [{3}]' -f
        $rank,
        $timing.variant,
        ([double]$timing.geometry_total_ms).ToString(
            '0.000',
            [Globalization.CultureInfo]::InvariantCulture
        ),
        $timing.timing_source
    )

    $rank++
}

Write-Host "  JSON: $timingPath"
Write-Host "  CSV:  $(Join-Path $ResultsDir 'timing-summary.csv')"

if ($FailOnMismatch -and $hasMismatch) {
    throw 'Geometry validation failed; at least one available variant differs from VBA.'
}
