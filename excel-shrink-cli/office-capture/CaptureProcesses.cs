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
        limits.Basic.LimitFlags = 0x2000; // KILL_ON_JOB_CLOSE
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

    public static bool Stop(Process process)
    {
        // Add() retained this handle before the child ran. Never send WM_CLOSE:
        // a busy Excel window can block that synchronous GUI call indefinitely.
        // The private workbook copy does not need to be saved on shutdown.
        IntPtr handle = process.Handle;
        if (WaitForSingleObject(handle, 0) == 0) return true;
        if (!TerminateProcess(handle, 0))
        {
            int error = Marshal.GetLastWin32Error();
            if (WaitForSingleObject(handle, 0) != 0) throw new Win32Exception(error);
        }
        return WaitForSingleObject(handle, 3000) == 0;
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
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateProcess(IntPtr process, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(IntPtr process, uint milliseconds);
}
