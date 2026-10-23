using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

// Query identity and terminate through the same handle, even if Windows reuses the PID.
public static class ArchiveProcesses
{
    public static string CurrentStartedTicks()
    {
        return StartedTicks(GetCurrentProcess()).ToString(System.Globalization.CultureInfo.InvariantCulture);
    }

    public static string StopIfOwned(int pid, string expectedStarted)
    {
        long expected;
        if (pid <= 0 || !Int64.TryParse(expectedStarted, out expected) || expected <= 0)
            throw new ArgumentException("Invalid process identity in the archive ownership journal.");

        const uint access = 0x1000 | 0x0001 | 0x00100000; // QUERY_LIMITED_INFORMATION | TERMINATE | SYNCHRONIZE
        using (var process = OpenProcess(access, false, pid))
        {
            if (process.IsInvalid)
            {
                int error = Marshal.GetLastWin32Error();
                if (error == 87 || error == 1168) return "gone";
                throw new Win32Exception(error, "Cannot verify archive launcher PID " + pid);
            }
            // A launcher may exit between journal enumeration and this check.
            if (HasExited(process)) return "gone";
            long actual;
            try { actual = StartedTicks(process.DangerousGetHandle()); }
            catch (Win32Exception) { if (HasExited(process)) return "gone"; throw; }
            if (actual != expected) return "reused";
            if (!TerminateProcess(process, 1))
            {
                int error = Marshal.GetLastWin32Error();
                if (HasExited(process)) return "gone";
                throw new Win32Exception(error, "Cannot stop archive launcher PID " + pid);
            }
            uint waited = WaitForSingleObject(process, 10000);
            if (waited == 0) return "stopped";
            if (waited == 258) throw new TimeoutException("Archive launcher PID " + pid + " did not exit.");
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot wait for archive launcher PID " + pid);
        }
    }

    static bool HasExited(SafeFileHandle process)
    {
        uint status = WaitForSingleObject(process, 0);
        if (status == 0) return true;
        if (status == 258) return false;
        throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot inspect archive launcher state.");
    }

    static long StartedTicks(IntPtr process)
    {
        long created, exited, kernel, user;
        if (!GetProcessTimes(process, out created, out exited, out kernel, out user))
            throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot read archive launcher birth time.");
        return DateTime.FromFileTimeUtc(created).Ticks;
    }

    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern SafeFileHandle OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetProcessTimes(IntPtr process, out long created, out long exited, out long kernel, out long user);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateProcess(SafeFileHandle process, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(SafeFileHandle process, uint milliseconds);
}
