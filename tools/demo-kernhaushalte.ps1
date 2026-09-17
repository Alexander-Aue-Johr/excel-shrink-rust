[CmdletBinding()]
param(
    [string]$InputPath,
    [string]$ExcelShrinkExe,
    [string]$OutputDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName PresentationFramework
Add-Type -AssemblyName System.IO.Compression.FileSystem

if (-not ('ExcelShrinkNativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class ExcelShrinkNativeMethods
{
    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
}
'@
}

if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = Join-Path $PSScriptRoot '..\corpus-local\original_destatis_xlsx_files\statistischer-bericht-rechnungsergebnis-kernhaushalt-gemeinden-2140331217005.xlsx'
}
if ([string]::IsNullOrWhiteSpace($ExcelShrinkExe)) {
    $ExcelShrinkExe = Join-Path $PSScriptRoot '..\target\profiling\excel-shrink-cli.exe'
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $PSScriptRoot '..\.tmp\kernhaushalte-demo'
}

function Show-InfoDialog {
    param(
        [Parameter(Mandatory)] [string]$Message,
        [Parameter(Mandatory)] [string]$Title
    )

    [System.Windows.MessageBox]::Show(
        $Message,
        $Title,
        [System.Windows.MessageBoxButton]::OK,
        [System.Windows.MessageBoxImage]::Information
    ) | Out-Null
}

function Show-StartDialog {
    param(
        [Parameter(Mandatory)] [string]$Message,
        [Parameter(Mandatory)] [string]$Title
    )

    return [System.Windows.MessageBox]::Show(
        $Message,
        $Title,
        [System.Windows.MessageBoxButton]::OKCancel,
        [System.Windows.MessageBoxImage]::Information
    ) -eq [System.Windows.MessageBoxResult]::OK
}

function Release-ComObject {
    param([object]$ComObject)

    if ($null -ne $ComObject -and [System.Runtime.InteropServices.Marshal]::IsComObject($ComObject)) {
        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($ComObject)
    }
}

function Bring-ExcelToForeground {
    param([Parameter(Mandatory)] [object]$Excel)

    try {
        $handle = [IntPtr]$Excel.Hwnd
        if ($handle -ne [IntPtr]::Zero) {
            # SW_RESTORE = 9. This restores a minimized window before focusing it.
            [void][ExcelShrinkNativeMethods]::ShowWindowAsync($handle, 9)
            [void][ExcelShrinkNativeMethods]::SetForegroundWindow($handle)
        }
    }
    catch {
        # Foreground activation is a visual enhancement and must not abort the demo.
    }
}

function Get-WorkbookXmlHash {
    param([Parameter(Mandatory)] [string]$WorkbookPath)

    $archive = $null
    $stream = $null
    $sha256 = $null
    try {
        $archive = [System.IO.Compression.ZipFile]::OpenRead($WorkbookPath)
        $entry = $archive.GetEntry('xl/workbook.xml')
        if ($null -eq $entry) {
            throw "The XLSX archive does not contain xl/workbook.xml: $WorkbookPath"
        }
        $stream = $entry.Open()
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '')
    }
    finally {
        if ($null -ne $sha256) { $sha256.Dispose() }
        if ($null -ne $stream) { $stream.Dispose() }
        if ($null -ne $archive) { $archive.Dispose() }
    }
}

function Prepare-DemoInput {
    param(
        [Parameter(Mandatory)] [string]$InputFile,
        [Parameter(Mandatory)] [string]$Executable,
        [Parameter(Mandatory)] [string]$WorkingDirectory
    )

    $repairDirectory = Join-Path $WorkingDirectory 'workbook-xml-repair'
    New-Item -ItemType Directory -Path $repairDirectory -Force | Out-Null
    $repairedPath = Join-Path $repairDirectory ([System.IO.Path]::GetFileName($InputFile))

    $repairOutput = & $Executable `
        $InputFile `
        $repairDirectory `
        --only-clean-workbook `
        --disable-multiprocessing `
        --force-overwrite `
        --console-log-level error 2>&1
    $repairExitCode = $LASTEXITCODE
    if ($repairExitCode -ne 0) {
        throw "Workbook XML preparation failed with exit code ${repairExitCode}:`n$($repairOutput -join [Environment]::NewLine)"
    }
    if (-not (Test-Path -LiteralPath $repairedPath -PathType Leaf)) {
        throw "Workbook XML preparation did not create its output:`n$repairedPath"
    }

    $originalHash = Get-WorkbookXmlHash -WorkbookPath $InputFile
    $repairedHash = Get-WorkbookXmlHash -WorkbookPath $repairedPath
    if ($originalHash -eq $repairedHash) {
        Remove-Item -LiteralPath $repairedPath -Force
        return [pscustomobject]@{
            Path          = $InputFile
            RepairApplied = $false
            BaselineLabel = 'Open original Excel file'
            Disclosure    = 'Workbook XML check: no defined-name repair was needed; the original file was used unchanged.'
        }
    }

    return [pscustomobject]@{
        Path          = $repairedPath
        RepairApplied = $true
        BaselineLabel = 'Open repaired baseline workbook'
        Disclosure    = 'Workbook XML check: excel-shrink removed duplicate defined-name collisions from a temporary demo copy. The downloaded source file was not modified. All measured runs use the same repaired input.'
    }
}

