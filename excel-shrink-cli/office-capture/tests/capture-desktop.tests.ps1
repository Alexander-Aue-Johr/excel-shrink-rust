$ErrorActionPreference = 'Stop'
$captureRoot = Split-Path -Parent $PSScriptRoot
Add-Type -Path @((Join-Path $captureRoot 'CaptureProcesses.cs'), (Join-Path $captureRoot 'CaptureDesktop.cs'))
$helper = @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public static class DesktopTest {
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint id);
    [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool GetUserObjectInformation(IntPtr handle, int index, StringBuilder value, int size, out int required);
    static string Name(IntPtr handle) {
        var result = new StringBuilder(256);
        int required;
        if (!GetUserObjectInformation(handle, 2, result, result.Capacity * 2, out required)) throw new Win32Exception();
        return result.ToString();
    }
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    public static string CurrentName() { return Name(GetThreadDesktop(GetCurrentThreadId())); }
    public static string InputName() {
        var handle = OpenInputDesktop(0, false, 1);
        try { return Name(handle); } finally { CloseDesktop(handle); }
    }
}
'@
Add-Type $helper
$root = Join-Path $env:TEMP ('es-desktop-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$helperPath = Join-Path $root 'DesktopTest.cs'
[IO.File]::WriteAllText($helperPath, $helper)
$resultPath = Join-Path $root 'desktop.txt'
$childScript = Join-Path $root 'child.ps1'
@'
param($HelperPath, $ResultPath)
Add-Type -Path $HelperPath
[IO.File]::WriteAllText($ResultPath, [DesktopTest]::CurrentName())
Start-Sleep 60
'@ | Set-Content -LiteralPath $childScript -Encoding UTF8
$desktop = New-Object CaptureDesktop
$owned = New-Object CaptureProcesses
$child = $null
try {
    $before = [DesktopTest]::InputName()
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "{0}" -HelperPath "{1}" -ResultPath "{2}"' -f $childScript, $helperPath, $resultPath
    $child = $desktop.Start((Join-Path $PSHOME 'powershell.exe'), $arguments, $owned)
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $resultPath)) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Child did not publish desktop identity.' }
        Start-Sleep -Milliseconds 100
    }
    $name = [IO.File]::ReadAllText($resultPath)
    if (-not $name.StartsWith('es-capture-')) { throw "Unexpected child desktop: $name" }
    if ($name -eq $before -or [DesktopTest]::InputName() -ne $before) { throw 'Input desktop was changed.' }
    $owned.Dispose()
    if (-not $child.WaitForExit(5000)) { throw 'Isolated process survived job cleanup.' }
    Write-Host 'PASS: child is isolated from keyboard input; input desktop unchanged; owned process terminated.'
} finally {
    $owned.Dispose()
    if ($null -ne $child) { $child.Dispose() }
    $desktop.Dispose()
    Remove-Item -LiteralPath $root -Recurse -Force
}
