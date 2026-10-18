param([string]$CorpusRoot,[string]$Workbook)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName PresentationFramework,PresentationCore,WindowsBase
. (Join-Path $PSScriptRoot '_shared\reporting.ps1')
. (Join-Path $PSScriptRoot '_shared\suite\controller.ps1')
$repo=Get-VVRepo $PSScriptRoot
if(-not $CorpusRoot){$CorpusRoot=Join-Path $repo 'corpus-local'}
$state=Join-Path $repo '.tmp\visual-validation\experiment-last-request.json'
$xaml=@'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Title="Visual validation" Width="920" Height="890" MinWidth="760" MinHeight="620" WindowStartupLocation="CenterScreen" Background="#F3F6F9">
 <Window.Resources><Style TargetType="TextBox"><Setter Property="Padding" Value="7"/><Setter Property="Margin" Value="0,3,12,8"/></Style><Style TargetType="ComboBox"><Setter Property="Padding" Value="7"/><Setter Property="Margin" Value="0,3,12,8"/></Style><Style TargetType="Button"><Setter Property="Padding" Value="12,7"/><Setter Property="Margin" Value="0,0,8,0"/></Style></Window.Resources>
 <DockPanel>
  <Border DockPanel.Dock="Top" Background="#142D44" Padding="22"><StackPanel><TextBlock Text="Excel Shrink · Visual validation" FontSize="23" FontWeight="SemiBold" Foreground="White"/><TextBlock Text="Capture methods, geometry and reproducible comparisons" Foreground="#CAD8E5" Margin="0,5,0,0"/></StackPanel></Border>
  <Border DockPanel.Dock="Bottom" Background="White" Padding="18"><DockPanel><StackPanel DockPanel.Dock="Right" Orientation="Horizontal"><Button x:Name="Cancel" Content="Close"/><Button x:Name="Run" Content="Run experiment" IsDefault="True"/></StackPanel><TextBlock x:Name="Status" TextWrapping="Wrap" VerticalAlignment="Center"/></DockPanel></Border>
  <ScrollViewer VerticalScrollBarVisibility="Auto"><StackPanel Margin="22">
   <WrapPanel Margin="0,0,0,16"><Button x:Name="Suite" Content="Reproduce capture decision"/><Button x:Name="Reports" Content="Reports"/><Button x:Name="Evidence" Content="Published results"/><Button x:Name="Prune" Content="Prune history"/></WrapPanel>
   <TextBlock Text="The suite uses the declared comparison blocks. Individual experiments below do not start additional comparison groups." TextWrapping="Wrap" Foreground="#536A7F" Margin="0,0,0,16"/>
   <Grid><Grid.ColumnDefinitions><ColumnDefinition/><ColumnDefinition/></Grid.ColumnDefinitions><StackPanel><TextBlock Text="Experiment"/><ComboBox x:Name="Experiment"/></StackPanel><StackPanel Grid.Column="1"><TextBlock Text="Drawing implementation"/><ComboBox x:Name="Drawing"/></StackPanel></Grid>
   <TextBlock Text="Corpus directory"/><TextBox x:Name="Corpus"/>
   <TextBlock Text="Workbook · exact filename, corpus-relative path, or all"/><DockPanel><Button x:Name="Browse" Content="Browse…" DockPanel.Dock="Right"/><TextBox x:Name="Workbook" Text="all"/></DockPanel>
   <StackPanel x:Name="SheetControls"><WrapPanel Margin="0,2,0,6"><Button x:Name="LoadSheets" Content="Load worksheets"/><CheckBox x:Name="AllSheets" Content="All visible worksheets" IsChecked="True" VerticalAlignment="Center"/></WrapPanel><ListBox x:Name="Sheets" Height="145" SelectionMode="Extended" Margin="0,0,12,12" IsEnabled="False"/></StackPanel>
   <Grid x:Name="CaptureControls"><Grid.ColumnDefinitions><ColumnDefinition/><ColumnDefinition/><ColumnDefinition/><ColumnDefinition/></Grid.ColumnDefinitions><StackPanel><TextBlock Text="Bounds strategy"/><ComboBox x:Name="Strategy"/></StackPanel><StackPanel Grid.Column="1"><TextBlock Text="Host"/><ComboBox x:Name="Host"/></StackPanel><StackPanel Grid.Column="2"><TextBlock Text="Overflow verification"/><ComboBox x:Name="Overflow"/></StackPanel><StackPanel Grid.Column="3"><TextBlock Text="Raw image budget"/><ComboBox x:Name="Pixels"/></StackPanel></Grid>
   <Grid><Grid.ColumnDefinitions><ColumnDefinition/><ColumnDefinition/><ColumnDefinition/></Grid.ColumnDefinitions><StackPanel><TextBlock Text="Startup limit (seconds)"/><TextBox x:Name="Startup" Text="300"/></StackPanel><StackPanel Grid.Column="1"><TextBlock Text="Capture limit (seconds)"/><TextBox x:Name="Capture" Text="1200"/></StackPanel><StackPanel Grid.Column="2"><TextBlock Text="Session limit (seconds)"/><TextBox x:Name="Session" Text="21600"/></StackPanel></Grid>
   <Expander Header="Advanced settings" Margin="0,12,0,12"><StackPanel Margin="0,10,0,0"><Grid><Grid.ColumnDefinitions><ColumnDefinition/><ColumnDefinition/><ColumnDefinition/></Grid.ColumnDefinitions><StackPanel><TextBlock Text="Setup limit"/><TextBox x:Name="Setup" Text="600"/><TextBlock Text="Native call limit"/><TextBox x:Name="Native" Text="90"/><TextBlock Text="Batch size"/><ComboBox x:Name="Batch"/></StackPanel><StackPanel Grid.Column="1"><TextBlock Text="Inactivity limit"/><TextBox x:Name="Inactivity" Text="120"/><TextBlock Text="Postprocessing limit"/><TextBox x:Name="Post" Text="600"/><TextBlock Text="Tile packing"/><ComboBox x:Name="Packing"/></StackPanel><StackPanel Grid.Column="2"><TextBlock Text="Repetitions"/><TextBox x:Name="Repeats" Text="1"/><TextBlock Text="Right probe width (pixels)"/><TextBox x:Name="Probe" Text="1920"/><TextBlock Text="Rust Excel startup"/><ComboBox x:Name="Launch"/></StackPanel></Grid><WrapPanel><CheckBox x:Name="Context" Content="Local whole-versus-tiles validation" Margin="0,5,18,5"/><CheckBox x:Name="Detailed" Content="Detailed diagnostics" Margin="0,5,18,5"/><CheckBox x:Name="Wait" Content="Wait for taskpane Start" Margin="0,5,18,5"/><CheckBox x:Name="Debug" Content="office.debug.js" Margin="0,5,18,5"/></WrapPanel></StackPanel></Expander>
   <TextBlock x:Name="Description" Foreground="#536A7F" TextWrapping="Wrap"/>
  </StackPanel></ScrollViewer>
 </DockPanel>
