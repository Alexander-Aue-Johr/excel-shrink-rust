using System;
using System.Runtime.InteropServices;
public static class NativeWindow
{
    private const uint LeftButtonDown = 2;
    private const uint LeftButtonUp = 4;
    private const uint RootAncestor = 2;
    private const uint AbortIfHung = 2;
    private const uint MessageTimeoutMilliseconds = 2000;

    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);

    [DllImport("user32.dll")]
    private static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll")]
    private static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);

    [StructLayout(LayoutKind.Sequential)]
    private struct Point { public int X, Y; }

    [DllImport("user32.dll")]
    private static extern IntPtr WindowFromPoint(Point point);

    [DllImport("user32.dll")]
    private static extern IntPtr GetAncestor(IntPtr window, uint flags);

    [DllImport("user32.dll")]
    private static extern bool ScreenToClient(IntPtr window, ref Point point);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr SendMessageTimeout(
        IntPtr window, uint message, UIntPtr wParam, IntPtr lParam,
        uint flags, uint timeout, out UIntPtr result);

    private delegate bool EnumerateWindow(IntPtr window, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumerateWindow callback, IntPtr parameter);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, System.Text.StringBuilder name, int maximum);

    /// <summary>Find native child panes of the exact owned Excel window without walking worksheet accessibility cells.</summary>
    public static IntPtr[] FindChildWindows(IntPtr owner, int pid, string className)
    {
        uint ownerPid;
        GetWindowThreadProcessId(owner, out ownerPid);
        if (owner == IntPtr.Zero || ownerPid != pid)
            throw new InvalidOperationException("Pane lookup requires the owned Excel window.");
        var windows = new System.Collections.Generic.List<IntPtr>();
        EnumerateWindow callback = delegate(IntPtr window, IntPtr parameter)
        {
            uint actualPid;
            GetWindowThreadProcessId(window, out actualPid);
            var name = new System.Text.StringBuilder(256);
            GetClassName(window, name, name.Capacity);
            if (actualPid == pid && GetAncestor(window, RootAncestor) == owner && name.ToString() == className)
                windows.Add(window);
            return true;
        };
        EnumChildWindows(owner, callback, IntPtr.Zero);
        return windows.ToArray();
    }
    /// <summary>Send window messages to the owned button host without global input.</summary>
    public static void SendClick(IntPtr owner, int pid, int x, int y)
    {
        var point = new Point { X = x, Y = y };
        IntPtr target = WindowFromPoint(point);
        uint actualPid;
        GetWindowThreadProcessId(target, out actualPid);
        if (actualPid != pid || GetAncestor(target, RootAncestor) != owner)
            throw new InvalidOperationException("Button host is not inside owned Excel.");
        if (!ScreenToClient(target, ref point))
            throw new InvalidOperationException("Cannot translate button coordinates.");
        IntPtr coordinates = new IntPtr((point.X & 0xffff) | ((point.Y & 0xffff) << 16));
        SendPointerMessage(target, 0x0200, 0, coordinates); // WM_MOUSEMOVE
        SendPointerMessage(target, 0x0201, 1, coordinates); // WM_LBUTTONDOWN, MK_LBUTTON
        SendPointerMessage(target, 0x0202, 0, coordinates); // WM_LBUTTONUP
    }

    private static void SendPointerMessage(IntPtr target, uint message, uint buttons, IntPtr coordinates)
    {
        UIntPtr result;
        if (SendMessageTimeout(target, message, new UIntPtr(buttons), coordinates,
            AbortIfHung, MessageTimeoutMilliseconds, out result) == IntPtr.Zero)
            throw new InvalidOperationException("Excel did not accept the pointer message in time.");
    }

    public static void Click(IntPtr window, int pid, int x, int y)
    {
        SetForegroundWindow(window);
        System.Threading.Thread.Sleep(200);
        uint actualPid;
        GetWindowThreadProcessId(GetForegroundWindow(), out actualPid);
        if (actualPid != pid) throw new InvalidOperationException("Owned Excel is not foreground.");
        if (!SetCursorPos(x, y)) throw new InvalidOperationException("Cannot position pointer.");
        mouse_event(LeftButtonDown, 0, 0, 0, UIntPtr.Zero);
        mouse_event(LeftButtonUp, 0, 0, 0, UIntPtr.Zero);
    }
}
