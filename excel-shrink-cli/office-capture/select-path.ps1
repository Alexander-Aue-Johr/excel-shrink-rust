param(
    [ValidateSet('folder', 'file', 'program')][string]$Kind = 'folder',
    [string]$InitialPath = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName System.Windows.Forms

$initialDirectory = $env:USERPROFILE
if ($InitialPath -and (Test-Path -LiteralPath $InitialPath -PathType Container)) {
    $initialDirectory = $InitialPath
} elseif ($InitialPath -and (Test-Path -LiteralPath $InitialPath -PathType Leaf)) {
    $initialDirectory = Split-Path -Parent $InitialPath
}

$dialog = if ($Kind -eq 'folder') {
    $picker = [Windows.Forms.FolderBrowserDialog]::new()
    $picker.Description = 'Select a folder'
    $picker.SelectedPath = $initialDirectory
    $picker.ShowNewFolderButton = $false
    $picker
} else {
    $picker = [Windows.Forms.OpenFileDialog]::new()
    $picker.InitialDirectory = $initialDirectory
    $picker.CheckFileExists = $true
    $picker.Multiselect = $false
    $picker.Filter = if ($Kind -eq 'program') {
        'Windows executables (*.exe)|*.exe|All files (*.*)|*.*'
    } else {
        'Workbooks, ZIPs and reports|*.xlsx;*.xlsm;*.xls;*.zip;*.json|All files (*.*)|*.*'
    }
    $picker
}

# An owner window keeps the dialog above the browser instead of behind it.
$owner = [Windows.Forms.Form]::new()
$owner.TopMost = $true
$owner.ShowInTaskbar = $false
$owner.Opacity = 0
try {
    $owner.Show()
    $selected = $dialog.ShowDialog($owner) -eq [Windows.Forms.DialogResult]::OK
    $path = if (-not $selected) { $null } elseif ($Kind -eq 'folder') {
        $dialog.SelectedPath
    } else {
        $dialog.FileName
    }
    @{ path = $path } | ConvertTo-Json -Compress
} finally {
    $dialog.Dispose()
    $owner.Dispose()
}
