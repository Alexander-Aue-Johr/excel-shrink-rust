param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Prepare', 'Recalculate')]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$Source,

    [Parameter(Mandatory = $true)]
    [string]$Target
)

$ErrorActionPreference = 'Stop'
$excel = New-Object -ComObject Excel.Application
$excel.Visible = $false
$excel.DisplayAlerts = $false
$book = $null

try {
    $book = $excel.Workbooks.Open($Source, 0, $false)

    if ($Mode -eq 'Prepare') {
        $rainbow = $book.Worksheets.Item('RGB sweep')
        $objects = $book.Worksheets.Add()
        $objects.Name = 'Objects'

        $chart = $objects.ChartObjects().Add(40, 60, 380, 230)
        $chart.Name = 'Rainbow values chart'
        $chart.Chart.ChartType = 4  # xlLine
        $chart.Chart.SetSourceData($rainbow.Range('A1:A16'))
        $chart.Chart.HasTitle = $true
        $chart.Chart.ChartTitle.Text = 'Rainbow values A1:A16'
        $chart.Chart.HasLegend = $false

        $drawing = $objects.Shapes.AddShape(1, 480, 80, 170, 100)  # msoShapeRectangle
        $drawing.Name = 'Static rectangle'
        $drawing.TextFrame.Characters().Text = 'Unchanged drawing'
    }

    $excel.CalculateFullRebuild()
    $book.SaveAs($Target, 51)  # xlOpenXMLWorkbook

    $objects = $book.Worksheets.Item('Objects')
    $chart = $objects.ChartObjects('Rainbow values chart')
    $drawing = $objects.Shapes.Item('Static rectangle')
    [pscustomobject]@{
        chart = [pscustomobject]@{
            name = $chart.Name
            top_left = $chart.TopLeftCell.Address($false, $false)
            bottom_right = $chart.BottomRightCell.Address($false, $false)
            left = $chart.Left
            top = $chart.Top
            width = $chart.Width
            height = $chart.Height
        }
        drawing = [pscustomobject]@{
            name = $drawing.Name
            top_left = $drawing.TopLeftCell.Address($false, $false)
            bottom_right = $drawing.BottomRightCell.Address($false, $false)
            left = $drawing.Left
            top = $drawing.Top
            width = $drawing.Width
            height = $drawing.Height
        }
    } | ConvertTo-Json -Depth 4 -Compress
}
finally {
    if ($null -ne $book) { $book.Close($false) }
    $excel.Quit()
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($excel)
}
