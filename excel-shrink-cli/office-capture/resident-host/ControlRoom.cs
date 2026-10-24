using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Microsoft.Extensions.FileProviders;

// This process owns the browser-facing control plane. The corpus scheduler and
// resident comparison groups exchange atomic JSON snapshots with it, so neither
// a browser reload nor an idle tab participates in the scientific comparison.
internal static class ControlRoom
{
    private const long SnapshotLimit = 32 * 1024 * 1024;
    private static readonly HashSet<int> PreviewWidths = [0, 128, 256, 512, 1024];
    private static readonly string[] ControlKeys = ["target_groups", "paused", "open_timeout_seconds", "job_timeout_seconds", "equal_preview_width"];

    public static async Task RunAsync(string settingsPath)
    {
        var settings = JsonNode.Parse(await File.ReadAllBytesAsync(settingsPath))?.AsObject()
            ?? throw new ArgumentException("Missing control room settings.");
        var root = Path.GetFullPath(RequiredString(settings, "root"));
        var web = Path.GetFullPath(RequiredString(settings, "web_root"));
        var port = RequiredInteger(settings, "port");
        var hostVersion = (string?)settings["host_version"] ?? "unspecified";
        if (port is < 1024 or > 65535 || !Directory.Exists(web))
            throw new ArgumentException("Invalid control room port or web root.");
        Directory.CreateDirectory(root);
        Directory.CreateDirectory(Path.Combine(root, "groups"));
        var controlPath = Path.Combine(root, "control.json");
        var initial = Defaults();
        if (settings["initial_controls"] is JsonObject supplied) initial = MergeControls(initial, supplied);
        if (File.Exists(controlPath)) initial = MergeControls(Defaults(), await ReadObject(controlPath));
        else await WriteAtomic(controlPath, initial);
        var controlGate = new SemaphoreSlim(1, 1);
        var started = DateTimeOffset.UtcNow;

        var builder = WebApplication.CreateBuilder();
        builder.Logging.SetMinimumLevel(LogLevel.Warning);
        builder.WebHost.UseUrls($"http://127.0.0.1:{port}");
        builder.WebHost.ConfigureKestrel(server => server.Limits.MaxRequestBodySize = 16 * 1024);
        var app = builder.Build();
        app.Use(async (context, next) => {
            context.Response.Headers.CacheControl = "no-store";
            context.Response.Headers["X-Content-Type-Options"] = "nosniff";
            if (!LocalRequest(context.Request, port)) {
                context.Response.StatusCode = StatusCodes.Status403Forbidden;
                await context.Response.WriteAsJsonAsync(new { error = "Only the local control room origin is allowed." });
                return;
            }
            try { await next(); }
            catch (Exception error) when (error is ArgumentException or InvalidOperationException or JsonException or IOException or UnauthorizedAccessException or FormatException) {
                if (context.Response.HasStarted) throw;
                context.Response.StatusCode = StatusCodes.Status400BadRequest;
                await context.Response.WriteAsJsonAsync(new { error = error.Message });
            }
        });
        app.UseDefaultFiles(new DefaultFilesOptions { FileProvider = new PhysicalFileProvider(web) });
        app.UseStaticFiles(new StaticFileOptions { FileProvider = new PhysicalFileProvider(web) });

        app.MapGet("/health", () => Results.Json(new {
            ok = true, kind = "excel-shrink-control-room", root, port, host_version = hostVersion, started_at = started, live = true
        }));

        app.MapGet("/api/state", async () => {
            var diagnostics = new JsonArray();
            var queue = await ReadSnapshot(Path.Combine(root, "queue.json"), diagnostics) ?? new JsonObject();
            CheckRunnerOwnership(queue, diagnostics);
            queue["groups"] ??= new JsonArray();
            queue["controls"] = await ReadControls(controlPath, initial, diagnostics);
            queue["memory"] = MemorySnapshot();
            queue["server"] = new JsonObject {
                ["started_at"] = started.ToString("O"), ["utc"] = DateTimeOffset.UtcNow.ToString("O"),
                ["port"] = port, ["live"] = true
            };
            queue["diagnostics"] = diagnostics;
            return Results.Json(queue);
        });

        app.MapGet("/api/groups/{id}", async (string id) => {
            id = SafeId(id);
            var diagnostics = new JsonArray();
            var queue = await ReadSnapshot(Path.Combine(root, "queue.json"), diagnostics);
            if (queue is not null) CheckRunnerOwnership(queue, diagnostics);
            var summary = (queue?["groups"] as JsonArray)?.OfType<JsonObject>()
                .FirstOrDefault(group => (string?)group["id"] == id)?.DeepClone().AsObject();
            var scheduledStatus = (string?)summary?["status"];
            var directory = ConfinedPath(root, $"groups/{id}");
            var detail = await ReadSnapshot(Path.Combine(directory, "state.json"), diagnostics);
            detail ??= await ReadSnapshot(Path.Combine(directory, "session.json"), diagnostics);
            if (summary is null && detail is null) return Results.NotFound(new { error = "Unknown comparison group." });
            var merged = summary ?? new JsonObject();
            if (detail is not null) {
                if (detail["id"] is not null && (string?)detail["id"] != id)
                    throw new ArgumentException("Group snapshot identity differs from its directory.");
                foreach (var entry in detail) merged[entry.Key] = entry.Value?.DeepClone();
            }
            if (scheduledStatus is "queued" or "complete" or "completed" or "failed" or "skipped" or "unchanged" or "differences" or "waiting_cleanup" or "interrupted" or "retrying_classic") {
                merged["capture_status"] = detail?["status"]?.DeepClone();
                merged["status"] = scheduledStatus;
            }
            var launcher = await ReadSnapshot(Path.Combine(directory, "launcher.json"), diagnostics);
            if (launcher is not null) merged["launcher"] = launcher;
            merged["id"] = id;
            merged["available"] = detail is not null;
            merged["diagnostics"] = diagnostics;
            return Results.Json(merged);
        });

        app.MapPost("/api/control", async (HttpRequest request) => {
            if (!request.HasJsonContentType()) return Results.StatusCode(StatusCodes.Status415UnsupportedMediaType);
            var patch = (await JsonNode.ParseAsync(request.Body))?.AsObject()
                ?? throw new ArgumentException("Expected a JSON object of control changes.");
            await controlGate.WaitAsync();
            try {
                var current = MergeControls(Defaults(), await ReadObject(controlPath));
                var next = MergeControls(current, patch);
                await WriteAtomic(controlPath, next);
                return Results.Json(new { ok = true, controls = next });
            } finally { controlGate.Release(); }
        });

        app.MapGet("/data/{**path}", (string path) => {
            var full = ConfinedPath(root, path);
            var contentType = Path.GetExtension(full).ToLowerInvariant() switch {
                ".png" => "image/png", ".webp" => "image/webp", ".jpg" or ".jpeg" => "image/jpeg", _ => null
            };
            if (contentType is null || !File.Exists(full)) return Results.NotFound();
            return Results.File(full, contentType, enableRangeProcessing: true);
        });

        await app.RunAsync();
    }

