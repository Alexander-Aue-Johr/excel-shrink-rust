$ErrorActionPreference = 'Stop'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot '..\run.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$helper = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-DotNet8' }, $true)
Invoke-Expression $helper.Extent.Text

function Assert([bool]$condition, [string]$message) {
    if (-not $condition) { throw $message }
}

$root = Join-Path $env:TEMP ('es-dotnet-test-' + [Guid]::NewGuid().ToString('N'))
$oldDotNet = $env:EXCEL_SHRINK_DOTNET
$oldLocalAppData = $env:LOCALAPPDATA
$script:globalDotNet = $null
function Get-Command {
    [CmdletBinding()] param([string]$Name)
    if ($Name -ne 'dotnet.exe') { throw "Unexpected command lookup: $Name" }
    if ($script:globalDotNet) { return [pscustomobject]@{ Source = $script:globalDotNet } }
}

function New-DotNet([string]$name, [string]$sdk, [string]$runtimes) {
    $path = Join-Path $root ($name + '.exe')
    Copy-Item -LiteralPath (Join-Path $root 'fixture.exe') -Destination $path
    [IO.File]::WriteAllText($path + '.sdk', $sdk)
    [IO.File]::WriteAllText($path + '.runtimes', $runtimes)
    return $path
}

function Assert-Rejected([string]$message) {
    $rejected = $false
    try { Get-DotNet8 | Out-Null }
    catch { $rejected = $_.Exception.Message -like '*Install the .NET 8 SDK*' }
    Assert $rejected $message
}

New-Item -ItemType Directory -Path $root | Out-Null
try {
    $env:LOCALAPPDATA = $root
    Add-Type -OutputAssembly (Join-Path $root 'fixture.exe') -OutputType ConsoleApplication -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
public static class DotNetFixture {
    public static int Main(string[] args) {
        string path = Process.GetCurrentProcess().MainModule.FileName;
        if (File.Exists(path + ".fail")) return 13;
        if (args.Length != 1) return 2;
        string suffix = args[0] == "--list-sdks" ? ".sdk" : args[0] == "--list-runtimes" ? ".runtimes" : null;
        if (suffix == null) return 2;
        Console.WriteLine(File.ReadAllText(path + suffix));
        return 0;
    }
}
'@
    $net8 = "Microsoft.NETCore.App 8.0.26 [C:\fixture\shared]`nMicrosoft.AspNetCore.App 8.0.26 [C:\fixture\shared]"
    $newer = New-DotNet 'newer' '9.0.313 [C:\fixture\sdk]' $net8
    $env:EXCEL_SHRINK_DOTNET = $newer
    Assert ((Get-DotNet8) -eq $newer) 'A newer SDK with both .NET 8 runtimes was rejected.'

    $current = New-DotNet 'current' '8.0.400 [C:\fixture\sdk]' $net8
    $env:EXCEL_SHRINK_DOTNET = $current
    Assert ((Get-DotNet8) -eq $current) 'The .NET 8 SDK was rejected.'

    $old = New-DotNet 'old' '7.0.400 [C:\fixture\sdk]' $net8
    $env:EXCEL_SHRINK_DOTNET = $old
    Assert-Rejected 'An SDK too old to build net8.0 was accepted.'

    $newRuntime = New-DotNet 'new-runtime' '9.0.313 [C:\fixture\sdk]' 'Microsoft.NETCore.App 9.0.15 [C:\fixture\shared]'
    $env:EXCEL_SHRINK_DOTNET = $newRuntime
    Assert-Rejected 'A newer runtime was incorrectly treated as a .NET 8 runtime.'

    $coreOnly = New-DotNet 'core-only' '9.0.313 [C:\fixture\sdk]' 'Microsoft.NETCore.App 8.0.26 [C:\fixture\shared]'
    $env:EXCEL_SHRINK_DOTNET = $coreOnly
    Assert-Rejected 'The capture web host was accepted without ASP.NET Core 8.'

    $script:globalDotNet = $newer
    Assert ((Get-DotNet8) -eq $newer) 'An incomplete override prevented fallback to a compatible SDK.'

    $notWindows = Join-Path $root 'not-windows.exe'
    [IO.File]::WriteAllText($notWindows, '#!/bin/sh')
    $env:EXCEL_SHRINK_DOTNET = $notWindows
    Assert ((Get-DotNet8) -eq $newer) 'A non-Windows executable prevented SDK discovery.'

    [IO.File]::WriteAllText($current + '.fail', '')
    $env:EXCEL_SHRINK_DOTNET = $current
    Assert ((Get-DotNet8) -eq $newer) 'A failed SDK probe prevented discovery of a working installation.'
    Write-Host 'PASS: SDK discovery accepts newer SDKs, checks .NET 8 runtimes, and skips unusable candidates.'
} finally {
    $env:EXCEL_SHRINK_DOTNET = $oldDotNet
    $env:LOCALAPPDATA = $oldLocalAppData
    $resolved = [IO.Path]::GetFullPath($root)
    $temporaryRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notlike 'es-dotnet-test-*') {
        throw "Refusing to remove an unexpected test directory: $resolved"
    }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