function Show-InputSelectionWindow {
    param(
        [Parameter(Mandatory)] [string]$DefaultPath,
        [Parameter(Mandatory)] [string]$DestatisDirectory,
        [Parameter(Mandatory)] [string]$BaselinePath
    )

    [xml]$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Title="Excel Shrink - Select a file"
        Width="720" Height="470" WindowStartupLocation="CenterScreen"
        Background="#F4F7FB" ResizeMode="NoResize">
  <Grid Margin="28">
    <Grid.RowDefinitions>
      <RowDefinition Height="Auto"/>
      <RowDefinition Height="Auto"/>
      <RowDefinition Height="Auto"/>
      <RowDefinition Height="*"/>
      <RowDefinition Height="Auto"/>
    </Grid.RowDefinitions>

    <StackPanel Grid.Row="0">
      <TextBlock Text="Which Excel file should be demonstrated?" FontSize="25" FontWeight="Bold" Foreground="#14213D"/>
      <TextBlock Text="Destatis files are downloaded automatically when needed." FontSize="14" Foreground="#52606D" Margin="0,6,0,0"/>
      <TextBlock x:Name="SourceInfo" FontSize="12" Foreground="#7B8794" Margin="0,5,0,0" TextWrapping="Wrap"/>
    </StackPanel>

    <Border Grid.Row="1" Background="White" CornerRadius="10" Padding="16" Margin="0,20,0,0">
      <StackPanel>
        <RadioButton x:Name="DestatisRadio" Content="Use a Destatis test file" IsChecked="True" FontWeight="SemiBold" FontSize="14"/>
        <ComboBox x:Name="DestatisCombo" Height="34" Margin="24,10,0,0" VerticalContentAlignment="Center"/>
      </StackPanel>
    </Border>

    <Border Grid.Row="2" Background="White" CornerRadius="10" Padding="16" Margin="0,12,0,0">
      <StackPanel>
        <RadioButton x:Name="OwnFileRadio" Content="Choose your own Excel file" FontWeight="SemiBold" FontSize="14"/>
        <Grid Margin="24,10,0,0">
          <Grid.ColumnDefinitions>
            <ColumnDefinition Width="*"/>
            <ColumnDefinition Width="Auto"/>
          </Grid.ColumnDefinitions>
          <TextBox x:Name="OwnFileText" Grid.Column="0" Height="34" IsEnabled="False" VerticalContentAlignment="Center" Padding="8,0"/>
          <Button x:Name="BrowseButton" Grid.Column="1" Content="Browse..." Width="120" Height="34" Margin="10,0,0,0" IsEnabled="False"/>
        </Grid>
      </StackPanel>
    </Border>

    <TextBlock x:Name="StatusText" Grid.Row="3" Foreground="#C92A2A" Margin="4,12,4,0" TextWrapping="Wrap"/>

    <StackPanel Grid.Row="4" Orientation="Horizontal" HorizontalAlignment="Right" Margin="0,18,0,0">
      <Button x:Name="CancelButton" Content="Cancel" Width="110" Height="38" Margin="0,0,10,0"/>
      <Button x:Name="StartButton" Content="Start demo" Width="130" Height="38" Background="#14213D" Foreground="White" FontWeight="SemiBold" BorderThickness="0"/>
    </StackPanel>
  </Grid>
</Window>
'@

    $reader = New-Object System.Xml.XmlNodeReader $xaml
    $window = [Windows.Markup.XamlReader]::Load($reader)
    $destatisRadio = $window.FindName('DestatisRadio')
    $ownFileRadio = $window.FindName('OwnFileRadio')
    $destatisCombo = $window.FindName('DestatisCombo')
    $ownFileText = $window.FindName('OwnFileText')
    $browseButton = $window.FindName('BrowseButton')
    $statusText = $window.FindName('StatusText')
    $window.FindName('SourceInfo').Text = "Link source: $BaselinePath`nDownload cache: $DestatisDirectory"
    $state = @{ SelectedPath = $null }

    $destatisChoices = @()
    if (Test-Path -LiteralPath $BaselinePath -PathType Leaf) {
        $baseline = Get-Content -LiteralPath $BaselinePath -Raw | ConvertFrom-Json
        foreach ($case in $baseline.cases) {
            $fileName = if (-not [string]::IsNullOrWhiteSpace([string]$case.cached_file_name)) {
                [string]$case.cached_file_name
            }
            else {
                [string]$case.relative_input
            }
            if ([string]::IsNullOrWhiteSpace($fileName) -or [string]::IsNullOrWhiteSpace([string]$case.download_url)) {
                continue
            }
            $destatisChoices += [pscustomobject]@{
                Label       = "$($case.id) - $fileName"
                LocalPath   = Join-Path $DestatisDirectory $fileName
                DownloadUrl = [string]$case.download_url
            }
        }
    }
    elseif (Test-Path -LiteralPath $DestatisDirectory -PathType Container) {
        foreach ($file in (Get-ChildItem -LiteralPath $DestatisDirectory -Filter '*.xlsx' -File | Sort-Object Name)) {
            $destatisChoices += [pscustomobject]@{
                Label       = $file.Name
                LocalPath   = $file.FullName
                DownloadUrl = $null
            }
        }
    }

    $defaultFullPath = [System.IO.Path]::GetFullPath($DefaultPath)
    $selectedIndex = 0
    for ($index = 0; $index -lt $destatisChoices.Count; $index++) {
        $item = New-Object System.Windows.Controls.ComboBoxItem
        $item.Content = $destatisChoices[$index].Label
        $item.Tag = $destatisChoices[$index]
        [void]$destatisCombo.Items.Add($item)
        if ($destatisChoices[$index].LocalPath -eq $defaultFullPath) {
            $selectedIndex = $index
        }
    }

    if ($destatisCombo.Items.Count -gt 0) {
        $destatisCombo.SelectedIndex = $selectedIndex
    }
    else {
        $destatisRadio.IsEnabled = $false
        $destatisCombo.IsEnabled = $false
        $ownFileRadio.IsChecked = $true
        $ownFileText.IsEnabled = $true
        $browseButton.IsEnabled = $true
        $statusText.Text = 'No Destatis files were found in the baseline or corpus directory.'
    }
    $ownFileText.Text = $defaultFullPath

    $destatisRadio.Add_Checked({
        $destatisCombo.IsEnabled = $true
        $ownFileText.IsEnabled = $false
        $browseButton.IsEnabled = $false
        $statusText.Text = ''
    })
    $ownFileRadio.Add_Checked({
        $destatisCombo.IsEnabled = $false
        $ownFileText.IsEnabled = $true
        $browseButton.IsEnabled = $true
        $statusText.Text = ''
    })
    $browseButton.Add_Click({
        $dialog = New-Object Microsoft.Win32.OpenFileDialog
        $dialog.Title = 'Select an Excel file'
        $dialog.Filter = 'Excel workbooks (*.xlsx)|*.xlsx'
        $dialog.CheckFileExists = $true
        $dialog.Multiselect = $false
        if (Test-Path -LiteralPath $ownFileText.Text -PathType Leaf) {
            $dialog.InitialDirectory = [System.IO.Path]::GetDirectoryName($ownFileText.Text)
            $dialog.FileName = [System.IO.Path]::GetFileName($ownFileText.Text)
        }
        if ($dialog.ShowDialog($window) -eq $true) {
            $ownFileText.Text = $dialog.FileName
            $statusText.Text = ''
        }
    })
    $window.FindName('CancelButton').Add_Click({ $window.Close() })
    $window.FindName('StartButton').Add_Click({
        $candidate = $null
        if ($destatisRadio.IsChecked) {
            if ($null -ne $destatisCombo.SelectedItem) {
                $choice = $destatisCombo.SelectedItem.Tag
                $candidate = [string]$choice.LocalPath
                if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
                    if ([string]::IsNullOrWhiteSpace([string]$choice.DownloadUrl)) {
                        $statusText.Text = 'No download URL is configured for this file.'
                        return
                    }

                    try {
                        $partialPath = $null
                        $statusText.Foreground = '#1C7ED6'
                        $statusText.Text = 'Downloading the Destatis file. Please wait...'
                        $window.Dispatcher.Invoke([action]{ }, [System.Windows.Threading.DispatcherPriority]::Render)
                        New-Item -ItemType Directory -Path ([System.IO.Path]::GetDirectoryName($candidate)) -Force | Out-Null
                        $partialPath = "$candidate.partial"
                        Remove-Item -LiteralPath $partialPath -Force -ErrorAction SilentlyContinue
                        Invoke-WebRequest -Uri $choice.DownloadUrl -OutFile $partialPath -UseBasicParsing
                        Move-Item -LiteralPath $partialPath -Destination $candidate -Force
                        $statusText.Foreground = '#087F5B'
                        $statusText.Text = 'Download complete.'
                    }
                    catch {
                        if ($null -ne $partialPath) {
                            Remove-Item -LiteralPath $partialPath -Force -ErrorAction SilentlyContinue
                        }
                        $statusText.Foreground = '#C92A2A'
                        $statusText.Text = "Download failed: $($_.Exception.Message)"
                        return
                    }
                }
            }
        }
        else {
            $candidate = $ownFileText.Text.Trim()
        }

        if ([string]::IsNullOrWhiteSpace($candidate) -or -not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            $statusText.Text = 'Please select an existing XLSX file.'
            return
        }
        if ([System.IO.Path]::GetExtension($candidate) -ne '.xlsx') {
            $statusText.Text = 'Please select a file with the .xlsx extension.'
            return
        }

        $state.SelectedPath = [System.IO.Path]::GetFullPath($candidate)
        $window.DialogResult = $true
        $window.Close()
    })

    [void]$window.ShowDialog()
    return $state.SelectedPath
}

