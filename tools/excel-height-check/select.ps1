param([string]$Destination,[string]$ProfilesFile)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
$choice=[Windows.Forms.MessageBox]::Show('Ja: Excel-Datei auswaehlen. Nein: ganzen Ordner auswaehlen.','Excel JSON comparison','YesNoCancel')
if($choice -eq 'Cancel'){exit 3}
if($choice -eq 'Yes'){
 $picker=New-Object Windows.Forms.OpenFileDialog
 $picker.Filter='Excel workbook (*.xlsx)|*.xlsx'
}else{
 $picker=New-Object Windows.Forms.FolderBrowserDialog
 $picker.Description='Ordner mit XLSX-Dateien auswaehlen (ohne Unterordner)'
}
try{
 if($picker.ShowDialog() -ne 'OK'){exit 3}
 $path=if($choice -eq 'Yes'){$picker.FileName}else{$picker.SelectedPath}
}finally{$picker.Dispose()}
$form=New-Object Windows.Forms.Form
$form.Text='Shrink-Profil';$form.Width=530;$form.Height=150;$form.StartPosition='CenterScreen'
$combo=New-Object Windows.Forms.ComboBox
$combo.Left=15;$combo.Top=15;$combo.Width=480;$combo.DropDownStyle='DropDownList'
$profiles=Get-Content -LiteralPath $ProfilesFile -Raw -Encoding UTF8|ConvertFrom-Json
foreach($p in $profiles){[void]$combo.Items.Add([string]$p)}
$combo.SelectedIndex=0
$ok=New-Object Windows.Forms.Button
$ok.Text='Start';$ok.Left=400;$ok.Top=55;$ok.DialogResult='OK'
$form.Controls.Add($combo);$form.Controls.Add($ok);$form.AcceptButton=$ok
try{
 if($form.ShowDialog() -ne 'OK'){exit 3}
 $result=[ordered]@{path=$path;profile=[string]$combo.SelectedItem}
 [IO.File]::WriteAllText($Destination,($result|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
}finally{$form.Dispose()}
