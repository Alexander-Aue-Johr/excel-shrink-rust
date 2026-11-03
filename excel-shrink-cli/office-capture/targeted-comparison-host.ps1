# Persistent Excel sessions for the targeted comparison CLI. One process per lane.
$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
Add-Type -Path (Join-Path $PSScriptRoot 'CaptureProcesses.cs')
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ExcelProcessId {
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr handle, out uint processId);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint threadId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern bool GetUserObjectInformation(IntPtr handle, int index, System.Text.StringBuilder name, uint length, out uint needed);
    public static string DesktopName(IntPtr window) {
        uint pid, needed;
        var desktop = GetThreadDesktop(GetWindowThreadProcessId(window, out pid));
        var name = new System.Text.StringBuilder(256);
        if (!GetUserObjectInformation(desktop, 2, name, 512, out needed)) throw new System.ComponentModel.Win32Exception();
        return name.ToString();
    }
}
'@

Import-Module (Join-Path $PSScriptRoot 'CheckPerformance.psm1') -Force

Import-Module (Join-Path $PSScriptRoot 'TargetedResident.psm1') -Force -DisableNameChecking
Import-Module (Join-Path $PSScriptRoot 'TargetedPerformanceCache.psm1') -Force -DisableNameChecking
$ownedExcel = New-Object CaptureProcesses
try {
    while ($null -ne ($line = [Console]::ReadLine())) {
        try {
            $command = $line | ConvertFrom-Json
            $timer = [Diagnostics.Stopwatch]::StartNew()
            $before = Get-TargetedOperationSample $command
            $result = switch ($command.kind) {
                'cache_environment' { Get-TargetedCacheEnvironment }
                'open' { Open-TargetedWorkbook $command $ownedExcel }
                'open_batch' { Open-TargetedWorkbooks $command $ownedExcel }
                'close' { Close-TargetedWorkbook $command.lane }
                'capture_batch' { Invoke-TargetedCaptureBatch $command }
                'optimize' { Optimize-TargetedWorkbook $command }
                'save_as' { SaveAs-TargetedWorkbook $command }
                default { Invoke-TargetedJob $command }
            }
            $after = Get-TargetedOperationSample $command
            [Console]::WriteLine((@{ok=$true; result=$result; operation_measurements=@{
                kind=$command.kind; lane=$command.lane; elapsed_seconds=$timer.Elapsed.TotalSeconds
                path=$command.path; output=$command.output
                before=$before; after=$after
            }} | ConvertTo-Json -Depth 16 -Compress))
        } catch {
            $failure = $_.Exception.Message + ' ' + $_.ScriptStackTrace
            [Console]::WriteLine((@{ok=$false; error=$failure; operation_measurements=@{
                kind=$command.kind; lane=$command.lane; elapsed_seconds=$timer.Elapsed.TotalSeconds
                path=$command.path; output=$command.output
                before=$before; after=(Get-TargetedOperationSample $command); error=$failure
            }} | ConvertTo-Json -Depth 16 -Compress))
        }
    }
} finally {
    $ownedExcel.Dispose()
}
