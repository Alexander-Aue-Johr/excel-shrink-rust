$ErrorActionPreference = 'Stop'
$module = Import-Module (Join-Path $PSScriptRoot '../TargetedChartSheets.psm1') -Force -PassThru
& $module {
    function Invoke-OwnedChartOperation {
        param($Entry, [scriptblock]$Operation)
        & $Operation $script:book $null
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
    $object = [pscustomobject]@{Top=12; Left=24; Width=100; Height=80; Chart=$chart}
    $objects = [pscustomobject]@{value=$object}
    $objects | Add-Member ScriptMethod Item { param($Name) $this.value }
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
        'Embedded chart export restores state on success/failure and preserves zero-area evidence.'
    } finally {
        if (Test-Path -LiteralPath $output) { Remove-Item -LiteralPath $output }
    }
}
