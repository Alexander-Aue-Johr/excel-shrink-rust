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
    $panes = @(Get-PerformanceControls $Root | Where-Object {
        $_.class -eq 'MsoWorkPane' -and $_.name -eq 'Workbook Performance'
    })
    if ($panes.Count -ne 1) { return $null }
    $controls = @(Get-PerformanceControls $panes[0].element)
    $buttons = @($controls | Where-Object {
        $_.type -eq 'ControlType.Button' -and $_.enabled -and
        $_.name -match '^(Optimize all|Optimize All|Alle optimieren)$'
    })
    # Completion is positive evidence; a missing button alone is never success.
    $completion = @($controls | Where-Object { $_.name -eq 'Workbook optimized' })
    $noWork = @($controls | Where-Object {
        $_.name -match '^(No optimization needed|No optimizations needed|Nothing to optimize)$'
    })
    return @{buttons=$buttons; completion=$completion; noWork=$noWork;
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
    $Session.workbook.Activate()
    $excel.CommandBars.ExecuteMso('CheckPerformance')
    $root = [Windows.Automation.AutomationElement]::FromHandle([IntPtr]$excel.Hwnd)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $clicked = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        $state = Get-PerformancePaneState $root
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
