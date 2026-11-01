param(
    [Parameter(Mandatory=$true)][string]$Session,
    [Parameter(Mandatory=$true)][string]$Worker,
    [Parameter(Mandatory=$true)][string]$Output,
    [switch]$Optimize,
    [switch]$HiddenOptimize,
    [switch]$ClickOptimize,
    [switch]$MessageOptimize
)
$ErrorActionPreference = 'Stop'
Add-Type -Path @((Join-Path $Worker 'CaptureProcesses.cs'), (Join-Path $Worker 'CaptureDesktop.cs'))
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -Path (Join-Path $PSScriptRoot 'DesktopIdentity.cs')
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

function Connect-OwnedExcel($SessionPath) {
    $connection = [pscustomobject]@{
        Process=$null; Scope=$null; Excel=$null; Book=$null; Root=$null; WasVisible=$null
    }
    try {
        $metrics = Get-Content -LiteralPath (Join-Path $SessionPath 'metrics.json') -Raw | ConvertFrom-Json
        $client = $metrics.clients.one
        $process = Get-Process -Id $client.pid
        $started = [DateTimeOffset]::new($process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
        if ($process.ProcessName -ne 'EXCEL' -or [math]::Abs($started - $client.process_started_unix_ms) -gt 2) {
            throw 'Isolated Excel process identity changed.'
        }
        $connection.Process = $process
        $scope = [CaptureErrorChecking]::TryDisable($process, $env:EXCEL_SHRINK_EXPERIMENT_DESKTOP)
        $connection.Scope = $scope
        if ($null -eq $scope) { throw 'The isolated Excel instance has no accessible native window.' }
        # This experiment borrows the exact owned instance; no global GetActiveObject.
        $field = [CaptureErrorChecking].GetField('options', [Reflection.BindingFlags]'NonPublic,Instance')
        $excel = $field.GetValue($scope).Application
        $connection.Excel = $excel
        $book = $excel.ActiveWorkbook
        $connection.Book = $book
        if ($book.Name -ne 'one.xlsx') { throw 'Unexpected workbook; no command was executed.' }
        $connection.Root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$excel.Hwnd)
        $connection.WasVisible = $excel.Visible
        return $connection
    } catch {
        Close-OwnedExcelConnection $connection
        throw
    }
}

function Close-OwnedExcelConnection($Connection) {
    if ($null -eq $Connection) { return }
    try {
        if ($null -ne $Connection.WasVisible) {
            $Connection.Excel.Visible = $Connection.WasVisible
        }
    } finally {
        try {
            if ($null -ne $Connection.Scope) { $Connection.Scope.Dispose() }
        } finally {
            foreach ($reference in @($Connection.Book, $Connection.Excel)) {
                if ($null -ne $reference -and [Runtime.InteropServices.Marshal]::IsComObject($reference)) {
                    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($reference)
                }
            }
        }
    }
}

function Record-NativeCapabilities($Connection, $Report) {
    $process = $Connection.Process
    $excel = $Connection.Excel
    $book = $Connection.Book
    $Report.pid = $process.Id
    $Report.excel_version = $excel.Version
    $Report.excel_build = $excel.Build
    $Report.excel_desktop = [DesktopIdentity]::WindowName([IntPtr]$excel.Hwnd)
    $Report.input_desktop = [DesktopIdentity]::InputName()
    $Report.probe_desktop = [DesktopIdentity]::CurrentName()
    $Report.com_candidates = @($excel | Get-Member | Where-Object Name -match 'Performance|Optimi|Cleanup' | Select-Object Name,MemberType,Definition)
    $Report.workbook_com_candidates = @($book | Get-Member | Where-Object Name -match 'Performance|Optimi|Cleanup' | Select-Object Name,MemberType,Definition)
    $commands = $excel.CommandBars
    foreach ($id in @('CheckPerformance','WorkbookPerformance','WorkbookPerformanceCheck',
        'CheckWorkbookPerformance','OptimizeWorkbook','OptimizeSheet','OptimizeAll','WorkbookPerformancePane')) {
        try {
            $Report.com_commands += @{id=$id; label=$commands.GetLabelMso($id);
                enabled=$commands.GetEnabledMso($id); visible=$commands.GetVisibleMso($id)}
        } catch { $Report.com_commands += @{id=$id; error=$_.Exception.Message} }
    }
}

