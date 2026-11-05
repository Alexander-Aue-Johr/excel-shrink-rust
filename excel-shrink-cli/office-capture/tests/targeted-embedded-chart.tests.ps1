$ErrorActionPreference = 'Stop'
$module = Import-Module (Join-Path $PSScriptRoot '../TargetedChartSheets.psm1') -Force -PassThru
& $module {
    function Invoke-OwnedChartOperation {
        param($Entry, [scriptblock]$Operation)
        & $Operation $script:book $script:chartSheets
    }
    $active = [pscustomobject]@{activations=0}
    $active | Add-Member ScriptMethod Activate { $this.activations++ }
    $chart = [pscustomobject]@{fail=$false; exports=0}
    $chart | Add-Member ScriptMethod Refresh { }
    $chart | Add-Member ScriptMethod Export {
        param($Path, $Format, $Interactive)
        $this.exports++
        if ($this.fail) { throw 'Export failed' }
        [IO.File]::WriteAllBytes($Path, [byte[]](1,2,3))
        return $true
    }
    $object = [pscustomobject]@{Name='Chart 3'; Top=12; Left=24; Width=100; Height=80; Chart=$chart}
    $objects = [pscustomobject]@{value=$object; Count=1; last=$null}
    $objects | Add-Member ScriptMethod Item { param($Name) $this.last=$Name; $this.value }
    $sheet = [pscustomobject]@{Visible=0; objects=$objects}
    $sheet | Add-Member ScriptMethod ChartObjects { $this.objects }
    $sheet | Add-Member ScriptMethod Activate { }
    $sheets = [pscustomobject]@{value=$sheet}
    $sheets | Add-Member ScriptMethod Item { param($Name) $this.value }
    $script:book = [pscustomobject]@{ActiveSheet=$active; Worksheets=$sheets}
    $output = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N')+'.png')
    $command = @{sheet='Sheet2'; chart='Chart 3'; output=$output}
    try {
        $result = Export-TargetedEmbeddedChart @{} $command
        if ($result.path -ne $output -or $result.geometry.export_method -ne 'native-existing-process') { throw 'Export evidence missing' }
        if ($sheet.Visible -ne 0 -or $active.activations -ne 1) { throw 'View was not restored after success' }
        $chart.fail = $true
        $failed = $false
        try { Export-TargetedEmbeddedChart @{} $command } catch { $failed = $true }
        if (-not $failed -or $sheet.Visible -ne 0 -or $active.activations -ne 2) { throw 'Failure was hidden or view was not restored' }
        $object.Height = 0
        $result = Export-TargetedEmbeddedChart @{} $command
        if (-not $result.non_rendering -or $chart.exports -ne 2) { throw 'Zero-area chart was exported' }
        $object.Height = 80
        $chart.fail = $false
        $command.chart_index = 0
        $null = Export-TargetedEmbeddedChart @{} $command
        if ($objects.last -isnot [int] -or $objects.last -ne 1) { throw 'Indexed fallback must use the one-based native object position.' }
        $command.chart = 'Renamed'
        $failed = $false
        try { Export-TargetedEmbeddedChart @{} $command } catch { $failed = $true }
        if (-not $failed) { throw 'A changed chart order must not export another object.' }
        'Embedded chart export restores state on success/failure and preserves zero-area evidence.'
    } finally {
        if (Test-Path -LiteralPath $output) { Remove-Item -LiteralPath $output }
    }
    $window = [pscustomobject]@{WindowState=-4137; Width=800; Height=600; Left=10; Top=20; Zoom=75}
    $windows = [pscustomobject]@{value=$window}
    $windows | Add-Member ScriptMethod Item { param($Index) $this.value }
    $standalone = [pscustomobject]@{Visible=0; SizeWithWindow=$false;
        PageSetup=[pscustomobject]@{ChartSize=2}; ChartArea=[pscustomobject]@{Width=724.5; Height=442.5}}
    $standalone | Add-Member ScriptMethod Activate { }
    $standalone | Add-Member ScriptMethod Refresh { }
    $standalone | Add-Member ScriptMethod Export {
        param($Path, $Format, $Interactive)
        if (-not $this.SizeWithWindow -or $script:book.Windows.value.Width -ne 1000 -or
            $script:book.Windows.value.Height -ne 750) { throw 'Chart did not adopt the export viewport' }
        [IO.File]::WriteAllBytes($Path, [byte[]](1,2,3))
        return $true
    }
    $script:chartSheets = [pscustomobject]@{value=$standalone}
    $script:chartSheets | Add-Member ScriptMethod Item { param($Name) $this.value }
    $script:book | Add-Member NoteProperty Windows $windows
    try {
        $null = Export-TargetedChartSheet @{} @{chart='fatigue plot'; output=$output}
        if ($standalone.SizeWithWindow -or $standalone.Visible -ne 0 -or
            $window.Width -ne 800 -or $window.Height -ne 600 -or $window.Zoom -ne 75 -or
            $window.WindowState -ne -4137 -or $standalone.PageSetup.ChartSize -ne 2) {
            throw 'Chart export did not restore the previous view'
        }
        'Chart sheet adopts the export viewport and restores its previous view.'
    } finally {
        if (Test-Path -LiteralPath $output) { Remove-Item -LiteralPath $output }
    }

}
