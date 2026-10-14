using System;
using System.IO;
using System.Text;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;

// Windows-only support. The job owns only the worker and inherited descendants.
// COM servers created outside the job are NOT claimed as owned; the controller stops
// the suite if Excel or the reserved ports remain occupied after a case.
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
        if(!AssignProcessToJobObject(handle,process.Handle))throw new Win32Exception(Marshal.GetLastWin32Error());
    }
    public void Terminate() {if(handle!=IntPtr.Zero&&!TerminateJobObject(handle,124))throw new Win32Exception(Marshal.GetLastWin32Error());}
    public void Dispose() {if(handle!=IntPtr.Zero){CloseHandle(handle);handle=IntPtr.Zero;}GC.SuppressFinalize(this);}
    ~VVSuiteJob(){Dispose();}
}
public sealed class VVPixelDigest {
    public string Algorithm {get;set;}
    public string Sha256 {get;set;}
    public int Width {get;set;}
    public int Height {get;set;}
}
public static class VVSuitePixels {
    static uint BE(byte[] a,int p){return ((uint)a[p]<<24)|((uint)a[p+1]<<16)|((uint)a[p+2]<<8)|a[p+3];}
    public static VVPixelDigest Read(string path,long maxPixels) {
        using(var stream=File.OpenRead(path)) {
            var h=new byte[24];int got=0,n;
            while(got<h.Length&&(n=stream.Read(h,got,h.Length-got))>0)got+=n;
            var sig=new byte[]{137,80,78,71,13,10,26,10};
            if(got!=24)throw new InvalidDataException("Truncated PNG header");
            for(int i=0;i<8;i++)if(h[i]!=sig[i])throw new InvalidDataException("Not PNG");
            if(Encoding.ASCII.GetString(h,12,4)!="IHDR")throw new InvalidDataException("Missing IHDR");
            uint w=BE(h,16),height=BE(h,20);
            if(w==0||height==0||w>int.MaxValue/4||height>int.MaxValue||(ulong)w*height>(ulong)maxPixels)
                throw new InvalidDataException("Offline pixel verification budget exceeded");
            stream.Position=0;
            var decoder=BitmapDecoder.Create(stream,BitmapCreateOptions.PreservePixelFormat,BitmapCacheOption.OnLoad);
            if(decoder.Frames.Count!=1)throw new InvalidDataException("Expected one PNG frame");
            var frame=decoder.Frames[0];
            if(frame.PixelWidth!=(int)w||frame.PixelHeight!=(int)height)throw new InvalidDataException("PNG dimensions changed during decode");
            var pixels=new FormatConvertedBitmap(frame,PixelFormats.Bgra32,null,0);
            int stride=checked((int)w*4);var row=new byte[stride];
            using(var sha=SHA256.Create()) {
                byte[] prefix=Encoding.ASCII.GetBytes("vv-bgra32-v1\n"+w.ToString(System.Globalization.CultureInfo.InvariantCulture)+"x"+height.ToString(System.Globalization.CultureInfo.InvariantCulture)+"\n");
                sha.TransformBlock(prefix,0,prefix.Length,prefix,0);
                for(int y=0;y<(int)height;y++) {pixels.CopyPixels(new Int32Rect(0,y,(int)w,1),row,stride,0);sha.TransformBlock(row,0,row.Length,row,0);}
                sha.TransformFinalBlock(new byte[0],0,0);
                return new VVPixelDigest{Algorithm="vv-bgra32-v1",Sha256=BitConverter.ToString(sha.Hash).Replace("-","").ToLowerInvariant(),Width=(int)w,Height=(int)height};
            }
        }
    }
}
