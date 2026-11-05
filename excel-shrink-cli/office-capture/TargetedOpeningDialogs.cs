using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

/// <summary>Bounded, read-only observations of dialogs belonging to one owned Excel PID.</summary>
public static class TargetedOpeningDialogs
{
    public sealed class Dialog
    {
        public long Handle;
        public string ClassName;
        public string[] Names;
        public bool IsModal;
        public bool HasPasswordInput;
        public bool HasProgressBar;
        public int DecisionButtons;
    }

    delegate bool Visitor(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumDesktopWindows(IntPtr desktop, Visitor visitor, IntPtr parameter);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint thread);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr OpenDesktop(string name, uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, Visitor visitor, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongW")] static extern int GetWindowLong(IntPtr window, int index);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetClassName(IntPtr window, StringBuilder name, int count);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetWindowText(IntPtr window, StringBuilder text, int count);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr count,
        StringBuilder text, uint flags, uint timeout, out IntPtr result);

    static string ClassName(IntPtr window)
    {
        var name = new StringBuilder(128);
        GetClassName(window, name, name.Capacity);
        return name.ToString();
    }

    static bool IsDialogClass(string name)
    {
        return name == "NUIDialog" || name == "#32770"
            || name.StartsWith("bosa_sdm_", StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>Inspect the explicit private desktop without switching the user's input desktop.
    /// Only visible classic dialogs or owned modal windows are returned. Child reads have a total 500 ms budget;
    /// password fields and other editable inputs are never read.</summary>
    public static Dialog[] Inspect(int pid, string desktopName)
    {
        var result = new List<Dialog>();
        bool ownsHandle = !String.IsNullOrEmpty(desktopName);
        IntPtr desktop = ownsHandle ? OpenDesktop(desktopName, 0, false, 0x0041)
            : GetThreadDesktop(GetCurrentThreadId());
        if (desktop == IntPtr.Zero) throw new Win32Exception();
        var clock = Stopwatch.StartNew();
        try
        {
            EnumDesktopWindows(desktop, (window, parameter) => {
                uint actualPid;
                GetWindowThreadProcessId(window, out actualPid);
                if (actualPid != (uint)pid || !IsWindowVisible(window)) return true;
                string className = ClassName(window);
                IntPtr owner = GetWindow(window, 4); // GW_OWNER
                uint ownerPid;
                GetWindowThreadProcessId(owner, out ownerPid);
                bool modal = owner != IntPtr.Zero && ownerPid == (uint)pid && !IsWindowEnabled(owner);
                // Embedded Office sign-in windows do not use the classic dialog classes.
                if (!IsDialogClass(className) && !modal) return true;
                if (result.Count >= 8 || clock.ElapsedMilliseconds >= 500) return false;
                var names = new List<string>();
                var title = new StringBuilder(512);
                GetWindowText(window, title, title.Capacity);
                if (title.Length != 0) names.Add(title.ToString());
                var dialog = new Dialog {
                    Handle = window.ToInt64(), ClassName = className,
                    IsModal = modal
                };
                int visited = 0;
                EnumChildWindows(window, (child, unused) => {
                    if (++visited > 64 || clock.ElapsedMilliseconds >= 500) return false;
                    uint childPid;
                    GetWindowThreadProcessId(child, out childPid);
                    if (childPid != (uint)pid || !IsWindowVisible(child)) return true;
                    string childClass = ClassName(child);
                    if (childClass.IndexOf("progress", StringComparison.OrdinalIgnoreCase) >= 0)
                        dialog.HasProgressBar = true;
                    if (childClass.IndexOf("Edit", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        dialog.HasPasswordInput |= (GetWindowLong(child, -16) & 0x20) != 0; // ES_PASSWORD
                        return true;
                    }
                    if (childClass == "Button" && IsWindowEnabled(child))
                    {
                        int buttonType = GetWindowLong(child, -16) & 0x0f;
                        if (buttonType == 0 || buttonType == 1 || buttonType == 14 || buttonType == 15)
                            dialog.DecisionButtons++;
                    }
                    var text = new StringBuilder(512);
                    IntPtr received;
                    // WM_GETTEXT, SMTO_ABORTIFHUNG: no synchronous UI Automation provider calls.
                    if (SendMessageTimeout(child, 0x000D, (IntPtr)text.Capacity, text, 0x0002, 25, out received) != IntPtr.Zero
                        && text.Length != 0) names.Add(text.ToString());
                    return true;
                }, IntPtr.Zero);
                dialog.Names = names.ToArray();
                result.Add(dialog);
                return true;
            }, IntPtr.Zero);
        }
        finally { if (ownsHandle) CloseDesktop(desktop); }
        return result.ToArray();
    }

    // Compatibility for existing diagnostic callers.
    public static IntPtr[] Find(int pid)
    {
        var handles = new List<IntPtr>();
        foreach (var dialog in Inspect(pid, Environment.GetEnvironmentVariable("EXCEL_SHRINK_EXPECTED_DESKTOP")))
            handles.Add(new IntPtr(dialog.Handle));
        return handles.ToArray();
    }
}
