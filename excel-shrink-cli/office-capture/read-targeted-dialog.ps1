# Read accessible dialog labels in a disposable process, bounded by the caller.
param(
    [Parameter(Mandatory=$true)][int]$ExcelProcessId,
    [Parameter(Mandatory=$true)][long]$Handle,
    [Parameter(Mandatory=$true)][string]$Output,
    [string]$DesktopName = $env:EXCEL_SHRINK_EXPECTED_DESKTOP
)
$ErrorActionPreference = 'Stop'
Add-Type -Path (Join-Path $PSScriptRoot 'TargetedOpeningDialogs.cs')
$dialog = [TargetedOpeningDialogs]::Inspect($ExcelProcessId, $DesktopName) |
    Where-Object { $_.Handle -eq $Handle -and $_.IsModal }
if ($null -eq $dialog) { throw 'The dialog no longer belongs to the owned Excel process.' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$Handle)
if ($root.Current.ProcessId -ne $ExcelProcessId) { throw 'The dialog process changed.' }
$queue = New-Object 'Collections.Generic.Queue[Windows.Automation.AutomationElement]'
$queue.Enqueue($root)
$walker = [Windows.Automation.TreeWalker]::RawViewWalker
$names = New-Object 'Collections.Generic.List[string]'
$clock = [Diagnostics.Stopwatch]::StartNew()
$visited = 0
while ($queue.Count -gt 0 -and $visited -lt 128 -and $clock.Elapsed.TotalSeconds -lt 1) {
    $element = $queue.Dequeue()
    $visited++
    $current = $element.Current
    # Inspect labels only. Never read editable fields, password values or text patterns.
    if ($current.IsPassword -or $current.ControlType -eq [Windows.Automation.ControlType]::Edit) { continue }
    if ($current.Name) {
        $text = $current.Name.Substring(0, [Math]::Min(2048, $current.Name.Length))
        $names.Add(($text -replace '[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}', '[email redacted]'))
    }
    $child = $walker.GetFirstChild($element)
    while ($null -ne $child -and $queue.Count -lt 128) {
        $queue.Enqueue($child)
        $child = $walker.GetNextSibling($child)
    }
}
$result = @{pid=$ExcelProcessId; handle=$Handle; names=@($names.ToArray()); observed_at=[DateTime]::UtcNow.ToString('o')}
[IO.File]::WriteAllText($Output, ($result | ConvertTo-Json -Depth 4 -Compress), [Text.UTF8Encoding]::new($false))