    private static JsonObject Defaults() => new() {
        ["target_groups"] = 1, ["paused"] = false, ["open_timeout_seconds"] = 600,
        ["job_timeout_seconds"] = 300, ["equal_preview_width"] = 512
    };

    private static JsonObject MergeControls(JsonObject current, JsonObject patch)
    {
        var next = current.DeepClone().AsObject();
        foreach (var entry in patch) {
            if (!ControlKeys.Contains(entry.Key, StringComparer.Ordinal))
                throw new ArgumentException($"Unknown control '{entry.Key}'.");
            if (entry.Key == "paused") {
                if (entry.Value is not JsonValue flag || !flag.TryGetValue<bool>(out var paused))
                    throw new ArgumentException("paused must be a boolean.");
                next[entry.Key] = paused;
            } else {
                if (entry.Value is not JsonValue scalar || !scalar.TryGetValue<int>(out var value))
                    throw new ArgumentException($"{entry.Key} must be an integer.");
                if (entry.Key == "equal_preview_width" ? !PreviewWidths.Contains(value) : value <= 0)
                    throw new ArgumentException($"Invalid value for {entry.Key}.");
                next[entry.Key] = value;
            }
        }
        return next;
    }

    private static async Task<JsonObject> ReadControls(string path, JsonObject fallback, JsonArray diagnostics)
    {
        try { return MergeControls(Defaults(), await ReadObject(path)); }
        catch (Exception error) when (error is IOException or JsonException or InvalidOperationException or ArgumentException) {
            diagnostics.Add(new JsonObject { ["file"] = "control.json", ["error"] = error.Message });
            return fallback.DeepClone().AsObject();
        }
    }

