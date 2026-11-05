# Standalone chart sheets use the existing PID-validated resident, never a new Excel application.
$ErrorActionPreference = 'Stop'

function Invoke-OwnedChartOperation {
    <# .SYNOPSIS
    Borrows the exact resident application and releases every COM reference after one operation.
    #>
    param($Entry, [scriptblock]$Operation)
    $scope = [CaptureErrorChecking]::TryDisable($Entry.process, $env:EXCEL_SHRINK_EXPECTED_DESKTOP)
    if ($null -eq $scope) { throw 'Owned Excel has no accessible workbook window.' }
    $excel = $null
    $book = $null
    $charts = $null
    try {
        $excel = $scope.GetOwnedApplication()
        $book = $excel.ActiveWorkbook
        if ($book.FullName -ne $Entry.staged) { throw 'Unexpected chart-sheet workbook.' }
        $charts = $book.Charts
        & $Operation $book $charts
    } finally {
        foreach ($reference in @($charts, $book, $excel)) {
            if ($null -ne $reference -and [Runtime.InteropServices.Marshal]::IsComObject($reference)) {
                [void][Runtime.InteropServices.Marshal]::ReleaseComObject($reference)
            }
        }
        $scope.Dispose()
    }
}

function Get-TargetedChartSheets {
    <# .SYNOPSIS
    Avoids COM entirely for files without standalone chart-sheet parts.
    #>
    param($Entry)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($Entry.path)
    try {
        $hasChartSheets = @($archive.Entries | Where-Object {
            $_.FullName.StartsWith('xl/chartsheets/') -and $_.FullName.EndsWith('.xml')
        }).Count -gt 0
    } finally { $archive.Dispose() }
    if (-not $hasChartSheets) { return }
    Invoke-OwnedChartOperation $Entry {
        param($Book, $Charts)
        for ($index = 1; $index -le $Charts.Count; $index++) {
            $chart = $null
            try {
                $chart = $Charts.Item($index)
                @{sheet=[string]$chart.Name; name=[string]$chart.Name; kind='chart_sheet'}
            } finally {
                if ($null -ne $chart) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($chart) }
            }
        }
    }
}

function Export-TargetedChartSheet {
    <# .SYNOPSIS
    Exports a chart sheet at a consistent viewport, restoring the previous view without saving.
    #>
    param($Entry, $Command)
    Invoke-OwnedChartOperation $Entry {
        param($Book, $Charts)
        $chart = $null
        $active = $null
        $windows = $null
        $window = $null
        $area = $null
        $page = $null
        $view = $null
        $visibility = $null
        $chartView = $null
        try {
            $active = $Book.ActiveSheet
            $chart = $Charts.Item($Command.chart)
            $visibility = $chart.Visible
            $windows = $Book.Windows
            $window = $windows.Item(1)
            $view = @{state=$window.WindowState; width=$window.Width; height=$window.Height;
                left=$window.Left; top=$window.Top}
            $chartView = @{size_with_window=$chart.SizeWithWindow; zoom=$null}
            $chart.SizeWithWindow = $true # Apply the requested viewport before exporting.
            $window.WindowState = -4143 # xlNormal: each state must use the same export viewport.
            $window.Left = 0
            $window.Top = 0
            $window.Width = 1000
            $window.Height = 750
            if ($visibility -ne -1) { $chart.Visible = -1 } # xlSheetVisible, restored without saving.
            $chart.Activate()
            $chartView.zoom = $window.Zoom
            $window.Zoom = 100
            $page = $chart.PageSetup
            $chartView.chart_size = $page.ChartSize
            $page.ChartSize = 1 # xlFullPage: detach printable chart size from the current pane layout.
            $area = $chart.ChartArea
            $chart.Refresh()
            $complete = $false
            # Excel can report true while creating an empty file before the first render.
            # The Rust command deadline also bounds any blocked COM call.
            for ($attempt = 0; $attempt -lt 10; $attempt++) {
                Start-Sleep -Milliseconds 200
                $exported = $chart.Export($Command.output, 'PNG', $false)
                if ($exported -and [IO.File]::Exists($Command.output) -and
                    (Get-Item -LiteralPath $Command.output).Length -gt 0) {
                    $complete = $true
                    break
                }
                $chart.Refresh()
            }
            if (-not $complete) { throw 'Chart-sheet export did not produce a nonempty image.' }
            @{path=$Command.output; geometry=@{width=$area.Width; height=$area.Height;
                visibility=$visibility; page_chart_size=$chartView.chart_size}}
        } finally {
            try {
                if ($null -ne $chartView) {
                    if ($chartView.ContainsKey('chart_size')) { $page.ChartSize = $chartView.chart_size }
                    if ($null -ne $chartView.zoom) { $window.Zoom = $chartView.zoom }
                    $chart.SizeWithWindow = $chartView.size_with_window
                }
                if ($null -ne $view) {
                    $window.Width = $view.width
                    $window.Height = $view.height
                    $window.Left = $view.left
                    $window.Top = $view.top
                    $window.WindowState = $view.state
                }
                if ($null -ne $active) { $active.Activate() }
                if ($null -ne $visibility) { $chart.Visible = $visibility }
            } finally {
                foreach ($reference in @($page, $area, $window, $windows, $chart, $active)) {
                    if ($null -ne $reference -and [Runtime.InteropServices.Marshal]::IsComObject($reference)) {
                        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($reference)
                    }
                }
            }
        }
    }
}

