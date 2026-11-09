# Check Performance has no verified public optimization API. Keep its native
# adapter separate from workbook lifecycle and never send global pointer input.
$ErrorActionPreference = 'Stop'

function Get-PerformanceControls {
    <# .SYNOPSIS
    Enumerates a bounded native control tree, tolerating disappearing elements.
    #>
    param($Root)
    $walker = [Windows.Automation.TreeWalker]::RawViewWalker
    $queue = New-Object 'Collections.Generic.Queue[Windows.Automation.AutomationElement]'
    $queue.Enqueue($Root)
    $visited = 0
    while ($queue.Count -gt 0 -and $visited -lt 1200) {
        $element = $queue.Dequeue()
        $visited++
        try {
            $current = $element.Current
            [pscustomobject]@{element=$element; name=$current.Name;
                class=$current.ClassName; type=$current.ControlType.ProgrammaticName;
                enabled=$current.IsEnabled}
            $child = $walker.GetFirstChild($element)
            while ($null -ne $child -and $queue.Count -lt 1200) {
                $queue.Enqueue($child)
                $child = $walker.GetNextSibling($child)
            }
        } catch [Windows.Automation.ElementNotAvailableException] { }
    }
}

function Get-PerformancePaneState {
    <# .SYNOPSIS
    Resolves a unique performance pane and reads its action and completion text.
    #>
    param($Root)
    # Native child handles avoid expanding Excel's potentially enormous sheet UIA tree.
    $current = $Root.Current
    $handles = @([NativeWindow]::FindOwnedDescendants(
        [IntPtr]$current.NativeWindowHandle, $current.ProcessId, 'MsoWorkPane'))
    $candidates = if ($handles.Count -gt 0) {
        @($handles | ForEach-Object { [Windows.Automation.AutomationElement]::FromHandle($_) })
    } else {
        # Some Office providers expose a pane without its own native child window.
        $condition = [Windows.Automation.PropertyCondition]::new(
            [Windows.Automation.AutomationElement]::ClassNameProperty, 'MsoWorkPane')
        @($Root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition))
    }
    $panes = @($candidates | Where-Object {
        $_.Current.Name -match '^(Workbook Performance|Arbeitsmappenleistung|Leistung der Arbeitsmappe)$'
    })
    if ($panes.Count -ne 1) { return $null }
    $controls = @(Get-PerformanceControls $panes[0])
    $buttons = @($controls | Where-Object {
        $_.type -eq 'ControlType.Button' -and $_.enabled -and
        $_.name -match '^(Optimize all|Optimize All|Alle optimieren)$'
    })
    # Sheet rows also expose an Optimize All action. The workbook action is
    # above the sheet list. Collapse duplicate accessibility nodes at that point.
    if ($buttons.Count -gt 1) {
        $positions = @($buttons | ForEach-Object {
            $bounds = $_.element.Current.BoundingRectangle
            if ($bounds.Width -gt 0 -and $bounds.Height -gt 0) {
                [pscustomobject]@{control=$_; top=$bounds.Top; left=$bounds.Left}
            }
        } | Sort-Object top, left)
        if ($positions.Count -gt 0) {
            $top = @($positions | Where-Object { $_.top -eq $positions[0].top })
            $points = @($top | Select-Object -ExpandProperty left -Unique)
            $buttons = if ($points.Count -eq 1) { @($top[0].control) } else { @() }
        }
    }
    # Completion is positive evidence; a missing button alone is never success.
    $completion = @($controls | Where-Object { $_.name -match '^(Workbook optimized|Arbeitsmappe optimiert)$' })
    $noWork = @($controls | Where-Object {
        $_.name -match '^(No optimization needed|No optimizations needed|Nothing to optimize|Keine Optimierung erforderlich|Keine Optimierungen erforderlich|Keine Optimierung notwendig)$'
    })
    return @{buttons=$buttons; completion=$completion; noWork=$noWork;
        lookup_method=$(if ($handles.Count -gt 0) { 'owned_native_handles' } else { 'accessibility_fallback' });
        text=@($controls | ForEach-Object name | Where-Object { $_ })}
}

