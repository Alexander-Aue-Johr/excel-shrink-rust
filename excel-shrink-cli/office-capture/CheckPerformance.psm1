# Check Performance has no verified public optimization API. Keep its native
# adapter separate from workbook lifecycle and never send global pointer input.
$ErrorActionPreference = 'Stop'

function Get-PerformanceControls {
    <# .SYNOPSIS
    Enumerates a bounded native control tree, tolerating disappearing elements.
    #>
    param($Root)
    $walker = [Windows.Automation.TreeWalker]::ControlViewWalker
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
    param([IntPtr]$Owner, [int]$ProcessId)
    # Enumerating Excel's entire accessibility tree can enter the worksheet cell
    # provider and block. Resolve native task panes first, then inspect one pane.
    $panes = @([NativeWindow]::FindChildWindows($Owner, $ProcessId, 'MsoWorkPane') | ForEach-Object {
        $pane = [Windows.Automation.AutomationElement]::FromHandle($_)
        if ($pane.Current.Name -match '^(Workbook Performance|Arbeitsmappenleistung)$') { $pane }
    })
    if ($panes.Count -ne 1) { return $null }
    $controls = @(Get-PerformanceControls $panes[0])
    return Get-PerformanceControlState $controls
}

function Get-PerformanceControlState {
    <# .SYNOPSIS
    Classifies localized pane observations; absence of an action is not completion.
    #>
    param([object[]]$Controls)
    $buttons = @($controls | Where-Object {
        $_.type -eq 'ControlType.Button' -and $_.enabled -and
        $_.name -match '^(Optimize all|Optimize All|Alle optimieren)$'
    })
    # Completion is positive evidence; a missing button alone is never success.
    $completion = @($controls | Where-Object { $_.name -match '^(Workbook optimized|Arbeitsmappe optimiert)$' })
    $noWork = @($controls | Where-Object {
        $_.name -match '^(No optimization needed|No optimizations needed|Nothing to optimize|Keine Optimierung erforderlich|Keine Optimierungen erforderlich)$'
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
    # Keep diagnostics on stderr; stdout carries the host response protocol.
    [Console]::Error.WriteLine('[check-performance] activating owned workbook')
    $Session.workbook.Activate()
    [Console]::Error.WriteLine('[check-performance] ExecuteMso CheckPerformance')
    $excel.CommandBars.ExecuteMso('CheckPerformance')
    [Console]::Error.WriteLine('[check-performance] command returned; locating pane')
    $owner = [IntPtr]$excel.Hwnd
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $clicked = $false
    $previousText = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $state = Get-PerformancePaneState $owner $Session.process.Id
        if ($null -ne $state) {
            $text = ($state.text | Select-Object -First 24) -join ' | '
            if ($text -ne $previousText) {
                [Console]::Error.WriteLine(('[check-performance] pane: {0}' -f $text))
                $previousText = $text
            }
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
                [Console]::Error.WriteLine('[check-performance] optimize action sent')
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
