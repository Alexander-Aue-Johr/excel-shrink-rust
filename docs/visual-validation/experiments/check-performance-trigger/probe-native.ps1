param(
    [Parameter(Mandatory=$true)][string]$Session,
    [Parameter(Mandatory=$true)][string]$Worker,
    [Parameter(Mandatory=$true)][string]$Output,
    [switch]$Optimize,
    [switch]$HiddenOptimize,
    [switch]$ClickOptimize
)
$ErrorActionPreference = 'Stop'
Add-Type -Path @((Join-Path $Worker 'CaptureProcesses.cs'), (Join-Path $Worker 'CaptureDesktop.cs'))
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -Path (Join-Path $PSScriptRoot 'AccessibleAction.cs') -ReferencedAssemblies Accessibility

function Get-Controls($Root) {
    $walker = [Windows.Automation.TreeWalker]::RawViewWalker
    $queue = New-Object 'Collections.Generic.Queue[Windows.Automation.AutomationElement]'
    $queue.Enqueue($Root)
    $count = 0
    while ($queue.Count -gt 0 -and $count -lt 1200) {
        $element = $queue.Dequeue()
        $count++
        try {
            $current = $element.Current
            if ($null -ne $current) {
                [pscustomobject]@{element=$element; name=$current.Name;
                    id=$current.AutomationId; class=$current.ClassName;
                    type=$current.ControlType.ProgrammaticName;
                    offscreen=$current.IsOffscreen; enabled=$current.IsEnabled}
            }
            $child = $walker.GetFirstChild($element)
            while ($null -ne $child -and $queue.Count -lt 1200) {
                $queue.Enqueue($child)
                $child = $walker.GetNextSibling($child)
            }
        } catch [Windows.Automation.ElementNotAvailableException] { }
    }
}

function Describe-Controls($Controls) {
    @($Controls | Select-Object name,id,class,type,offscreen,enabled)
}

function Open-PerformancePane($Root) {
    $controls = @(Get-Controls $Root)
    $review = @($controls | Where-Object {$_.type -eq 'ControlType.TabItem' -and $_.name -match '^(Review|\u00dcberpr\u00fcfen)$'})
    if ($review.Count -eq 1) {
        $review[0].element.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
        $controls = @(Get-Controls $Root)
    }
    $buttons = @($controls | Where-Object {$_.type -eq 'ControlType.Button' -and
        $_.name -match '^(Check Performance|Leistung \u00fcberpr\u00fcfen|Leistung pr\u00fcfen)$'})
    if ($buttons.Count -ne 1) { return @{invoked=$false; reason='No unique Check Performance button'; controls=(Describe-Controls $controls)} }
    $buttons[0].element.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 500
    return @{invoked=$true; button=$buttons[0].name; controls=(Describe-Controls @(Get-Controls $Root))}
}

