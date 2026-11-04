using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

/// <summary>Read native dialog handles only for the requested Excel PID on this desktop.</summary>
public static class TargetedOpeningDialogs
{
    delegate bool Visitor(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(Visitor visitor, IntPtr parameter);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr window, StringBuilder name, int count);

    public static IntPtr[] Find(int pid)
    {
        var result = new List<IntPtr>();
        EnumWindows((window, parameter) => {
            uint actualPid;
            GetWindowThreadProcessId(window, out actualPid);
            if (actualPid != (uint)pid) return true;
            var name = new StringBuilder(128);
            GetClassName(window, name, name.Capacity);
            if (name.ToString() == "NUIDialog" || name.ToString() == "#32770")
                result.Add(window);
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }
}