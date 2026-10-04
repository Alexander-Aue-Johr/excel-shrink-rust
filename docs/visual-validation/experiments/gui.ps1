param(
    [string]$CorpusRoot,
    [string]$Workbook
)

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName PresentationFramework
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase

$ExperimentsRoot = $PSScriptRoot
$RepoRoot = [IO.Path]::GetFullPath((Join-Path $ExperimentsRoot '..\..\..'))
$RangeRunner = Join-Path $ExperimentsRoot 'range-image\run.ps1'
$HeightLimitRunner = Join-Path $ExperimentsRoot 'range-image-height-limit\run.ps1'
$CalibrationRunner = Join-Path $ExperimentsRoot 'range-image-calibration\run.ps1'
$ShapeRunner = Join-Path $ExperimentsRoot 'shape-bounds\run.ps1'
$StateRoot = Join-Path $RepoRoot '.tmp\visual-validation'
$StatePath = Join-Path $StateRoot 'launcher-last-config.json'
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

if ([string]::IsNullOrWhiteSpace($CorpusRoot)) {
    $CorpusRoot = Join-Path $RepoRoot 'corpus-local'
}
else {
    $CorpusRoot = [IO.Path]::GetFullPath($CorpusRoot)
}

function Read-LastConfiguration {
    if (-not (Test-Path -LiteralPath $StatePath -PathType Leaf)) {
        return $null
    }

    try {
        return (
            [IO.File]::ReadAllText($StatePath, [Text.Encoding]::UTF8) |
            ConvertFrom-Json
        )
    }
    catch {
        Write-Warning (
            'Ignoring unreadable visual-validation launcher state: ' +
            $_.Exception.Message
        )
        return $null
    }
}

