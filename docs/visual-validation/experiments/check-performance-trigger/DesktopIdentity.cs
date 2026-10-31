using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public static class DesktopIdentity
{
    [DllImport("user32.dll")]
    private static extern IntPtr GetThreadDesktop(uint thread);
    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll")]
    private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")]
    private static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool GetUserObjectInformation(
        IntPtr handle, int index, StringBuilder value, int size, out int required);

    private static string Name(IntPtr handle)
    {
        var value = new StringBuilder(256);
        int required;
        if (!GetUserObjectInformation(handle, 2, value, value.Capacity * 2, out required))
            throw new Win32Exception();
        return value.ToString();
    }

    public static string CurrentName() { return Name(GetThreadDesktop(GetCurrentThreadId())); }

    public static string WindowName(IntPtr window)
    {
        uint pid;
        return Name(GetThreadDesktop(GetWindowThreadProcessId(window, out pid)));
    }

    public static string InputName()
    {
        IntPtr desktop = OpenInputDesktop(0, false, 1);
        try { return Name(desktop); }
        finally { CloseDesktop(desktop); }
    }
}
