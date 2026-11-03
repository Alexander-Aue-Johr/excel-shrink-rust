$ErrorActionPreference = 'Stop'
$module = Join-Path $PSScriptRoot '../WindowsLongPaths.psm1'
Import-Module $module -Force

function Assert-Equal([string]$Actual, [string]$Expected) {
    if ($Actual -cne $Expected) { throw "Expected '$Expected', got '$Actual'." }
}

function Test-PathForms {
    <# .SYNOPSIS
    Checks that local and UNC paths are prefixed only when they are long.
    #>
    Assert-Equal (ConvertTo-WindowsLongPath 'C:\short\file.json') 'C:\short\file.json'
    Assert-Equal (ConvertTo-WindowsLongPath ('C:\' + ('a' * 250))) ('\\?\C:\' + ('a' * 250))
    Assert-Equal (ConvertTo-WindowsLongPath ('\\server\share\' + ('a' * 250))) ('\\?\UNC\server\share\' + ('a' * 250))
    Assert-Equal (ConvertTo-WindowsLongPath ('\\?\C:\' + ('a' * 250))) ('\\?\C:\' + ('a' * 250))
}

function Test-LongFilePublication {
    <# .SYNOPSIS
    Writes and publishes an actual long file path on a native Windows drive.
    #>
    $root = Join-Path $env:TEMP ('excel-shrink-long-path-' + [Guid]::NewGuid().ToString('N'))
    $directory = Join-Path $root (('a' * 50) + '\' + ('b' * 40) + '\' + ('c' * 25))
    $nativeDirectory = ConvertTo-WindowsLongPath $directory
    try {
        $null = [IO.Directory]::CreateDirectory($nativeDirectory)
        $target = ConvertTo-WindowsLongPath (Join-Path $directory 'metrics.json')
        $pending = ConvertTo-WindowsLongPath (Join-Path $directory ('metrics.json.' + [Guid]::NewGuid().ToString('N') + '.pending'))
        if ($target.StartsWith('\\?\') -or $pending.Length -le 260) {
            throw 'The fixture must mix a short target with a long pending path.'
        }
        [IO.File]::WriteAllText($pending, 'ok')
        [IO.File]::Move($pending, $target)
        Assert-Equal ([IO.File]::ReadAllText($target)) 'ok'
        [IO.File]::WriteAllText($pending, 'updated')
        [IO.File]::Replace($pending, $target, ($target + '.backup'))
        Assert-Equal ([IO.File]::ReadAllText($target)) 'updated'
    } finally {
        $nativeRoot = '\\?\' + $root
        if ([IO.Directory]::Exists($nativeRoot)) { [IO.Directory]::Delete($nativeRoot, $true) }
    }
}

Test-PathForms
Test-LongFilePublication
Write-Output 'Windows long-path tests passed.'
