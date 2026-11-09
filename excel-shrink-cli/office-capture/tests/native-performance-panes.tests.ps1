$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -Path (Join-Path $PSScriptRoot '../NativeWindow.cs')

# Create hidden native controls. No window is shown and no global input is sent.
$window = New-Object Windows.Forms.Form
$button = New-Object Windows.Forms.Button
try {
    $window.Controls.Add($button)
    $owner = $window.Handle
    $child = $button.Handle
    $className = [Windows.Automation.AutomationElement]::FromHandle($child).Current.ClassName
    $matches = @([NativeWindow]::FindOwnedDescendants($owner, $PID, $className))
    if ($matches.Count -ne 1 -or $matches[0] -ne $child) {
        throw 'Native child lookup did not return the owned matching control.'
    }
    if (@([NativeWindow]::FindOwnedDescendants($owner, $PID, 'MissingPerformancePane')).Count -ne 0) {
        throw 'Unknown native classes must produce no candidates.'
    }
    $rejected = $false
    try { [void][NativeWindow]::FindOwnedDescendants($owner, $PID + 1, $className) }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'A mismatched owner process was accepted.' }
    Write-Output 'Native pane handle ownership and class-filter checks passed.'
} finally {
    $window.Dispose()
    $button.Dispose()
}
