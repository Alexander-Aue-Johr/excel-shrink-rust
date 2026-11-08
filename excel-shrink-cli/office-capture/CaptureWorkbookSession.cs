using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;

/// <summary>
/// Switch read-only workbooks in one window-verified Excel application.
/// The caller owns process termination and transfers one COM application reference.
/// A failed operation faults the session, requiring its owner to discard the process.
/// </summary>
public sealed class CaptureWorkbookSession : IDisposable
{
    // Excel's automation object model uses LCID 1033 independently of display language.
    static readonly CultureInfo AutomationCulture = CultureInfo.GetCultureInfo(1033);
    readonly Process process;
    readonly long processBirthTicks;
    readonly string seedPath;
    object application;
    object currentWorkbook;
    bool faulted;

    public CaptureWorkbookSession(object ownedApplication, Process ownedProcess, string seed)
    {
        if (ownedApplication == null) throw new ArgumentNullException("ownedApplication");
        if (ownedProcess == null) throw new ArgumentNullException("ownedProcess");
        application = ownedApplication;
        process = ownedProcess;
        processBirthTicks = process.StartTime.ToUniversalTime().Ticks;
        seedPath = Path.GetFullPath(seed);
    }

    public void Initialize()
    {
        try
        {
            VerifyProcess();
            VerifySeedOnly();
            Set(application, "Visible", true);
            Set(application, "DisplayAlerts", false);
            Set(application, "AutomationSecurity", 3);
            Set(application, "EnableEvents", false);
            Set(application, "Calculation", -4135);
            Set(application, "CalculateBeforeSave", false);
            object recovery = Get(application, "AutoRecover");
            try { Set(recovery, "Enabled", false); }
            finally { Release(recovery); }
        }
        catch { faulted = true; throw; }
    }

    /// <summary>Disable link updates and require the exact staged file to open read-only.</summary>
    public void OpenReadOnly(string path)
    {
        EnsureUsable();
        if (currentWorkbook != null) throw new InvalidOperationException("Close the current workbook before opening another.");
        string expectedPath = Path.GetFullPath(path);
        if (String.Equals(expectedPath, seedPath, StringComparison.OrdinalIgnoreCase))
            throw new ArgumentException("The seed cannot be a comparison workbook.", "path");
        try
        {
            VerifyProcess();
            VerifySeedOnly();
            object workbooks = Get(application, "Workbooks");
            try
            {
                currentWorkbook = Invoke(workbooks, "Open", new object[] {
                    expectedPath, 0, true, Type.Missing, "", "", true
                });
                VerifyProcess();
                if (!SamePath(Get(currentWorkbook, "FullName"), expectedPath) ||
                    !Convert.ToBoolean(Get(currentWorkbook, "ReadOnly")) ||
                    Convert.ToInt32(Get(workbooks, "Count")) != 2)
                    throw new InvalidOperationException("Opened workbook identity or read-only state differs from the request.");
            }
            finally { Release(workbooks); }
        }
        catch { faulted = true; throw; }
    }

    public void CloseCurrent()
    {
        EnsureUsable();
        try
        {
            VerifyProcess();
            if (currentWorkbook != null)
            {
                Invoke(currentWorkbook, "Close", new object[] { false });
                Release(currentWorkbook);
                currentWorkbook = null;
            }
            VerifySeedOnly();
        }
        catch { faulted = true; throw; }
    }

    /// <summary>Rebuild the current comparison and empty seed, requiring Excel's completed state.</summary>
    public void Rebuild()
    {
        RequireCurrentWorkbook();
        try
        {
            VerifyProcess();
            Invoke(application, "CalculateFullRebuild", new object[0]);
            VerifyProcess();
            if (Convert.ToInt32(Get(application, "CalculationState")) != 0)
                throw new InvalidOperationException("Excel did not complete the full rebuild.");
        }
        catch { faulted = true; throw; }
    }

