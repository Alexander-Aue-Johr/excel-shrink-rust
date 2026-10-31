using System;
using System.Runtime.InteropServices;
public static class NativeWindow
{
    private const uint LeftButtonDown = 2;
    private const uint LeftButtonUp = 4;

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