function Save-PanelImage($Connection, $Report, $Directory) {
    Add-Type -AssemblyName System.Drawing
    Add-Type -Path (Join-Path $Worker 'NativeWindow.cs')
    $root = $Connection.Root
    $excel = $Connection.Excel
    $bounds = $root.Current.BoundingRectangle
    $bitmap = [Drawing.Bitmap]::new([int]$bounds.Width, [int]$bounds.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $dc = $graphics.GetHdc()
    try {
        $report.window_image_captured = [NativeWindow]::PrintWindow([IntPtr]$excel.Hwnd, $dc, 2)
    } finally { $graphics.ReleaseHdc($dc) }
    $bitmap.Save((Join-Path $Directory 'panel.png'))
    $graphics.Dispose()
    $bitmap.Dispose()
}

function Test-PerformancePaneOpening($Connection, $Report, $Directory) {
    $excel = $Connection.Excel
    $root = $Connection.Root
    $commands = $excel.CommandBars
    $excel.Visible = $false
    try {
        $commands.ExecuteMso('CheckPerformance')
        $Report.hidden_com = @{invoked=$true}
    } catch { $Report.hidden_com = @{invoked=$false; error=$_.Exception.Message} }
    try { $Report.hidden = Open-PerformancePane $root }
    catch { $Report.hidden = @{invoked=$false; error=$_.Exception.Message} }
    $excel.Visible = $true
    $commands.ExecuteMso('CheckPerformance')
    Start-Sleep -Seconds 10
    Save-PanelImage $Connection $Report $Directory
    $Report.visible_com = @{invoked=$true; controls=(Describe-Controls @(Get-Controls $root))}
    $Report.visible = @{controls=(Describe-Controls @(Get-Controls $root))}
}

function Find-OptimizationTarget($Root, $Report) {
    $controls = @(Get-Controls $Root)
    # Scope the deep traversal to the pane, avoiding the worksheet's cell tree.
    $panes = @($controls | Where-Object {$_.name -eq 'Workbook Performance' -and $_.class -eq 'MsoWorkPane'})
    if ($panes.Count -eq 1) { $controls = @(Get-Controls $panes[0].element) }
    $Report.performance_controls = Describe-Controls $controls
    $buttons = @($controls | Where-Object {$_.type -eq 'ControlType.Button' -and
        $_.name -match '^(Optimize all|Optimize All|Alle optimieren)$'})
    if ($buttons.Count -ne 1 -or -not $buttons[0].enabled) {
        $Report.optimization = @{status='no_unique_enabled_optimize_all'; controls=(Describe-Controls $controls)}
        return $null
    }
    return [pscustomobject]@{Button=$buttons[0].element; Name=$buttons[0].name; Pane=$panes[0].element}
}

function Invoke-OptimizationAction($Connection, $Target, $Report, [switch]$Hidden, [switch]$Click, [switch]$Messages) {
    $excel = $Connection.Excel
    $process = $Connection.Process
    $button = $Target.Button
    $paneHandle = [IntPtr]$Target.Pane.Current.NativeWindowHandle
    if ($Hidden) { $excel.Visible = $false }
    $report.optimization.excel_visible_at_action = $excel.Visible
    $pattern = $null
    if ($Messages) {
        $rectangle = $button.Current.BoundingRectangle
        [NativeWindow]::SendClick([IntPtr]$excel.Hwnd, $process.Id,
            [int]($rectangle.X + $rectangle.Width / 2),
            [int]($rectangle.Y + $rectangle.Height / 2))
        $Report.optimization.method = 'window_messages'
    } elseif ($Click) {
        if (-not $excel.Visible) { throw 'Pointer input requires visible Excel.' }
        $rectangle = $button.Current.BoundingRectangle
        [NativeWindow]::Click([IntPtr]$excel.Hwnd, $process.Id,
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
}

function Wait-OptimizationCompletion($Pane) {
    $after = @()
    for ($attempt=0; $attempt -lt 15; $attempt++) {
        Start-Sleep -Seconds 1
        $after = @(Get-Controls $Pane)
        if ($after | Where-Object {$_.name -match '\b(optimized|optimiert)\b'}) {
            return @{status='completion_text_observed'; controls=(Describe-Controls $after)}
        }
    }
    return @{status='invoked_completion_unverified'; controls=(Describe-Controls $after)}
}

function Test-WorkbookOptimization($Connection, $Report, $Directory, [switch]$Hidden, [switch]$Click, [switch]$Messages) {
    $target = Find-OptimizationTarget $Connection.Root $Report
    if ($null -eq $target) { return }

    $book = $Connection.Book
    $book.Save()
    $book.SaveCopyAs((Join-Path $Directory 'before-optimization.xlsx'))
    $Report.optimization = @{status='button_found'; button=$target.Name;
        patterns=@($target.Button.GetSupportedPatterns() | ForEach-Object ProgrammaticName)}
    Invoke-OptimizationAction $Connection $target $Report -Hidden:$Hidden -Click:$Click -Messages:$Messages
    $Report.optimization.status = 'invoked_completion_unverified'
    $completion = Wait-OptimizationCompletion $target.Pane
    $Report.optimization.status = $completion.status
    $book.SaveCopyAs((Join-Path $Directory 'after-optimization.xlsx'))
    $Report.after = $completion.controls
}

$report = [ordered]@{com_commands=@(); hidden=$null; visible=$null; optimization=@{status='not_requested'}}
$connection = $null
$directory = [IO.Path]::GetDirectoryName($Output)
try {
    $connection = Connect-OwnedExcel $Session
    Record-NativeCapabilities $connection $report
    Test-PerformancePaneOpening $connection $report $directory
    if ($Optimize) {
        Test-WorkbookOptimization $connection $report $directory -Hidden:$HiddenOptimize -Click:$ClickOptimize -Messages:$MessageOptimize
    }
} catch { $report.error = $_.Exception.Message }
finally {
    try { Close-OwnedExcelConnection $connection }
    finally {
        [IO.File]::WriteAllText($Output, ($report | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false))
    }
}