function Open-WorkbookAndReadFirstValue {
    param([Parameter(Mandatory)] [string]$Path)

    $excel = $null
    $workbook = $null
    $worksheets = $null
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

    try {
        $excel = New-Object -ComObject Excel.Application
        $excel.Visible = $true
        $excel.DisplayAlerts = $false
        $excel.ScreenUpdating = $true
        Bring-ExcelToForeground -Excel $excel

        $workbook = $excel.Workbooks.Open($Path, 0, $true)
        Bring-ExcelToForeground -Excel $excel
        $worksheets = $workbook.Worksheets

        $foundSheet = $null
        $foundCell = $null

        foreach ($sheet in $worksheets) {
            $after = $null
            try {
                # Excel constants: xlValues=-4163, xlPart=2, xlByRows=1, xlNext=1.
                # Starting after the last possible cell makes Find return the first
                # non-empty cell in row-major order.
                $after = $sheet.Cells.Item($sheet.Rows.Count, $sheet.Columns.Count)
                $cell = $sheet.Cells.Find('*', $after, -4163, 2, 1, 1, $false, $false, $false)
                if ($null -ne $cell) {
                    $foundSheet = $sheet.Name
                    $foundCell = $cell
                    break
                }
            }
            finally {
                Release-ComObject $after
            }
        }

        if ($null -eq $foundCell) {
            throw "The workbook does not contain a cell with a value."
        }

        $value = $foundCell.Value2
        if ($value -is [System.Array]) {
            $value = $value.GetValue(0)
        }

        $displayValue = if ($null -eq $value) { '<empty>' } else { [string]$value }
        $address = $foundCell.Address($false, $false)
        $stopwatch.Stop()

        Release-ComObject $foundCell

        return [pscustomobject]@{
            Excel       = $excel
            Workbook    = $workbook
            Worksheets  = $worksheets
            Sheet       = $foundSheet
            Address     = $address
            Value       = $displayValue
            ElapsedMs   = $stopwatch.Elapsed.TotalMilliseconds
        }
    }
    catch {
        $stopwatch.Stop()
        if ($null -ne $workbook) {
            try { $workbook.Close($false) } catch { }
        }
        if ($null -ne $excel) {
            try { $excel.Quit() } catch { }
        }
        Release-ComObject $worksheets
        Release-ComObject $workbook
        Release-ComObject $excel
        throw
    }
}