    private static async Task<JsonObject?> ReadSnapshot(string path, JsonArray diagnostics)
    {
        if (!File.Exists(path)) return null;
        try { return await ReadObject(path); }
        catch (Exception error) when (error is IOException or JsonException or InvalidOperationException) {
            diagnostics.Add(new JsonObject { ["file"] = Path.GetFileName(path), ["error"] = error.Message });
            return null;
        }
    }

    private static async Task<JsonObject> ReadObject(string path)
    {
        // Atomic rename is the producer contract. Delete sharing lets a writer
        // replace a snapshot while an HTTP response still reads the previous one.
        await using var stream = new FileStream(path, FileMode.Open, FileAccess.Read,
            FileShare.ReadWrite | FileShare.Delete, 16 * 1024, FileOptions.Asynchronous);
        if (stream.Length > SnapshotLimit) throw new IOException("Snapshot exceeds the 32 MiB metadata limit.");
        return (await JsonNode.ParseAsync(stream))?.AsObject() ?? throw new JsonException("Expected an object snapshot.");
    }

    private static async Task WriteAtomic(string path, JsonObject value)
    {
        var temporary = path + "." + Guid.NewGuid().ToString("N") + ".pending";
        try {
            await File.WriteAllBytesAsync(temporary, JsonSerializer.SerializeToUtf8Bytes(value));
            for (var attempt = 0; ; attempt++) {
                try { File.Move(temporary, path, overwrite: true); break; }
                catch (IOException) when (attempt < 19) { await Task.Delay(50); }
            }
        } finally {
            if (File.Exists(temporary)) File.Delete(temporary);
        }
    }

    private static bool LocalRequest(HttpRequest request, int port)
    {
        static bool IsLoopbackName(string name) => name.Equals("localhost", StringComparison.OrdinalIgnoreCase)
            || name is "127.0.0.1" or "[::1]" or "::1";
        if (!IsLoopbackName(request.Host.Host) || request.Host.Port != port) return false;
        var origin = request.Headers.Origin.ToString();
        if (origin.Length != 0) {
            if (!Uri.TryCreate(origin, UriKind.Absolute, out var uri) || uri.Scheme != "http" ||
                uri.Port != port || !string.Equals(uri.Host.Trim('[', ']'), request.Host.Host.Trim('[', ']'), StringComparison.OrdinalIgnoreCase) ||
                uri.UserInfo.Length != 0 || uri.AbsolutePath != "/" || uri.Query.Length != 0 || uri.Fragment.Length != 0)
                return false;
        }
        return request.Headers["Sec-Fetch-Site"].ToString() is not "cross-site";
    }

