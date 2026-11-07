using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

// Position only the two explicitly owned inspection windows on the pointer's monitor.
public static class InspectionWindows
{
    [StructLayout(LayoutKind.Sequential)]
    struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    struct MonitorInfo { public int Size; public Rect Monitor, Work; public uint Flags; }

    [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(Point point, uint flags);
    [DllImport("user32.dll", SetLastError=true)] static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    [DllImport("user32.dll", SetLastError=true)] static extern bool MoveWindow(IntPtr window, int x, int y, int width, int height, bool repaint);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);

    public static void Arrange(IntPtr left, IntPtr right)
    {
        var previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
        try {
            Point pointer;
            if (!GetCursorPos(out pointer)) throw new Win32Exception();
            var info = new MonitorInfo { Size = Marshal.SizeOf(typeof(MonitorInfo)) };
            if (!GetMonitorInfo(MonitorFromPoint(pointer, 2), ref info)) throw new Win32Exception();
            int width = info.Work.Right - info.Work.Left;
            int height = info.Work.Bottom - info.Work.Top;
            ShowWindow(left, 9);
            ShowWindow(right, 9);
            if (!MoveWindow(left, info.Work.Left, info.Work.Top, width / 2, height, true)
                || !MoveWindow(right, info.Work.Left + width / 2, info.Work.Top, width - width / 2, height, true))
                throw new Win32Exception();
        } finally {
            if (previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous);
        }
    }
}