function Close-ExcelResult {
    param([object]$Result)

    if ($null -eq $Result) {
        return
    }

    try { $Result.Workbook.Close($false) } catch { }
    try { $Result.Excel.Quit() } catch { }
    try { Release-ComObject $Result.Worksheets } catch { }
    try { Release-ComObject $Result.Workbook } catch { }
    try { Release-ComObject $Result.Excel } catch { }
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}

function Format-Duration {
    param([double]$Milliseconds)

    if ($Milliseconds -lt 1000) {
        return ('{0:N0} ms' -f $Milliseconds)
    }
    return ('{0:N3} s' -f ($Milliseconds / 1000))
}

function Show-ComparisonWindow {
    param(
        [Parameter(Mandatory)] [string]$CellText,
        [Parameter(Mandatory)] [string]$ValueCheck,
        [Parameter(Mandatory)] [string]$InputDisclosure,
        [Parameter(Mandatory)] [string]$BaselineLabel,
        [Parameter(Mandatory)] [double]$OriginalMs,
        [Parameter(Mandatory)] [double]$MultiShrinkMs,
        [Parameter(Mandatory)] [double]$MultiOpenMs,
        [Parameter(Mandatory)] [double]$MultiTotalMs,
        [Parameter(Mandatory)] [double]$SingleShrinkMs,
        [Parameter(Mandatory)] [double]$SingleOpenMs,
        [Parameter(Mandatory)] [double]$SingleTotalMs
    )

    [xml]$xaml = @'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Title="Excel Shrink - Performance results"
        Width="920" Height="840" WindowStartupLocation="CenterScreen"
        Background="#F4F7FB" ResizeMode="CanMinimize">
  <Grid Margin="28">
    <Grid.RowDefinitions>
      <RowDefinition Height="Auto"/>
      <RowDefinition Height="Auto"/>
      <RowDefinition Height="Auto"/>
      <RowDefinition Height="Auto"/>
      <RowDefinition Height="Auto"/>
    </Grid.RowDefinitions>

    <StackPanel Grid.Row="0">
      <TextBlock Text="Excel Shrink Performance" FontSize="30" FontWeight="Bold" Foreground="#14213D"/>
      <TextBlock Text="Original vs. multicore vs. single-core" FontSize="16" Foreground="#52606D" Margin="0,4,0,0"/>
    </StackPanel>

    <Border Grid.Row="1" Background="White" CornerRadius="10" Padding="16" Margin="0,18,0,0">
      <StackPanel>
        <TextBlock x:Name="CellText" FontSize="15" FontWeight="SemiBold" Foreground="#14213D"/>
        <TextBlock x:Name="ValueCheck" FontSize="13" Foreground="#52606D" Margin="0,5,0,0" TextWrapping="Wrap"/>
      </StackPanel>
    </Border>

    <Border Grid.Row="2" Background="White" CornerRadius="10" Padding="18" Margin="0,14,0,0">
      <StackPanel>
        <TextBlock Text="Total time comparison" FontSize="19" FontWeight="Bold" Foreground="#14213D"/>
        <TextBlock Text="Shorter is better" FontSize="12" Foreground="#7B8794" Margin="0,2,0,14"/>

        <TextBlock x:Name="OriginalLabel" FontSize="13" FontWeight="SemiBold"/>
        <Border Background="#E7ECF2" Height="30" CornerRadius="5" Margin="0,4,0,11">
          <Border x:Name="OriginalBar" Background="#607D8B" HorizontalAlignment="Left" CornerRadius="5"/>
        </Border>

        <TextBlock x:Name="MultiLabel" FontSize="13" FontWeight="SemiBold"/>
        <Border Background="#E7ECF2" Height="30" CornerRadius="5" Margin="0,4,0,11">
          <StackPanel x:Name="MultiBarGroup" Orientation="Horizontal" HorizontalAlignment="Left">
            <Border x:Name="MultiShrinkBar" Background="#19A974" Height="30" CornerRadius="5,0,0,5"/>
            <Border x:Name="MultiOpenBar" Background="#3B82F6" Height="30" CornerRadius="0,5,5,0"/>
          </StackPanel>
        </Border>

        <TextBlock x:Name="SingleLabel" FontSize="13" FontWeight="SemiBold"/>
        <Border Background="#E7ECF2" Height="30" CornerRadius="5" Margin="0,4,0,0">
          <StackPanel x:Name="SingleBarGroup" Orientation="Horizontal" HorizontalAlignment="Left">
            <Border x:Name="SingleShrinkBar" Background="#FF9F1C" Height="30" CornerRadius="5,0,0,5"/>
            <Border x:Name="SingleOpenBar" Background="#3B82F6" Height="30" CornerRadius="0,5,5,0"/>
          </StackPanel>
        </Border>

        <StackPanel Orientation="Horizontal" Margin="0,13,0,0">
          <Border Width="13" Height="13" Background="#19A974" CornerRadius="2" Margin="0,1,6,0"/>
          <TextBlock Text="Multicore shrink" FontSize="12" Foreground="#52606D" Margin="0,0,18,0"/>
          <Border Width="13" Height="13" Background="#FF9F1C" CornerRadius="2" Margin="0,1,6,0"/>
          <TextBlock Text="Single-core shrink" FontSize="12" Foreground="#52606D" Margin="0,0,18,0"/>
          <Border Width="13" Height="13" Background="#3B82F6" CornerRadius="2" Margin="0,1,6,0"/>
          <TextBlock Text="Open shrunk workbook in Excel" FontSize="12" Foreground="#52606D"/>
        </StackPanel>
      </StackPanel>
    </Border>

    <Grid Grid.Row="3" Margin="0,14,0,0">
      <Grid.ColumnDefinitions>
        <ColumnDefinition Width="*"/>
        <ColumnDefinition Width="14"/>
        <ColumnDefinition Width="*"/>
      </Grid.ColumnDefinitions>
      <Border Grid.Column="0" Background="#E8F7F1" BorderBrush="#19A974" BorderThickness="1" CornerRadius="10" Padding="16">
        <StackPanel>
          <TextBlock Text="MULTICORE" FontWeight="Bold" FontSize="17" Foreground="#087F5B"/>
          <TextBlock x:Name="MultiDetails" FontSize="14" Margin="0,8,0,0" TextWrapping="Wrap" LineHeight="23"/>
          <TextBlock x:Name="MultiStatement" FontSize="16" FontWeight="Bold" Foreground="#087F5B" Margin="0,12,0,0" TextWrapping="Wrap"/>
        </StackPanel>
      </Border>
      <Border Grid.Column="2" Background="#FFF4E5" BorderBrush="#FF9F1C" BorderThickness="1" CornerRadius="10" Padding="16">
        <StackPanel>
          <TextBlock Text="SINGLECORE" FontWeight="Bold" FontSize="17" Foreground="#B85C00"/>
          <TextBlock x:Name="SingleDetails" FontSize="14" Margin="0,8,0,0" TextWrapping="Wrap" LineHeight="23"/>
          <TextBlock x:Name="SingleStatement" FontSize="16" FontWeight="Bold" Foreground="#B85C00" Margin="0,12,0,0" TextWrapping="Wrap"/>
        </StackPanel>
      </Border>
    </Grid>

    <Button x:Name="CloseButton" Grid.Row="4" Content="Close Excel" Width="170" Height="40"
            HorizontalAlignment="Right" Margin="0,18,0,0" Background="#14213D" Foreground="White"
            FontWeight="SemiBold" BorderThickness="0"/>
  </Grid>
</Window>
'@

    $reader = New-Object System.Xml.XmlNodeReader $xaml
    $window = [Windows.Markup.XamlReader]::Load($reader)
    $window.FindName('CellText').Text = $CellText
    $window.FindName('ValueCheck').Text = "$ValueCheck`n$InputDisclosure"

    $window.FindName('OriginalLabel').Text = "${BaselineLabel}: $(Format-Duration $OriginalMs)"
    $window.FindName('MultiLabel').Text = "Excel Shrink multicore + open file: $(Format-Duration $MultiTotalMs)"
    $window.FindName('SingleLabel').Text = "Excel Shrink single-core + open file: $(Format-Duration $SingleTotalMs)"

    $maximum = [math]::Max($OriginalMs, [math]::Max($MultiTotalMs, $SingleTotalMs))
    $chartWidth = 790.0
    $window.FindName('OriginalBar').Width = [math]::Max(4, $chartWidth * $OriginalMs / $maximum)

    $multiBarWidth = [math]::Max(4, $chartWidth * $MultiTotalMs / $maximum)
    $multiMeasuredMs = [math]::Max(0.001, $MultiShrinkMs + $MultiOpenMs)
    $multiShrinkWidth = $multiBarWidth * $MultiShrinkMs / $multiMeasuredMs
    $window.FindName('MultiBarGroup').Width = $multiBarWidth
    $window.FindName('MultiShrinkBar').Width = $multiShrinkWidth
    $window.FindName('MultiOpenBar').Width = [math]::Max(1, $multiBarWidth - $multiShrinkWidth)

    $singleBarWidth = [math]::Max(4, $chartWidth * $SingleTotalMs / $maximum)
    $singleMeasuredMs = [math]::Max(0.001, $SingleShrinkMs + $SingleOpenMs)
    $singleShrinkWidth = $singleBarWidth * $SingleShrinkMs / $singleMeasuredMs
    $window.FindName('SingleBarGroup').Width = $singleBarWidth
    $window.FindName('SingleShrinkBar').Width = $singleShrinkWidth
    $window.FindName('SingleOpenBar').Width = [math]::Max(1, $singleBarWidth - $singleShrinkWidth)

    $multiRatio = if ($MultiTotalMs -gt 0) { $OriginalMs / $MultiTotalMs } else { [double]::PositiveInfinity }
    $singleRatio = if ($SingleTotalMs -gt 0) { $OriginalMs / $SingleTotalMs } else { [double]::PositiveInfinity }
    $multiTimeReduction = if ($OriginalMs -gt 0) { (1 - ($MultiTotalMs / $OriginalMs)) * 100 } else { 0 }
    $singleTimeReduction = if ($OriginalMs -gt 0) { (1 - ($SingleTotalMs / $OriginalMs)) * 100 } else { 0 }
    $multiStatement = if ($multiRatio -ge 1) {
        "Excel Shrink multicore + opening the Excel file was $($multiRatio.ToString('N2'))x as fast as opening the original file ($($multiTimeReduction.ToString('N1'))% less elapsed time)."
    } else {
        "Excel Shrink multicore + opening the Excel file was $($multiRatio.ToString('N2'))x as fast as opening the original file ($([math]::Abs($multiTimeReduction).ToString('N1'))% more elapsed time)."
    }
    $singleStatement = if ($singleRatio -ge 1) {
        "Excel Shrink single-core + opening the Excel file was $($singleRatio.ToString('N2'))x as fast as opening the original file ($($singleTimeReduction.ToString('N1'))% less elapsed time)."
    } else {
        "Excel Shrink single-core + opening the Excel file was $($singleRatio.ToString('N2'))x as fast as opening the original file ($([math]::Abs($singleTimeReduction).ToString('N1'))% more elapsed time)."
    }

    $window.FindName('MultiDetails').Text = "Shrink: $(Format-Duration $MultiShrinkMs)`nOpen Excel: $(Format-Duration $MultiOpenMs)`nTotal: $(Format-Duration $MultiTotalMs)"
    $window.FindName('SingleDetails').Text = "Shrink: $(Format-Duration $SingleShrinkMs)`nOpen Excel: $(Format-Duration $SingleOpenMs)`nTotal: $(Format-Duration $SingleTotalMs)"
    $window.FindName('MultiStatement').Text = $multiStatement
    $window.FindName('SingleStatement').Text = $singleStatement
    $window.FindName('CloseButton').Add_Click({ $window.Close() })

    [void]$window.ShowDialog()
}