    /// <summary>Serialize an XLSX snapshot without changing the open workbook or replacing an existing file.</summary>
    public void SaveCopy(string output)
    {
        RequireCurrentWorkbook();
        string destination = Path.GetFullPath(output);
        if (!String.Equals(Path.GetExtension(destination), ".xlsx", StringComparison.OrdinalIgnoreCase))
            throw new ArgumentException("Workbook snapshots require an .xlsx destination.", "output");
        if (File.Exists(destination)) throw new IOException("Workbook snapshot destination already exists.");
        string pending = Path.Combine(Path.GetDirectoryName(destination), ".snapshot-" + Guid.NewGuid().ToString("N") + ".xlsx");
        try
        {
            VerifyProcess();
            if (Convert.ToInt32(Get(currentWorkbook, "FileFormat")) != 51)
                throw new InvalidOperationException("Snapshot source must be an XLSX workbook.");
            string before = Convert.ToString(Get(currentWorkbook, "FullName"));
            Invoke(currentWorkbook, "SaveCopyAs", new object[] { pending });
            VerifyProcess();
            if (!SamePath(Get(currentWorkbook, "FullName"), before) || new FileInfo(pending).Length == 0)
                throw new InvalidOperationException("Workbook snapshot changed identity or produced an empty file.");
            // Move publishes only a completed snapshot and refuses a concurrent destination.
            File.Move(pending, destination);
        }
        catch { faulted = true; throw; }
        finally
        {
            if (File.Exists(pending))
            {
                try { File.Delete(pending); }
                catch (IOException error) { Console.Error.WriteLine("Snapshot cleanup failed: " + error.Message); }
                catch (UnauthorizedAccessException error) { Console.Error.WriteLine("Snapshot cleanup failed: " + error.Message); }
            }
        }
    }

    void RequireCurrentWorkbook()
    {
        EnsureUsable();
        if (currentWorkbook == null) throw new InvalidOperationException("No comparison workbook is open.");
    }

    void EnsureUsable()
    {
        if (application == null) throw new ObjectDisposedException("CaptureWorkbookSession");
        if (faulted) throw new InvalidOperationException("The workbook session failed and its process must be discarded.");
    }

    void VerifyProcess()
    {
        EnsureUsable();
        uint owner;
        GetWindowThreadProcessId(new IntPtr(Convert.ToInt64(Get(application, "Hwnd"))), out owner);
        if (process.HasExited || owner != (uint)process.Id ||
            process.StartTime.ToUniversalTime().Ticks != processBirthTicks)
            throw new InvalidOperationException("Excel application no longer belongs to the owned process.");
    }

    void VerifySeedOnly()
    {
        object workbooks = Get(application, "Workbooks");
        object seed = null;
        try
        {
            if (Convert.ToInt32(Get(workbooks, "Count")) != 1)
                throw new InvalidOperationException("Expected only the worker seed workbook.");
            seed = workbooks.GetType().InvokeMember("Item", BindingFlags.GetProperty, null,
                workbooks, new object[] { 1 }, AutomationCulture);
            if (!SamePath(Get(seed, "FullName"), seedPath))
                throw new InvalidOperationException("The remaining workbook is not the worker seed.");
        }
        finally { Release(seed); Release(workbooks); }
    }

    static bool SamePath(object value, string path)
    {
        return String.Equals(Path.GetFullPath(Convert.ToString(value)), path, StringComparison.OrdinalIgnoreCase);
    }

    static object Get(object target, string name)
    {
        return target.GetType().InvokeMember(name, BindingFlags.GetProperty, null, target, null, AutomationCulture);
    }

    static void Set(object target, string name, object value)
    {
        target.GetType().InvokeMember(name, BindingFlags.SetProperty, null, target, new object[] { value }, AutomationCulture);
    }

    static object Invoke(object target, string name, object[] arguments)
    {
        return target.GetType().InvokeMember(name, BindingFlags.InvokeMethod | BindingFlags.OptionalParamBinding,
            null, target, arguments, AutomationCulture);
    }

    static void Release(object value)
    {
        if (value != null && Marshal.IsComObject(value)) Marshal.ReleaseComObject(value);
    }

    /// <summary>Release COM references. The caller retains responsibility for the owned process.</summary>
    public void Dispose()
    {
        Release(currentWorkbook);
        currentWorkbook = null;
        Release(application);
        application = null;
    }

    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
