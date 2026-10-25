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

        // A recycled PID may now belong to a process we are allowed to inspect
        // but not terminate. Establish identity before requesting kill rights.
        const uint query = 0x1000 | 0x00100000; // QUERY_LIMITED_INFORMATION | SYNCHRONIZE
        using (var identity = OpenProcess(query, false, pid))
        {
            if (identity.IsInvalid)
            {
                int error = Marshal.GetLastWin32Error();
                if (error == 87 || error == 1168) return "gone";
                throw Failure(error, "query identity", pid);
            }
            // A launcher may exit between journal enumeration and this check.
            if (HasExited(identity)) return "gone";
            long actual;
            try { actual = StartedTicks(identity.DangerousGetHandle()); }
            catch (Win32Exception) { if (HasExited(identity)) return "gone"; throw; }
            if (actual != expected) return "reused";
            using (var process = OpenProcess(query | 0x0001, false, pid))
            {
                if (process.IsInvalid)
                {
                    int error = Marshal.GetLastWin32Error();
                    if (HasExited(identity) || error == 87 || error == 1168) return "gone";
                    throw Failure(error, "acquire termination handle", pid);
                }
                // Never trust the PID alone across the second OpenProcess call.
                if (HasExited(process)) return "gone";
                if (StartedTicks(process.DangerousGetHandle()) != expected) return "reused";
                if (!TerminateProcess(process, 1))
                {
                    int error = Marshal.GetLastWin32Error();
                    if (HasExited(process)) return "gone";
                    throw Failure(error, "terminate", pid);
                }
                uint waited = WaitForSingleObject(process, 10000);
                if (waited == 0) return "stopped";
                if (waited == 258) throw new TimeoutException("Archive launcher PID " + pid + " did not exit after termination (10000 ms).");
                throw Failure(Marshal.GetLastWin32Error(), "wait for exit", pid);
            }
        }
    }

    static Win32Exception Failure(int error, string phase, int pid)
    {
        return new Win32Exception(error, "Archive launcher PID " + pid + ": " + phase +
            " failed (Win32 " + error + ": " + new Win32Exception(error).Message + ").");
    }

    static bool HasExited(SafeFileHandle process)
    {
        uint status = WaitForSingleObject(process, 0);
        if (status == 0) return true;
        if (status == 258) return false;
        throw Failure(Marshal.GetLastWin32Error(), "inspect process state", 0);
    }

    static long StartedTicks(IntPtr process)
    {
        long created, exited, kernel, user;
        if (!GetProcessTimes(process, out created, out exited, out kernel, out user))
            throw Failure(Marshal.GetLastWin32Error(), "read process birth time", 0);
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
