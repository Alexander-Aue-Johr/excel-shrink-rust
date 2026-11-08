using System;
using System.Runtime.InteropServices;

public static class ExcelProcessId
{
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr handle, out uint processId);

    [DllImport("user32.dll")]
    private static extern IntPtr GetThreadDesktop(uint threadId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool GetUserObjectInformation(
        IntPtr handle, int index, System.Text.StringBuilder name, uint length, out uint needed);

    public static string DesktopName(IntPtr window)
    {
        uint pid, needed;
        var desktop = GetThreadDesktop(GetWindowThreadProcessId(window, out pid));
        var name = new System.Text.StringBuilder(256);
        if (!GetUserObjectInformation(desktop, 2, name, 512, out needed))
            throw new System.ComponentModel.Win32Exception();
        return name.ToString();
    }
}