    private static string ConfinedPath(string root, string relative)
    {
        if (string.IsNullOrWhiteSpace(relative) || relative.Contains('\\') || relative.Contains(':') || Path.IsPathRooted(relative))
            throw new ArgumentException("Invalid relative asset path.");
        var segments = relative.Split('/');
        if (segments.Any(segment => segment is "" or "." or ".." || segment.EndsWith('.') || segment.EndsWith(' ')
            || segment.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0))
            throw new ArgumentException("Invalid asset path segment.");
        var path = Path.GetFullPath(Path.Combine(root, Path.Combine(segments)));
        var prefix = root.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar) + Path.DirectorySeparatorChar;
        if (!path.StartsWith(prefix, OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal))
            throw new ArgumentException("Asset lies outside the control room data directory.");
        var current = root;
        foreach (var segment in segments) {
            current = Path.Combine(current, segment);
            if ((File.Exists(current) || Directory.Exists(current)) && (File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                throw new ArgumentException("Linked assets are not served by the control room.");
        }
        return path;
    }

    private static string SafeId(string value) => Regex.IsMatch(value, "\\A[A-Za-z0-9][A-Za-z0-9_-]{0,127}\\z")
        ? value : throw new ArgumentException("Invalid comparison group id.");
    private static string RequiredString(JsonObject value, string key) => (string?)value[key]
        ?? throw new ArgumentException($"Missing {key}.");
    private static int RequiredInteger(JsonObject value, string key) => (int?)value[key]
        ?? throw new ArgumentException($"Missing {key}.");

    private static JsonObject MemorySnapshot()
    {
        if (OperatingSystem.IsWindows()) {
            var status = new MemoryStatus { Length = (uint)Marshal.SizeOf<MemoryStatus>() };
            if (GlobalMemoryStatusEx(ref status)) return new JsonObject {
                ["total_bytes"] = status.TotalPhysical, ["available_bytes"] = status.AvailablePhysical,
                ["load_percent"] = status.Load, ["source"] = "GlobalMemoryStatusEx"
            };
        }
        return new JsonObject { ["total_bytes"] = null, ["available_bytes"] = null, ["load_percent"] = null, ["source"] = "unavailable" };
    }

    private static void CheckRunnerOwnership(JsonObject queue, JsonArray diagnostics)
    {
        bool? alive = null;
        var reason = "Runner process identity has not been recorded.";
        if (queue["runner_pid"] is JsonValue pidValue && pidValue.TryGetValue<int>(out var pid) && pid > 0 &&
            queue["runner_started_unix_ms"] is JsonValue timeValue && timeValue.TryGetValue<long>(out var expectedStart) && expectedStart > 0) {
            try {
                using var process = System.Diagnostics.Process.GetProcessById(pid);
                var actualStart = new DateTimeOffset(process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds();
                alive = !process.HasExited && Math.Abs(actualStart - expectedStart) <= 1;
                reason = "Runner exited or its process id now belongs to a different process.";
            } catch (Exception error) when (error is ArgumentException or InvalidOperationException) {
                alive = false;
                reason = "Runner process has exited.";
            } catch (System.ComponentModel.Win32Exception) {
                reason = "Runner process identity could not be verified.";
            }
        }
        queue["runner_alive"] = alive is null ? null : JsonValue.Create(alive.Value);
        if ((bool?)queue["running"] != true || alive == true) return;
        // This affects only the response, never the scheduler's durable queue.
        queue["runner_reported_running"] = true;
        queue["running"] = false;
        queue["phase"] = alive == false ? "interrupted" : "owner_unverified";
        diagnostics.Add(new JsonObject { ["file"] = "queue.json", ["error"] = reason });
        if (alive == false && queue["groups"] is JsonArray groups) {
            foreach (var group in groups.OfType<JsonObject>()) {
                if ((bool?)group["active"] != true) continue;
                group["active"] = false;
                group["status"] = "interrupted";
            }
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MemoryStatus
    {
        public uint Length, Load;
        public ulong TotalPhysical, AvailablePhysical, TotalPageFile, AvailablePageFile;
        public ulong TotalVirtual, AvailableVirtual, AvailableExtendedVirtual;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GlobalMemoryStatusEx(ref MemoryStatus status);
}