function Invoke-WorkbookPerformanceOptimization {
    <# .SYNOPSIS
    Optimizes an owned, visible workbook on its private desktop and confirms completion.
    .DESCRIPTION
    Uses the experimentally verified window-message action. The Rust caller
    supplies an overall command deadline, including potentially blocked COM/UIA
    calls. Unknown pane states fail explicitly instead of certifying success.
    #>
    param($Session, [ValidateRange(1, 86400)][int]$TimeoutSeconds)
    function Write-PerformanceTrace([string]$Text) {
        if ($Session.trace_path) { [IO.File]::AppendAllText($Session.trace_path, ([DateTime]::UtcNow.ToString('o') + ' ' + $Text + "`n")) }
    }
    if ($null -eq $Session) { throw 'Check Performance workbook is not open.' }
    $excel = $Session.excel
    $desktop = [ExcelProcessId]::DesktopName([IntPtr]$excel.Hwnd)
    if (-not $env:EXCEL_SHRINK_EXPECTED_DESKTOP -or
        $desktop -eq 'Default' -or $desktop -ne $env:EXCEL_SHRINK_EXPECTED_DESKTOP) {
        throw 'Check Performance requires the owned private desktop.'
    }
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    if (-not ('NativeWindow' -as [type])) {
        Add-Type -Path (Join-Path $PSScriptRoot 'NativeWindow.cs')
    }
    $Session.process.Refresh()
    if ($Session.process.HasExited) { throw 'Owned Check Performance Excel process exited.' }
    Write-PerformanceTrace 'Activating owned workbook'
    $Session.workbook.Activate()
    if ($Session.reset_pane) {
        $existingRoot = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$excel.Hwnd)
        if ($null -ne (Get-PerformancePaneState $existingRoot)) {
            $excel.CommandBars.ExecuteMso('CheckPerformance')
            $closeDeadline = [DateTime]::UtcNow.AddSeconds(5)
            while ($null -ne (Get-PerformancePaneState $existingRoot)) {
                if ([DateTime]::UtcNow -ge $closeDeadline) {
                    throw 'Previous workbook performance pane did not close.'
                }
                Start-Sleep -Milliseconds 100
            }
        }
    }
    Write-PerformanceTrace 'Opening Check Performance'
    $excel.CommandBars.ExecuteMso('CheckPerformance')
    Write-PerformanceTrace 'Check Performance command returned'
    $root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$excel.Hwnd)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $clicked = $false
    $observed = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        $state = Get-PerformancePaneState $root
        if (-not $observed -and $null -ne $state) {
            Write-PerformanceTrace ('Pane lookup ' + $state.lookup_method)
            Write-PerformanceTrace ($state.text -join ' | ')
            $observed = $true
        }
        if ($null -ne $state) {
            if ($clicked -and $state.completion.Count -gt 0) {
                return @{status='optimization_confirmed'; method='window_messages';
                    desktop=$desktop; completion_text=$state.text}
            }
            if (-not $clicked -and $state.noWork.Count -gt 0) {
                return @{status='no_optimization_needed'; method='pane_inspection';
                    desktop=$desktop; completion_text=$state.text}
            }
            if (-not $clicked -and $state.buttons.Count -eq 1) {
                $rectangle = $state.buttons[0].element.Current.BoundingRectangle
                [NativeWindow]::SendClick([IntPtr]$excel.Hwnd, $Session.process.Id,
                    [int]($rectangle.X + $rectangle.Width / 2),
                    [int]($rectangle.Y + $rectangle.Height / 2))
                $clicked = $true
            }
        }
        Start-Sleep -Milliseconds 250
    }
    $lastText = if ($null -eq $state) { 'No unique Workbook Performance pane.' } else {
        ($state.text | Select-Object -First 12) -join ' | '
    }
    throw "Check Performance completion was not confirmed (action sent: $clicked). $lastText"
}

Export-ModuleMember -Function Invoke-WorkbookPerformanceOptimization