</Window>
'@
$reader=[Xml.XmlNodeReader]::new([xml]$xaml);try{$window=[Windows.Markup.XamlReader]::Load($reader)}finally{$reader.Close()}
$ui=@{};foreach($name in @('Suite','Reports','Evidence','Prune','Experiment','Drawing','Corpus','Browse','Workbook','SheetControls','Sheets','AllSheets','LoadSheets','CaptureControls','Strategy','Host','Overflow','Pixels','Startup','Capture','Session','Setup','Native','Inactivity','Post','Repeats','Probe','Batch','Packing','Launch','Context','Detailed','Wait','Debug','Description','Run','Cancel','Status')){$ui[$name]=$window.FindName($name)}
function Choices($box,$items,$selected){$box.ItemsSource=@($items|ForEach-Object{[pscustomobject]@{Value=$_[0];Label=$_[1]}});$box.DisplayMemberPath='Label';$box.SelectedValuePath='Value';$box.SelectedValue=$selected;if($null -eq $box.SelectedItem){$box.SelectedIndex=0}}
Choices $ui.Experiment @(@('range-image','Office.js capture'),@('vba-picture','VBA CopyPicture / chart'),@('shape-bounds','Drawing bounds'),@('units','Renderer units'),@('markers','Renderer markers'),@('row-heights','Row-height geometry'),@('height-plateau','Height plateau'),@('native-bitmap','VBA-local GDI+ bitmap'),@('native-emf','VBA-local GDI+ EMF'),@('native-memory','VBA pair / memory'),@('native-png-memory','VBA pair / PNG + memory'),@('native-rows','VBA row heights')) 'range-image'
Choices $ui.Drawing @(@('native','VBA / Excel-DNA / COM group'),@('vsto','VSTO'),@('officejs-dotnet','Office.js / .NET'),@('officejs-rust','Office.js / Rust')) 'native'
Choices $ui.Strategy @(@('F','F · Rust plan'),@('A','A · UsedRange prefetch'),@('B','B · UsedRange per batch'),@('C','C · Content + drawings'),@('D','D · Horizontal XOR'),@('E','E · Horizontal + vertical XOR')) 'F'
Choices $ui.Host @(@('dotnet','.NET'),@('rust','Rust')) 'dotnet';Choices $ui.Overflow @(@('selective','Selective'),@('heuristic','Heuristic')) 'selective'
Choices $ui.Pixels @(@('16','16 MP'),@('32','32 MP'),@('64','64 MP')) '32';Choices $ui.Batch @(@('1','1'),@('2','2'),@('4','4'),@('8','8'),@('16','16'),@('32','32'),@('64','64')) '1'
Choices $ui.Packing @(@('budget','Budget-aware'),@('balanced','Balanced subdivision')) 'budget';Choices $ui.Launch @(@('excel-x','Excel /x'),@('rust-com','Rust COM')) 'excel-x'
$ui.Corpus.Text=$CorpusRoot;if($Workbook){$ui.Workbook.Text=$Workbook};$script:action=$null
function Read-Number($c,[int]$min,[int]$max){$v=0;if(-not[int]::TryParse($c.Text,[ref]$v) -or $v -lt $min -or $v -gt $max){throw "Expected $min to $max"};return $v}
function Update-Experiment {
 $k=[string]$ui.Experiment.SelectedValue;$ui.Drawing.IsEnabled=($k -eq 'shape-bounds');$ui.CaptureControls.IsEnabled=($k -eq 'range-image')
 $ui.Workbook.IsEnabled=($k -notin @('units','markers','row-heights','height-plateau'))
 $ui.SheetControls.Visibility=if($k -eq 'range-image' -or ($k -like 'native-*' -and $k -ne 'native-rows')){'Visible'}else{'Collapsed'}
 $ui.Description.Text=if($k -eq 'range-image'){'Selected method: F / .NET / Selective / 32 MP. Safety limits and internal visible context remain active. Native image calls are serialized.'}elseif($k -eq 'shape-bounds'){'Drawing geometry is measured separately from screenshot acquisition. Missing native prerequisites remain visible.'}elseif($k -like 'native-*' -or $k -eq 'vba-picture'){'Clipboard-based reference: do not use the system clipboard concurrently. Its raster policy and coverage are not assumed equivalent to Office.js.'}else{'Uses the declared synthetic fixtures or fixed reference input. Diagnostic completion is not a performance or production certificate.'}
}
$ui.Experiment.Add_SelectionChanged({Update-Experiment});$ui.AllSheets.Add_Click({$ui.Sheets.IsEnabled=-not[bool]$ui.AllSheets.IsChecked})
$ui.Workbook.Add_TextChanged({$ui.Sheets.ItemsSource=$null;$ui.AllSheets.IsChecked=$true;$ui.Sheets.IsEnabled=$false})
$ui.Browse.Add_Click({$dialog=[Microsoft.Win32.OpenFileDialog]::new();$dialog.Filter='Excel workbooks (*.xlsx)|*.xlsx';if($dialog.ShowDialog()){$ui.Workbook.Text=$dialog.FileName}})
$ui.LoadSheets.Add_Click({try{
 if($ui.Workbook.Text -in @('all','*')){throw 'Choose one workbook for sheet selection.'}
 $temp=Join-Path $repo ('.tmp\visual-validation\catalogue-'+[Guid]::NewGuid().ToString('N'));[IO.Directory]::CreateDirectory($temp)|Out-Null
 $req=Join-Path $temp 'request.json';$out=Join-Path $temp 'sheets.json';Write-VVJson $req @{schema='vv-experiment-request-v1';mode='single';corpus_root=$ui.Corpus.Text;workbook=$ui.Workbook.Text}
 $code=Invoke-VVEvaluationPython $repo @('--request',$req,'--list-sheets','--write-resolved',$out);if($code){throw 'Cannot read catalogue; see terminal.'}
 $ui.Sheets.ItemsSource=@((Read-VVJson $out)|Where-Object{$_.state -eq 'visible'}|ForEach-Object{[pscustomobject]@{Name=$_.name;Label=('{0} · {1}' -f $_.index,($_.name|ConvertTo-Json -Compress))}});$ui.Sheets.DisplayMemberPath='Label';$ui.Status.Text='Exact worksheet names loaded; Ctrl/Shift selects multiple entries.'
}catch{$ui.Status.Text=$_.Exception.Message}})
$ui.Reports.Add_Click({try{& (Join-Path $PSScriptRoot 'reports.ps1') -Action Open}catch{$ui.Status.Text=$_.Exception.Message}})
$ui.Evidence.Add_Click({try{Invoke-Item -LiteralPath (Join-Path $repo 'docs\visual-validation\evidence\index.html')}catch{$ui.Status.Text=$_.Exception.Message}})
$ui.Prune.Add_Click({try{& (Join-Path $PSScriptRoot 'reports.ps1') -Action Clean -KeepLast 10 -ConfirmInGui}catch{$ui.Status.Text=$_.Exception.Message}})
$ui.Suite.Add_Click({$script:action='suite';$window.DialogResult=$true;$window.Close()});$ui.Cancel.Add_Click({$window.Close()})
$ui.Run.Add_Click({try{
 $k=[string]$ui.Experiment.SelectedValue
 $settings=@{strategy=[string]$ui.Strategy.SelectedValue;host_kind=[string]$ui.Host.SelectedValue;overflow_mode=[string]$ui.Overflow.SelectedValue;tile_megapixels=[int]$ui.Pixels.SelectedValue;batch_size=[int]$ui.Batch.SelectedValue;tile_packing=[string]$ui.Packing.SelectedValue;excel_launch_mode=$(if($ui.Host.SelectedValue -eq 'rust'){[string]$ui.Launch.SelectedValue}else{'excel-x'});context_validation=$(if($ui.Context.IsChecked){'first-seam'}else{'off'});diagnostic_level=$(if($ui.Detailed.IsChecked){'detailed'}else{'normal'});wait_for_start=[bool]$ui.Wait.IsChecked;use_office_debug_js=[bool]$ui.Debug.IsChecked;overflow_probe_width_px=(Read-Number $ui.Probe 64 8192)}
 $lim=@{setup_timeout_seconds=(Read-Number $ui.Setup 30 1800);startup_timeout_seconds=(Read-Number $ui.Startup 10 1800);capture_timeout_seconds=(Read-Number $ui.Capture 30 7200);native_call_timeout_seconds=(Read-Number $ui.Native 10 600);inactivity_timeout_seconds=(Read-Number $ui.Inactivity 10 1800);postprocess_timeout_seconds=(Read-Number $ui.Post 30 1800);session_timeout_seconds=(Read-Number $ui.Session 60 86400)}
 $request=@{schema='vv-experiment-request-v1';mode='single';experiment=$k;corpus_root=$ui.Corpus.Text;settings=$settings;limits=$lim;repetitions=(Read-Number $ui.Repeats 1 5)}
 if($k -notin @('units','markers','row-heights','height-plateau')){$request.workbook=$ui.Workbook.Text}
 if($k -eq 'shape-bounds'){$request.implementation=[string]$ui.Drawing.SelectedValue}
 if($ui.SheetControls.Visibility -eq 'Visible' -and -not $ui.AllSheets.IsChecked){$names=@($ui.Sheets.SelectedItems|ForEach-Object{[string]$_.Name});if(-not $names.Count){throw 'Select worksheets or enable All visible worksheets.'};$request.sheets=$names}
 Write-VVJson $state $request;$script:action='single';$window.DialogResult=$true;$window.Close()
}catch{$ui.Status.Text=$_.Exception.Message}})
try{
 $saved=Read-VVJson $state
 if($saved){
  if($saved.experiment){$ui.Experiment.SelectedValue=$saved.experiment}
  if($saved.workbook -and -not $Workbook){$ui.Workbook.Text=$saved.workbook}
  foreach($p in @(@('Strategy','strategy'),@('Host','host_kind'),@('Overflow','overflow_mode'),@('Pixels','tile_megapixels'),@('Batch','batch_size'),@('Packing','tile_packing'),@('Launch','excel_launch_mode'))){if($null -ne $saved.settings.($p[1])){$ui[$p[0]].SelectedValue=[string]$saved.settings.($p[1])}}
  foreach($p in @(@('Startup','startup_timeout_seconds'),@('Capture','capture_timeout_seconds'),@('Session','session_timeout_seconds'),@('Setup','setup_timeout_seconds'),@('Native','native_call_timeout_seconds'),@('Inactivity','inactivity_timeout_seconds'),@('Post','postprocess_timeout_seconds'))){if($null -ne $saved.limits.($p[1])){$ui[$p[0]].Text=[string]$saved.limits.($p[1])}}
 }
}catch{Write-Warning ('Saved settings: '+$_.Exception.Message)}
Update-Experiment;[void]$window.ShowDialog()
if($script:action -eq 'suite'){& (Join-Path $PSScriptRoot 'reproduce.ps1') -CorpusRoot $ui.Corpus.Text}
elseif($script:action -eq 'single'){& (Join-Path $PSScriptRoot 'reproduce.ps1') -Single -SelectionFile $state}