function Write-LastConfiguration($Configuration) {
    New-Item -ItemType Directory -Path $StateRoot -Force | Out-Null

    $json = ($Configuration | ConvertTo-Json -Depth 20) + "`n"
    $temporaryPath = Join-Path `
        $StateRoot `
        ('launcher-last-config.' + [Guid]::NewGuid().ToString('N') + '.tmp')

    try {
        [IO.File]::WriteAllText($temporaryPath, $json, $Utf8NoBom)

        Move-Item `
            -LiteralPath $temporaryPath `
            -Destination $StatePath `
            -Force
    }
    finally {
        Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
    }
}

function New-ChoiceItem([string]$Label, [string]$Value) {
    return [pscustomobject]@{
        Label = $Label
        Value = $Value
    }
}

function Set-ChoiceItems($ComboBox, [object[]]$Items, [string]$Value) {
    $ComboBox.ItemsSource = $Items
    $ComboBox.DisplayMemberPath = 'Label'
    $ComboBox.SelectedValuePath = 'Value'

    $selected = $Items | Where-Object { $_.Value -eq $Value } | Select-Object -First 1
    if ($null -eq $selected) {
        $selected = $Items | Select-Object -First 1
    }
    $ComboBox.SelectedItem = $selected
}

function Get-ChoiceValue($ComboBox) {
    if ($null -eq $ComboBox.SelectedItem) {
        return ''
    }
    return [string]$ComboBox.SelectedItem.Value
}

function Resolve-ExistingCorpus([string]$Value) {
    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw 'Corpus root must not be empty.'
    }

    $full = [IO.Path]::GetFullPath($Value)
    if (-not (Test-Path -LiteralPath $full -PathType Container)) {
        throw "Corpus directory does not exist: $full"
    }
    return $full
}

function Get-WorkbookChoices([string]$Root) {
    $choices = New-Object 'System.Collections.Generic.List[object]'
    $choices.Add((New-ChoiceItem 'All workbooks' 'all'))

    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        return $choices.ToArray()
    }

    foreach ($file in @(
        Get-ChildItem `
            -LiteralPath $Root `
            -Recurse `
            -File `
            -Filter '*.xlsx' `
            -ErrorAction SilentlyContinue |
        Where-Object { -not $_.Name.StartsWith('~$') } |
        Sort-Object FullName
    )) {
        $relative = $file.FullName
        try {
            $base = $Root.TrimEnd('\', '/')
            if ($file.FullName.StartsWith($base, [StringComparison]::OrdinalIgnoreCase)) {
                $relative = $file.FullName.Substring($base.Length).TrimStart('\', '/')
            }
        }
        catch {
        }

        $choices.Add((New-ChoiceItem $relative $file.FullName))
    }

    return $choices.ToArray()
}

function Select-WorkbookValue($ComboBox, [string]$Value) {
    if ([string]::IsNullOrWhiteSpace($Value)) {
        $Value = 'all'
    }

    $match = @($ComboBox.ItemsSource) |
        Where-Object {
            $_.Value -ieq $Value -or
            ([IO.Path]::GetFileName([string]$_.Value) -ieq $Value)
        } |
        Select-Object -First 1

    if ($null -ne $match) {
        $ComboBox.SelectedItem = $match
    }
    else {
        $ComboBox.SelectedItem = @($ComboBox.ItemsSource) | Select-Object -First 1
    }
}

function Get-ShapeParameterDefinitions {
    if (-not (Test-Path -LiteralPath $ShapeRunner -PathType Leaf)) {
        return @()
    }

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile(
        $ShapeRunner,
        [ref]$tokens,
        [ref]$parseErrors
    )

    if (@($parseErrors).Count -gt 0) {
        throw (
            'Could not parse shape-bounds runner: ' +
            (@($parseErrors | ForEach-Object { $_.Message }) -join ' | ')
        )
    }

    $command = Get-Command -Name $ShapeRunner -CommandType ExternalScript
    $skip = @('CorpusRoot', 'NonInteractive')
    $definitions = New-Object 'System.Collections.Generic.List[object]'

    foreach ($parameterAst in @($ast.ParamBlock.Parameters)) {
        $name = $parameterAst.Name.VariablePath.UserPath
        if ($skip -contains $name) {
            continue
        }

        $metadata = $command.Parameters[$name]
        if ($null -eq $metadata) {
            continue
        }

        $validateSet = @(
            $metadata.Attributes |
            Where-Object { $_ -is [System.Management.Automation.ValidateSetAttribute] } |
            Select-Object -First 1
        )

        $validValues = @()
        if ($validateSet.Count -gt 0) {
            $validValues = @($validateSet[0].ValidValues)
        }

        $definitions.Add([pscustomobject]@{
            Name = $name
            ParameterType = $metadata.ParameterType
            ValidValues = $validValues
            Mandatory = [bool](
                @(
                    $metadata.Attributes |
                    Where-Object {
                        $_ -is [System.Management.Automation.ParameterAttribute] -and
                        $_.Mandatory
                    }
                ).Count -gt 0
            )
        })
    }

    return $definitions.ToArray()
}

function Get-LastShapeValue($LastConfig, [string]$Name) {
    if ($null -eq $LastConfig -or $null -eq $LastConfig.shape_parameters) {
        return $null
    }

    $property = $LastConfig.shape_parameters.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

$last = Read-LastConfiguration

$xaml = @"
<Window
    xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
    Title="Excel Shrink · Visual validation"
    Width="820"
    Height="860"
    MinWidth="720"
    MinHeight="620"
    WindowStartupLocation="CenterScreen"
    ResizeMode="CanResizeWithGrip">
  <Grid Margin="18">
    <Grid.RowDefinitions>
      <RowDefinition Height="Auto"/>
      <RowDefinition Height="*"/>
      <RowDefinition Height="Auto"/>
    </Grid.RowDefinitions>

    <StackPanel Grid.Row="0" Margin="0,0,0,14">
      <TextBlock
          FontSize="22"
          FontWeight="SemiBold"
          Text="Visual validation experiment"/>
      <TextBlock
          Margin="0,5,0,0"
          Foreground="#666666"
          Text="Configure the same experiment runner without stepping through terminal menus."/>
    </StackPanel>

    <ScrollViewer
        Grid.Row="1"
        VerticalScrollBarVisibility="Auto"
        HorizontalScrollBarVisibility="Disabled">
      <StackPanel>
        <Grid Margin="0,0,0,12">
          <Grid.ColumnDefinitions>
            <ColumnDefinition Width="190"/>
            <ColumnDefinition Width="*"/>
          </Grid.ColumnDefinitions>
          <TextBlock
              Grid.Column="0"
              VerticalAlignment="Center"
              FontWeight="SemiBold"
              Text="Experiment"/>
          <ComboBox
              x:Name="ExperimentBox"
              Grid.Column="1"
              MinWidth="360"/>
        </Grid>

        <Grid Margin="0,0,0,12">
          <Grid.ColumnDefinitions>
            <ColumnDefinition Width="190"/>
            <ColumnDefinition Width="*"/>
            <ColumnDefinition Width="Auto"/>
          </Grid.ColumnDefinitions>
          <TextBlock
              Grid.Column="0"
              VerticalAlignment="Center"
              FontWeight="SemiBold"
              Text="Corpus root"/>
          <TextBox
              x:Name="CorpusRootBox"
              Grid.Column="1"
              MinWidth="360"
              VerticalContentAlignment="Center"/>
          <Button
              x:Name="RefreshWorkbooksButton"
              Grid.Column="2"
              Margin="8,0,0,0"
              Padding="10,4"
              Content="Refresh workbooks"/>
        </Grid>

        <GroupBox
            x:Name="RangeImageGroup"
            Header="Get picture"
            Padding="12"
            Margin="0,0,0,12">
          <StackPanel>
            <Grid Margin="0,0,0,9">
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Bounds strategy"/>
              <ComboBox x:Name="StrategyBox" Grid.Column="1"/>
            </Grid>

            <Grid Margin="0,0,0,9"><Grid.ColumnDefinitions><ColumnDefinition Width="175"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Overflow verification (F)"/><ComboBox x:Name="OverflowModeBox" Grid.Column="1"/>
            </Grid>
            <Grid Margin="0,0,0,9"><Grid.ColumnDefinitions><ColumnDefinition Width="175"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Capture pixel budget"/><ComboBox x:Name="TileBudgetBox" Grid.Column="1"/>
            </Grid>
            <Grid Margin="0,0,0,9"><Grid.ColumnDefinitions><ColumnDefinition Width="175"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Tile packing"/><ComboBox x:Name="TilePackingBox" Grid.Column="1"/>
            </Grid>
            <Grid Margin="0,0,0,9">
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Implementation"/>
              <ComboBox x:Name="ImplementationBox" Grid.Column="1"/>
            </Grid>

            <Grid Margin="0,0,0,9">
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Workbook"/>
              <ComboBox x:Name="WorkbookBox" Grid.Column="1"/>
            </Grid>

            <Grid Margin="0,0,0,9">
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Office.js batch size"/>
              <ComboBox x:Name="BatchSizeBox" Grid.Column="1"/>
            </Grid>

            <Grid Margin="0,0,0,9">
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Rust Excel launch"/>
              <ComboBox x:Name="RustLaunchBox" Grid.Column="1"/>
            </Grid>

            <Grid Margin="0,0,0,9">
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Overflow probe width"/>
              <TextBox x:Name="OverflowWidthBox" Grid.Column="1"/>
            </Grid>

            <Grid Margin="0,0,0,9">
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Overflow probe height"/>
              <TextBox x:Name="OverflowHeightBox" Grid.Column="1"/>
            </Grid>

            <Grid Margin="0,0,0,9">
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Office.js startup"/>
              <CheckBox
                  x:Name="WaitForStartBox"
                  Grid.Column="1"
                  VerticalAlignment="Center"
                  Content="Wait for Start button in task pane"/>
            </Grid>

            <Grid>
              <Grid.ColumnDefinitions>
                <ColumnDefinition Width="175"/>
                <ColumnDefinition Width="*"/>
              </Grid.ColumnDefinitions>
              <TextBlock Grid.Column="0" VerticalAlignment="Center" Text="Office.js diagnostics"/>
              <ComboBox x:Name="DiagnosticsBox" Grid.Column="1"/>
            </Grid>
          </StackPanel>
        </GroupBox>

        <GroupBox x:Name="HeightLimitGroup" Header="Get picture diagnostics" Padding="12" Margin="0,0,0,12" Visibility="Collapsed">
          <StackPanel>
            <TextBlock x:Name="DiagnosticDescription" TextWrapping="Wrap"/>
            <StackPanel x:Name="DiagnosticSuitePanel" Margin="0,10,0,0">
              <TextBlock Text="Synthetic test suite"/>
              <ComboBox x:Name="DiagnosticSuiteBox"/>
            </StackPanel>
            <CheckBox x:Name="DiagnosticWaitBox" Margin="0,10,0,0" Content="Wait for Start in task pane (open DevTools first)"/>
            <CheckBox x:Name="DiagnosticBreakBox" Margin="0,6,0,0" Content="Pause with debugger; when an error occurs"/>
            <CheckBox x:Name="DiagnosticDebugJsBox" Margin="0,6,0,0" Content="Use office.debug.js (otherwise normal Office.js)"/>
            <TextBlock Margin="0,10,0,0" Foreground="#666666" TextWrapping="Wrap"
                Text="Extended errors and persistent per-probe diagnostics are always enabled. The 32766 value is a hypothesis, not a hard-coded pass/fail threshold."/>
          </StackPanel>
        </GroupBox>

        <GroupBox
            x:Name="ShapeBoundsGroup"
            Header="Get shape bounds"
            Padding="12"
            Margin="0,0,0,12"
            Visibility="Collapsed">
          <StackPanel>
            <TextBlock
                Margin="0,0,0,10"
                Foreground="#666666"
                TextWrapping="Wrap"
                Text="Options are read from the current shape-bounds PowerShell runner. Leave an optional value empty to use that runner's non-interactive default."/>
            <StackPanel x:Name="ShapeParametersPanel"/>
          </StackPanel>
        </GroupBox>

        <Border
            Padding="10"
            Background="#F3F3F3"
            CornerRadius="4">
          <TextBlock
              x:Name="StatePathText"
              TextWrapping="Wrap"
              Foreground="#555555"/>
        </Border>
      </StackPanel>
    </ScrollViewer>

    <StackPanel Grid.Row="2" Margin="0,14,0,0">
    <WrapPanel Margin="0,0,0,10">
      <Button x:Name="OpenReportsButton" Padding="10,5" Margin="0,0,8,0" Content="Reports"/>
      <Button x:Name="OpenLatestReportButton" Padding="10,5" Margin="0,0,8,0" Content="Latest report"/>
      <Button x:Name="CleanReportsButton" Padding="10,5" Margin="0,0,8,0" Content="Prune history (keep 10)"/>
      <Button x:Name="RefreshReportsButton" Padding="10,5" Margin="0,0,8,0" Content="Rebuild report views"/>
      <Button x:Name="RecoverReportsButton" Padding="10,5" Content="Recover interrupted runs"/>
    </WrapPanel>
    <Grid>
      <Grid.ColumnDefinitions>
        <ColumnDefinition Width="*"/>
        <ColumnDefinition Width="Auto"/>
        <ColumnDefinition Width="Auto"/>
      </Grid.ColumnDefinitions>
      <TextBlock
          x:Name="StatusText"
          Grid.Column="0"
          Margin="0,0,12,0"
          VerticalAlignment="Center"
          Foreground="#666666"
          TextWrapping="Wrap"/>
      <Button
          x:Name="CancelButton"
          Grid.Column="1"
          MinWidth="90"
          Margin="0,0,8,0"
          Padding="14,6"
          IsCancel="True"
          Content="Cancel"/>
      <Button
          x:Name="RunButton"
          Grid.Column="2"
          MinWidth="110"
          Padding="14,6"
          IsDefault="True"
          Content="Run"/>
    </Grid>
    </StackPanel>
  </Grid>
</Window>
"@

[xml]$xamlDocument = $xaml
$reader = New-Object System.Xml.XmlNodeReader $xamlDocument
$window = [Windows.Markup.XamlReader]::Load($reader)

$ExperimentBox = $window.FindName('ExperimentBox')
$CorpusRootBox = $window.FindName('CorpusRootBox')
$RefreshWorkbooksButton = $window.FindName('RefreshWorkbooksButton')
$RangeImageGroup = $window.FindName('RangeImageGroup')
$HeightLimitGroup = $window.FindName('HeightLimitGroup')
$DiagnosticDescription = $window.FindName('DiagnosticDescription')
$DiagnosticSuitePanel = $window.FindName('DiagnosticSuitePanel')
$DiagnosticSuiteBox = $window.FindName('DiagnosticSuiteBox')
$DiagnosticWaitBox = $window.FindName('DiagnosticWaitBox')
$DiagnosticBreakBox = $window.FindName('DiagnosticBreakBox')
$DiagnosticDebugJsBox = $window.FindName('DiagnosticDebugJsBox')
Set-ChoiceItems $DiagnosticSuiteBox @(
    (New-ChoiceItem 'All original units/marker diagnostics' 'all'),
    (New-ChoiceItem 'XML units, fonts, zoom and rounding' 'units'),
    (New-ChoiceItem 'Saturation and clipping/scaling markers' 'markers'),
    (New-ChoiceItem 'Row heights: fonts, customHeight and geometry timing' 'row-heights')
) $(if ($null -ne $last -and $last.diagnostic_suite) { [string]$last.diagnostic_suite } else { 'all' })
$DiagnosticWaitBox.IsChecked = $null -ne $last -and [bool]$last.diagnostic_wait
$DiagnosticBreakBox.IsChecked = $null -ne $last -and [bool]$last.diagnostic_break
$DiagnosticDebugJsBox.IsChecked = $null -ne $last -and [bool]$last.diagnostic_debug_js
$ShapeBoundsGroup = $window.FindName('ShapeBoundsGroup')
$StrategyBox = $window.FindName('StrategyBox')
$OverflowModeBox = $window.FindName('OverflowModeBox')
$TileBudgetBox = $window.FindName('TileBudgetBox')
$TilePackingBox = $window.FindName('TilePackingBox')
$ImplementationBox = $window.FindName('ImplementationBox')
$WorkbookBox = $window.FindName('WorkbookBox')
$BatchSizeBox = $window.FindName('BatchSizeBox')
$RustLaunchBox = $window.FindName('RustLaunchBox')
$OverflowWidthBox = $window.FindName('OverflowWidthBox')
$OverflowHeightBox = $window.FindName('OverflowHeightBox')
$WaitForStartBox = $window.FindName('WaitForStartBox')
$DiagnosticsBox = $window.FindName('DiagnosticsBox')
$ShapeParametersPanel = $window.FindName('ShapeParametersPanel')
$StatePathText = $window.FindName('StatePathText')
$StatusText = $window.FindName('StatusText')
$RunButton = $window.FindName('RunButton')
$window.FindName('RefreshReportsButton').Add_Click({
    & (Join-Path $ExperimentsRoot 'reports.ps1') -Action Refresh
})
$window.FindName('OpenReportsButton').Add_Click({
    & (Join-Path $PSScriptRoot 'reports.ps1') -Action Open
})
$window.FindName('OpenLatestReportButton').Add_Click({
    & (Join-Path $PSScriptRoot 'reports.ps1') -Action Latest
})
$window.FindName('CleanReportsButton').Add_Click({
    & (Join-Path $PSScriptRoot 'reports.ps1') -Action Clean -KeepLast 10 -ConfirmInGui
})
$window.FindName('RecoverReportsButton').Add_Click({
    & (Join-Path $PSScriptRoot 'reports.ps1') -Action Recover
    & (Join-Path $PSScriptRoot 'reports.ps1') -Action Latest
})

Set-ChoiceItems $ExperimentBox @(
    (New-ChoiceItem 'Get picture' 'range-image'),
    (New-ChoiceItem 'Get picture height limit / plateau' 'range-image-height-limit'),
    (New-ChoiceItem 'Get picture units and markers' 'range-image-calibration'),
    (New-ChoiceItem 'Get shape bounds' 'shape-bounds')
) $(if ($null -ne $last -and $last.experiment) { [string]$last.experiment } else { 'range-image' })

Set-ChoiceItems $OverflowModeBox @(
    (New-ChoiceItem 'A - Current pixel heuristic' 'heuristic'),
    (New-ChoiceItem 'B - Selective whitespace verification' 'selective'),
    (New-ChoiceItem 'Compare A and B (fresh workbook per arm)' 'compare')
) $(if ($null -ne $last -and $last.overflow_mode) { [string]$last.overflow_mode } else { 'heuristic' })
Set-ChoiceItems $TileBudgetBox @(
    (New-ChoiceItem '16 MP - Standard' '16'), (New-ChoiceItem '32 MP - Larger tiles (experimental)' '32'),
    (New-ChoiceItem '64 MP - Large tiles (experimental)' '64')
) $(if ($null -ne $last -and $last.tile_megapixels) { [string]$last.tile_megapixels } else { '16' })
Set-ChoiceItems $TilePackingBox @(
    (New-ChoiceItem 'Budget-aware row boundaries' 'budget'), (New-ChoiceItem 'Balanced subdivision (previous baseline)' 'balanced')
) $(if ($null -ne $last -and $last.tile_packing) { [string]$last.tile_packing } else { 'budget' })
Set-ChoiceItems $StrategyBox @(
    (New-ChoiceItem 'A - UsedRange prefetch' 'A'),
    (New-ChoiceItem 'B - UsedRange immediately before each picture batch' 'B'),
    (New-ChoiceItem 'C - content + drawings + 10 row/column margin' 'C'),
    (New-ChoiceItem 'D - C bounds + measured horizontal overflow' 'D'),
    (New-ChoiceItem 'E - D + measured vertical overflow' 'E'),
    (New-ChoiceItem 'F - WSL Rust OOXML bounds + horizontal render overflow' 'F'),
    (New-ChoiceItem 'Run all strategies' 'all')
) $(if ($null -ne $last -and $last.strategy) { [string]$last.strategy } else { 'A' })

Set-ChoiceItems $ImplementationBox @(
    (New-ChoiceItem 'Office.js - Rust host' 'officejs-rust'),
    (New-ChoiceItem 'Office.js - ASP.NET host' 'officejs-dotnet'),
    (New-ChoiceItem 'VBA CopyPicture baseline' 'vba'),
    (New-ChoiceItem 'Run all implementations' 'all')
) $(if ($null -ne $last -and $last.implementation) { [string]$last.implementation } else { 'officejs-rust' })

Set-ChoiceItems $BatchSizeBox @(
    (New-ChoiceItem '1' '1'),
    (New-ChoiceItem '2' '2'),
    (New-ChoiceItem '4' '4'),
    (New-ChoiceItem '8' '8'),
    (New-ChoiceItem '16' '16'),
    (New-ChoiceItem '32' '32'),
    (New-ChoiceItem '64' '64')
) $(if ($null -ne $last -and $last.batch_size) { [string]$last.batch_size } else { '8' })

Set-ChoiceItems $RustLaunchBox @(
    (New-ChoiceItem 'Rust COM (default)' 'rust-com'),
    (New-ChoiceItem 'Excel /x process launch' 'excel-x')
) $(if ($null -ne $last -and $last.rust_excel_launch_mode) {
    [string]$last.rust_excel_launch_mode
} else {
    'rust-com'
})

Set-ChoiceItems $DiagnosticsBox @(
    (New-ChoiceItem 'Normal' 'normal'),
    (New-ChoiceItem 'Extended error logging' 'extended'),
    (New-ChoiceItem 'Extended error logging + office.debug.js' 'debug-js')
) $(if ($null -ne $last -and $last.diagnostics) { [string]$last.diagnostics } else { 'normal' })

$CorpusRootBox.Text = if ($null -ne $last -and $last.corpus_root) {
    [string]$last.corpus_root
} else {
    $CorpusRoot
}

$OverflowWidthBox.Text = if ($null -ne $last -and $last.overflow_probe_width_px) {
    [string]$last.overflow_probe_width_px
} else {
    '1920'
}

$OverflowHeightBox.Text = if ($null -ne $last -and $last.overflow_probe_height_px) {
    [string]$last.overflow_probe_height_px
} else {
    '1080'
}

$WaitForStartBox.IsChecked = [bool](
    $null -ne $last -and
    $null -ne $last.wait_for_start -and
    [bool]$last.wait_for_start
)

$StatePathText.Text = "Last configuration: $StatePath"
$StatusText.Text = if ($null -ne $last) {
    'Loaded the previous GUI configuration.'
} else {
    'No previous GUI configuration found; runner defaults are selected.'
}

$shapeControls = @{}
$shapeDefinitions = @(Get-ShapeParameterDefinitions)

foreach ($definition in $shapeDefinitions) {
    $grid = New-Object Windows.Controls.Grid
    $grid.Margin = [Windows.Thickness]::new(0, 0, 0, 9)

    $leftColumn = New-Object Windows.Controls.ColumnDefinition
    $leftColumn.Width = [Windows.GridLength]::new(175)
    $rightColumn = New-Object Windows.Controls.ColumnDefinition
    $rightColumn.Width = [Windows.GridLength]::new(1, [Windows.GridUnitType]::Star)
    [void]$grid.ColumnDefinitions.Add($leftColumn)
    [void]$grid.ColumnDefinitions.Add($rightColumn)

    $label = New-Object Windows.Controls.TextBlock
    $label.Text = $definition.Name
    $label.VerticalAlignment = [Windows.VerticalAlignment]::Center
    [Windows.Controls.Grid]::SetColumn($label, 0)
    [void]$grid.Children.Add($label)

    $lastValue = Get-LastShapeValue $last $definition.Name
    $control = $null
    $kind = 'text'

    if ($definition.ParameterType -eq [System.Management.Automation.SwitchParameter]) {
        $kind = 'switch'
        $control = New-Object Windows.Controls.CheckBox
        $control.VerticalAlignment = [Windows.VerticalAlignment]::Center
        $control.Content = 'Enabled'
        $control.IsChecked = [bool]($null -ne $lastValue -and [bool]$lastValue)
    }
    elseif (@($definition.ValidValues).Count -gt 0) {
        $kind = 'choice'
        $control = New-Object Windows.Controls.ComboBox
        $items = New-Object 'System.Collections.Generic.List[object]'

        if (-not $definition.Mandatory) {
            $items.Add((New-ChoiceItem '(runner default)' ''))
        }
        foreach ($value in @($definition.ValidValues)) {
            $items.Add((New-ChoiceItem ([string]$value) ([string]$value)))
        }

        Set-ChoiceItems `
            $control `
            $items.ToArray() `
            $(if ($null -ne $lastValue) { [string]$lastValue } else { '' })
    }
    else {
        $control = New-Object Windows.Controls.TextBox
        if ($null -ne $lastValue) {
            $control.Text = [string]$lastValue
        }
    }

    [Windows.Controls.Grid]::SetColumn($control, 1)
    [void]$grid.Children.Add($control)
    [void]$ShapeParametersPanel.Children.Add($grid)

    $shapeControls[$definition.Name] = [pscustomobject]@{
        Definition = $definition
        Control = $control
        Kind = $kind
    }
}

if ($shapeDefinitions.Count -eq 0) {
    $message = New-Object Windows.Controls.TextBlock
    $message.TextWrapping = [Windows.TextWrapping]::Wrap
    $message.Foreground = [Windows.Media.Brushes]::DarkGoldenrod
    $message.Text = (
        'No additional shape-bounds parameters were discovered. ' +
        'The GUI will run that experiment with its non-interactive defaults.'
    )
    [void]$ShapeParametersPanel.Children.Add($message)
}

function Refresh-WorkbookBox([string]$PreferredValue) {
    try {
        $root = Resolve-ExistingCorpus $CorpusRootBox.Text
        $choices = @(Get-WorkbookChoices $root)
        $WorkbookBox.ItemsSource = $choices
        $WorkbookBox.DisplayMemberPath = 'Label'
        $WorkbookBox.SelectedValuePath = 'Value'
        Select-WorkbookValue $WorkbookBox $PreferredValue
        $StatusText.Text = "Found $([Math]::Max(0, $choices.Count - 1)) workbook(s)."
    }
    catch {
        $WorkbookBox.ItemsSource = @((New-ChoiceItem 'All workbooks' 'all'))
        $WorkbookBox.DisplayMemberPath = 'Label'
        $WorkbookBox.SelectedValuePath = 'Value'
        $WorkbookBox.SelectedIndex = 0
        $StatusText.Text = $_.Exception.Message
    }
}

$preferredWorkbook = if (-not [string]::IsNullOrWhiteSpace($Workbook)) {
    $Workbook
}
elseif ($null -ne $last -and $last.workbook) {
    [string]$last.workbook
}
else {
    'all'
}
Refresh-WorkbookBox $preferredWorkbook

function Update-ExperimentVisibility {
    $experiment = Get-ChoiceValue $ExperimentBox

    $RangeImageGroup.Visibility = [Windows.Visibility]::Collapsed
    $HeightLimitGroup.Visibility = [Windows.Visibility]::Collapsed
    $ShapeBoundsGroup.Visibility = [Windows.Visibility]::Collapsed

    switch ($experiment) {
        'range-image' {
            $RangeImageGroup.Visibility = [Windows.Visibility]::Visible
        }
        'range-image-height-limit' {
            $HeightLimitGroup.Visibility = [Windows.Visibility]::Visible
            $HeightLimitGroup.Header = 'Hospital workbook: height plateau'
            $DiagnosticDescription.Text = 'Fixed workbook krankenhausverzeichnis-3500100227005.xlsx, sheet " KHV_2022", columns A:FX. Calibrate PNG scale, binary-search the first plateau row and repeat nearby rows.'
            $DiagnosticSuitePanel.Visibility = [Windows.Visibility]::Collapsed
        }
        'range-image-calibration' {
            $HeightLimitGroup.Visibility = [Windows.Visibility]::Visible
            $HeightLimitGroup.Header = 'Synthetic OOXML: units and markers'
            $DiagnosticDescription.Text = 'Synthetic OOXML suites: units and markers, or row-height follow-up (fonts, default versus custom height, geometry timing). Row-height follow-up produces a review ZIP. No corpus workbook needed.'
            $DiagnosticSuitePanel.Visibility = [Windows.Visibility]::Visible
        }
        'shape-bounds' {
            $ShapeBoundsGroup.Visibility = [Windows.Visibility]::Visible
        }
    }
}

$StrategyBox.Add_SelectionChanged({ $OverflowModeBox.IsEnabled = ((Get-ChoiceValue $StrategyBox) -eq 'F') })
$OverflowModeBox.IsEnabled = ((Get-ChoiceValue $StrategyBox) -eq 'F')
$ExperimentBox.Add_SelectionChanged({
    Update-ExperimentVisibility
})
Update-ExperimentVisibility

$RefreshWorkbooksButton.Add_Click({
    $current = Get-ChoiceValue $WorkbookBox
    Refresh-WorkbookBox $current
})

$script:Selection = $null

$RunButton.Add_Click({
    try {
        $experiment = Get-ChoiceValue $ExperimentBox
        $corpus = if ($experiment -eq 'range-image-calibration') { [string]$CorpusRootBox.Text } else { Resolve-ExistingCorpus $CorpusRootBox.Text }

        if ($experiment -eq 'range-image') {
            $width = 0
            if (
                -not [int]::TryParse($OverflowWidthBox.Text, [ref]$width) -or
                $width -lt 64 -or
                $width -gt 8192
            ) {
                throw 'Overflow probe width must be an integer from 64 through 8192.'
            }

            $height = 0
            if (
                -not [int]::TryParse($OverflowHeightBox.Text, [ref]$height) -or
                $height -lt 64 -or
                $height -gt 8192
            ) {
                throw 'Overflow probe height must be an integer from 64 through 8192.'
            }

            $workbookValue = Get-ChoiceValue $WorkbookBox
            if ([string]::IsNullOrWhiteSpace($workbookValue)) {
                $workbookValue = 'all'
            }

            $selection = [ordered]@{
                schema = 1
                experiment = 'range-image'
                corpus_root = $corpus
                strategy = Get-ChoiceValue $StrategyBox
                overflow_mode = $(if ((Get-ChoiceValue $StrategyBox) -eq 'F') { Get-ChoiceValue $OverflowModeBox } else { 'heuristic' })
                tile_megapixels = [int](Get-ChoiceValue $TileBudgetBox)
                tile_packing = Get-ChoiceValue $TilePackingBox
                implementation = Get-ChoiceValue $ImplementationBox
                workbook = $workbookValue
                batch_size = [int](Get-ChoiceValue $BatchSizeBox)
                rust_excel_launch_mode = Get-ChoiceValue $RustLaunchBox
                overflow_probe_width_px = $width
                overflow_probe_height_px = $height
                wait_for_start = [bool]$WaitForStartBox.IsChecked
                diagnostics = Get-ChoiceValue $DiagnosticsBox
                shape_parameters = [ordered]@{}
            }
        }
        elseif ($experiment -in @('range-image-height-limit', 'range-image-calibration')) {
            $selection = [ordered]@{
                schema = 1
                experiment = $experiment
                corpus_root = $corpus
                diagnostic_suite = Get-ChoiceValue $DiagnosticSuiteBox
                diagnostic_wait = [bool]$DiagnosticWaitBox.IsChecked
                diagnostic_break = [bool]$DiagnosticBreakBox.IsChecked
                diagnostic_debug_js = [bool]$DiagnosticDebugJsBox.IsChecked
                strategy = 'A'
                implementation = 'officejs-dotnet'
                workbook = 'krankenhausverzeichnis-3500100227005.xlsx'
                batch_size = 1
                rust_excel_launch_mode = 'excel-x'
                overflow_probe_width_px = 1920
                overflow_probe_height_px = 1080
                wait_for_start = [bool]$DiagnosticWaitBox.IsChecked
                diagnostics = 'extended'
                shape_parameters = [ordered]@{}
            }
        }
        else {
            $values = [ordered]@{}

            foreach ($name in @($shapeControls.Keys | Sort-Object)) {
                $entry = $shapeControls[$name]
                $definition = $entry.Definition

                if ($entry.Kind -eq 'switch') {
                    $enabled = [bool]$entry.Control.IsChecked
                    if ($enabled) {
                        $values[$name] = $true
                    }
                    continue
                }

                $raw = if ($entry.Kind -eq 'choice') {
                    Get-ChoiceValue $entry.Control
                }
                else {
                    [string]$entry.Control.Text
                }

                if ([string]::IsNullOrWhiteSpace($raw)) {
                    if ($definition.Mandatory) {
                        throw "Shape-bounds parameter '$name' is mandatory."
                    }
                    continue
                }

                if (
                    $definition.ParameterType -eq [int] -or
                    $definition.ParameterType -eq [long]
                ) {
                    $parsed = 0L
                    if (-not [long]::TryParse($raw, [ref]$parsed)) {
                        throw "Shape-bounds parameter '$name' must be an integer."
                    }
                    $values[$name] = $parsed
                }
                else {
                    $values[$name] = $raw
                }
            }

            $selection = [ordered]@{
                schema = 1
                experiment = 'shape-bounds'
                corpus_root = $corpus
                strategy = 'A'
                implementation = ''
                workbook = 'all'
                batch_size = 8
                rust_excel_launch_mode = 'rust-com'
                overflow_probe_width_px = 1920
                overflow_probe_height_px = 1080
                wait_for_start = $false
                diagnostics = 'normal'
                shape_parameters = $values
            }
        }

        Write-LastConfiguration $selection
        $script:Selection = $selection
        $window.DialogResult = $true
    }
    catch {
        [Windows.MessageBox]::Show(
            $_.Exception.Message,
            'Visual validation configuration',
            [Windows.MessageBoxButton]::OK,
            [Windows.MessageBoxImage]::Error
        ) | Out-Null
    }
})

$result = $window.ShowDialog()
if ($result -ne $true -or $null -eq $script:Selection) {
    Write-Host 'Visual-validation GUI cancelled.'
    exit 0
}

$selection = $script:Selection
Write-Host ''
Write-Host "Saved GUI configuration: $StatePath"
Write-Host "Running visual-validation experiment: $($selection.experiment)"

if ($selection.experiment -eq 'range-image') {
    $runnerArgs = @{
        CorpusRoot = [string]$selection.corpus_root
        Strategy = [string]$selection.strategy
        OverflowMode = [string]$selection.overflow_mode
        TileMegapixels = [int]$selection.tile_megapixels
        TilePacking = [string]$selection.tile_packing
        Implementation = [string]$selection.implementation
        BatchSize = [int]$selection.batch_size
        OverflowProbeWidthPx = [int]$selection.overflow_probe_width_px
        OverflowProbeHeightPx = [int]$selection.overflow_probe_height_px
        RustExcelLaunchMode = [string]$selection.rust_excel_launch_mode
        NonInteractive = $true
    }

    if (
        -not [string]::IsNullOrWhiteSpace([string]$selection.workbook) -and
        [string]$selection.workbook -notin @('all', '*')
    ) {
        $runnerArgs.Workbook = [string]$selection.workbook
    }

    if ([bool]$selection.wait_for_start) {
        $runnerArgs.WaitForStart = $true
    }

    switch ([string]$selection.diagnostics) {
        'extended' {
            $runnerArgs.ExtendedErrorLogging = $true
        }
        'debug-js' {
            $runnerArgs.ExtendedErrorLogging = $true
            $runnerArgs.UseOfficeDebugJs = $true
        }
    }

    & $RangeRunner @runnerArgs
}
elseif ($selection.experiment -in @('range-image-height-limit', 'range-image-calibration')) {
    $diagnosticArgs = @{
        CorpusRoot = [string]$selection.corpus_root
        NonInteractive = $true
        WaitForStart = [bool]$selection.diagnostic_wait
        BreakOnError = [bool]$selection.diagnostic_break
        UseOfficeDebugJs = [bool]$selection.diagnostic_debug_js
    }
    if ($selection.experiment -eq 'range-image-calibration') {
        $diagnosticArgs.Suite = [string]$selection.diagnostic_suite
        & $CalibrationRunner @diagnosticArgs
    } else {
        & $HeightLimitRunner @diagnosticArgs
    }
}
else {
    $runnerArgs = @{
        CorpusRoot = [string]$selection.corpus_root
        NonInteractive = $true
    }

    if ($null -ne $selection.shape_parameters) {
        if ($selection.shape_parameters -is [System.Collections.IDictionary]) {
            foreach ($key in $selection.shape_parameters.Keys) {
                $runnerArgs[[string]$key] = $selection.shape_parameters[$key]
            }
        }
        else {
            foreach ($property in $selection.shape_parameters.PSObject.Properties) {
                $runnerArgs[$property.Name] = $property.Value
            }
        }
    }

    & $ShapeRunner @runnerArgs
}

if (-not $?) {
    exit 1
}
