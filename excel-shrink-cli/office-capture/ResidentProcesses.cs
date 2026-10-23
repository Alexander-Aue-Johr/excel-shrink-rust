using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

// A resident session keeps several private workbook copies alive. Assign Excel
// before its first instruction so its Office/WebView children share the budget
// and are closed if the launcher is interrupted.
public sealed class ResidentProcesses : IDisposable
{
    readonly SafeFileHandle job;

    public ResidentProcesses(int memoryMiB)
    {
        if (memoryMiB < 1024 || memoryMiB > 65536)
            throw new ArgumentOutOfRangeException("memoryMiB");
        job = new SafeFileHandle(CreateJobObject(IntPtr.Zero, null), true);
        if (job.IsInvalid) throw new Win32Exception();
        var limits = new ExtendedLimits();
        limits.JobMemory = new UIntPtr((ulong)memoryMiB * 1024 * 1024);
        limits.Basic.LimitFlags = 0x2200; // KILL_ON_JOB_CLOSE | JOB_MEMORY
        if (!SetInformationJobObject(job, 9, ref limits, (uint)Marshal.SizeOf(typeof(ExtendedLimits))))
        {
            var error = new Win32Exception();
            job.Dispose();
            throw error;
        }
    }

    public void Add(Process process)
    {
        if (!AssignProcessToJobObject(job, process.Handle)) throw new Win32Exception();
    }

    public Process Start(string executable, string arguments)
    {
        if (executable.Contains("\"")) throw new ArgumentException("Invalid executable path");
        var startup = new StartupInfo();
        startup.Size = Marshal.SizeOf(typeof(StartupInfo));
        // Use the interactive desktop: background desktops can stall Office.js.
        // Initially show the window so Excel can initialize its task pane; the
        // launcher minimizes it after the first configuration request.
        startup.Flags = 1; // STARTF_USESHOWWINDOW
        startup.ShowWindow = 1; // SW_SHOWNORMAL
        ProcessInfo info;
        if (!CreateProcess(executable, new StringBuilder("\"" + executable + "\" " + arguments),
            IntPtr.Zero, IntPtr.Zero, false, 0x00000004, IntPtr.Zero, null, ref startup, out info))
            throw new Win32Exception();
        try
        {
            var process = Process.GetProcessById((int)info.ProcessId);
            try
            {
                Add(process);
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

    public static void Minimize(Process process)
    {
        process.Refresh();
        if (!process.HasExited)
        {
            var window = process.MainWindowHandle;
            if (window != IntPtr.Zero && !IsIconic(window)) ShowWindowAsync(window, 6);
        }
    }

    public static void Stop(Process process)
    {
        process.Refresh();
        if (!process.HasExited && !TerminateProcess(process.Handle, 0)) throw new Win32Exception();
        process.WaitForExit(3000);
    }

    public static void PublishFile(string pending, string target)
    {
        // PowerShell 5 binds a null File.Replace backup path as an empty string.
        if (System.IO.File.Exists(target)) System.IO.File.Replace(pending, target, null);
        else System.IO.File.Move(pending, target);
    }

    public static void PublishOnce(string pending, string target)
    {
        try { System.IO.File.Move(pending, target); }
        catch (System.IO.IOException)
        {
            // Rust and the launcher may independently announce completion. Keep
            // the first marker, including its original time for the wait bound.
            if (!System.IO.File.Exists(target)) throw;
            System.IO.File.Delete(pending);
        }
    }

    public ulong PeakBytes
    {
        get
        {
            var info = new ExtendedLimits();
            if (!QueryInformationJobObject(job, 9, ref info, (uint)Marshal.SizeOf(typeof(ExtendedLimits)), IntPtr.Zero))
                throw new Win32Exception();
            return info.PeakJobMemory.ToUInt64();
        }
    }

    public static ulong AvailablePhysicalBytes { get { return GetMemory().AvailablePhysical; } }
    public static ulong AvailableCommitBytes { get { return GetMemory().AvailablePage; } }
    static MemoryStatus GetMemory()
    {
        var info = new MemoryStatus { Length = (uint)Marshal.SizeOf(typeof(MemoryStatus)) };
        if (!GlobalMemoryStatusEx(ref info)) throw new Win32Exception();
        return info;
    }

    public void Dispose() { job.Dispose(); }

    [StructLayout(LayoutKind.Sequential)]
    struct MemoryStatus
    {
        public uint Length, Load;
        public ulong TotalPhysical, AvailablePhysical, TotalPage, AvailablePage;
        public ulong TotalVirtual, AvailableVirtual, Extended;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct BasicLimits
    {
        public long ProcessTime, JobTime;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSet, MaximumWorkingSet;
        public uint ActiveProcesses;
        public UIntPtr Affinity;
        public uint PriorityClass, SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct IoCounters
    {
        public ulong ReadOperations, WriteOperations, OtherOperations;
        public ulong ReadBytes, WriteBytes, OtherBytes;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ExtendedLimits
    {
        public BasicLimits Basic;
        public IoCounters Io;
        public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
    }
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

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(SafeFileHandle job, int informationClass, ref ExtendedLimits information, uint length);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(SafeFileHandle job, int informationClass, ref ExtendedLimits information, uint length, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(SafeFileHandle job, IntPtr process);
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
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GlobalMemoryStatusEx(ref MemoryStatus status);
    [DllImport("user32.dll")]
    static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")]
    static extern bool ShowWindowAsync(IntPtr window, int command);
}
