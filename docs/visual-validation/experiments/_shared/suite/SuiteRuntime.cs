using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;

// Pure Windows Job Object process ownership. No WPF or image decoder is loaded
// when the comparison suite starts. Unrelated Excel processes are never owned.
public sealed class VVSuiteJob : IDisposable {
    [StructLayout(LayoutKind.Sequential)] struct BasicLimit {
        public long ProcessTime, JobTime; public uint Flags;
        public UIntPtr MinWorkingSet, MaxWorkingSet; public uint ActiveLimit;
        public UIntPtr Affinity; public uint Priority, Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] struct IoCounters {
        public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes;
    }
    [StructLayout(LayoutKind.Sequential)] struct ExtendedLimit {
        public BasicLimit Basic; public IoCounters Io;
        public UIntPtr ProcessMemory, JobMemory, PeakProcess, PeakJob;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr CreateJobObject(IntPtr attrs, string name);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool SetInformationJobObject(IntPtr job, int kind, IntPtr info, uint length);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool result);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool TerminateJobObject(IntPtr job, uint code);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool CloseHandle(IntPtr handle);
    IntPtr handle;
    public VVSuiteJob() {
        handle=CreateJobObject(IntPtr.Zero,null);
        if(handle==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        var info=new ExtendedLimit();info.Basic.Flags=0x00002000; // KILL_ON_JOB_CLOSE
        int size=Marshal.SizeOf(typeof(ExtendedLimit));IntPtr p=Marshal.AllocHGlobal(size);
        try {Marshal.StructureToPtr(info,p,false);if(!SetInformationJobObject(handle,9,p,(uint)size))throw new Win32Exception(Marshal.GetLastWin32Error());}
        catch {Dispose();throw;} finally {Marshal.FreeHGlobal(p);}
    }
    public void Assign(Process process) {
        if(handle==IntPtr.Zero)throw new ObjectDisposedException("VVSuiteJob");
        bool already;
        if(!IsProcessInJob(process.Handle,handle,out already))throw new Win32Exception(Marshal.GetLastWin32Error());
        if(already)return;
        if(!AssignProcessToJobObject(handle,process.Handle))throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    public void Terminate() {if(handle!=IntPtr.Zero&&!TerminateJobObject(handle,124))throw new Win32Exception(Marshal.GetLastWin32Error());}
    public void Dispose() {if(handle!=IntPtr.Zero){CloseHandle(handle);handle=IntPtr.Zero;}GC.SuppressFinalize(this);}
    ~VVSuiteJob(){Dispose();}
}
