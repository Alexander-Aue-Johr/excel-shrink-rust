function New-CorpusLaunchDialog {
    param($Configuration, [string]$ConfigurationPath, [string]$Repo, [string]$Executable, [string[]]$ImportResults=@())
    Add-Type -AssemblyName PresentationFramework
    Add-Type -AssemblyName System.Windows.Forms
    [xml]$xaml=@'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
 Title="Excel Shrink · Corpus run" Width="940" Height="790" MinWidth="800" MinHeight="650" WindowStartupLocation="CenterScreen" Background="#F4F7FB">
 <Grid Margin="24"><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
  <StackPanel><TextBlock Text="Start or resume a corpus run" FontSize="25" FontWeight="SemiBold" Foreground="#14213D"/><TextBlock Text="Existing results are retained. This selection only determines work for this machine." Foreground="#52606D" Margin="0,7,0,16" TextWrapping="Wrap"/></StackPanel>
  <ScrollViewer Grid.Row="1" VerticalScrollBarVisibility="Auto"><StackPanel>
   <Border Background="White" CornerRadius="8" Padding="16"><StackPanel>
    <TextBlock Text="Corpus" FontWeight="SemiBold"/>
    <Grid Margin="0,8,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
     <ComboBox x:Name="Corpus" Height="32" VerticalContentAlignment="Center"/><Button x:Name="AddCorpus" Grid.Column="1" Content="Add corpus" Padding="12,5" Margin="10,0,0,0"/><Button x:Name="BindArchive" Grid.Column="2" Content="Link local ZIP" Padding="12,5" Margin="8,0,0,0"/>
    </Grid><TextBlock x:Name="ArchiveStatus" Margin="0,8,0,0" FontSize="12" Foreground="#52606D" TextWrapping="Wrap"/>
   </StackPanel></Border>
   <Border Background="White" CornerRadius="8" Padding="16" Margin="0,12,0,0"><StackPanel>
    <TextBlock Text="Partition by file size" FontWeight="SemiBold"/>
    <StackPanel Orientation="Horizontal" Margin="0,8,0,0"><TextBlock Text="Partitions" VerticalAlignment="Center" Margin="0,0,10,0"/><Button x:Name="FewerParts" Content="−" Width="30" Height="30"/><TextBox x:Name="Parts" Width="55" Height="30" TextAlignment="Center" VerticalContentAlignment="Center"/><Button x:Name="MoreParts" Content="+" Width="30" Height="30"/><Button x:Name="ApplyParts" Content="Partition" Padding="12,5" Margin="8,0,0,0"/></StackPanel>
    <WrapPanel x:Name="PartitionButtons" Margin="0,10,0,0"/>
    <TextBlock Text="Sorted by original file size, with approximately equal file counts per partition. Completed files do not change the boundaries." FontSize="12" Foreground="#52606D" Margin="0,5,0,0" TextWrapping="Wrap"/>
    <StackPanel Orientation="Horizontal" Margin="0,14,0,0"><TextBlock Text="Order within selection:" VerticalAlignment="Center" Margin="0,0,16,0"/><RadioButton x:Name="SmallFirst" Content="Smallest first" GroupName="Order" Margin="0,0,16,0" VerticalAlignment="Center"/><RadioButton x:Name="LargeFirst" Content="Largest first" GroupName="Order" VerticalAlignment="Center"/></StackPanel>
   </StackPanel></Border>
   <Border Background="White" CornerRadius="8" Padding="16" Margin="0,12,0,0"><Grid>
    <Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
    <StackPanel Margin="0,0,12,0"><TextBlock Text="Comparison groups" FontWeight="SemiBold"/><TextBox x:Name="Groups" Height="30" Margin="0,7,0,0" VerticalContentAlignment="Center" Padding="7,0"/><TextBlock Text="0 = start paused; increase later in the Observatory." FontSize="11" Foreground="#52606D" Margin="0,5,0,0"/></StackPanel>
    <StackPanel Grid.Column="1"><TextBlock Text="Machine name in report" FontWeight="SemiBold"/><TextBox x:Name="Machine" Height="30" Margin="0,7,0,0" VerticalContentAlignment="Center" Padding="7,0"/></StackPanel>
    <CheckBox x:Name="Hidden" Grid.Row="1" Content="Open Excel hidden" Margin="0,15,0,0"/>
    <CheckBox x:Name="Export" Grid.Row="1" Grid.Column="1" Content="Export a transferable results ZIP when finished" Margin="0,15,0,0"/>
    <TextBlock Grid.Row="2" Grid.ColumnSpan="2" Text="The run continues after VS Code closes. Use the Observatory to control it." FontSize="12" Foreground="#52606D" Margin="0,12,0,0" TextWrapping="Wrap"/>
   </Grid></Border>
   <Border Background="White" CornerRadius="8" Padding="16" Margin="0,12,0,0"><StackPanel>
    <TextBlock Text="Targeted captures for very tall sheets" FontWeight="SemiBold"/>
    <WrapPanel Margin="0,8,0,0">
     <TextBlock Text="Above" VerticalAlignment="Center"/><TextBox x:Name="TallThreshold" Width="65" Margin="8,0" Padding="5"/><TextBlock Text="capture rows, retain at each end" VerticalAlignment="Center"/>
     <TextBox x:Name="TallEdges" Width="65" Margin="8,0" Padding="5"/><TextBlock Text="capture rows" VerticalAlignment="Center"/>
    </WrapPanel>
    <TextBlock Text="Areas with change indicators are also captured. Omissions remain visible in the report. Changes apply to new groups." FontSize="12" Foreground="#52606D" Margin="0,8,0,0" TextWrapping="Wrap"/>
   </StackPanel></Border>
   <Expander Header="Advanced settings" Margin="0,14,0,0"><Grid Margin="0,10,0,0">
    <Grid.ColumnDefinitions><ColumnDefinition Width="210"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
    <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
    <TextBlock Text="Output directory" VerticalAlignment="Center"/><TextBox x:Name="Output" Grid.Column="1" Margin="0,3" Padding="5"/>
    <TextBlock Grid.Row="1" Text="Download directory" VerticalAlignment="Center"/><TextBox x:Name="Downloads" Grid.Row="1" Grid.Column="1" Margin="0,3" Padding="5"/>
    <TextBlock Grid.Row="2" Text="Observatory port" VerticalAlignment="Center"/><TextBox x:Name="Port" Grid.Row="2" Grid.Column="1" Margin="0,3" Padding="5"/>
    <TextBlock Grid.Row="3" Text="RAM estimates (MiB)" VerticalAlignment="Center"/><WrapPanel Grid.Row="3" Grid.Column="1"><TextBlock Text="Rust " VerticalAlignment="Center"/><TextBox x:Name="RustMemory" Width="65" Margin="0,3,10,3" Padding="5"/><TextBlock Text="Excel " VerticalAlignment="Center"/><TextBox x:Name="ExcelMemory" Width="65" Margin="0,3,10,3" Padding="5"/><TextBlock Text="Group " VerticalAlignment="Center"/><TextBox x:Name="GroupMemory" Width="65" Margin="0,3" Padding="5"/></WrapPanel>
    <TextBlock Grid.Row="4" Text="Reserved system RAM (MiB)" VerticalAlignment="Center"/><TextBox x:Name="Reserve" Grid.Row="4" Grid.Column="1" Margin="0,3" Padding="5" ToolTip="Minimum free physical RAM and commit headroom. Default: off (0); allowed: 0..65536 MiB."/>
    <TextBlock Grid.Row="5" Text="Timeouts (seconds)" VerticalAlignment="Center"/><WrapPanel Grid.Row="5" Grid.Column="1"><TextBlock Text="Open " VerticalAlignment="Center"/><TextBox x:Name="OpenTimeout" Width="70" Margin="0,3,14,3" Padding="5"/><TextBlock Text="Job " VerticalAlignment="Center"/><TextBox x:Name="JobTimeout" Width="70" Margin="0,3" Padding="5"/></WrapPanel>
    <CheckBox x:Name="Build" Grid.Row="6" Grid.ColumnSpan="2" Content="Update the release build before starting (after checking for active runs)" Margin="0,10,0,5"/>
    <TextBlock x:Name="Compatibility" Grid.Row="7" Grid.ColumnSpan="2" FontSize="11" Foreground="#52606D" Margin="0,4" TextWrapping="Wrap"/>
    <StackPanel Grid.Row="8" Grid.ColumnSpan="2" Orientation="Horizontal"><Button x:Name="Import" Content="Import results ZIPs from other machines" Padding="9,5"/><TextBlock x:Name="Imports" VerticalAlignment="Center" Margin="10,0,0,0" Text="None selected"/></StackPanel>
   </Grid></Expander>
  </StackPanel></ScrollViewer>
  <StackPanel Grid.Row="2" Margin="0,12,0,0"><TextBlock x:Name="RunStatus" Foreground="#52606D" TextWrapping="Wrap"/><TextBlock x:Name="Status" Foreground="#B32929" TextWrapping="Wrap"/></StackPanel>
  <StackPanel Grid.Row="3" Orientation="Horizontal" HorizontalAlignment="Right" Margin="0,14,0,0"><Button x:Name="StopRun" Content="Stop run" Padding="12,8" Margin="0,0,10,0" IsEnabled="False"/><Button x:Name="OpenObservatory" Content="Open Observatory" Padding="12,8" Margin="0,0,10,0"/><Button x:Name="Save" Content="Save only" Padding="12,8" Margin="0,0,10,0"/><Button x:Name="Cancel" Content="Close" Padding="12,8" Margin="0,0,10,0"/><Button x:Name="Start" Content="Start / resume run" Padding="16,8" Background="#14213D" Foreground="White" FontWeight="SemiBold"/></StackPanel>
 </Grid>
</Window>
'@
    $window=[Windows.Markup.XamlReader]::Load([Xml.XmlNodeReader]::new($xaml))
    $ui=@{}; foreach($name in @('Corpus','AddCorpus','BindArchive','ArchiveStatus','Parts','FewerParts','MoreParts','ApplyParts','PartitionButtons','SmallFirst','LargeFirst','Groups','Machine','Hidden','Export','TallThreshold','TallEdges','Output','Downloads','Port','RustMemory','ExcelMemory','GroupMemory','Reserve','OpenTimeout','JobTimeout','Build','Compatibility','Import','Imports','Status','RunStatus','StopRun','OpenObservatory','Save','Cancel','Start')){$ui[$name]=$window.FindName($name)}
    $state=@{ partition=[int]$Configuration.settings.partition; partitions=[int]$Configuration.settings.partitions; imports=@($ImportResults); result=$null; stopping=$null; stopOutput=$null }
    if($state.imports.Count){$ui.Imports.Text="$($state.imports.Count) selected"}
    $fields=@{ Groups='comparison_groups';Machine='machine_label';TallThreshold='tall_sheet_threshold';TallEdges='tall_sheet_edge_tiles';Output='output_root';Downloads='download_root';Port='control_room_port';RustMemory='worker_memory_mib';ExcelMemory='excel_memory_mib';GroupMemory='resident_memory_mib';Reserve='system_reserve_mib';OpenTimeout='open_timeout_seconds';JobTimeout='job_timeout_seconds';Parts='partitions' }
    foreach($name in $fields.Keys){$ui[$name].Text=[string]$Configuration.settings.($fields[$name])}
    $ui.Hidden.IsChecked=[bool]$Configuration.settings.hide_excel; $ui.Export.IsChecked=[bool]$Configuration.settings.export_results; $ui.Build.IsChecked=[bool]$Configuration.settings.build_release
    $ui.LargeFirst.IsChecked=$Configuration.settings.order -eq 'size-desc';$ui.SmallFirst.IsChecked=-not $ui.LargeFirst.IsChecked
    $ui.Compatibility.Text="$(@($Configuration.settings.reuse_prepared_from).Count) preparation builds already approved. A new build does not automatically approve additional builds."
    $showSource={
        if($ui.Corpus.SelectedItem){
            $entry=$ui.Corpus.SelectedItem.Tag; $path=Get-Value $entry 'local_archive'
            if($path -and [IO.File]::Exists((Resolve-StudyPath $Repo $path))){$ui.ArchiveStatus.Text="Reusing the existing ZIP in place: $path"}
            elseif(Get-Value $entry 'url'){$ui.ArchiveStatus.Text='The ZIP is downloaded if needed. Existing downloads are reused.'}
            else{$ui.ArchiveStatus.Text='No local ZIP on this machine. Link the existing corpus archive.'}
        }
    }.GetNewClosure()
    $refreshCorpus={
        $ui.Corpus.Items.Clear()
        foreach($entry in $Configuration.corpora){$item=[Windows.Controls.ComboBoxItem]::new();$item.Content=$entry.name;$item.Tag=$entry;[void]$ui.Corpus.Items.Add($item);if($entry.name -eq $Configuration.settings.corpus){$ui.Corpus.SelectedItem=$item}}
        if($ui.Corpus.SelectedIndex -lt 0){$ui.Corpus.SelectedIndex=0}
        & $showSource
    }.GetNewClosure()
    $renderParts={
        $count=0
        if(-not [int]::TryParse($ui.Parts.Text,[ref]$count) -or $count -lt 1 -or $count -gt 1000){throw 'The partition count must be between 1 and 1000.'}
        if($state.partition -gt $count){$state.partition=0}
        $state.partitions=$count
        $ui.PartitionButtons.Children.Clear()
        for($part=0;$part -le $count;$part++){
            $button=[Windows.Controls.RadioButton]::new();$button.GroupName='Partition';$button.Tag=[pscustomobject]@{Part=$part;State=$state};$button.Margin='0,0,12,8';$button.Padding='4';$button.IsChecked=$state.partition -eq $part
            $button.Content=if($part -eq 0){'Entire corpus'}else{'{0}/{1} · {2:0.#}–{3:0.#} %' -f $part,$count,(100.0*($part-1)/$count),(100.0*$part/$count)}
            $button.Add_Checked({param($sender,$eventArgs);$sender.Tag.State.partition=[int]$sender.Tag.Part});[void]$ui.PartitionButtons.Children.Add($button)
        }
    }.GetNewClosure()
    $readSettings={
        $count=0
        if(-not [int]::TryParse($ui.Parts.Text,[ref]$count) -or $count -ne $state.partitions){throw 'Apply the changed count with Partition first, then check the selected partition.'}
        foreach($name in $fields.Keys){$Configuration.settings.($fields[$name])=$ui[$name].Text}
        $Configuration.settings.partition=$state.partition;$Configuration.settings.corpus=$ui.Corpus.SelectedItem.Tag.name
        $Configuration.settings.order=if($ui.SmallFirst.IsChecked){'size-asc'}else{'size-desc'}
        $Configuration.settings.hide_excel=[bool]$ui.Hidden.IsChecked;$Configuration.settings.export_results=[bool]$ui.Export.IsChecked;$Configuration.settings.build_release=[bool]$ui.Build.IsChecked
        Assert-StudySettings $Configuration.settings
    }.GetNewClosure()
    $ui.Corpus.Add_SelectionChanged($showSource)
    $ui.ApplyParts.Add_Click({try{& $renderParts;$ui.Status.Text=''}catch{$ui.Status.Text=$_.Exception.Message}}.GetNewClosure())
    $ui.FewerParts.Add_Click({$n=0;if([int]::TryParse($ui.Parts.Text,[ref]$n)){$ui.Parts.Text=[string][Math]::Max(1,$n-1);& $renderParts}}.GetNewClosure())
    $ui.MoreParts.Add_Click({$n=0;if([int]::TryParse($ui.Parts.Text,[ref]$n)){$ui.Parts.Text=[string][Math]::Min(1000,$n+1);& $renderParts}}.GetNewClosure())
    $ui.AddCorpus.Add_Click({
        try {
            $added=Show-NewCorpusDialog $window $Configuration $Repo
            if($added){$Configuration.settings.corpus=$added.name;& $refreshCorpus;Save-StudyJson $ConfigurationPath $Configuration;$ui.Status.Text='Corpus saved in the local catalog.'}
        }catch{$ui.Status.Text=$_.Exception.Message}
    }.GetNewClosure())
    $ui.BindArchive.Add_Click({
        try{
            $dialog=[Microsoft.Win32.OpenFileDialog]::new();$dialog.Filter='ZIP archives (*.zip)|*.zip';$dialog.Title='Link the same unmodified corpus archive'
            if($dialog.ShowDialog($window)){
                $entry=$ui.Corpus.SelectedItem.Tag
                Set-StudyArchiveBinding $entry $dialog.FileName $Repo $ui.Output.Text
                Save-StudyJson $ConfigurationPath $Configuration;& $showSource;$ui.Status.Text='ZIP linked. Files and existing results remain in place.'
            }
        }catch{$ui.Status.Text=$_.Exception.Message}
    }.GetNewClosure())
    $ui.Import.Add_Click({$dialog=[Microsoft.Win32.OpenFileDialog]::new();$dialog.Filter='Results ZIPs (*.zip)|*.zip';$dialog.Multiselect=$true;if($dialog.ShowDialog($window)){$state.imports=@($dialog.FileNames);$ui.Imports.Text="$($state.imports.Count) selected"}}.GetNewClosure())
    $ui.OpenObservatory.Add_Click({$port=0;if([int]::TryParse($ui.Port.Text,[ref]$port) -and $port -ge 1024 -and $port -le 65535){Start-Process "http://127.0.0.1:$port/"}}.GetNewClosure())
    $ui.StopRun.Add_Click({
        try {
            $output=Resolve-StudyPath $Repo $ui.Output.Text
            $state.stopping=Request-StudyRunStop $output ([int]$ui.Port.Text)
            $state.stopOutput=$output
            $ui.StopRun.IsEnabled=$false
            $ui.Status.Foreground='#35755A'
            $ui.Status.Text='Stopping: finishing active workbooks and Excel cleanup. Wait before pulling or rebuilding.'
        } catch {$ui.Status.Foreground='#B32929';$ui.Status.Text=$_.Exception.Message}
    }.GetNewClosure())
    $runTimer=[Windows.Threading.DispatcherTimer]::new()
    $runTimer.Interval=[TimeSpan]::FromSeconds(1)
    $runTimer.Add_Tick({
        try {
            $output=if($state.stopOutput){$state.stopOutput}else{Resolve-StudyPath $Repo $ui.Output.Text}
            if($state.stopping){Complete-StudyLegacyStop $output $state.stopping}
            $current=Get-StudyRunStatus $output
            $ui.StopRun.IsEnabled=$current.Busy -and $null -ne $current.Process -and $null -eq $state.stopping -and -not $current.Stopping
            $ui.Start.IsEnabled=$current.Ended -and $null -eq $state.stopping
            $ui.RunStatus.Text=if($current.Ended){'Runner stopped. Ready to pull, rebuild or resume.'}elseif($state.stopping -or $current.Stopping){'Stopping: finishing admitted workbooks and Excel cleanup. Wait before updating.'}elseif($current.Process){"Runner active (PID $($current.Process.Id)). Stop it here before updating."}else{'Output is busy; runner identity could not be verified.'}
            if($state.stopping -and $current.Ended){
                $state.stopping=$null;$state.stopOutput=$null
                $ui.Status.Foreground='#35755A'
                $ui.Status.Text='Runner stopped. Results retained. You can now pull and rebuild, then resume this run.'
                $ui.Start.Content='Start / resume run';$ui.Start.IsEnabled=$true
            }
        } catch {$ui.Status.Foreground='#B32929';$ui.Status.Text=$_.Exception.Message}
    }.GetNewClosure())
    $window.Add_Closed({$runTimer.Stop()}.GetNewClosure())
    $window.Add_Closing({param($sender,$eventArgs)
        if($state.stopping -and -not (Get-Value $state.stopping.Queue 'stop_supported' $false)){
            $eventArgs.Cancel=$true
            $ui.Status.Text='Keep this window open while the older runner finishes its active workbooks. It will be stopped automatically once drained.'
        }
    }.GetNewClosure())
    $runTimer.Start()
    $ui.Cancel.Add_Click({$window.Close()}.GetNewClosure())
    $ui.Save.Add_Click({try{& $readSettings;Save-StudyJson $ConfigurationPath $Configuration;$ui.Status.Foreground='#35755A';$ui.Status.Text='Configuration saved. No run started.'}catch{$ui.Status.Foreground='#B32929';$ui.Status.Text=$_.Exception.Message}}.GetNewClosure())
    $ui.Start.Add_Click({
        $ui.Start.IsEnabled=$false
        try{
            & $readSettings
            $stamp=(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8)
            $runDirectory=Join-Path ([IO.Path]::GetDirectoryName($ConfigurationPath)) ('runs/'+$stamp)
            $plan=New-StudyLaunchPlan $Configuration $Repo $Executable $runDirectory $state.imports
            $state.result=Start-StudyLaunch $Configuration $ConfigurationPath $Repo $plan
            $ui.Status.Foreground='#35755A';$ui.Status.Text="Process started (PID $($state.result.Pid)). Preparation appears in the Observatory once its server is ready. You can close this window and VS Code. Logs: $runDirectory"
            $ui.Start.Content='Run already started'
        }catch{$ui.Status.Foreground='#B32929';$ui.Status.Text=$_.Exception.Message;$ui.Start.IsEnabled=$true}
    }.GetNewClosure())
    & $refreshCorpus; & $renderParts
    return [pscustomobject]@{ Window=$window; Controls=$ui; State=$state }
}

