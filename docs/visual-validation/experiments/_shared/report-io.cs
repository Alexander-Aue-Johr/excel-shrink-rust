using System;
using System.IO;
using System.Text;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using Microsoft.Win32.SafeHandles;

// Report-only IO for Windows PowerShell 5.1. No Office/WPF dependencies.
// Use Unicode Win32 APIs and extended UNC paths; do not change machine policy.
public sealed class VVReportEntry {
    public string FullName {get;set;}
    public string DirectoryName {get;set;}
    public string Name {get;set;}
    public string Extension {get;set;}
    public bool PSIsContainer {get;set;}
    public FileAttributes Attributes {get;set;}
    public long Length {get;set;}
    public DateTime LastWriteTimeUtc {get;set;}
}
public sealed class VVReportTransfer {
    public long Length {get;set;}
    public string Sha256 {get;set;}
}
public static class VVReportIO {
    const uint InvalidAttributes = 0xffffffff;
    static readonly IntPtr InvalidHandle = new IntPtr(-1);
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    struct FindData {
        public uint attributes;
        public System.Runtime.InteropServices.ComTypes.FILETIME creation, access, write;
        public uint sizeHigh, sizeLow, reserved0, reserved1;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=260)] public string name;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=14)] public string alternate;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern uint GetFullPathNameW(string name, uint size, StringBuilder buffer, IntPtr filePart);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern uint GetFileAttributesW(string name);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern IntPtr FindFirstFileW(string pattern, out FindData data);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern bool FindNextFileW(IntPtr handle, out FindData data);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool FindClose(IntPtr handle);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern SafeFileHandle CreateFileW(string name, uint access, uint share,
        IntPtr security, uint creation, uint flags, IntPtr template);
    static Exception Error(string operation, string path, int code) {
        return new IOException(operation + " failed (Win32 " + code + ", path length " +
            path.Length + "): " + path, new Win32Exception(code));
    }
    public static string FullPath(string path) {
        if(String.IsNullOrWhiteSpace(path) || path.IndexOf('\0')>=0)
            throw new ArgumentException("Empty or invalid filesystem path.");
        path=path.Replace('/','\\');
        if(path.StartsWith(@"\\?\UNC\",StringComparison.OrdinalIgnoreCase)) path=@"\\"+path.Substring(8);
        else if(path.StartsWith(@"\\?\",StringComparison.Ordinal)) {
            path=path.Substring(4);
            if(path.Length<3 || path[1]!=':' || path[2]!='\\')
                throw new ArgumentException("Unsupported device path.");
        }
        if(path.StartsWith(@"\\.\",StringComparison.Ordinal))
            throw new ArgumentException("Device paths are not report paths.");
        var buf=new StringBuilder(32768);
        uint n=GetFullPathNameW(path,(uint)buf.Capacity,buf,IntPtr.Zero);
        if(n==0)throw Error("GetFullPathNameW",path,Marshal.GetLastWin32Error());
        if(n>=buf.Capacity || n>32750)throw new PathTooLongException("Extended path is too long.");
        return buf.ToString();
    }
    static string Extended(string path) {
        string full=FullPath(path);
        return full.StartsWith(@"\\",StringComparison.Ordinal) ? @"\\?\UNC\"+full.Substring(2) : @"\\?\"+full;
    }
    public static bool FileExists(string path) {
        uint a=GetFileAttributesW(Extended(path));
        if(a==InvalidAttributes) {
            int e=Marshal.GetLastWin32Error();
            if(e==2 || e==3)return false;
            throw Error("GetFileAttributesW",path,e);
        }
        return (a & (uint)FileAttributes.Directory)==0;
    }
    public static FileStream OpenRead(string path) {
        // Readers never mutate input, artifact, or report files.
        var h=CreateFileW(Extended(path),0x80000000,7,IntPtr.Zero,3,0x08000000,IntPtr.Zero);
        if(h.IsInvalid) {int e=Marshal.GetLastWin32Error();h.Dispose();throw Error("CreateFileW",path,e);}
        try{return new FileStream(h,FileAccess.Read,65536,false);}
        catch {h.Dispose();throw;}
    }
    public static string ReadAllText(string path) {
        using(var s=OpenRead(path))
        using(var r=new StreamReader(s,Encoding.UTF8,true))return r.ReadToEnd();
    }
    public static IEnumerable<string> ReadLines(string path) {
        using(var s=OpenRead(path))
        using(var r=new StreamReader(s,Encoding.UTF8,true)) {
            string line;
            while((line=r.ReadLine())!=null)yield return line;
        }
    }
    public static string HashStream(Stream input) {
        using(var hash=SHA256.Create())return BitConverter.ToString(hash.ComputeHash(input)).Replace("-","").ToLowerInvariant();
    }
    public static string HashFile(string path) {
        using(var input=OpenRead(path))return HashStream(input);
    }
    public static VVReportTransfer CopyAndHash(Stream input, Stream output) {
        byte[] buffer=new byte[65536];long size=0;int n;
        using(var hash=SHA256.Create()) {
            while((n=input.Read(buffer,0,buffer.Length))>0) {
                output.Write(buffer,0,n);hash.TransformBlock(buffer,0,n,buffer,0);size+=n;
            }
            hash.TransformFinalBlock(new byte[0],0,0);
            return new VVReportTransfer{Length=size,Sha256=BitConverter.ToString(hash.Hash).Replace("-","").ToLowerInvariant()};
        }
    }
    public static VVReportEntry[] Children(string directory) {
        string full=FullPath(directory).TrimEnd('\\');
        if(full.Length==2 && full[1]==':')full+="\\";
        var results=new List<VVReportEntry>();FindData data;
        IntPtr h=FindFirstFileW(Extended(full.TrimEnd('\\')+@"\*"),out data);
        if(h==InvalidHandle) {
            int e=Marshal.GetLastWin32Error();
            if(e==2)return results.ToArray(); // Empty, existing directory.
            throw Error("FindFirstFileW",full,e);
        }
        try {
            do {
                if(data.name=="." || data.name=="..")continue;
                long ticks=((long)(uint)data.write.dwHighDateTime<<32)|(uint)data.write.dwLowDateTime;
                string name=data.name;
                int dot=name.LastIndexOf('.');
                results.Add(new VVReportEntry{FullName=full.TrimEnd('\\')+"\\"+name,
                    DirectoryName=full,Name=name,Extension=dot>=0?name.Substring(dot):"",
                    Attributes=(FileAttributes)data.attributes,PSIsContainer=(data.attributes&16)!=0,
                    Length=(long)(((ulong)data.sizeHigh<<32)|data.sizeLow),
                    LastWriteTimeUtc=DateTime.FromFileTimeUtc(ticks)});
            } while(FindNextFileW(h,out data));
            int e=Marshal.GetLastWin32Error();
            if(e!=18)throw Error("FindNextFileW",full,e);
        } finally {FindClose(h);}
        return results.ToArray();
    }
}
