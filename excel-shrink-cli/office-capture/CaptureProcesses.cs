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
// Native object access is deliberately scoped to an Excel process already owned
// by the capture launcher. Never use the Running Object Table or GetActiveObject:
// those may return an unrelated workbook in the user's interactive Excel.
public sealed class CaptureErrorChecking : IDisposable
{
    readonly Process process;
    object options;
    readonly bool previousBackgroundChecking;

    CaptureErrorChecking(Process process, object options, bool previous)
    {
        this.process = process;
        this.options = options;
        previousBackgroundChecking = previous;
    }

    public int ProcessId { get { return process.Id; } }
    public bool PreviousBackgroundChecking { get { return previousBackgroundChecking; } }
    public bool IsDisabled
    {
        get
        {
            if (options == null) throw new ObjectDisposedException("CaptureErrorChecking");
            return !Convert.ToBoolean(Property(options, "BackgroundChecking"));
        }
    }

    // Null means that this exact process has not exposed its native workbook
    // window yet. The launcher's existing startup deadline bounds the retries.
    public static CaptureErrorChecking TryDisable(Process process)
    {
        return TryDisable(process, null);
    }

    public static CaptureErrorChecking TryDisable(Process process, string desktopName)
    {
        if (process == null) throw new ArgumentNullException("process");
        if (process.HasExited) throw new InvalidOperationException("Owned Excel process already exited.");
        if (!String.Equals(process.ProcessName, "EXCEL", StringComparison.OrdinalIgnoreCase))
            throw new ArgumentException("Error indicators may only be configured on owned Excel processes.", "process");

        if (String.IsNullOrEmpty(desktopName)) desktopName = null;

        foreach (IntPtr handle in WorkbookWindows(process, desktopName))
        {
            object window = null;
            object application = null;
            object checking = null;
            CaptureErrorChecking pending = null;
            try
            {
                uint owner;
                GetWindowThreadProcessId(handle, out owner);
                if (owner != (uint)process.Id || process.HasExited) continue;
                Guid dispatch = new Guid("00020400-0000-0000-C000-000000000046");
                int result = AccessibleObjectFromWindow(handle, 0xfffffff0, ref dispatch, out window);
                if (result < 0 || window == null) continue;
                application = Property(window, "Application");
                IntPtr main = new IntPtr(Convert.ToInt64(Property(application, "Hwnd")));
                GetWindowThreadProcessId(main, out owner);
                if (owner != (uint)process.Id || process.HasExited)
                    throw new InvalidOperationException("Native Excel window does not belong to the owned capture process.");

                DisableWorkbookRecovery(application);
                checking = Property(application, "ErrorCheckingOptions");
                bool previous = Convert.ToBoolean(Property(checking, "BackgroundChecking"));
                pending = new CaptureErrorChecking(process, checking, previous);
                checking = null; // The scope now owns exactly this COM reference.
                SetProperty(pending.options, "BackgroundChecking", false);
                if (!pending.IsDisabled)
                    throw new InvalidOperationException("Owned Excel did not disable background error indicators.");
                var scope = pending;
                pending = null;
                return scope;
            }
            catch (COMException error)
            {
                if (!IsBusy(error)) throw;
            }
            catch (System.Reflection.TargetInvocationException error)
            {
                var busy = error.InnerException as COMException;
                if (busy == null || !IsBusy(busy)) throw;
            }
            finally
            {
                if (pending != null) pending.Dispose();
                Release(checking);
                Release(application);
                Release(window);
            }
        }
        return null;
    }

    static void DisableWorkbookRecovery(object application)
    {
        object workbook = null;
        try
        {
            workbook = Property(application, "ActiveWorkbook");
            if (workbook == null)
                throw new InvalidOperationException("Owned Excel has no active capture workbook.");
            // Only the disposable workbook changes; personal Excel recovery settings remain intact.
            SetProperty(workbook, "EnableAutoRecover", false);
            if (Convert.ToBoolean(Property(workbook, "EnableAutoRecover")))
                throw new InvalidOperationException("Owned Excel did not disable workbook AutoRecover.");
        }
        finally { Release(workbook); }
    }

    public void Dispose()
    {
        var checking = options;
        if (checking == null) return;
        options = null;
        try
        {
            // Capture copies are never saved here. Restore the instance option
            // before shutdown so normal Excel preferences are not retained.
            if (!process.HasExited) SetProperty(checking, "BackgroundChecking", previousBackgroundChecking);
        }
        finally { Release(checking); }
    }

    static bool IsBusy(COMException error)
    {
        uint code = unchecked((uint)error.ErrorCode);
        return code == 0x80010001 || code == 0x8001010a || // rejected / retry later
            code == 0x80010108 || code == 0x800401fd || // disconnected while starting
            code == 0x800ac472; // Excel temporarily ignores automation while busy
    }
    static object Property(object instance, string name)
    {
        return instance.GetType().InvokeMember(name, System.Reflection.BindingFlags.GetProperty,
            null, instance, null, System.Globalization.CultureInfo.InvariantCulture);
    }

    static void SetProperty(object instance, string name, object value)
    {
        instance.GetType().InvokeMember(name, System.Reflection.BindingFlags.SetProperty,
            null, instance, new[] { value }, System.Globalization.CultureInfo.InvariantCulture);
    }

    static void Release(object instance)
    {
        if (instance != null && Marshal.IsComObject(instance)) Marshal.ReleaseComObject(instance);
    }

    static System.Collections.Generic.List<IntPtr> WorkbookWindows(Process process, string desktopName)
    {
        var result = new System.Collections.Generic.List<IntPtr>();
        IntPtr desktop = desktopName == null ? GetThreadDesktop(GetCurrentThreadId())
            : OpenDesktop(desktopName, 0, false, 0x0041); // READOBJECTS | ENUMERATE
        if (desktop == IntPtr.Zero) throw new Win32Exception();
        try
        {
            EnumWindowsProc top = delegate(IntPtr window, IntPtr unused)
            {
                uint topOwner;
                GetWindowThreadProcessId(window, out topOwner);
                if (topOwner != (uint)process.Id) return true;
                EnumWindowsProc child = delegate(IntPtr candidate, IntPtr ignored)
                {
                    uint owner;
                    GetWindowThreadProcessId(candidate, out owner);
                    if (owner != (uint)process.Id) return true;
                    var name = new System.Text.StringBuilder(32);
                    GetClassName(candidate, name, name.Capacity);
                    if (name.ToString() == "EXCEL7") result.Add(candidate);
                    return true;
                };
                EnumChildWindows(window, child, IntPtr.Zero);
                return true;
            };
            // The caller supplies its private desktop name. Never switch the
            // input desktop, and ignore every window not owned by this PID.
            EnumDesktopWindows(desktop, top, IntPtr.Zero);
        }
        finally { if (desktopName != null) CloseDesktop(desktop); }
        return result;
    }
    delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [DllImport("kernel32.dll")]
    static extern uint GetCurrentThreadId();
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr OpenDesktop(string name, uint flags, bool inherit, uint access);
    [DllImport("user32.dll")]
    static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll")]
    static extern IntPtr GetThreadDesktop(uint thread);
    [DllImport("user32.dll")]
    static extern bool EnumDesktopWindows(IntPtr desktop, EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")]
    static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetClassName(IntPtr window, System.Text.StringBuilder className, int maximum);
    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("oleacc.dll")]
    static extern int AccessibleObjectFromWindow(IntPtr window, uint objectId, ref Guid interfaceId,
        [MarshalAs(UnmanagedType.Interface)] out object result);
}
