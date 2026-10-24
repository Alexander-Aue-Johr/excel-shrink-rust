function New-CorpusLaunchDialog {
    param($Configuration, [string]$ConfigurationPath, [string]$Repo, [string]$Executable, [string[]]$ImportResults=@())
    Add-Type -AssemblyName PresentationFramework
    Add-Type -AssemblyName System.Windows.Forms
    [xml]$xaml=@'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
 Title="Excel Shrink · Korpuslauf" Width="940" Height="790" MinWidth="800" MinHeight="650" WindowStartupLocation="CenterScreen" Background="#F4F7FB">
 <Grid Margin="24"><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
  <StackPanel><TextBlock Text="Korpuslauf starten oder fortsetzen" FontSize="25" FontWeight="SemiBold" Foreground="#14213D"/><TextBlock Text="Vorhandene Ergebnisse bleiben erhalten. Die Auswahl bestimmt nur die Arbeit dieses Rechners." Foreground="#52606D" Margin="0,7,0,16" TextWrapping="Wrap"/></StackPanel>
  <ScrollViewer Grid.Row="1" VerticalScrollBarVisibility="Auto"><StackPanel>
   <Border Background="White" CornerRadius="8" Padding="16"><StackPanel>
    <TextBlock Text="Korpus" FontWeight="SemiBold"/>
    <Grid Margin="0,8,0,0"><Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
     <ComboBox x:Name="Corpus" Height="32" VerticalContentAlignment="Center"/><Button x:Name="AddCorpus" Grid.Column="1" Content="Korpus hinzufügen" Padding="12,5" Margin="10,0,0,0"/><Button x:Name="BindArchive" Grid.Column="2" Content="Lokales ZIP zuordnen" Padding="12,5" Margin="8,0,0,0"/>
    </Grid><TextBlock x:Name="ArchiveStatus" Margin="0,8,0,0" FontSize="12" Foreground="#52606D" TextWrapping="Wrap"/>
   </StackPanel></Border>
   <Border Background="White" CornerRadius="8" Padding="16" Margin="0,12,0,0"><StackPanel>
    <TextBlock Text="Aufteilung nach Dateigröße" FontWeight="SemiBold"/>
    <StackPanel Orientation="Horizontal" Margin="0,8,0,0"><TextBlock Text="Partitionen" VerticalAlignment="Center" Margin="0,0,10,0"/><Button x:Name="FewerParts" Content="−" Width="30" Height="30"/><TextBox x:Name="Parts" Width="55" Height="30" TextAlignment="Center" VerticalContentAlignment="Center"/><Button x:Name="MoreParts" Content="+" Width="30" Height="30"/><Button x:Name="ApplyParts" Content="Aufteilen" Padding="12,5" Margin="8,0,0,0"/></StackPanel>
    <WrapPanel x:Name="PartitionButtons" Margin="0,10,0,0"/>
    <TextBlock Text="Aufsteigend nach ursprünglicher Dateigröße; möglichst gleich viele Dateien pro Teil. Bereits erledigte Dateien ändern die Grenzen nicht." FontSize="12" Foreground="#52606D" Margin="0,5,0,0" TextWrapping="Wrap"/>
    <StackPanel Orientation="Horizontal" Margin="0,14,0,0"><TextBlock Text="Reihenfolge innerhalb der Auswahl:" VerticalAlignment="Center" Margin="0,0,16,0"/><RadioButton x:Name="SmallFirst" Content="Kleinste zuerst" GroupName="Order" Margin="0,0,16,0" VerticalAlignment="Center"/><RadioButton x:Name="LargeFirst" Content="Größte zuerst" GroupName="Order" VerticalAlignment="Center"/></StackPanel>
   </StackPanel></Border>
   <Border Background="White" CornerRadius="8" Padding="16" Margin="0,12,0,0"><Grid>
    <Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions><Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
    <StackPanel Margin="0,0,12,0"><TextBlock Text="Vergleichsgruppen" FontWeight="SemiBold"/><TextBox x:Name="Groups" Height="30" Margin="0,7,0,0" VerticalContentAlignment="Center" Padding="7,0"/><TextBlock Text="0 = pausiert starten; später im Observatory erhöhen." FontSize="11" Foreground="#52606D" Margin="0,5,0,0"/></StackPanel>
    <StackPanel Grid.Column="1"><TextBlock Text="Rechnername im Bericht" FontWeight="SemiBold"/><TextBox x:Name="Machine" Height="30" Margin="0,7,0,0" VerticalContentAlignment="Center" Padding="7,0"/></StackPanel>
    <CheckBox x:Name="Hidden" Grid.Row="1" Content="Excel verborgen öffnen" Margin="0,15,0,0"/>
    <CheckBox x:Name="Export" Grid.Row="1" Grid.Column="1" Content="Nach Abschluss Ergebnis-ZIP zum Übertragen erstellen" Margin="0,15,0,0"/>
    <TextBlock Grid.Row="2" Grid.ColumnSpan="2" Text="Der Lauf bleibt nach dem Schließen von VS Code aktiv. Die Steuerung erfolgt im Observatory." FontSize="12" Foreground="#52606D" Margin="0,12,0,0" TextWrapping="Wrap"/>
   </Grid></Border>
   <Border Background="White" CornerRadius="8" Padding="16" Margin="0,12,0,0"><StackPanel>
    <TextBlock Text="Sehr hohe Sheets gezielt prüfen" FontWeight="SemiBold"/>
    <WrapPanel Margin="0,8,0,0">
     <TextBlock Text="Ab mehr als" VerticalAlignment="Center"/><TextBox x:Name="TallThreshold" Width="65" Margin="8,0" Padding="5"/><TextBlock Text="Capture-Zeilen kürzen · oben und unten jeweils" VerticalAlignment="Center"/>
     <TextBox x:Name="TallEdges" Width="65" Margin="8,0" Padding="5"/><TextBlock Text="Capture-Zeilen behalten" VerticalAlignment="Center"/>
    </WrapPanel>
    <TextBlock Text="Zusätzlich werden Bereiche mit Änderungsindikatoren erfasst. Auslassungen bleiben im Bericht sichtbar. Änderungen gelten für neue Gruppen." FontSize="12" Foreground="#52606D" Margin="0,8,0,0" TextWrapping="Wrap"/>
   </StackPanel></Border>
   <Expander Header="Erweiterte Einstellungen" Margin="0,14,0,0"><Grid Margin="0,10,0,0">
    <Grid.ColumnDefinitions><ColumnDefinition Width="210"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
    <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/><RowDefinition Height="Auto"/></Grid.RowDefinitions>
    <TextBlock Text="Ergebnisverzeichnis" VerticalAlignment="Center"/><TextBox x:Name="Output" Grid.Column="1" Margin="0,3" Padding="5"/>
    <TextBlock Grid.Row="1" Text="Downloadverzeichnis" VerticalAlignment="Center"/><TextBox x:Name="Downloads" Grid.Row="1" Grid.Column="1" Margin="0,3" Padding="5"/>
    <TextBlock Grid.Row="2" Text="Observatory-Port" VerticalAlignment="Center"/><TextBox x:Name="Port" Grid.Row="2" Grid.Column="1" Margin="0,3" Padding="5"/>
    <TextBlock Grid.Row="3" Text="RAM-Limits (MiB)" VerticalAlignment="Center"/><WrapPanel Grid.Row="3" Grid.Column="1"><TextBlock Text="Rust " VerticalAlignment="Center"/><TextBox x:Name="RustMemory" Width="65" Margin="0,3,10,3" Padding="5"/><TextBlock Text="Excel " VerticalAlignment="Center"/><TextBox x:Name="ExcelMemory" Width="65" Margin="0,3,10,3" Padding="5"/><TextBlock Text="Gruppe " VerticalAlignment="Center"/><TextBox x:Name="GroupMemory" Width="65" Margin="0,3" Padding="5"/></WrapPanel>
    <TextBlock Grid.Row="4" Text="Freizuhaltender RAM (MiB)" VerticalAlignment="Center"/><TextBox x:Name="Reserve" Grid.Row="4" Grid.Column="1" Margin="0,3" Padding="5"/>
    <TextBlock Grid.Row="5" Text="Zeitlimits (Sekunden)" VerticalAlignment="Center"/><WrapPanel Grid.Row="5" Grid.Column="1"><TextBlock Text="Öffnen " VerticalAlignment="Center"/><TextBox x:Name="OpenTimeout" Width="70" Margin="0,3,14,3" Padding="5"/><TextBlock Text="Auftrag " VerticalAlignment="Center"/><TextBox x:Name="JobTimeout" Width="70" Margin="0,3" Padding="5"/></WrapPanel>
    <CheckBox x:Name="Build" Grid.Row="6" Grid.ColumnSpan="2" Content="Vor dem Start Release-Build aktualisieren (erst nach Prüfung auf aktive Läufe)" Margin="0,10,0,5"/>
    <TextBlock x:Name="Compatibility" Grid.Row="7" Grid.ColumnSpan="2" FontSize="11" Foreground="#52606D" Margin="0,4" TextWrapping="Wrap"/>
    <StackPanel Grid.Row="8" Grid.ColumnSpan="2" Orientation="Horizontal"><Button x:Name="Import" Content="Ergebnis-ZIPs anderer Rechner hinzufügen" Padding="9,5"/><TextBlock x:Name="Imports" VerticalAlignment="Center" Margin="10,0,0,0" Text="Keine ausgewählt"/></StackPanel>
   </Grid></Expander>
  </StackPanel></ScrollViewer>
  <TextBlock x:Name="Status" Grid.Row="2" Margin="0,12,0,0" Foreground="#B32929" TextWrapping="Wrap"/>
  <StackPanel Grid.Row="3" Orientation="Horizontal" HorizontalAlignment="Right" Margin="0,14,0,0"><Button x:Name="OpenObservatory" Content="Observatory öffnen" Padding="12,8" Margin="0,0,10,0"/><Button x:Name="Save" Content="Nur speichern" Padding="12,8" Margin="0,0,10,0"/><Button x:Name="Cancel" Content="Schließen" Padding="12,8" Margin="0,0,10,0"/><Button x:Name="Start" Content="Lauf starten / fortsetzen" Padding="16,8" Background="#14213D" Foreground="White" FontWeight="SemiBold"/></StackPanel>
 </Grid>
</Window>
'@
    $window=[Windows.Markup.XamlReader]::Load([Xml.XmlNodeReader]::new($xaml))
    $ui=@{}; foreach($name in @('Corpus','AddCorpus','BindArchive','ArchiveStatus','Parts','FewerParts','MoreParts','ApplyParts','PartitionButtons','SmallFirst','LargeFirst','Groups','Machine','Hidden','Export','TallThreshold','TallEdges','Output','Downloads','Port','RustMemory','ExcelMemory','GroupMemory','Reserve','OpenTimeout','JobTimeout','Build','Compatibility','Import','Imports','Status','OpenObservatory','Save','Cancel','Start')){$ui[$name]=$window.FindName($name)}
    $state=@{ partition=[int]$Configuration.settings.partition; partitions=[int]$Configuration.settings.partitions; imports=@($ImportResults); result=$null }
    if($state.imports.Count){$ui.Imports.Text="$($state.imports.Count) ausgewählt"}
    $fields=@{ Groups='comparison_groups';Machine='machine_label';TallThreshold='tall_sheet_threshold';TallEdges='tall_sheet_edge_tiles';Output='output_root';Downloads='download_root';Port='control_room_port';RustMemory='worker_memory_mib';ExcelMemory='excel_memory_mib';GroupMemory='resident_memory_mib';Reserve='system_reserve_mib';OpenTimeout='open_timeout_seconds';JobTimeout='job_timeout_seconds';Parts='partitions' }
    foreach($name in $fields.Keys){$ui[$name].Text=[string]$Configuration.settings.($fields[$name])}
    $ui.Hidden.IsChecked=[bool]$Configuration.settings.hide_excel; $ui.Export.IsChecked=[bool]$Configuration.settings.export_results; $ui.Build.IsChecked=[bool]$Configuration.settings.build_release
    $ui.LargeFirst.IsChecked=$Configuration.settings.order -eq 'size-desc';$ui.SmallFirst.IsChecked=-not $ui.LargeFirst.IsChecked
    $ui.Compatibility.Text="$(@($Configuration.settings.reuse_prepared_from).Count) bereits freigegebene Vorbereitungs-Builds. Ein neuer Build übernimmt keine zusätzliche Freigabe automatisch."
    $showSource={
        if($ui.Corpus.SelectedItem){
            $entry=$ui.Corpus.SelectedItem.Tag; $path=Get-Value $entry 'local_archive'
            if($path -and [IO.File]::Exists((Resolve-StudyPath $Repo $path))){$ui.ArchiveStatus.Text="Vorhandenes ZIP wird direkt wiederverwendet: $path"}
            elseif(Get-Value $entry 'url'){$ui.ArchiveStatus.Text='ZIP wird bei Bedarf heruntergeladen. Bereits vorhandene Downloads werden wiederverwendet.'}
            else{$ui.ArchiveStatus.Text='Noch kein lokales ZIP auf diesem Rechner. Bitte das vorhandene Korpusarchiv zuordnen.'}
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
        if(-not [int]::TryParse($ui.Parts.Text,[ref]$count) -or $count -lt 1 -or $count -gt 1000){throw 'Die Anzahl der Partitionen muss zwischen 1 und 1000 liegen.'}
        if($state.partition -gt $count){$state.partition=0}
        $state.partitions=$count
        $ui.PartitionButtons.Children.Clear()
        for($part=0;$part -le $count;$part++){
            $button=[Windows.Controls.RadioButton]::new();$button.GroupName='Partition';$button.Tag=[pscustomobject]@{Part=$part;State=$state};$button.Margin='0,0,12,8';$button.Padding='4';$button.IsChecked=$state.partition -eq $part
            $button.Content=if($part -eq 0){'Gesamter Korpus'}else{'{0}/{1} · {2:0.#}–{3:0.#} %' -f $part,$count,(100.0*($part-1)/$count),(100.0*$part/$count)}
            $button.Add_Checked({param($sender,$eventArgs);$sender.Tag.State.partition=[int]$sender.Tag.Part});[void]$ui.PartitionButtons.Children.Add($button)
        }
    }.GetNewClosure()
    $readSettings={
        $count=0
        if(-not [int]::TryParse($ui.Parts.Text,[ref]$count) -or $count -ne $state.partitions){throw 'Die geänderte Anzahl zuerst mit Aufteilen übernehmen und die Partition prüfen.'}
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
            if($added){$Configuration.settings.corpus=$added.name;& $refreshCorpus;Save-StudyJson $ConfigurationPath $Configuration;$ui.Status.Text='Korpus im lokalen Katalog gespeichert.'}
        }catch{$ui.Status.Text=$_.Exception.Message}
    }.GetNewClosure())
    $ui.BindArchive.Add_Click({
        try{
            $dialog=[Microsoft.Win32.OpenFileDialog]::new();$dialog.Filter='ZIP-Archive (*.zip)|*.zip';$dialog.Title='Dasselbe unveränderte Korpusarchiv zuordnen'
            if($dialog.ShowDialog($window)){
                $entry=$ui.Corpus.SelectedItem.Tag
                Set-StudyArchiveBinding $entry $dialog.FileName $Repo $ui.Output.Text
                Save-StudyJson $ConfigurationPath $Configuration;& $showSource;$ui.Status.Text='ZIP zugeordnet. Dateien und bestehende Ergebnisse wurden nicht verschoben.'
            }
        }catch{$ui.Status.Text=$_.Exception.Message}
    }.GetNewClosure())
    $ui.Import.Add_Click({$dialog=[Microsoft.Win32.OpenFileDialog]::new();$dialog.Filter='Ergebnis-ZIPs (*.zip)|*.zip';$dialog.Multiselect=$true;if($dialog.ShowDialog($window)){$state.imports=@($dialog.FileNames);$ui.Imports.Text="$($state.imports.Count) ausgewählt"}}.GetNewClosure())
    $ui.OpenObservatory.Add_Click({$port=0;if([int]::TryParse($ui.Port.Text,[ref]$port) -and $port -ge 1024 -and $port -le 65535){Start-Process "http://127.0.0.1:$port/"}}.GetNewClosure())
    $ui.Cancel.Add_Click({$window.Close()}.GetNewClosure())
    $ui.Save.Add_Click({try{& $readSettings;Save-StudyJson $ConfigurationPath $Configuration;$ui.Status.Foreground='#35755A';$ui.Status.Text='Konfiguration gespeichert. Kein Lauf gestartet.'}catch{$ui.Status.Foreground='#B32929';$ui.Status.Text=$_.Exception.Message}}.GetNewClosure())
    $ui.Start.Add_Click({
        $ui.Start.IsEnabled=$false
        try{
            & $readSettings
            $stamp=(Get-Date -Format 'yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8)
            $runDirectory=Join-Path ([IO.Path]::GetDirectoryName($ConfigurationPath)) ('runs/'+$stamp)
            $plan=New-StudyLaunchPlan $Configuration $Repo $Executable $runDirectory $state.imports
            $state.result=Start-StudyLaunch $Configuration $ConfigurationPath $Repo $plan
            $ui.Status.Foreground='#35755A';$ui.Status.Text="Lauf gestartet (PID $($state.result.Pid)). Das Fenster und VS Code können geschlossen werden. Logs: $runDirectory"
            $ui.Start.Content='Weiterer Start gesperrt'
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
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" Title="Korpus hinzufügen" Width="670" Height="310" ResizeMode="NoResize" WindowStartupLocation="CenterOwner" Background="#F4F7FB">
 <StackPanel Margin="24"><TextBlock Text="Name"/><TextBox x:Name="Name" Height="30" Margin="0,6,0,12" Padding="6"/><TextBlock Text="ZIP-Download-URL oder vorhandene lokale ZIP-Datei"/><DockPanel Margin="0,6,0,0"><Button x:Name="Browse" DockPanel.Dock="Right" Content="ZIP wählen" Padding="12,5" Margin="8,0,0,0"/><TextBox x:Name="Source" Height="30" Padding="6"/></DockPanel><TextBlock x:Name="Status" Foreground="#B32929" Margin="0,10,0,0" TextWrapping="Wrap"/><Button x:Name="Save" Content="Hinzufügen" HorizontalAlignment="Right" Padding="14,7" Margin="0,12,0,0"/></StackPanel>
</Window>
'@
    $window=[Windows.Markup.XamlReader]::Load([Xml.XmlNodeReader]::new($xaml));$window.Owner=$Owner
    $name=$window.FindName('Name');$source=$window.FindName('Source');$status=$window.FindName('Status');$result=@{value=$null}
    $window.FindName('Browse').Add_Click({$dialog=[Microsoft.Win32.OpenFileDialog]::new();$dialog.Filter='ZIP-Archive (*.zip)|*.zip';if($dialog.ShowDialog($window)){$source.Text=$dialog.FileName}}.GetNewClosure())
    $window.FindName('Save').Add_Click({try{$result.value=Add-StudyCorpus $Configuration $name.Text $source.Text $Repo;$window.Close()}catch{$status.Text=$_.Exception.Message}}.GetNewClosure())
    [void]$window.ShowDialog();return $result.value
}
