$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$sources = @('CaptureProcesses.cs', 'CaptureDesktop.cs', 'CaptureWorkbookSession.cs', 'CaptureWorkbookWorker.cs') |
    ForEach-Object { Join-Path $root $_ }
Add-Type -Path $sources -ReferencedAssemblies @('System', 'System.Core', 'System.Web.Extensions')
$read = [CaptureWorkbookWorker].GetMethod('ReadCommand', [Reflection.BindingFlags]'NonPublic,Static')
$previous = [Console]::In

function Read-Fixture([string]$Text) {
    [Console]::SetIn([IO.StringReader]::new($Text))
    return $read.Invoke($null, @())
}

function Assert-FramingFailure([string]$Text, [Type]$Expected) {
    try { [void](Read-Fixture $Text) }
    catch {
        $error = $_.Exception.GetBaseException()
        if ($error.GetType() -ne $Expected) { throw }
        return
    }
    throw 'Invalid command framing was accepted.'
}

try {
    if ((Read-Fixture "{}`r`n") -ne '{}') { throw 'CRLF frame differs.' }
    if ($null -ne (Read-Fixture '')) { throw 'EOF was not recognized.' }
    if ((Read-Fixture (('x' * 16384) + "`n")).Length -ne 16384) { throw 'Boundary frame differs.' }
    Assert-FramingFailure ('x' * 16385) ([IO.InvalidDataException])
    Assert-FramingFailure '{' ([IO.EndOfStreamException])
} finally {
    [Console]::SetIn($previous)
}
Write-Output 'Workbook worker framing checks passed.'
