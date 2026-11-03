$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -Path (Join-Path $PSScriptRoot '../NativeWindow.cs')
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class PaneTestClassName {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder text, int maximum);
    public static string Read(IntPtr window) {
        var text = new StringBuilder(256);
        GetClassName(window, text, text.Capacity);
        return text.ToString();
    }
}
"@
# Construct handles without displaying a window or taking focus.
$form = New-Object Windows.Forms.Form
$button = New-Object Windows.Forms.Button
$form.Controls.Add($button)
try {
    $owner = $form.Handle
    $child = $button.Handle
    $className = [PaneTestClassName]::Read($child)
    $found = @([NativeWindow]::FindChildWindows($owner, $PID, $className))
    if ($found.Count -ne 1 -or $found[0] -ne $child) { throw 'Native child lookup missed the owned control.' }
    if (@([NativeWindow]::FindChildWindows($owner, $PID, 'NotAnExistingClass')).Count -ne 0) {
        throw 'Class filter selected an unrelated control.'
    }
    $rejected = $false
    try { [void][NativeWindow]::FindChildWindows($owner, [int]::MaxValue, $className) }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'Foreign process identity was accepted.' }
    $rejected = $false
    try { [void][NativeWindow]::FindChildWindows([IntPtr]::Zero, $PID, $className) }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'Missing owner handle was accepted.' }
} finally {
    $form.Dispose()
}
'Owned native pane lookup tests passed.'
$module = Import-Module (Join-Path $PSScriptRoot '../CheckPerformance.psm1') -PassThru -Force
function Read-PaneState([object[]]$Controls) {
    & $module { param($Observed) Get-PerformanceControlState $Observed } $Controls
}
$german = Read-PaneState @(
    [pscustomobject]@{name='Arbeitsmappe optimiert';type='ControlType.Text';enabled=$true}
)
if ($german.completion.Count -ne 1) { throw 'Observed German completion was not recognized.' }
$english = Read-PaneState @(
    [pscustomobject]@{name='Workbook optimized';type='ControlType.Text';enabled=$true}
)
if ($english.completion.Count -ne 1) { throw 'English completion was not recognized.' }
$clean = Read-PaneState @(
    [pscustomobject]@{name='Keine Optimierung erforderlich';type='ControlType.Text';enabled=$true}
)
if ($clean.noWork.Count -ne 1) { throw 'Observed German no-work confirmation was not recognized.' }
$unknown = Read-PaneState @(
    [pscustomobject]@{name='Some cells may be optimized';type='ControlType.Text';enabled=$true},
    [pscustomobject]@{name='Alle optimieren';type='ControlType.Button';enabled=$false}
)
if ($unknown.completion.Count -ne 0 -or $unknown.noWork.Count -ne 0 -or $unknown.buttons.Count -ne 0) {
    throw 'Unknown or disabled pane state was accepted.'
}
'Localized positive confirmation tests passed.'