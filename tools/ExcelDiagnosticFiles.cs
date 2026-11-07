using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;
using Microsoft.Win32.SafeHandles;

// The diagnostic's own process header identifies its writer. Deletion requires
// both a departed writer and exclusive access to the exact file being inspected.
public static class ExcelDiagnosticFiles
{
    // Retire only old cache files during an Office-free interval. Exclusive
    // access and handle-based deletion protect a racing reader or path change.
    public static string RemoveOfficeCacheFile(string path, string root, int minimumAgeSeconds)
    {
        foreach (string name in new[] { "EXCEL", "WINWORD", "POWERPNT", "OUTLOOK", "MSACCESS", "ONENOTE", "MSPUB" })
        {
            var processes = System.Diagnostics.Process.GetProcessesByName(name);
            bool running = processes.Length != 0;
            foreach (var process in processes) process.Dispose();
            if (running) return "office-running";
        }
        using (var file = CreateFile(path, 0x80000000 | 0x00010000, 0, IntPtr.Zero, 3, 0x00200000, IntPtr.Zero))
        {
            if (file.IsInvalid) return "locked-or-unreadable";
            FileInformation info;
            if (!GetFileInformationByHandle(file, out info) || (info.Attributes & 0x410) != 0) return "unsafe-path";
            var resolved = new StringBuilder(32768);
            uint length = GetFinalPathNameByHandle(file, resolved, (uint)resolved.Capacity, 0);
            if (length == 0 || length >= resolved.Capacity) return "unsafe-path";
            string actual = resolved.ToString();
            if (actual.StartsWith(@"\\?\", StringComparison.Ordinal)) actual = actual.Substring(4);
            if (!actual.StartsWith(root.TrimEnd('\\') + @"\", StringComparison.OrdinalIgnoreCase) ||
                !String.Equals(Path.GetExtension(actual), ".tmp", StringComparison.OrdinalIgnoreCase)) return "unsafe-path";
            DateTime cutoff = DateTime.UtcNow.AddSeconds(-minimumAgeSeconds);
            if (DateTime.FromFileTimeUtc(ToLong(info.Written)) > cutoff ||
                DateTime.FromFileTimeUtc(ToLong(info.Created)) > cutoff) return "recent";
            var disposition = new FileDisposition { DeleteFile = true };
            return SetFileInformationByHandle(file, 4, ref disposition, (uint)Marshal.SizeOf(typeof(FileDisposition)))
                ? "removed" : "delete-failed";
        }
    }

    public sealed class Result
    {
        public string Outcome;
        public long Length;
        public int Pid;
        public long StartedTicks;
        public string LogUtc;
        public int Win32Error;
        public string FileIdentity;
        public long? AllocatedBytes;
        public long CompressionSavedBytes;
        public bool CompressionAttempted;
    }

    public sealed class Owner
    {
        public int Pid;
        public long StartedTicks;
    }

    public sealed class Observation
    {
        public string FileIdentity;
        public Owner[] Owners;
    }

    static readonly Regex Header = new Regex(@"(?m)^(?<time>\d{1,2}/\d{1,2}/\d{4} \d{1,2}:\d{2}:\d{2}\.\d{3})\tEXCEL \(0x(?<pid>[0-9a-fA-F]+)\)\t", RegexOptions.CultureInvariant);

    public static Result InspectAndRemove(string path, string root, int minimumAgeSeconds, string observedIdentity, Owner[] observedOwners)
    {
        return InspectAndRemove(path, root, minimumAgeSeconds, observedIdentity, observedOwners, false);
    }

    public static Result InspectAndRemove(string path, string root, int minimumAgeSeconds, string observedIdentity, Owner[] observedOwners, bool compressUnknown)
    {
        var result = new Result { Outcome = "unknown" };
        // No sharing: even a live logger that permits FILE_SHARE_DELETE must not
        // lose its file. OPEN_REPARSE_POINT prevents following a replaced link.
        using (var file = CreateFile(path, 0x80000000 | 0x00010000 | (compressUnknown ? 0x40000000u : 0), 0, IntPtr.Zero, 3, 0x00200000, IntPtr.Zero))
        {
            if (file.IsInvalid)
            {
                result.Win32Error = Marshal.GetLastWin32Error();
                result.Outcome = result.Win32Error == 32 || result.Win32Error == 33 ? "locked" : "unreadable";
                return result;
            }
            var finalPath = new StringBuilder(32768);
            uint pathLength = GetFinalPathNameByHandle(file, finalPath, (uint)finalPath.Capacity, 0);
            if (pathLength == 0 || pathLength >= finalPath.Capacity) return result;
            string resolved = finalPath.ToString();
            if (resolved.StartsWith(@"\\?\UNC\", StringComparison.OrdinalIgnoreCase)) resolved = @"\\" + resolved.Substring(8);
            else if (resolved.StartsWith(@"\\?\", StringComparison.Ordinal)) resolved = resolved.Substring(4);
            if (!resolved.StartsWith(root.TrimEnd('\\') + @"\", StringComparison.OrdinalIgnoreCase) ||
                !String.Equals(Path.GetExtension(resolved), ".log", StringComparison.OrdinalIgnoreCase)) return result;

            FileInformation info;
            if (!GetFileInformationByHandle(file, out info) || (info.Attributes & 0x400) != 0) return result;
            result.FileIdentity = Identity(info);
            result.Length = ((long)info.SizeHigh << 32) | info.SizeLow;
            CompressionInformation compression;
            if (GetFileInformationByHandleEx(file, 8, out compression, (uint)Marshal.SizeOf(typeof(CompressionInformation))))
                result.AllocatedBytes = compression.CompressedFileSize;
            DateTime created = DateTime.FromFileTimeUtc(ToLong(info.Created));
            DateTime written = DateTime.FromFileTimeUtc(ToLong(info.Written));
            DateTime now = DateTime.UtcNow;
            if (written > now.AddSeconds(-Math.Max(0, minimumAgeSeconds))) { result.Outcome = "recent"; return result; }
            using (var input = new FileStream(file, FileAccess.Read))
            {
                // Bounded header parsing: logs can be preallocated to many MB.
                byte[] bytes = new byte[65536];
                int length = input.Read(bytes, 0, bytes.Length);
                var matches = Header.Matches(Encoding.UTF8.GetString(bytes, 0, length));
                // Restart Manager observes file users, not the original writer.
                // Even a departed reader cannot authorize deleting a blank log.
                if (matches.Count == 0)
                    return PreserveUnknown(file, result, compressUnknown, info.Attributes);
                DateTime first;
                int pid;
                if (!DateTime.TryParseExact(matches[0].Groups["time"].Value, "M/d/yyyy H:mm:ss.fff", CultureInfo.InvariantCulture,
                    DateTimeStyles.AssumeUniversal | DateTimeStyles.AdjustToUniversal, out first) ||
                    !Int32.TryParse(matches[0].Groups["pid"].Value, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out pid) || pid <= 0)
                    return PreserveUnknown(file, result, compressUnknown, info.Attributes);
                // On-disk Excel headers use UTC, as does file creation. A copied
                // or clock-ambiguous log is retained instead of inferring reuse.
                if (first > now || Math.Abs((first - created).TotalSeconds) > 60)
                    return PreserveUnknown(file, result, compressUnknown, info.Attributes);
                foreach (Match match in matches)
                    if (!String.Equals(match.Groups["pid"].Value, matches[0].Groups["pid"].Value, StringComparison.OrdinalIgnoreCase))
                        return PreserveUnknown(file, result, compressUnknown, info.Attributes);
                result.Pid = pid;
                result.LogUtc = first.ToString("o", CultureInfo.InvariantCulture);
                string ownership = OwnerState(pid, first > created ? first : created, result);
                if (ownership == "unknown") return PreserveUnknown(file, result, compressUnknown, info.Attributes);
                if (ownership != "exited" && ownership != "reused") { result.Outcome = ownership; return result; }

                if (observedIdentity == result.FileIdentity && observedOwners != null && observedOwners.Length > 0)
                {
                    // This is additional protection for observed readers. The
                    // header writer check above must succeed independently.
                    foreach (Owner owner in observedOwners)
                    {
                        string state = ObservedOwnerState(owner);
                        if (state == "unknown") return PreserveUnknown(file, result, compressUnknown, info.Attributes);
                        if (state != "exited") { result.Outcome = state; return result; }
                    }
                }
                var disposition = new FileDisposition { DeleteFile = true };
                if (!SetFileInformationByHandle(file, 4, ref disposition, (uint)Marshal.SizeOf(typeof(FileDisposition))))
                {
                    result.Win32Error = Marshal.GetLastWin32Error();
                    result.Outcome = "delete-failed";
                    return result;
                }
                result.Outcome = "removed-" + ownership;
                return result;
            }
        }
    }

    static Result PreserveUnknown(SafeFileHandle file, Result result, bool compress, uint attributes)
    {
        if (compress && result.Length >= 65536 && (attributes & 0x800) == 0)
        {
            // Keep unknown logs byte-for-byte. NTFS compression changes storage,
            // not logical data or length. The same exclusive handle prevents
            // concurrent use; future ordinary and memory-mapped writes still work.
            ushort format = 1; // COMPRESSION_FORMAT_DEFAULT (LZNT1)
            uint returned;
            result.CompressionAttempted = true;
            if (DeviceIoControl(file, 0x0009C040, ref format, 2, IntPtr.Zero, 0, out returned, IntPtr.Zero))
            {
                long? before = result.AllocatedBytes;
                CompressionInformation compression;
                if (GetFileInformationByHandleEx(file, 8, out compression, (uint)Marshal.SizeOf(typeof(CompressionInformation))))
                {
                    result.AllocatedBytes = compression.CompressedFileSize;
                    if (before.HasValue) result.CompressionSavedBytes = Math.Max(0, before.Value - compression.CompressedFileSize);
                }
                result.Outcome = "unknown-compressed";
            }
            else { result.Win32Error = Marshal.GetLastWin32Error(); result.Outcome = "unknown-compression-failed"; }
        }
        return result;
    }

    static string Identity(FileInformation info)
    {
        return info.Volume.ToString("X8") + ":" + info.IndexHigh.ToString("X8") + info.IndexLow.ToString("X8") + ":" + ToLong(info.Created).ToString(CultureInfo.InvariantCulture);
    }

    static string ObservedOwnerState(Owner owner)
    {
        if (owner == null || owner.Pid <= 0 || owner.StartedTicks <= 0) return "unknown";
        using (var process = OpenProcess(0x1000 | 0x00100000, false, owner.Pid))
        {
            if (process.IsInvalid) {
                int error = Marshal.GetLastWin32Error();
                return error == 87 || error == 1168 ? "exited" : "unknown";
            }
            uint state = WaitForSingleObject(process, 0);
            if (state == 0) return "exited";
            if (state != 258) return "unknown";
            long created, exited, kernel, user;
            if (!GetProcessTimes(process, out created, out exited, out kernel, out user)) return "unknown";
            return DateTime.FromFileTimeUtc(created).Ticks == owner.StartedTicks ? "live" : "exited";
        }
    }

    public static Observation ObserveOwners(string path)
    {
        // Attribute-only access is compatible with a logger that holds the data
        // open. Persist file ID + creation time, never filename GUID alone.
        using (var file = CreateFile(path, 0x80, 3, IntPtr.Zero, 3, 0x00200000, IntPtr.Zero))
        {
            FileInformation info;
            if (file.IsInvalid || !GetFileInformationByHandle(file, out info) || (info.Attributes & 0x400) != 0) return null;
            uint session;
            var key = new StringBuilder(33);
            if (RmStartSession(out session, 0, key) != 0) return null;
            try
            {
                if (RmRegisterResources(session, 1, new string[] { path }, 0, null, 0, null) != 0) return null;
                uint needed, count = 0, reasons = 0;
                int status = RmGetList(session, out needed, ref count, null, ref reasons);
                if (status != 234 || needed == 0 || needed > 64) return null;
                var users = new RestartProcess[needed];
                count = needed;
                if (RmGetList(session, out needed, ref count, users, ref reasons) != 0) return null;
                var owners = new List<Owner>();
                int observer = System.Diagnostics.Process.GetCurrentProcess().Id;
                for (int i = 0; i < count; i++)
                    if (users[i].Process.Pid != observer)
                        owners.Add(new Owner { Pid = users[i].Process.Pid, StartedTicks = DateTime.FromFileTimeUtc(ToLong(users[i].Process.Started)).Ticks });
                if (owners.Count == 0) return null;
                // Ensure path registration did not observe a replacement file.
                FileInformation after;
                if (!GetFileInformationByHandle(file, out after) || Identity(after) != Identity(info)) return null;
                return new Observation { FileIdentity = Identity(info), Owners = owners.ToArray() };
            }
            finally { RmEndSession(session); }
        }
    }

    static string OwnerState(int pid, DateTime logged, Result result)
    {
        using (var process = OpenProcess(0x1000 | 0x00100000, false, pid))
        {
            if (process.IsInvalid)
            {
                result.Win32Error = Marshal.GetLastWin32Error();
                return result.Win32Error == 87 || result.Win32Error == 1168 ? "exited" : "unknown";
            }
            uint state = WaitForSingleObject(process, 0);
            if (state == 0) return "exited";
            if (state != 258) { result.Win32Error = Marshal.GetLastWin32Error(); return "unknown"; }
            long created, exited, kernel, user;
            if (!GetProcessTimes(process, out created, out exited, out kernel, out user))
            { result.Win32Error = Marshal.GetLastWin32Error(); return "unknown"; }
            result.StartedTicks = DateTime.FromFileTimeUtc(created).Ticks;
            // A PID can be reused. Only a strict, five-second chronological gap
            // proves this is a later process; ambiguous times remain protected.
            return DateTime.FromFileTimeUtc(created) > logged.AddSeconds(5) ? "reused" : "live";
        }
    }

    static long ToLong(System.Runtime.InteropServices.ComTypes.FILETIME value)
    { return ((long)(uint)value.dwHighDateTime << 32) | (uint)value.dwLowDateTime; }

    [StructLayout(LayoutKind.Sequential)] struct FileDisposition { [MarshalAs(UnmanagedType.Bool)] public bool DeleteFile; }
    [StructLayout(LayoutKind.Sequential)] struct CompressionInformation {
        public long CompressedFileSize;
        public ushort CompressionFormat;
        public byte CompressionUnitShift, ChunkShift, ClusterShift, Reserved1, Reserved2, Reserved3;
    }
    [StructLayout(LayoutKind.Sequential)] struct FileInformation {
        public uint Attributes;
        public System.Runtime.InteropServices.ComTypes.FILETIME Created, Accessed, Written;
        public uint Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow;
    }
    [StructLayout(LayoutKind.Sequential)] struct UniqueProcess { public int Pid; public System.Runtime.InteropServices.ComTypes.FILETIME Started; }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] struct RestartProcess {
        public UniqueProcess Process;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=256)] public string Name;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=64)] public string Service;
        public int Type;
        public uint Status, Session;
        [MarshalAs(UnmanagedType.Bool)] public bool Restartable;
    }
    [DllImport("rstrtmgr.dll", CharSet=CharSet.Unicode)] static extern int RmStartSession(out uint session, uint flags, StringBuilder key);
    [DllImport("rstrtmgr.dll", CharSet=CharSet.Unicode)] static extern int RmRegisterResources(uint session, uint files, string[] names, uint applications, UniqueProcess[] processes, uint services, string[] serviceNames);
    [DllImport("rstrtmgr.dll")] static extern int RmGetList(uint session, out uint needed, ref uint count, [In, Out] RestartProcess[] users, ref uint reasons);
    [DllImport("rstrtmgr.dll")] static extern int RmEndSession(uint session);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFile(string path, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern uint GetFinalPathNameByHandle(SafeFileHandle file, StringBuilder path, uint size, uint flags);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetFileInformationByHandle(SafeFileHandle file, out FileInformation information);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetFileInformationByHandleEx(SafeFileHandle file, int type, out CompressionInformation information, uint size);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool DeviceIoControl(SafeFileHandle file, uint code, ref ushort input, uint inputSize, IntPtr output, uint outputSize, out uint returned, IntPtr overlapped);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetFileInformationByHandle(SafeFileHandle file, int type, ref FileDisposition information, uint size);
    [DllImport("kernel32.dll", SetLastError=true)] static extern SafeFileHandle OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint WaitForSingleObject(SafeFileHandle process, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetProcessTimes(SafeFileHandle process, out long created, out long exited, out long kernel, out long user);
}