function Export-TargetedEmbeddedChart {
    <# .SYNOPSIS
    Exports an Office.js-inaccessible chart through the same PID-validated Excel process.
    Restores the active sheet and visibility without saving the temporary render state.
    #>
    param($Entry, $Command)
    Invoke-OwnedChartOperation $Entry {
        param($Book, $Charts)
        $active = $null
        $sheet = $null
        $objects = $null
        $object = $null
        $chart = $null
        $visibility = $null
        try {
            $active = $Book.ActiveSheet
            $sheet = $Book.Worksheets.Item($Command.sheet)
            $visibility = $sheet.Visible
            $objects = $sheet.ChartObjects()
            if ($null -ne $Command.chart_index) {
                $index = [int]$Command.chart_index
                if ($index -lt 0 -or $index -ge $objects.Count) { throw 'Invalid chart collection index.' }
                $object = $objects.Item($index + 1)
                if ($object.Name -cne $Command.chart) { throw 'Chart collection order changed since enumeration.' }
            } else {
                $object = $objects.Item($Command.chart)
            }
            $geometry = @{
                top=$object.Top; left=$object.Left; width=$object.Width; height=$object.Height
                export_method='native-existing-process'
            }
            if ($object.Width -eq 0 -or $object.Height -eq 0) {
                return @{non_rendering=$true; reason='zero-area-chart'; geometry=$geometry}
            }
            if ($visibility -ne -1) { $sheet.Visible = -1 }
            $sheet.Activate()
            $chart = $object.Chart
            $chart.Refresh()
            for ($attempt = 0; $attempt -lt 10; $attempt++) {
                Start-Sleep -Milliseconds 200
                $exported = $chart.Export($Command.output, 'PNG', $false)
                if ($exported -and [IO.File]::Exists($Command.output) -and
                    (Get-Item -LiteralPath $Command.output).Length -gt 0) {
                    return @{path=$Command.output; geometry=$geometry}
                }
                $chart.Refresh()
            }
            throw 'Embedded chart export did not produce a nonempty image.'
        } finally {
            try {
                if ($null -ne $active) { $active.Activate() }
                if ($null -ne $visibility) { $sheet.Visible = $visibility }
            } finally {
                foreach ($reference in @($chart, $object, $objects, $sheet, $active)) {
                    if ($null -ne $reference -and [Runtime.InteropServices.Marshal]::IsComObject($reference)) {
                        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($reference)
                    }
                }
            }
        }
    }
}

Export-ModuleMember -Function Get-TargetedChartSheets, Export-TargetedChartSheet, Export-TargetedEmbeddedChart