function Invoke-ShrinkAndOpen {
    param(
        [Parameter(Mandatory)] [string]$InputFile,
        [Parameter(Mandatory)] [string]$Executable,
        [Parameter(Mandatory)] [string]$OutputDir,
        [switch]$DisableMultiprocessing
    )

    New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
    $outputPath = Join-Path $OutputDir ([System.IO.Path]::GetFileName($InputFile))
    $arguments = @(
        $InputFile,
        $OutputDir,
        '--force-overwrite',
        '--remove-textless-default-font-representatives',
        '--remove-textless-style-representatives',
        '--remove-textless-column-style-representatives',
        '--remove-unreferenced-whitespace-cells',
        '--console-log-level',
        'error'
    )
    if ($DisableMultiprocessing) {
        $arguments += '--disable-multiprocessing'
    }

    $totalStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $shrinkStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $shrinkOutput = & $Executable @arguments 2>&1
    $shrinkExitCode = $LASTEXITCODE
    $shrinkStopwatch.Stop()

    if ($shrinkExitCode -ne 0) {
        throw "excel-shrink failed with exit code ${shrinkExitCode}:`n$($shrinkOutput -join [Environment]::NewLine)"
    }
    if (-not (Test-Path -LiteralPath $outputPath -PathType Leaf)) {
        throw "The shrunk workbook was not created:`n$outputPath"
    }

    $excelResult = Open-WorkbookAndReadFirstValue -Path $outputPath
    $totalStopwatch.Stop()

    return [pscustomobject]@{
        ExcelResult = $excelResult
        ShrinkMs    = $shrinkStopwatch.Elapsed.TotalMilliseconds
        TotalMs     = $totalStopwatch.Elapsed.TotalMilliseconds
        OutputPath  = $outputPath
    }
}

