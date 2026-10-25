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
        return Start(executable, arguments, false);
    }

    public Process Start(string executable, string arguments, bool hidden)
    {
        if (executable.Contains("\"")) throw new ArgumentException("Invalid executable path");
        var startup = new StartupInfo();
        startup.Size = Marshal.SizeOf(typeof(StartupInfo));
        // Office.js initializes on the interactive desktop. Hidden mode keeps
        // the window hidden from startup while retaining that desktop.
        startup.Flags = 1; // STARTF_USESHOWWINDOW
        startup.ShowWindow = (short)(hidden ? 0 : 1); // SW_HIDE / SW_SHOWNORMAL
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

    public static void Hide(Process process)
    {
        process.Refresh();
        if (!process.HasExited)
        {
            var window = process.MainWindowHandle;
            if (window != IntPtr.Zero) ShowWindowAsync(window, 0); // SW_HIDE
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

    public sealed class MemorySample
    {
        public ulong PrivateBytes;
        public ulong? PrivateWorkingSetBytes;
        public ProcessSample[] Processes;
    }

    public sealed class ProcessSample
    {
        public int Pid;
        public string Name;
        public long StartedUnixMilliseconds;
        public long PrivateBytes;
        public long WorkingSetBytes;
    }

    // Enumerate the job rather than the launcher's Excel list: WebView children
    // and the HTTP host consume the same budget. The OS peak is not current use.
    public MemorySample SampleMemory()
    {
        for (int capacity = 64; capacity <= 65536; capacity *= 2)
        {
            int length = checked(8 + capacity * IntPtr.Size);
            IntPtr buffer = Marshal.AllocHGlobal(length);
            try
            {
                if (!QueryInformationJobObject(job, 3, buffer, (uint)length, IntPtr.Zero))
                {
                    int error = Marshal.GetLastWin32Error();
                    if (error == 234) continue; // ERROR_MORE_DATA: a child joined the job.
                    throw new Win32Exception(error);
                }
                int count = Marshal.ReadInt32(buffer, 4);
                if (count < 0 || count > capacity) throw new InvalidOperationException("Invalid job process list.");
                ulong bytes = 0;
                ulong? privateWorkingSet = 0;
                var processes = new System.Collections.Generic.List<ProcessSample>();
                for (int index = 0; index < count; index++)
                {
                    int pid = checked((int)Marshal.ReadIntPtr(buffer, 8 + index * IntPtr.Size).ToInt64());
                    try
                    {
                        using (var process = Process.GetProcessById(pid))
                        {
                            bool member;
                            if (!IsProcessInJob(process.Handle, job, out member)) throw new Win32Exception();
                            if (!member || process.HasExited) continue;
                            process.Refresh();
                            bytes = checked(bytes + (ulong)process.PrivateMemorySize64);
                            processes.Add(new ProcessSample { Pid = pid, Name = process.ProcessName,
                                StartedUnixMilliseconds = new DateTimeOffset(process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds(),
                                PrivateBytes = process.PrivateMemorySize64, WorkingSetBytes = process.WorkingSet64 });
                            // EX2 is supported on updated Windows 10/11 (September
                            // 2023 onward). Older systems keep conservative RAM
                            // reserves: never substitute the shared working set.
                            if (privateWorkingSet.HasValue)
                            {
                                var counters = new ProcessMemoryCountersEx2();
                                counters.Size = (uint)Marshal.SizeOf(typeof(ProcessMemoryCountersEx2));
                                counters.PrivateWorkingSet = new UIntPtr(UInt32.MaxValue);
                                if (GetProcessMemoryInfo(process.Handle, ref counters, counters.Size)
                                    && counters.PrivateWorkingSet.ToUInt64() != UInt32.MaxValue
                                    && counters.PrivateWorkingSet.ToUInt64() <= counters.WorkingSet.ToUInt64())
                                    privateWorkingSet = checked(privateWorkingSet.Value + counters.PrivateWorkingSet.ToUInt64());
                                else privateWorkingSet = null;
                            }
                        }
                    }
                    catch (ArgumentException) { } // Exited between enumeration and opening the handle.
                    catch (InvalidOperationException) { } // Exited while reading its counters.
                }
                return new MemorySample { PrivateBytes = bytes, PrivateWorkingSetBytes = privateWorkingSet, Processes = processes.ToArray() };
            }
            finally { Marshal.FreeHGlobal(buffer); }
        }
        throw new InvalidOperationException("Resident job process list exceeds its measurement budget.");
    }

    public static ulong AvailablePhysicalBytes { get { return GetMemory().AvailablePhysical; } }
    public static ulong AvailableCommitBytes { get { return GetMemory().AvailablePage; } }
    static MemoryStatus GetMemory()
    {
        var info = new MemoryStatus { Length = (uint)Marshal.SizeOf(typeof(MemoryStatus)) };
        if (!GlobalMemoryStatusEx(ref info)) throw new Win32Exception();
        return info;
    }

    // Diagnostic snapshot only: never select termination targets by PID/name.
    // The Job Object remains the authority for cleanup ownership.
    public string[] ProcessIdentities()
    {
        var identities = new System.Collections.Generic.List<string>();
        foreach (var process in Process.GetProcesses())
        {
            using (process)
            {
                try
                {
                    bool member;
                    if (IsProcessInJob(process.Handle, job, out member) && member && !process.HasExited)
                        identities.Add(process.Id + "|" + process.ProcessName + "|" +
                            process.StartTime.ToUniversalTime().ToString("o"));
                }
                catch (Win32Exception) { }
                catch (InvalidOperationException) { }
            }
        }
        return identities.ToArray();
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
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessMemoryCountersEx2
    {
        public uint Size, PageFaultCount;
        public UIntPtr PeakWorkingSet, WorkingSet, QuotaPeakPagedPool, QuotaPagedPool;
        public UIntPtr QuotaPeakNonPagedPool, QuotaNonPagedPool, PagefileUsage, PeakPagefileUsage;
        public UIntPtr PrivateUsage, PrivateWorkingSet;
        public ulong SharedCommitUsage;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(SafeFileHandle job, int informationClass, ref ExtendedLimits information, uint length);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(SafeFileHandle job, int informationClass, ref ExtendedLimits information, uint length, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(SafeFileHandle job, int informationClass, IntPtr information, uint length, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool IsProcessInJob(IntPtr process, SafeFileHandle job, out bool member);
    [DllImport("psapi.dll", SetLastError = true)]
    static extern bool GetProcessMemoryInfo(IntPtr process, ref ProcessMemoryCountersEx2 counters, uint length);
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
