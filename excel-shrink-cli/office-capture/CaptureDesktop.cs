using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

// Excel still has a rendered desktop, but keyboard input stays on the user's desktop.
public sealed class CaptureDesktop : IDisposable
{
    readonly string name = "es-capture-" + Guid.NewGuid().ToString("N");
    readonly IntPtr handle;

    public string Name { get { return name; } }

    public CaptureDesktop()
    {
        handle = CreateDesktop(name, null, IntPtr.Zero, 0, 0x01ff, IntPtr.Zero);
        if (handle == IntPtr.Zero) throw new Win32Exception();
    }

    public Process Start(string executable, string arguments, CaptureProcesses owned)
    {
        if (executable.Contains("\"")) throw new ArgumentException("Invalid executable path");
        var startup = new StartupInfo();
        startup.Size = Marshal.SizeOf(typeof(StartupInfo));
        startup.Desktop = name;
        var command = new StringBuilder("\"" + executable + "\" " + arguments);
        ProcessInfo info;
        // Assign the process to the cleanup job before it can create children.
        if (!CreateProcess(executable, command, IntPtr.Zero, IntPtr.Zero, false,
            0x00000004, IntPtr.Zero, null, ref startup, out info)) throw new Win32Exception();
        try
        {
            var process = Process.GetProcessById((int)info.ProcessId);
            try
            {
                owned.Add(process);
                if (ResumeThread(info.Thread) == UInt32.MaxValue) throw new Win32Exception();
                return process;
            }
            catch
            {
                TerminateProcess(info.Process, 1);
                process.Dispose();
                throw;
            }
        }
        finally
        {
            CloseHandle(info.Thread);
            CloseHandle(info.Process);
        }
    }

    public void Dispose() { CloseDesktop(handle); }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo
    {
        public int Size;
        public string Reserved, Desktop, Title;
        public int X, Y, Width, Height, Columns, Rows, Fill, Flags;
        public short ShowWindow, ReservedSize;
        public IntPtr ReservedBytes, Input, Output, Error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo
    {
        public IntPtr Process, Thread;
        public uint ProcessId, ThreadId;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateDesktop(string name, string device, IntPtr mode, uint flags, uint access, IntPtr security);
    [DllImport("user32.dll", SetLastError = true)]
    static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string application, StringBuilder command, IntPtr processAttributes,
        IntPtr threadAttributes, bool inheritHandles, uint flags, IntPtr environment, string directory,
        ref StartupInfo startup, out ProcessInfo process);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr handle);
}