$report = [ordered]@{com_commands=@(); hidden=$null; visible=$null; optimization=@{status='not_requested'}}
$scope = $null
$excel = $null
$book = $null
try {
    $metrics = Get-Content -LiteralPath (Join-Path $Session 'metrics.json') -Raw | ConvertFrom-Json
    $client = $metrics.clients.one
    $process = Get-Process -Id $client.pid
    $started = [DateTimeOffset]::new($process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
    if ($process.ProcessName -ne 'EXCEL' -or [math]::Abs($started - $client.process_started_unix_ms) -gt 2) {
        throw 'Isolated Excel process identity changed.'
    }
    $scope = [CaptureErrorChecking]::TryDisable($process)
    if ($null -eq $scope) { throw 'The isolated Excel instance has no accessible native window.' }
    # This experiment borrows the exact owned instance; no global GetActiveObject.
    $field = [CaptureErrorChecking].GetField('options', [Reflection.BindingFlags]'NonPublic,Instance')
    $excel = $field.GetValue($scope).Application
    $book = $excel.ActiveWorkbook
    if ($book.Name -ne 'one.xlsx') { throw 'Unexpected workbook; no command was executed.' }
    $report.pid = $process.Id
    $report.excel_version = $excel.Version
    $report.excel_build = $excel.Build
    $report.com_candidates = @($excel | Get-Member | Where-Object Name -match 'Performance|Optimi|Cleanup' | Select-Object Name,MemberType,Definition)
    $report.workbook_com_candidates = @($book | Get-Member | Where-Object Name -match 'Performance|Optimi|Cleanup' | Select-Object Name,MemberType,Definition)
    $commands = $excel.CommandBars
    foreach ($id in @('CheckPerformance','WorkbookPerformance','WorkbookPerformanceCheck',
        'CheckWorkbookPerformance','OptimizeWorkbook','OptimizeSheet','OptimizeAll','WorkbookPerformancePane')) {
        try {
            $report.com_commands += @{id=$id; label=$commands.GetLabelMso($id);
                enabled=$commands.GetEnabledMso($id); visible=$commands.GetVisibleMso($id)}
        } catch { $report.com_commands += @{id=$id; error=$_.Exception.Message} }
    }
    $root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$excel.Hwnd)
    $wasVisible = $excel.Visible
    try {
        $excel.Visible = $false
        try {
            $commands.ExecuteMso('CheckPerformance')
            $report.hidden_com = @{invoked=$true}
        } catch { $report.hidden_com = @{invoked=$false; error=$_.Exception.Message} }
        try { $report.hidden = Open-PerformancePane $root }
        catch { $report.hidden = @{invoked=$false; error=$_.Exception.Message} }
        $excel.Visible = $true
        $commands.ExecuteMso('CheckPerformance')
        Start-Sleep -Seconds 10
        Add-Type -AssemblyName System.Drawing
        Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ProbeWindowImage {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] private static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    public static void Click(IntPtr window, int pid, int x, int y) {
        SetForegroundWindow(window);
        System.Threading.Thread.Sleep(200);
        uint actualPid;
        GetWindowThreadProcessId(GetForegroundWindow(), out actualPid);
        if (actualPid != pid) throw new InvalidOperationException("Owned Excel is not foreground.");
        if (!SetCursorPos(x, y)) throw new InvalidOperationException("Cannot position pointer.");
        mouse_event(2, 0, 0, 0, UIntPtr.Zero);
        mouse_event(4, 0, 0, 0, UIntPtr.Zero);
    }
}
'@
        $bounds = $root.Current.BoundingRectangle
        $bitmap = [Drawing.Bitmap]::new([int]$bounds.Width, [int]$bounds.Height)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $dc = $graphics.GetHdc()
        try {
            $report.window_image_captured = [ProbeWindowImage]::PrintWindow([IntPtr]$excel.Hwnd, $dc, 2)
        } finally { $graphics.ReleaseHdc($dc) }
        $bitmap.Save((Join-Path ([IO.Path]::GetDirectoryName($Output)) 'panel.png'))
        $graphics.Dispose()
        $bitmap.Dispose()
        $report.visible_com = @{invoked=$true; controls=(Describe-Controls @(Get-Controls $root))}
        $report.visible = @{controls=(Describe-Controls @(Get-Controls $root))}
        if ($Optimize) {
            $controls = @(Get-Controls $root)
            # Scope the deep traversal to the pane, avoiding the worksheet's cell tree.
            $panes = @($controls | Where-Object {$_.name -eq 'Workbook Performance' -and $_.class -eq 'MsoWorkPane'})
            if ($panes.Count -eq 1) { $controls = @(Get-Controls $panes[0].element) }
            $report.performance_controls = Describe-Controls $controls
            $buttons = @($controls | Where-Object {$_.type -eq 'ControlType.Button' -and
                $_.name -match '^(Optimize all|Optimize All|Alle optimieren)$'})
            if ($buttons.Count -eq 1 -and $buttons[0].enabled) {
                $book.Save()
                $book.SaveCopyAs((Join-Path ([IO.Path]::GetDirectoryName($Output)) 'before-optimization.xlsx'))
                $button = $buttons[0].element
                $report.optimization = @{status='button_found'; button=$buttons[0].name;
                    patterns=@($button.GetSupportedPatterns() | ForEach-Object ProgrammaticName)}
                $paneHandle = [IntPtr]$panes[0].element.Current.NativeWindowHandle
                if ($HiddenOptimize) { $excel.Visible = $false }
                $report.optimization.excel_visible_at_action = $excel.Visible
                $pattern = $null
                if ($ClickOptimize) {
                    if (-not $excel.Visible) { throw 'Pointer input requires visible Excel.' }
                    $rectangle = $button.Current.BoundingRectangle
                    [ProbeWindowImage]::Click([IntPtr]$excel.Hwnd, $process.Id,
                        [int]($rectangle.X + $rectangle.Width / 2),
                        [int]($rectangle.Y + $rectangle.Height / 2))
                    $report.optimization.method = 'visible_pointer_input'
                } elseif ($button.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
                    $pattern.Invoke()
                } else {
                    $count = [AccessibleAction]::Invoke($paneHandle, 'Optimize all')
                    $report.optimization.msaa_matches = $count
                    $report.optimization.msaa_candidates = @([AccessibleAction]::Candidates)
                    if ($count -ne 1) { throw 'No unique Optimize all MSAA action.' }
                }
                $report.optimization.status = 'invoked_completion_unverified'
                for ($attempt=0; $attempt -lt 15; $attempt++) {
                    Start-Sleep -Seconds 1
                    $after = @(Get-Controls $panes[0].element)
                    if ($after | Where-Object {$_.name -match '\b(optimized|optimiert)\b'}) {
                        $report.optimization.status = 'completion_text_observed'
                        break
                    }
                }
                $book.SaveCopyAs((Join-Path ([IO.Path]::GetDirectoryName($Output)) 'after-optimization.xlsx'))
                $report.after = Describe-Controls $after
            } else { $report.optimization = @{status='no_unique_enabled_optimize_all'; controls=(Describe-Controls $controls)} }
        }
    } finally { $excel.Visible = $wasVisible }
} catch { $report.error = $_.Exception.Message }
finally {
    if ($null -ne $scope) { $scope.Dispose() }
    foreach ($reference in @($book,$excel)) {
        if ($null -ne $reference -and [Runtime.InteropServices.Marshal]::IsComObject($reference)) {
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($reference)
        }
    }
    [IO.File]::WriteAllText($Output, ($report | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
}
