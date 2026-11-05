$ErrorActionPreference = 'Stop'
$module = Import-Module (Join-Path $PSScriptRoot '../TargetedChartWarnings.psm1') -Force -PassThru
& $module {
    $text = 'Negative or zero values cannot be plotted correctly on log charts. Only positive values can be interpreted on a logarithmic scale.'
    if (-not (Test-TargetedLogChartWarning @($text) @('OK'))) { throw 'Known warning rejected' }
    foreach ($case in @(
        @{names=@($text); buttons=@('OK','Cancel')},
        @{names=@($text,'Other decision'); buttons=@('OK')},
        @{names=@('Workbook repair required'); buttons=@('OK')},
        @{names=@($text); buttons=@('Yes')},
        @{names=@(); buttons=@('OK')}
    )) {
        if (Test-TargetedLogChartWarning $case.names $case.buttons) { throw 'Unknown decision accepted' }
    }
    'Exact log-chart warning accepted; additional text, other prompts and multiple actions rejected.'
}
