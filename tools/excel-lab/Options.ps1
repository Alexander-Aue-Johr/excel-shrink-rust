function Show-ExcelLabOptions {
    param([string]$ToolRoot, [string[]]$Profiles)
    Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase
    if (-not ('ExcelLab.FolderPicker' -as [type])) { Add-Type -Path (Join-Path $ToolRoot 'FolderPicker.cs') }
    $xaml=[xml][IO.File]::ReadAllText((Join-Path $ToolRoot 'Options.xaml'))
    $reader=[Xml.XmlNodeReader]::new($xaml)
    try { $window=[Windows.Markup.XamlReader]::Load($reader) } finally { $reader.Close() }
    $controls=@{}
    foreach ($node in $xaml.SelectNodes('//*[@Name]')) { $controlName=$node.GetAttribute('Name'); $controls[$controlName]=$window.FindName($controlName) }
    $controls.ProfileBox.ItemsSource=$Profiles; $controls.ProfileBox.SelectedIndex=0
    $settingsPath=Join-Path $env:LOCALAPPDATA 'ExcelShrinkLab/options.json'
    $textFields=@('InputBox','OtherBox','PlanBox','BlockBox','MaxCellsBox','TileBox','MaxTilesBox','WorkersBox','MaxDiffBox','StallBox')
    $checkFields=@('ShapesCheck','HashCheck','ValuesCheck','ShotsCheck','MatrixCheck','TransparentCheck','VbaVisualCheck','RustVisualCheck','DiffCheck')
    if ([IO.File]::Exists($settingsPath)) {
        try {
            $saved=[IO.File]::ReadAllText($settingsPath) | ConvertFrom-Json
            foreach ($field in $textFields) { if ($saved.PSObject.Properties[$field]) { $controls[$field].Text=[string]$saved.$field } }
            foreach ($field in $checkFields) { if ($saved.PSObject.Properties[$field]) { $controls[$field].IsChecked=[bool]$saved.$field } }
            if ($saved.PSObject.Properties['profile'] -and $Profiles -contains [string]$saved.profile) { $controls.ProfileBox.SelectedItem=[string]$saved.profile }
        } catch { [Console]::Error.WriteLine('Ignoring invalid Excel Lab preferences: '+$_.Exception.Message) }
    }
    $state=@{result=$null}
    $pickFile={ param([string]$Field,[string]$Filter)
        $dialog=[Microsoft.Win32.OpenFileDialog]::new()
        $dialog.Filter=$Filter; $dialog.CheckFileExists=$true
        if ($dialog.ShowDialog($window)) { $controls[$Field].Text=$dialog.FileName }
    }.GetNewClosure()
    $controls.BrowseInput.Add_Click({ & $pickFile 'InputBox' 'Excel workbook (*.xlsx)|*.xlsx' }.GetNewClosure())
    $controls.BrowseOther.Add_Click({ & $pickFile 'OtherBox' 'Excel workbook (*.xlsx)|*.xlsx' }.GetNewClosure())
    $controls.BrowsePlan.Add_Click({ & $pickFile 'PlanBox' 'Excel Lab plan (*.json)|*.json' }.GetNewClosure())
    $controls.BrowseFolder.Add_Click({
        try {
            $handle=([Windows.Interop.WindowInteropHelper]::new($window)).Handle
            $path=[ExcelLab.FolderPicker]::Pick($handle)
            if ($path) { $controls.InputBox.Text=$path }
        } catch { [void][Windows.MessageBox]::Show($window,$_.Exception.Message,'Ordnerauswahl') }
    }.GetNewClosure())
    $controls.BuildButton.Add_Click({$state.result=@{action='build'}; $window.DialogResult=$true}.GetNewClosure())
    $controls.RunButton.Add_Click({
        try {
            if ([string]::IsNullOrWhiteSpace($controls.InputBox.Text)) { throw 'Bitte eine XLSX-Datei oder einen Ordner angeben.' }
            $mode=[string]$controls.ModeBox.SelectedItem.Tag
            if ($mode -eq 'compare' -and [string]::IsNullOrWhiteSpace($controls.OtherBox.Text)) { throw 'Bitte eine zweite XLSX-Datei angeben.' }
            $r=[ordered]@{action='measure';mode=$mode;input=$controls.InputBox.Text.Trim();other=$controls.OtherBox.Text.Trim();plan=$controls.PlanBox.Text.Trim();profile=[string]$controls.ProfileBox.SelectedItem;
                shapes=[bool]$controls.ShapesCheck.IsChecked;hash_values=[bool]$controls.HashCheck.IsChecked;compare_values=[bool]$controls.ValuesCheck.IsChecked;
                row_scope=[string]$controls.RowsBox.SelectedItem.Tag;screenshots=[bool]$controls.ShotsCheck.IsChecked;storage=[string]$controls.StorageBox.SelectedItem.Tag;
                copy_matrix=[bool]$controls.MatrixCheck.IsChecked;transparent_emf=[bool]$controls.TransparentCheck.IsChecked;
                compare_visual_vba=[bool]$controls.VbaVisualCheck.IsChecked;compare_visual_rust=[bool]$controls.RustVisualCheck.IsChecked;open_diff=[bool]$controls.DiffCheck.IsChecked}
            $numbers=@{block_cells=@('BlockBox',16384,1048576);max_cells=@('MaxCellsBox',1,2000000000);tile_points=@('TileBox',72,6000);max_tiles=@('MaxTilesBox',1,20000);workers=@('WorkersBox',1,32);max_differences=@('MaxDiffBox',0,100000);stall_seconds=@('StallBox',10,86400)}
            foreach ($key in $numbers.Keys) {
                $spec=$numbers[$key]; [long]$value=0
                if (-not [long]::TryParse($controls[$spec[0]].Text,[ref]$value) -or $value -lt $spec[1] -or $value -gt $spec[2]) { throw "Ungueltiger Wert fuer $key ($($spec[1])..$($spec[2]))." }
                $r[$key]=$value
            }
            $saved=[ordered]@{profile=$r.profile}
            foreach ($field in $textFields) { $saved[$field]=$controls[$field].Text }
            foreach ($field in $checkFields) { $saved[$field]=[bool]$controls[$field].IsChecked }
            [void][IO.Directory]::CreateDirectory((Split-Path $settingsPath -Parent))
            [IO.File]::WriteAllText($settingsPath,(ConvertTo-Json -InputObject $saved),[Text.UTF8Encoding]::new($false))
            $state.result=$r; $window.DialogResult=$true
        } catch { [void][Windows.MessageBox]::Show($window,$_.Exception.Message,'Excel Lab') }
    }.GetNewClosure())
    [void]$window.ShowDialog()
    return $state.result
}
