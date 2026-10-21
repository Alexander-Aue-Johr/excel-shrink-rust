using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

// The OS closes this job when the launcher exits, including an interrupted run.
// Only processes started by this capture are assigned to it.
public sealed class CaptureProcesses : IDisposable
{
    readonly SafeFileHandle job;

    public CaptureProcesses()
    {
        job = new SafeFileHandle(CreateJobObject(IntPtr.Zero, null), true);
        if (job.IsInvalid) throw new Win32Exception();
        var limits = new ExtendedLimits();
        uint memoryMiB = 4096;
        var configured = Environment.GetEnvironmentVariable("EXCEL_SHRINK_EXCEL_MEMORY_MIB");
        if (configured != null && (!UInt32.TryParse(configured, out memoryMiB) || memoryMiB < 256 || memoryMiB > 65536))
        {
            job.Dispose();
            throw new ArgumentException("EXCEL_SHRINK_EXCEL_MEMORY_MIB must be between 256 and 65536.");
        }
        // Bound the combined private memory of Excel, its add-ins and the host.
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

    public void Dispose() { job.Dispose(); }

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

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(SafeFileHandle job, int informationClass, ref ExtendedLimits information, uint length);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(SafeFileHandle job, IntPtr process);
}
