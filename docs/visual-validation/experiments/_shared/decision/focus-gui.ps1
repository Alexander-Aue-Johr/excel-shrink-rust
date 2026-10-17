# WPF is loaded only for an explicitly requested GUI, never for CLI runs or preflight.
function Show-VVFocusDialog {
    param([Parameter(Mandatory=$true)][string]$RepoRoot,[hashtable]$InitialRequest)
    Add-Type -AssemblyName PresentationFramework -ErrorAction Stop
    Add-Type -AssemblyName System.IO.Compression -ErrorAction Stop
    Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
    $catalog=Read-VVJson (Join-Path $RepoRoot 'docs\visual-validation\experiments\capture-focus-cases.json')
    $profiles=Read-VVJson (Join-Path $RepoRoot 'docs\visual-validation\experiments\capture-question-profiles.json')
    $statePath=Join-Path $RepoRoot '.tmp\visual-validation\focus-last-request.json'
    $initial=$InitialRequest
    if($null -eq $initial){$initial=@{schema='vv-focus-request-v1'}}
    if(@($initial.Keys|Where-Object{$_ -notin @('schema','corpus_root')}).Count -eq 0){
        $saved=Read-VVJson $statePath
        if($null -ne $saved){
            $savedPlain=Convert-VVPlainObject $saved
            if($initial.corpus_root){$savedPlain.corpus_root=$initial.corpus_root}
            $initial=$savedPlain
        }
    }
    $result=@{Request=$null}
    [xml]$markup=@'
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
 Title="Focused capture checks · Excel Shrink" Width="1060" Height="850" MinWidth="840" MinHeight="650" WindowStartupLocation="CenterScreen" Background="#F4F7FA">
 <Window.Resources>
  <Style TargetType="TextBox"><Setter Property="Padding" Value="7,5"/><Setter Property="Margin" Value="0,4,0,9"/></Style>
  <Style TargetType="ComboBox"><Setter Property="Padding" Value="7,5"/><Setter Property="Margin" Value="0,4,0,9"/></Style>
  <Style TargetType="Button"><Setter Property="Padding" Value="12,7"/><Setter Property="Margin" Value="0,4,8,4"/></Style>
  <Style TargetType="TextBlock"><Setter Property="Foreground" Value="#172B42"/></Style>
 </Window.Resources>
 <DockPanel Margin="22">
  <Border DockPanel.Dock="Top" Background="#142C43" Padding="18" Margin="0,0,0,16">
   <StackPanel><TextBlock Text="FOCUSED CAPTURE CHECKS" Foreground="#92C9EC" FontSize="12"/><TextBlock Text="Reproduce one issue, not the entire suite" Foreground="White" FontSize="23" Margin="0,5,0,5"/>
    <TextBlock Text="Exact worksheet scope · separate startup and capture budgets · existing evidence reports" Foreground="#D2DEE9" TextWrapping="Wrap"/></StackPanel>
  </Border>
  <StackPanel DockPanel.Dock="Bottom">
   <TextBlock x:Name="ErrorText" TextWrapping="Wrap" Foreground="#A13028" Margin="0,10,0,5"/>
   <Grid><Grid.ColumnDefinitions><ColumnDefinition/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
    <TextBlock Text="A new focused report is created. No winner is selected and no old result is overwritten." TextWrapping="Wrap" VerticalAlignment="Center"/>
    <StackPanel Grid.Column="1" Orientation="Horizontal"><Button x:Name="CancelButton" Content="Cancel" IsCancel="True"/><Button x:Name="RunButton" Content="Validate selection and run" IsDefault="True" Background="#125B88" Foreground="White"/></StackPanel>
   </Grid>
  </StackPanel>
  <ScrollViewer VerticalScrollBarVisibility="Auto">
   <Grid><Grid.ColumnDefinitions><ColumnDefinition Width="3*"/><ColumnDefinition Width="24"/><ColumnDefinition Width="2*"/></Grid.ColumnDefinitions>
    <StackPanel Grid.Column="0">
     <TextBlock Text="Question profile" FontWeight="SemiBold"/><ComboBox x:Name="ProfileBox"/>
     <TextBlock x:Name="ProfileDetails" TextWrapping="Wrap" Margin="0,4,0,12"/>
     <TextBlock Text="Corpus directory" FontWeight="SemiBold"/><TextBox x:Name="CorpusText"/>
     <RadioButton x:Name="NamedMode" GroupName="Selection" Content="Named issue cases" IsChecked="True" FontWeight="SemiBold" Margin="0,5,0,8"/>
     <ListBox x:Name="CasesList" SelectionMode="Multiple" Height="158" Padding="6"/>
     <TextBlock x:Name="CaseDetails" TextWrapping="Wrap" FontSize="12" Margin="0,6,0,12"/>
     <RadioButton x:Name="CustomMode" GroupName="Selection" Content="Custom workbook and worksheets" FontWeight="SemiBold" Margin="0,5,0,5"/>
     <TextBlock Text="Exact corpus-relative path, or a unique filename" FontSize="12"/>
     <TextBox x:Name="WorkbookText"/>
     <WrapPanel><Button x:Name="BrowseButton" Content="Browse workbook…"/><Button x:Name="LoadSheetsButton" Content="Load worksheet catalogue"/></WrapPanel>
     <ListBox x:Name="SheetsList" SelectionMode="Multiple" Height="182" Padding="5"/>
     <CheckBox x:Name="AllVisibleCheck" Content="All visible worksheets in this workbook" Margin="0,8,0,12"/>
     <RadioButton x:Name="StoredMode" GroupName="Selection" Content="Use targets from request file" FontWeight="SemiBold" Visibility="Collapsed" Margin="0,5,0,5"/>
     <TextBlock x:Name="StoredText" TextWrapping="Wrap" FontSize="12"/>
     <TextBlock Text="Sheet indexes are 1-based workbook positions. Quoted names preserve leading and trailing spaces. Hidden worksheets are not made visible." TextWrapping="Wrap" FontSize="12" Margin="0,8,0,12"/>
    </StackPanel>
    <StackPanel Grid.Column="2">
     <TextBlock Text="Capture configuration" FontSize="17" FontWeight="SemiBold" Margin="0,0,0,8"/>
     <TextBlock Text="Host"/><ComboBox x:Name="HostBox"/>
     <TextBlock Text="Overflow verification"/><ComboBox x:Name="OverflowBox"/>
     <TextBlock Text="Image budget (MP)"/><ComboBox x:Name="MegapixelsBox"/>
     <TextBlock Text="Local whole-versus-tiles reference"/><ComboBox x:Name="ContextBox"/>
     <TextBlock Text="Diagnostics"/><ComboBox x:Name="DiagnosticsBox"/>
     <TextBlock Text="Timeouts (seconds)" FontSize="17" FontWeight="SemiBold" Margin="0,10,0,8"/>
     <TextBlock Text="Setup — controller, plans and host build"/><TextBox x:Name="SetupText"/>
     <TextBlock Text="Startup — Excel launch to measurement start"/><TextBox x:Name="StartupText"/>
     <TextBlock Text="Capture — measurement start to result"/><TextBox x:Name="CaptureText"/>
     <Expander Header="Advanced limits and debugging" IsExpanded="False" Margin="0,8,0,8">
      <StackPanel Margin="0,8,0,0">
       <TextBlock Text="Inactivity after capture has started"/><TextBox x:Name="InactivityText"/>
       <TextBlock Text="Single native Office call"/><TextBox x:Name="NativeText"/>
       <TextBlock Text="Offline image verification and report"/><TextBox x:Name="PostprocessText"/>
       <TextBlock Text="Whole session — blank for derived finite budget"/><TextBox x:Name="SessionText"/>
       <TextBlock Text="Independent repetitions (1–5)"/><TextBox x:Name="RepetitionsText"/>
       <CheckBox x:Name="WaitCheck" Content="Wait for task-pane Start button" Margin="0,6,0,6"/>
       <CheckBox x:Name="DebugCheck" Content="Use Microsoft Office.js debug library" Margin="0,6,0,6"/>
      </StackPanel>
     </Expander>
     <TextBlock Text="Native-call and session limits remain finite. Increasing capture time does not permit a stuck native call to run forever." TextWrapping="Wrap" FontSize="12"/>
    </StackPanel>
   </Grid>
  </ScrollViewer>
 </DockPanel>
</Window>
'@
    $reader=[Xml.XmlNodeReader]::new($markup)
    try{$window=[Windows.Markup.XamlReader]::Load($reader)}finally{$reader.Close()}
    $available=[Windows.SystemParameters]::WorkArea
    $window.MinHeight=[Math]::Min(650,[Math]::Max(420,$available.Height-40))
    $window.Height=[Math]::Min(850,[Math]::Max(420,$available.Height-40))
    $window.MinWidth=[Math]::Min(840,[Math]::Max(640,$available.Width-40))
    $window.Width=[Math]::Min(1060,[Math]::Max(640,$available.Width-40))
    $ui=@{}
    foreach($name in @('ProfileBox','ProfileDetails','CorpusText','NamedMode','CasesList','CaseDetails','CustomMode','WorkbookText','BrowseButton','LoadSheetsButton','SheetsList','AllVisibleCheck','StoredMode','StoredText',
        'HostBox','OverflowBox','MegapixelsBox','ContextBox','DiagnosticsBox','SetupText','StartupText','CaptureText','InactivityText','NativeText','PostprocessText','SessionText','RepetitionsText','WaitCheck','DebugCheck','CancelButton','RunButton','ErrorText')){
        $ui[$name]=$window.FindName($name)
        if($null -eq $ui[$name]){throw ('Missing focused GUI control: '+$name)}
    }
    if($initial.corpus_root -and ([string]$initial.corpus_root).StartsWith('/') -and $RepoRoot -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\') {
        $initial.corpus_root='\\wsl.localhost\'+$Matches[1]+([string]$initial.corpus_root).Replace('/','\')
    }
    $ui.CorpusText.Text=if($initial.corpus_root){[string]$initial.corpus_root}else{Join-Path $RepoRoot 'corpus-local'}
    $comboDefinitions=@(
        @('HostBox','host_kind',@('dotnet','rust'),'dotnet'),
        @('OverflowBox','overflow_mode',@('selective','heuristic'),'selective'),
        @('MegapixelsBox','tile_megapixels',@(16,32,64),16),
        @('ContextBox','context_validation',@('off','first-seam'),'off'),
        @('DiagnosticsBox','diagnostic_level',@('detailed','normal'),'detailed'))
    foreach($d in $comboDefinitions){foreach($value in $d[2]){[void]$ui[$d[0]].Items.Add([string]$value)};$ui[$d[0]].SelectedItem=[string]$(if($initial.ContainsKey($d[1])){$initial[$d[1]]}else{$d[3]})}
    $fields=@{SetupText=@('setup_timeout_seconds',600,30,1800);StartupText=@('startup_timeout_seconds',300,10,1800);
        CaptureText=@('capture_timeout_seconds',1800,30,7200);InactivityText=@('inactivity_timeout_seconds',120,10,1800);
        NativeText=@('native_call_timeout_seconds',90,10,600);PostprocessText=@('postprocess_timeout_seconds',240,30,1800);
        RepetitionsText=@('repetitions',1,1,5)}
    foreach($name in $fields.Keys){$d=$fields[$name];$ui[$name].Text=[string]$(if($initial.ContainsKey($d[0])){$initial[$d[0]]}else{$d[1]})}
    if($initial.session_timeout_seconds){$ui.SessionText.Text=[string]$initial.session_timeout_seconds}
    $ui.WaitCheck.IsChecked=[bool]$initial.wait_for_start;$ui.DebugCheck.IsChecked=[bool]$initial.use_office_debug_js
    $ids=if($initial.case_ids){@($initial.case_ids)}else{@($catalog.default_case)}
    foreach($case in $catalog.cases){
        $item=[Windows.Controls.ListBoxItem]::new();$item.Content=$case.label;$item.Tag=$case;$item.ToolTip=$case.question
        [void]$ui.CasesList.Items.Add($item);$item.IsSelected=($ids -contains $case.id)
    }
    $ui.CasesList.Add_SelectionChanged({$ui.CaseDetails.Text=(@($ui.CasesList.SelectedItems|ForEach-Object{$_.Tag.question}) -join "`n")})
    $ui.CaseDetails.Text=(@($ui.CasesList.SelectedItems|ForEach-Object{$_.Tag.question}) -join "`n")
    if($initial.workbook){$ui.CustomMode.IsChecked=$true;$ui.WorkbookText.Text=[string]$initial.workbook}
    if($initial.targets){
        $ui.StoredMode.Visibility='Visible';$ui.StoredMode.IsChecked=$true
        $ui.StoredText.Text=(@($initial.targets|ForEach-Object{([string]$_.workbook)+' → '+$(if($_.sheets){($_.sheets -join ', ')}elseif($_.sheet_indexes){'indexes '+($_.sheet_indexes -join ', ')}else{'all visible'})}) -join "`n")
    }
    $customProfile=[Windows.Controls.ComboBoxItem]::new();$customProfile.Content='Custom / named issue cases';$customProfile.Tag=$null
    [void]$ui.ProfileBox.Items.Add($customProfile)
    foreach($profile in $profiles.profiles){$item=[Windows.Controls.ComboBoxItem]::new();$item.Content=$profile.label;$item.Tag=$profile;[void]$ui.ProfileBox.Items.Add($item)}
    $profileState=@{Loading=$true}
    $applyProfile={
        $selected=$ui.ProfileBox.SelectedItem.Tag
        $isCustom=($null -eq $selected)
        foreach($name in @('NamedMode','CasesList','CustomMode','WorkbookText','BrowseButton','LoadSheetsButton','AllVisibleCheck','SheetsList','StoredMode')){$ui[$name].IsEnabled=$isCustom}
        foreach($d in $comboDefinitions){$ui[$d[0]].IsEnabled=$true}
        if($isCustom){$ui.SheetsList.IsEnabled=(-not $ui.AllVisibleCheck.IsChecked);$ui.ProfileDetails.Text='Select exact cases below. No comparison is started implicitly.';return}
        $varied=@($selected.variants | ForEach-Object {$_.PSObject.Properties.Name} | Select-Object -Unique)
        $ui.ProfileDetails.Text=$selected.question+"`n"+(@($selected.targets|ForEach-Object{
            ([string]$_.id)+': '+$(if($_.sheets){($_.sheets -join ', ')}elseif($_.after_sheet){'visible worksheets after '+$_.after_sheet}else{'worksheet indexes '+($_.sheet_indexes -join ', ')})}) -join "`n")+"`nOnly this profile runs. No subsequent matrix is started."
        foreach($d in $comboDefinitions){
            $ui[$d[0]].IsEnabled=($varied -notcontains $d[1])
            $value=$selected.defaults.PSObject.Properties[$d[1]]
            if($null -ne $value -and (-not $profileState.Loading -or -not $initial.ContainsKey($d[1]))){$ui[$d[0]].SelectedItem=[string]$value.Value}
        }
        foreach($name in $fields.Keys){$key=$fields[$name][0];$value=$selected.defaults.PSObject.Properties[$key]
            if($null -ne $value -and (-not $profileState.Loading -or -not $initial.ContainsKey($key))){$ui[$name].Text=[string]$value.Value}}
        if(-not $profileState.Loading -or -not $initial.session_timeout_seconds){$ui.SessionText.Text=[string]$selected.session_seconds}
    }
    $ui.ProfileBox.Add_SelectionChanged($applyProfile)
    $ui.ProfileBox.SelectedIndex=0
    if($initial.profile){foreach($item in $ui.ProfileBox.Items){if($null -ne $item.Tag -and $item.Tag.id -ceq $initial.profile){$ui.ProfileBox.SelectedItem=$item}}}
    & $applyProfile
    $profileState.Loading=$false
    $loadedCatalogue=@{Path=$null;WorkbookText=$null;CorpusText=$null}
    $loadSheets={
        $ui.ErrorText.Text=''
        try {
            $root=[IO.Path]::GetFullPath($ui.CorpusText.Text)
            $value=[string]$ui.WorkbookText.Text
            if([string]::IsNullOrWhiteSpace($value)){throw 'Choose a workbook first.'}
            $path=if([IO.Path]::IsPathRooted($value)){$value}else{Join-Path $root $value}
            if(-not [IO.File]::Exists($path) -and $value -notmatch '[\\/]'){
                $matches=@(Get-VVChildItem -LiteralPath $root -File -Recurse -Filter '*.xlsx'|Where-Object{$_.Name -ceq $value})
                if($matches.Count -ne 1){throw 'Filename is missing or ambiguous. Use a corpus-relative path.'}
                $path=$matches[0].FullName
            }
            $path=[IO.Path]::GetFullPath($path)
            if(-not $path.StartsWith($root.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Workbook must be inside the selected corpus directory.'}
            $stream=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
            $archive=$null;$xmlReader=$null;$entryStream=$null
            try {
                $archive=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Read,$false)
                $entries=@($archive.Entries|Where-Object{$_.FullName -ceq 'xl/workbook.xml'})
                if($entries.Count -ne 1 -or $entries[0].Length -gt 4194304){throw 'Invalid or oversized worksheet catalogue.'}
                $settings=[Xml.XmlReaderSettings]::new();$settings.DtdProcessing=[Xml.DtdProcessing]::Prohibit;$settings.XmlResolver=$null;$settings.MaxCharactersInDocument=4194304
                $entryStream=$entries[0].Open();$xmlReader=[Xml.XmlReader]::Create($entryStream,$settings)
                $doc=[Xml.XmlDocument]::new();$doc.XmlResolver=$null;$doc.Load($xmlReader)
                $nodes=$doc.SelectNodes('/*[local-name()="workbook"]/*[local-name()="sheets"]/*[local-name()="sheet"]')
                if($nodes.Count -eq 0){throw 'Workbook has no worksheets.'}
                $ui.SheetsList.Items.Clear();$index=0
                foreach($sheet in $nodes){
                    $index++;$name=$sheet.GetAttribute('name');$state=$sheet.GetAttribute('state');if(-not $state){$state='visible'}
                    $item=[Windows.Controls.ListBoxItem]::new();$item.Tag=@{name=$name;index=$index;state=$state}
                    $item.Content=('{0} · {1} · {2}' -f $index,($name|ConvertTo-Json -Compress),$state)
                    $item.IsEnabled=($state -eq 'visible');[void]$ui.SheetsList.Items.Add($item)
                    if(($initial.sheets -ccontains $name) -or ($initial.sheet_indexes -contains $index)){$item.IsSelected=$item.IsEnabled}
                }
            } finally {if($null -ne $xmlReader){$xmlReader.Dispose()};if($null -ne $entryStream){$entryStream.Dispose()};if($null -ne $archive){$archive.Dispose()}else{$stream.Dispose()}}
            $loadedCatalogue.Path=$path;$loadedCatalogue.WorkbookText=$ui.WorkbookText.Text;$loadedCatalogue.CorpusText=$ui.CorpusText.Text
            $ui.CustomMode.IsChecked=$true
        } catch {$ui.ErrorText.Text=$_.Exception.Message}
    }
    $ui.LoadSheetsButton.Add_Click($loadSheets)
    $ui.BrowseButton.Add_Click({
        $dialog=[Microsoft.Win32.OpenFileDialog]::new();$dialog.Filter='Excel workbook (*.xlsx)|*.xlsx';$dialog.CheckFileExists=$true
        if([IO.Directory]::Exists($ui.CorpusText.Text)){$dialog.InitialDirectory=$ui.CorpusText.Text}
        if($dialog.ShowDialog($window)){$ui.WorkbookText.Text=$dialog.FileName;& $loadSheets}
    })
    $ui.AllVisibleCheck.Add_Checked({$ui.SheetsList.IsEnabled=$false})
    $ui.AllVisibleCheck.Add_Unchecked({$ui.SheetsList.IsEnabled=$true})
    if($initial.workbook){$ui.AllVisibleCheck.IsChecked=($null -eq $initial.sheets -and $null -eq $initial.sheet_indexes);& $loadSheets}
    $ui.CancelButton.Add_Click({$window.DialogResult=$false})
    $ui.RunButton.Add_Click({
        $ui.ErrorText.Text=''
        try {
            $request=@{schema='vv-focus-request-v1';corpus_root=[string]$ui.CorpusText.Text}
            if($null -ne $ui.ProfileBox.SelectedItem.Tag){$request.profile=[string]$ui.ProfileBox.SelectedItem.Tag.id}
            elseif($ui.NamedMode.IsChecked){
                $request.case_ids=@($ui.CasesList.SelectedItems|ForEach-Object{[string]$_.Tag.id})
                if($request.case_ids.Count -eq 0){throw 'Select at least one named case.'}
            } elseif($ui.StoredMode.IsChecked){$request.targets=@($initial.targets)}
            else {
                $request.workbook=[string]$ui.WorkbookText.Text
                if([string]::IsNullOrWhiteSpace($request.workbook)){throw 'Select a workbook.'}
                if(-not $ui.AllVisibleCheck.IsChecked){
                    if($loadedCatalogue.WorkbookText -cne $ui.WorkbookText.Text -or $loadedCatalogue.CorpusText -cne $ui.CorpusText.Text){throw 'Reload the worksheet catalogue after changing the workbook or corpus.'}
                    $request.sheets=@($ui.SheetsList.SelectedItems|Where-Object{$_.IsEnabled}|ForEach-Object{[string]$_.Tag.name})
                    if($request.sheets.Count -eq 0){throw 'Select worksheets or explicitly select all visible worksheets.'}
                }
            }
            foreach($d in $comboDefinitions){if(-not $ui[$d[0]].IsEnabled){continue};$value=$ui[$d[0]].SelectedItem;if($null -eq $value){throw ('Choose '+$d[1])};$request[$d[1]]=if($d[1] -eq 'tile_megapixels'){[int]$value}else{[string]$value}}
            foreach($name in $fields.Keys){
                $d=$fields[$name];$number=0
                if(-not [int]::TryParse($ui[$name].Text,[ref]$number) -or $number -lt $d[2] -or $number -gt $d[3]){throw ($d[0]+' must be '+$d[2]+'..'+$d[3]+'.')}
                $request[$d[0]]=$number
            }
            if($ui.SessionText.Text.Trim()){$number=0;if(-not [int]::TryParse($ui.SessionText.Text,[ref]$number) -or $number -lt 60 -or $number -gt 86400){throw 'Session timeout must be 60..86400 seconds.'};$request.session_timeout_seconds=$number}
            if($request.inactivity_timeout_seconds -lt $request.native_call_timeout_seconds+10){throw 'Inactivity timeout must exceed the native-call timeout by at least 10 seconds.'}
            if($request.capture_timeout_seconds -lt $request.native_call_timeout_seconds){throw 'Capture timeout must not be shorter than a native-call timeout.'}
            $request.wait_for_start=[bool]$ui.WaitCheck.IsChecked;$request.use_office_debug_js=[bool]$ui.DebugCheck.IsChecked
            Write-VVJson $statePath $request
            $result.Request=$request;$window.DialogResult=$true
        } catch {$ui.ErrorText.Text=$_.Exception.Message}
    })
    [void]$window.ShowDialog()
    return $result.Request
}