$originalResult = $null
$multiResult = $null
$singleResult = $null

try {
    $InputPath = [System.IO.Path]::GetFullPath($InputPath)
    $ExcelShrinkExe = [System.IO.Path]::GetFullPath($ExcelShrinkExe)
    $OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)

    $selectedInput = Show-InputSelectionWindow `
        -DefaultPath $InputPath `
        -DestatisDirectory (Join-Path $PSScriptRoot '..\corpus-local\original_destatis_xlsx_files') `
        -BaselinePath (Join-Path $PSScriptRoot '..\regression\baselines\smoke.json')
    if ([string]::IsNullOrWhiteSpace($selectedInput)) {
        return
    }
    $InputPath = [System.IO.Path]::GetFullPath($selectedInput)

    if (-not (Test-Path -LiteralPath $InputPath -PathType Leaf)) {
        throw "Input file not found:`n$InputPath"
    }
    if (-not (Test-Path -LiteralPath $ExcelShrinkExe -PathType Leaf)) {
        throw "excel-shrink was not found:`n$ExcelShrinkExe`n`nBuild it first with:`ncargo build --offline --profile profiling -p excel-shrink-cli"
    }

    $inputPreparation = Prepare-DemoInput `
        -InputFile $InputPath `
        -Executable $ExcelShrinkExe `
        -WorkingDirectory $OutputDirectory
    $demoInputPath = $inputPreparation.Path

    $selectedFileName = [System.IO.Path]::GetFileName($InputPath)
    if (-not (Show-StartDialog -Title 'Excel Shrink - Original workbook' -Message @"
The following workbook will now be opened visibly in Microsoft Excel:

$selectedFileName

The script will read the first cell containing a value and report the elapsed time.

$($inputPreparation.Disclosure)

Click OK to start timing.
"@)) { return }

    $originalResult = Open-WorkbookAndReadFirstValue -Path $demoInputPath
    Show-InfoDialog -Title 'Original workbook opened' -Message @"
The first cell containing a value is:

$($originalResult.Sheet)!$($originalResult.Address) = $($originalResult.Value)

Open and inspect: $(Format-Duration $originalResult.ElapsedMs)

Click OK to close Excel.
"@
    Close-ExcelResult $originalResult
    $originalSummary = $originalResult
    $originalResult = $null

    if (-not (Show-StartDialog -Title 'Excel Shrink - Multicore' -Message @"
The same workbook will now be shrunk with multicore processing enabled.

The shrunk workbook will then be opened visibly in Excel and inspected.

Click OK to start total timing.
"@)) { return }

    $multiResult = Invoke-ShrinkAndOpen `
        -InputFile $demoInputPath `
        -Executable $ExcelShrinkExe `
        -OutputDir (Join-Path $OutputDirectory 'multicore')

    Show-InfoDialog -Title 'Multicore result' -Message @"
