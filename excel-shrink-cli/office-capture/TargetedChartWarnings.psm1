# Recognize one informational chart warning on an already PID-validated Excel dialog.
$ErrorActionPreference = 'Stop'

function Test-TargetedLogChartWarning {
    <# .SYNOPSIS Match the complete informational message and its unique acknowledgement. #>
    param([string[]]$Names, [string[]]$Buttons)
    $warning = 'Negative or zero values cannot be plotted correctly on log charts. Only positive values can be interpreted on a logarithmic scale.'
    return $Names.Count -eq 1 -and $Names[0] -ceq $warning -and $Buttons.Count -eq 1 -and $Buttons[0] -ceq 'OK'
}

function Confirm-TargetedLogChartWarning {
    <# .SYNOPSIS
    Acknowledges only the known single-OK log-scale warning, retaining its exact text.
    Unknown dialogs and decisions requiring multiple buttons remain untouched.
    #>
    param($Dialog, [int]$ProcessId)
    if (-not $Dialog.IsModal -or $Dialog.ClassName -ne 'NUIDialog') { return $null }
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Dialog.Handle)
    if ($root.Current.ProcessId -ne $ProcessId) { throw 'Unexpected dialog owner.' }
    $walker = [Windows.Automation.TreeWalker]::RawViewWalker
    $queue = New-Object 'Collections.Generic.Queue[Windows.Automation.AutomationElement]'
    $queue.Enqueue($root)
    $names = @()
    $buttons = @()
    $visited = 0
    while ($queue.Count -gt 0 -and $visited -lt 64) {
        $element = $queue.Dequeue()
        $visited++
        $current = $element.Current
        if ($current.ProcessId -ne $ProcessId) { continue }
        if ($current.ControlType -eq [Windows.Automation.ControlType]::Text) { $names += $current.Name }
        if ($current.ControlType -eq [Windows.Automation.ControlType]::Button -and $current.IsEnabled) { $buttons += $element }
        $child = $walker.GetFirstChild($element)
        while ($null -ne $child -and $queue.Count -lt 64) {
            $queue.Enqueue($child)
            $child = $walker.GetNextSibling($child)
        }
    }
    if (-not (Test-TargetedLogChartWarning $names @($buttons | ForEach-Object { $_.Current.Name }))) { return $null }
    $pattern = $buttons[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
    return @{kind='logarithmic_chart_nonpositive_values'; names=$names; action='acknowledge_information'; pid=$ProcessId; handle=$Dialog.Handle; observed_at=[DateTime]::UtcNow.ToString('o')}
}

Export-ModuleMember -Function Confirm-TargetedLogChartWarning
