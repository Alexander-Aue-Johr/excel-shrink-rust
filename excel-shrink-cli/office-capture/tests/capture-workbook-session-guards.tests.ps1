$ErrorActionPreference = 'Stop'
Add-Type -Path (Join-Path $PSScriptRoot '../CaptureWorkbookSession.cs')
$root = Join-Path ([IO.Path]::GetTempPath()) ('workbook-guards-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($root)
$source = Join-Path $root 'source.xlsx'
[IO.File]::WriteAllText($source, 'existing source bytes')
$process = [Diagnostics.Process]::GetCurrentProcess()
# These checks must reject before any COM access. No Excel application is supplied.
$session = New-Object CaptureWorkbookSession -ArgumentList @([object]::new(), $process, $source)

function Assert-Guard([scriptblock]$Action, [Type]$Expected) {
    try { & $Action }
    catch {
        if ($_.Exception.GetBaseException().GetType() -ne $Expected) { throw }
        return
    }
    throw 'Workbook session guard did not reject the request.'
}

try {
    Assert-Guard { $session.SaveCopy((Join-Path $root 'copy.xlsx')) } ([InvalidOperationException])
    Assert-Guard { $session.SaveAs((Join-Path $root 'saved.xlsx')) } ([InvalidOperationException])
    Assert-Guard { $session.Rebuild() } ([InvalidOperationException])
    Assert-Guard { $session.OpenReadOnly($source) } ([ArgumentException])
    Assert-Guard { $session.OpenWorkingCopy($source) } ([ArgumentException])
    $current = [CaptureWorkbookSession].GetField('currentWorkbook', [Reflection.BindingFlags]'NonPublic,Instance')
    $current.SetValue($session, [object]::new())
    Assert-Guard { $session.SaveCopy($source) } ([IO.IOException])
    Assert-Guard { $session.SaveCopy((Join-Path $root 'copy.xls')) } ([ArgumentException])
    Assert-Guard { $session.SaveAs($source) } ([IO.IOException])
    Assert-Guard { $session.SaveAs((Join-Path $root 'saved.xls')) } ([ArgumentException])
    if ([IO.File]::ReadAllText($source) -ne 'existing source bytes') { throw 'Existing snapshot destination changed.' }
    if ([IO.File]::Exists((Join-Path $root 'copy.xlsx'))) { throw 'Guard published a snapshot.' }
    $session.Dispose()
    $session.Dispose()
    Assert-Guard { $session.CloseCurrent() } ([ObjectDisposedException])
} finally {
    $session.Dispose()
    $process.Dispose()
    $parent = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($root))
    if ($parent -ne [IO.Path]::GetTempPath().TrimEnd('\')) { throw 'Unexpected cleanup directory.' }
    [IO.Directory]::Delete($root, $true)
}
Write-Output 'Workbook snapshot, Save As and rebuild guards passed without Excel.'
