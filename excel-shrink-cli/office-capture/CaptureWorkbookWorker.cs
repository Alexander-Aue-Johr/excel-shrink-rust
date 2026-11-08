using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Threading;
using System.Web.Script.Serialization;

/// <summary>
/// Serial JSON command transport for one owned warm Excel application.
/// Capture HTTP hosts and evidence directories belong to the requesting comparison.
/// </summary>
public static class CaptureWorkbookWorker
{
    const int MaximumCommandCharacters = 16384;

    public static void Run(string excelExecutable, string seedPath)
    {
        Console.InputEncoding = new System.Text.UTF8Encoding(false);
        Console.OutputEncoding = new System.Text.UTF8Encoding(false);
        var json = new JavaScriptSerializer();
        using (var owned = new CaptureProcesses())
        using (var desktop = new CaptureDesktop())
        {
            var process = desktop.Start(excelExecutable, "/x \"" + seedPath + "\"", owned);
            Reply(json, new { event_type = "process_started", pid = process.Id,
                started_utc = process.StartTime.ToUniversalTime().ToString("o"), desktop = desktop.Name });
            CaptureErrorChecking checking = null;
            var startup = Stopwatch.StartNew();
            while (checking == null && startup.Elapsed.TotalSeconds < 110)
            {
                checking = CaptureErrorChecking.TryDisable(process, desktop.Name);
                if (checking == null) Thread.Sleep(100);
            }
            if (checking == null) throw new TimeoutException("Owned Excel automation window did not become ready.");
            using (checking)
            using (var session = new CaptureWorkbookSession(checking.GetOwnedApplication(), process, seedPath))
            {
                session.Initialize();
                Reply(json, new { event_type = "ready", pid = process.Id });
                string line;
                while ((line = ReadCommand()) != null)
                {
                    string requestId = null;
                    try
                    {
                        var request = json.Deserialize<Dictionary<string, object>>(line);
                        requestId = RequiredString(request, "id");
                        Guid identity;
                        if (!Guid.TryParse(requestId, out identity)) throw new InvalidDataException("Command ID must be a GUID.");
                        string kind = RequiredString(request, "kind");
                        if (kind == "open") session.OpenReadOnly(RequiredString(request, "path"));
                        else if (kind == "close") session.CloseCurrent();
                        else if (kind == "stop")
                        {
                            session.CloseCurrent();
                            Reply(json, new { id = requestId, ok = true, pid = process.Id });
                            return;
                        }
                        else throw new InvalidDataException("Unknown workbook worker command.");
                        Reply(json, new { id = requestId, ok = true, pid = process.Id });
                    }
                    catch (Exception error)
                    {
                        Reply(json, new { id = requestId, ok = false, error = error.GetBaseException().Message });
                        // A caller must replace this worker after any failed command.
                        return;
                    }
                }
            }
        }
    }

    static string RequiredString(Dictionary<string, object> request, string key)
    {
        object value;
        if (request == null || !request.TryGetValue(key, out value) || !(value is string) ||
            String.IsNullOrWhiteSpace((string)value))
            throw new InvalidDataException("Missing command field " + key + ".");
        return (string)value;
    }

    static string ReadCommand()
    {
        var line = new System.Text.StringBuilder();
        int next;
        while ((next = Console.In.Read()) != -1)
        {
            if (next == '\n') return line.ToString();
            if (line.Length >= MaximumCommandCharacters)
                throw new InvalidDataException("Workbook worker command exceeds its character limit.");
            if (next != '\r') line.Append((char)next);
        }
        if (line.Length != 0) throw new EndOfStreamException("Incomplete workbook worker command.");
        return null;
    }

    static void Reply(JavaScriptSerializer json, object value)
    {
        Console.WriteLine(json.Serialize(value));
        Console.Out.Flush();
    }
}