The first cell containing a value is:

$($multiResult.ExcelResult.Sheet)!$($multiResult.ExcelResult.Address) = $($multiResult.ExcelResult.Value)

Shrink time:                  $(Format-Duration $multiResult.ShrinkMs)
Open workbook:                $(Format-Duration $multiResult.ExcelResult.ElapsedMs)
Shrink + open + inspect:      $(Format-Duration $multiResult.TotalMs)

Click OK to close Excel. The single-core run follows next.
"@
    Close-ExcelResult $multiResult.ExcelResult

    if (-not (Show-StartDialog -Title 'Excel Shrink - Single-core' -Message @"
The same test now runs with --disable-multiprocessing.

The workbook is shrunk without multicore processing and then opened visibly in Excel.

Click OK to start total timing.
"@)) { return }

    $singleResult = Invoke-ShrinkAndOpen `
        -InputFile $demoInputPath `
        -Executable $ExcelShrinkExe `
        -OutputDir (Join-Path $OutputDirectory 'singlecore') `
        -DisableMultiprocessing

    $originalOpenMs = $originalSummary.ElapsedMs
    $multiOpenMs = $multiResult.ExcelResult.ElapsedMs
    $singleOpenMs = $singleResult.ExcelResult.ElapsedMs
    $multiValueMatches = (
        $originalSummary.Sheet -eq $multiResult.ExcelResult.Sheet -and
        $originalSummary.Address -eq $multiResult.ExcelResult.Address -and
        $originalSummary.Value -eq $multiResult.ExcelResult.Value
    )
    $singleValueMatches = (
        $originalSummary.Sheet -eq $singleResult.ExcelResult.Sheet -and
        $originalSummary.Address -eq $singleResult.ExcelResult.Address -and
        $originalSummary.Value -eq $singleResult.ExcelResult.Value
    )
    $valueCheck = if ($multiValueMatches -and $singleValueMatches) {
        'Check passed: cell address and value match the original in both results.'
    }
    else {
        'WARNING: A result has a different first-cell address or value than the original.'
    }

    Show-ComparisonWindow `
        -CellText "First cell: $($singleResult.ExcelResult.Sheet)!$($singleResult.ExcelResult.Address) = $($singleResult.ExcelResult.Value)" `
        -ValueCheck $valueCheck `
        -InputDisclosure $inputPreparation.Disclosure `
        -BaselineLabel $inputPreparation.BaselineLabel `
        -OriginalMs $originalOpenMs `
        -MultiShrinkMs $multiResult.ShrinkMs `
        -MultiOpenMs $multiOpenMs `
        -MultiTotalMs $multiResult.TotalMs `
        -SingleShrinkMs $singleResult.ShrinkMs `
        -SingleOpenMs $singleOpenMs `
        -SingleTotalMs $singleResult.TotalMs
}
catch {
    [System.Windows.MessageBox]::Show(
        $_.Exception.Message,
        'Excel Shrink - Error',
        [System.Windows.MessageBoxButton]::OK,
        [System.Windows.MessageBoxImage]::Error
    ) | Out-Null
    exit 1
}
finally {
    Close-ExcelResult $originalResult
    if ($null -ne $multiResult) { Close-ExcelResult $multiResult.ExcelResult }
    if ($null -ne $singleResult) { Close-ExcelResult $singleResult.ExcelResult }
}