function Show-CorpusLaunchDialog {
    param($Configuration, [string]$ConfigurationPath, [string]$Repo, [string]$Executable, [string[]]$ImportResults=@())
    $dialog=New-CorpusLaunchDialog $Configuration $ConfigurationPath $Repo $Executable $ImportResults
    [void]$dialog.Window.ShowDialog()
    return $dialog.State.result
}

function Show-NewCorpusDialog($Owner,$Configuration,[string]$Repo) {
    [xml]$xaml=@'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Title="Add corpus" Width="670" Height="310" ResizeMode="NoResize" WindowStartupLocation="CenterOwner" Background="#F4F7FB">
 <StackPanel Margin="24"><TextBlock Text="Name"/><TextBox x:Name="Name" Height="30" Margin="0,6,0,12" Padding="6"/><TextBlock Text="ZIP download URL or existing local ZIP file"/><DockPanel Margin="0,6,0,0"><Button x:Name="Browse" DockPanel.Dock="Right" Content="Choose ZIP" Padding="12,5" Margin="8,0,0,0"/><TextBox x:Name="Source" Height="30" Padding="6"/></DockPanel><TextBlock x:Name="Status" Foreground="#B32929" Margin="0,10,0,0" TextWrapping="Wrap"/><Button x:Name="Save" Content="Add" HorizontalAlignment="Right" Padding="14,7" Margin="0,12,0,0"/></StackPanel>
</Window>
'@
    $window=[Windows.Markup.XamlReader]::Load([Xml.XmlNodeReader]::new($xaml));$window.Owner=$Owner
    $name=$window.FindName('Name');$source=$window.FindName('Source');$status=$window.FindName('Status');$result=@{value=$null}
    $window.FindName('Browse').Add_Click({$dialog=[Microsoft.Win32.OpenFileDialog]::new();$dialog.Filter='ZIP archives (*.zip)|*.zip';if($dialog.ShowDialog($window)){$source.Text=$dialog.FileName}}.GetNewClosure())
    $window.FindName('Save').Add_Click({try{$result.value=Add-StudyCorpus $Configuration $name.Text $source.Text $Repo;$window.Close()}catch{$status.Text=$_.Exception.Message}}.GetNewClosure())
    [void]$window.ShowDialog();return $result.value
}
