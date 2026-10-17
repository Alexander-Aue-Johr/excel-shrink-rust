using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Runtime.InteropServices;
using System.ComponentModel;
using System.Diagnostics;
using Microsoft.Win32.SafeHandles;

// Shared by Windows PowerShell 5.1 and the ASP.NET host. No Office/WPF/package dependency.
// Publish a closed, unique sibling by rename. Never copy over or delete the live snapshot.
// No promise of power-loss durability across arbitrary remote filesystems is made.
public sealed class VVSnapshotReceipt {
    public string Path { get; set; }
    public long Bytes { get; set; }
    public int Attempts { get; set; }
}
public sealed class VVSnapshotIOException : IOException {
    public int NativeCode { get; private set; }
    public int Attempts { get; private set; }
    public string Operation { get; private set; }
    public string TargetPath { get; private set; }
    public string PendingPath { get; private set; }
    public bool PendingComplete { get; private set; }
    public VVSnapshotIOException(string op, string path, string pending, bool complete,
        int attempts, Exception inner) : base(
            "VV_SNAPSHOT_IO: " + op + " failed after " + attempts + " attempt(s); Win32 " +
            VVSnapshotFile.NativeCode(inner) + "; target=" + path +
            (pending == null ? "" : "; pending=" + pending + "; pending_complete=" + complete), inner) {
        NativeCode=VVSnapshotFile.NativeCode(inner);Attempts=attempts;Operation=op;
        TargetPath=path;PendingPath=pending;PendingComplete=complete;
    }
}
public static class VVSnapshotFile {
    static readonly int[] Delays = {20, 40, 80, 160, 320, 500};
    static long readRetries, publishRetries;
    public static long ReadRetries { get { return Interlocked.Read(ref readRetries); } }
    public static long PublishRetries { get { return Interlocked.Read(ref publishRetries); } }
    public const string Policy = "closed-sibling-rename-v1";

    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern uint GetFullPathNameW(string path, uint count, StringBuilder buffer, IntPtr filePart);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    static extern SafeFileHandle CreateFileW(string path, uint access, uint share, IntPtr security,
        uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, ExactSpelling=true, SetLastError=true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool MoveFileExW(string source, string target, uint flags);

    public static int NativeCode(Exception error) {
        for(int depth=0;error!=null && depth<24;depth++,error=error.InnerException) {
            var native=error as Win32Exception;
            if(native!=null)return native.NativeErrorCode;
            var snapshot=error as VVSnapshotIOException;
            if(snapshot!=null)return snapshot.NativeCode;
            uint hr=unchecked((uint)error.HResult);
            if((hr & 0xffff0000U)==0x80070000U)return (int)(hr & 0xffffU);
        }
        return 0;
    }
    public static bool IsSharingConflict(Exception error) {
        int code=NativeCode(error);return code==32 || code==33;
    }
    static bool Retryable(Exception error, bool publication) {
        int code=NativeCode(error);
        // ACCESS_DENIED can accompany replacement while another reader denies delete access.
        // Retrying is bounded; persistent permissions are never bypassed or called success.
        return code==32 || code==33 || (publication && code==5);
    }
    static string Absolute(string path) {
        if(Environment.OSVersion.Platform!=PlatformID.Win32NT)
            throw new PlatformNotSupportedException("Windows snapshot publication requires Windows.");
        if(String.IsNullOrWhiteSpace(path) || path.IndexOf('\0')>=0)
            throw new ArgumentException("Invalid snapshot path");
        path=path.Replace('/','\\');
        if(path.StartsWith(@"\\?\UNC\",StringComparison.OrdinalIgnoreCase))path=@"\\"+path.Substring(8);
        else if(path.StartsWith(@"\\?\",StringComparison.Ordinal))path=path.Substring(4);
        if(path.StartsWith(@"\\.\",StringComparison.Ordinal) ||
           !(path.StartsWith(@"\\",StringComparison.Ordinal) ||
             (path.Length>2 && Char.IsLetter(path[0]) && path[1]==':' && path[2]=='\\')))
            throw new ArgumentException("Expected an absolute file path, not a device or relative path");
        var b=new StringBuilder(32768);uint count=GetFullPathNameW(path,(uint)b.Capacity,b,IntPtr.Zero);
        if(count==0)throw new Win32Exception(Marshal.GetLastWin32Error(),"Cannot resolve snapshot path");
        if(count>=b.Capacity || count>32600)throw new PathTooLongException("Snapshot path exceeds extended-path budget");
        return b.ToString();
    }
    static string Extended(string full) {
        return full.StartsWith(@"\\",StringComparison.Ordinal)?@"\\?\UNC\"+full.Substring(2):@"\\?\"+full;
    }
    static FileStream Open(string full, bool create) {
        var h=CreateFileW(Extended(full),create?0x40000000U:0x80000000U,
            create?0U:7U,IntPtr.Zero,create?1U:3U,0x08000000U,IntPtr.Zero);
        if(h.IsInvalid){int code=Marshal.GetLastWin32Error();h.Dispose();throw new Win32Exception(code,"Snapshot CreateFileW");}
        try{return new FileStream(h,create?FileAccess.Write:FileAccess.Read,65536,false);}
        catch{h.Dispose();throw;}
    }
    static bool TryWslRename(string pending, string target) {
        // The Windows WSL UNC provider denies replacement while a Windows reader
        // holds the old inode, even when that reader shares delete access. Linux
        // rename preserves the old handle and publishes the closed sibling atomically.
        const string prefix = @"\\wsl.localhost\";
        const string legacyPrefix = @"\\wsl$\";
        string root = target.StartsWith(prefix,StringComparison.OrdinalIgnoreCase) ? prefix :
            target.StartsWith(legacyPrefix,StringComparison.OrdinalIgnoreCase) ? legacyPrefix : null;
        if(root==null || !pending.StartsWith(root,StringComparison.OrdinalIgnoreCase))return false;
        int end=target.IndexOf('\\',root.Length);
        if(end<0)return false;
        string distro=target.Substring(root.Length,end-root.Length);
        if(distro.Length==0)return false;
        foreach(char c in distro)
            if(!(Char.IsLetterOrDigit(c) || c=='-' || c=='_' || c=='.'))return false;
        string linuxTarget=target.Substring(end).Replace('\\','/');
        string linuxPending=pending.Substring(end).Replace('\\','/');
        // Windows path components cannot contain double quotes. No shell is used.
        var start=new ProcessStartInfo("wsl.exe", "-d "+distro+" --exec /usr/bin/mv -f -- \""+
            linuxPending+"\" \""+linuxTarget+"\"");
        start.UseShellExecute=false;start.CreateNoWindow=true;start.RedirectStandardError=true;
        using(var process=Process.Start(start)) {
            if(!process.WaitForExit(15000)) {
                process.Kill();throw new IOException("WSL snapshot rename timed out");
            }
            if(process.ExitCode!=0)
                throw new IOException("WSL snapshot rename failed for distro '"+distro+"' (exit "+process.ExitCode+"): "+process.StandardError.ReadToEnd());
        }
        return true;
    }
    public static string ReadTextOrNull(string path) {
        string full=Absolute(path);
        for(int attempt=0;;attempt++) {
            try {
                using(var s=Open(full,false))
                using(var r=new StreamReader(s,new UTF8Encoding(false,true),true))return r.ReadToEnd();
            } catch(Exception e) {
                int code=NativeCode(e);
                if(code==2 || code==3)return null; // Only genuinely absent snapshots are absent.
                if(!Retryable(e,false) || attempt==Delays.Length)
                    throw new VVSnapshotIOException("read",full,null,false,attempt+1,e);
                Interlocked.Increment(ref readRetries);Thread.Sleep(Delays[attempt]);
            }
        }
    }
    public static VVSnapshotReceipt WriteText(string path, string text) {
        if(text==null)throw new ArgumentNullException("text");
        string full=Absolute(path);
        string pending=full+"."+Guid.NewGuid().ToString("N")+".new";
        byte[] data=new UTF8Encoding(false,true).GetBytes(text);
        try {
            using(var stream=Open(pending,true)) {
                stream.Write(data,0,data.Length);stream.Flush(true);
            }
        } catch(Exception e) {
            // A failed/partial stage is retained and explicitly distinguished from a complete stage.
            throw new VVSnapshotIOException("stage",full,pending,false,1,e);
        }
        for(int attempt=0;;attempt++) {
            try {
                // REPLACE_EXISTING | WRITE_THROUGH; deliberately no COPY_ALLOWED.
                if(!MoveFileExW(Extended(pending),Extended(full),0x1U|0x8U)) {
                    int code=Marshal.GetLastWin32Error();
                    if(code!=5 || !TryWslRename(pending,full))
                        throw new Win32Exception(code,"Snapshot MoveFileExW");
                }
                return new VVSnapshotReceipt{Path=full,Bytes=data.LongLength,Attempts=attempt+1};
            } catch(Exception e) {
                if(!Retryable(e,true) || attempt==Delays.Length)
                    throw new VVSnapshotIOException("publish",full,pending,true,attempt+1,e);
                Interlocked.Increment(ref publishRetries);Thread.Sleep(Delays[attempt]);
            }
        }
    }
}
