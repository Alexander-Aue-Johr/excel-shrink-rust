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
    private static readonly string[] ControlKeys = ["target_groups", "paused", "open_timeout_seconds", "job_timeout_seconds", "equal_preview_width", "capture_tile_megapixels", "capture_batch_tiles", "tall_sheet_threshold", "tall_sheet_edge_tiles", "hide_excel"];

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
        var commandsDirectory = ConfinedPath(root, "commands");
        Directory.CreateDirectory(commandsDirectory);
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
            queue.Remove("progress");
            var progress = await ReadSnapshot(Path.Combine(root, "progress.json"), diagnostics, 64 * 1024);
            if (queue["run_id"] is not null && (string?)progress?["run_id"] == (string?)queue["run_id"])
                queue["progress"] = progress;
            queue["memory"] = MemorySnapshot();
            queue["server"] = new JsonObject {
                ["started_at"] = started.ToString("O"), ["utc"] = DateTimeOffset.UtcNow.ToString("O"),
                ["port"] = port, ["live"] = true
            };
            queue["diagnostics"] = diagnostics;
            return Results.Json(queue);
        });

        app.MapGet("/api/workers", async (HttpRequest request) => {
            var diagnostics = new JsonArray();
            var queue = await ReadSnapshot(Path.Combine(root, "queue.json"), diagnostics) ?? new JsonObject();
            CheckRunnerOwnership(queue, diagnostics);
            var offsetText = request.Query["offset"].ToString();
            var offset = 0;
            if (offsetText.Length > 0 && (!int.TryParse(offsetText, out offset) || offset < 0))
                throw new ArgumentException("Worker page offset must be a nonnegative integer.");
            var all = (queue["groups"] as JsonArray)?.OfType<JsonObject>().ToArray() ?? [];
            var selected = all.Where(group => (bool?)group["active"] == true).ToList();
            if (request.Query["include_recent"] == "true") {
                selected.AddRange(all.Where(group => (bool?)group["active"] != true &&
                    (string?)group["run_id"] == (string?)queue["run_id"] &&
                    (string?)group["status"] is "unchanged" or "differences" or "skipped" or "failed" or "interrupted" or "complete" or "completed")
                    .OrderByDescending(group => (long?)group["started_unix_ms"] ?? 0).Take(8));
            }
            const int pageSize = 64;
            var groups = new JsonArray();
            foreach (var group in selected.Skip(offset).Take(pageSize))
                groups.Add(await WorkerGroup(root, group, diagnostics));
            return Results.Json(new JsonObject {
                ["groups"] = groups, ["offset"] = offset, ["total_groups"] = selected.Count,
                ["next_offset"] = offset + pageSize < selected.Count ? JsonValue.Create(offset + pageSize) : null,
                ["active_groups"] = all.Count(group => (bool?)group["active"] == true),
                ["running"] = queue["running"]?.DeepClone(), ["runner_alive"] = queue["runner_alive"]?.DeepClone(),
                ["sampled_at"] = DateTimeOffset.UtcNow.ToString("O"), ["diagnostics"] = diagnostics
            });
        });

        app.MapGet("/api/groups/{id}", async (string id) => {
            id = SafeId(id);
            var diagnostics = new JsonArray();
            var queue = await ReadSnapshot(Path.Combine(root, "queue.json"), diagnostics);
            if (queue is not null) CheckRunnerOwnership(queue, diagnostics);
            var summary = (queue?["groups"] as JsonArray)?.OfType<JsonObject>()
                .FirstOrDefault(group => (string?)group["id"] == id)?.DeepClone().AsObject();
            var scheduledStatus = (string?)summary?["status"];
            var scheduledAttempt = (string?)summary?["attempt_id"];
            var directory = ConfinedPath(root, $"groups/{id}");
            var detail = await ReadSnapshot(Path.Combine(directory, "state.json"), diagnostics);
            detail ??= await ReadSnapshot(Path.Combine(directory, "session.json"), diagnostics);
            if (summary is not null && scheduledAttempt != (string?)detail?["attempt_id"])
                detail = null;
            if (summary is null && detail is null) return Results.NotFound(new { error = "Unknown comparison group." });
            var merged = summary ?? new JsonObject();
            if (detail is not null) {
                if (detail["id"] is not null && (string?)detail["id"] != id)
                    throw new ArgumentException("Group snapshot identity differs from its directory.");
                foreach (var entry in detail) merged[entry.Key] = entry.Value?.DeepClone();
            }
            if (scheduledAttempt is not null) merged["attempt_id"] = scheduledAttempt;
            if (scheduledStatus is "queued" or "preparing" or "restart_pending" or "complete" or "completed" or "failed" or "skipped" or "unchanged" or "differences" or "waiting_cleanup" or "restart_waiting_cleanup" or "interrupted" or "retrying_classic") {
                merged["capture_status"] = detail?["status"]?.DeepClone();
                merged["status"] = scheduledStatus;
            }
            var launcher = await ReadSnapshot(Path.Combine(directory, "launcher.json"), diagnostics);
            var launcherAttempt = (string?)launcher?["attempt_id"];
            var displayedAttempt = scheduledAttempt ?? (string?)merged["attempt_id"];
            if (displayedAttempt is not null && launcherAttempt != displayedAttempt) {
                merged["launcher_pending"] = true;
            } else if (launcher is not null) {
                CheckClientProcesses(launcher);
                merged["launcher"] = launcher;
            }
            merged["restart_commands"] = await RecentCommands(commandsDirectory, id, diagnostics);
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

        app.MapPost("/api/groups/{id}/restart", async (string id, HttpRequest request) => {
            id = SafeId(id);
            if (!request.HasJsonContentType()) return Results.StatusCode(StatusCodes.Status415UnsupportedMediaType);
            var body = (await JsonNode.ParseAsync(request.Body))?.AsObject()
                ?? throw new ArgumentException("Expected a JSON object.");
            if (body.Any(entry => entry.Key is not "client_id" and not "reason"))
                throw new ArgumentException("Unknown restart option.");
            var clientId = (string?)body["client_id"];
            if (clientId is not null) clientId = SafeId(clientId);
            var reason = (string?)body["reason"] ?? "Restart requested in the control room.";
            if (reason.Length > 1000) throw new ArgumentException("Restart reason exceeds 1000 characters.");
            await controlGate.WaitAsync();
            try {
                var diagnostics = new JsonArray();
                var queue = await ReadSnapshot(Path.Combine(root, "queue.json"), diagnostics);
                if (queue is null) return Results.Conflict(new { error = "The scheduler queue is not available." });
                CheckRunnerOwnership(queue, diagnostics);
                if ((bool?)queue["running"] != true || (bool?)queue["runner_alive"] != true)
                    return Results.Conflict(new { error = "Start the corpus runner before requesting a restart." });
                var group = (queue["groups"] as JsonArray)?.OfType<JsonObject>()
                    .FirstOrDefault(group => (string?)group["id"] == id);
                if (group is null) return Results.NotFound(new { error = "Unknown comparison group." });
                var runId = RequiredString(queue, "run_id");
                if ((string?)group["run_id"] != runId || (bool?)group["restart_eligible"] == false)
                    return Results.Conflict(new { error = "This group belongs to an earlier run; select it in a new corpus run to compare it again." });
                if (clientId is not null) {
                    var launcher = await ReadSnapshot(ConfinedPath(root, $"groups/{id}/launcher.json"), diagnostics);
                    if (launcher?["clients"] is not JsonObject clients || !clients.ContainsKey(clientId))
                        return Results.NotFound(new { error = "Unknown Excel client in this group." });
                }
                // A double-click retries the existing request instead of queuing
                // the same scientific comparison twice. The scheduler owns ack.
                foreach (var pending in (await RecentCommands(commandsDirectory, id, diagnostics)).OfType<JsonObject>()) {
                    if ((string?)pending["run_id"] == runId && (string?)pending["attempt_id"] == (string?)group["attempt_id"] &&
                        (string?)pending["status"] is "pending" or "restarting" or "queued")
                        return Results.Accepted($"/api/commands/{(string?)pending["id"]}", new { ok = true, restart_scope = "group", command = pending });
                }
                var commandId = Guid.NewGuid().ToString("N");
                var command = new JsonObject {
                    ["id"] = commandId, ["type"] = "restart_group", ["group_id"] = id,
                    ["run_id"] = runId, ["client_id"] = clientId, ["reason"] = reason,
                    ["attempt_id"] = group["attempt_id"]?.DeepClone(),
                    ["requested_unix_ms"] = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                    ["status"] = "pending", ["restart_scope"] = "group",
                    ["message"] = "The whole comparison will be requeued next after safe cleanup of its current peer groups."
                };
                await WriteAtomic(ConfinedPath(root, $"commands/{commandId}.json"), command);
                return Results.Accepted($"/api/commands/{commandId}", new { ok = true, restart_scope = "group", command });
            } finally { controlGate.Release(); }
        });

        app.MapGet("/api/commands/{id}", async (string id) => {
            id = SafeId(id);
            var command = await ReadSnapshot(ConfinedPath(root, $"commands/{id}.json"), new JsonArray());
            return command is null ? Results.NotFound(new { error = "Unknown command." }) : Results.Json(command);
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
        ["job_timeout_seconds"] = 300, ["equal_preview_width"] = 512,
        ["capture_tile_megapixels"] = 32, ["capture_batch_tiles"] = 1, ["hide_excel"] = false,
        ["tall_sheet_threshold"] = 40, ["tall_sheet_edge_tiles"] = 2
    };

    private static JsonObject MergeControls(JsonObject current, JsonObject patch)
    {
        var next = current.DeepClone().AsObject();
        foreach (var entry in patch) {
            if (!ControlKeys.Contains(entry.Key, StringComparer.Ordinal))
                throw new ArgumentException($"Unknown control '{entry.Key}'.");
            if (entry.Key is "paused" or "hide_excel") {
                if (entry.Value is not JsonValue flag || !flag.TryGetValue<bool>(out var paused))
                    throw new ArgumentException($"{entry.Key} must be a boolean.");
                next[entry.Key] = paused;
            } else {
                if (entry.Value is not JsonValue scalar || !scalar.TryGetValue<int>(out var value))
                    throw new ArgumentException($"{entry.Key} must be an integer.");
                if (entry.Key == "equal_preview_width" ? !PreviewWidths.Contains(value) : entry.Key == "target_groups" ? value < 0 : value <= 0)
                    throw new ArgumentException($"Invalid value for {entry.Key}.");
                if (entry.Key == "capture_tile_megapixels" && value > 32 || entry.Key == "capture_batch_tiles" && value > 16)
                    throw new ArgumentException($"Invalid value for {entry.Key}.");
                next[entry.Key] = value;
            }
        }
        if ((int?)next["target_groups"] == 0) {
            if ((bool?)patch["paused"] == true)
                throw new ArgumentException("Finish current work at group target 0; capture dispatch cannot also be paused.");
            // Draining is distinct from capture pause. Existing groups need
            // further capture commands to finish their complete comparison.
            next["paused"] = false;
        }
        return next;
    }

    private static async Task<JsonObject> WorkerGroup(string root, JsonObject summary, JsonArray diagnostics)
    {
        var id = SafeId(RequiredString(summary, "id"));
        var result = CopyFields(summary, "id", "workbook", "corpus", "status", "active", "attempt_id", "started_unix_ms");
        var attempt = (string?)summary["attempt_id"];
        var directory = ConfinedPath(root, $"groups/{id}");
        // These compact files exclude sheet geometry and images. A large
        // workbook's capture plan must not dominate worker-monitor polling.
        var coordinator = await ReadSnapshot(Path.Combine(directory, "coordinator.json"), diagnostics, 2 * 1024 * 1024);
        if (coordinator is not null && (attempt is null || (string?)coordinator["attempt_id"] == attempt)) {
            var monitor = CompactMonitor(coordinator);
            CheckProcess(monitor);
            result["coordinator"] = monitor;
        }
        var launcher = await ReadSnapshot(Path.Combine(directory, "launcher.json"), diagnostics, 2 * 1024 * 1024);
        var clients = new JsonArray();
        if (launcher is not null && (attempt is null || (string?)launcher["attempt_id"] == attempt)) {
            CheckClientProcesses(launcher);
            if (launcher["clients"] is JsonObject processes) foreach (var entry in processes) {
                if (entry.Value is not JsonObject client) continue;
                var item = CopyFields(client, "pid", "process_started_unix_ms", "process_alive", "status", "stage",
                    "stage_since", "requested_at", "opened_at", "hide_excel", "heartbeat_count", "last_heartbeat_at",
                    "active_command", "error", "last_launcher_event", "last_launcher_success");
                item["client_id"] = entry.Key;
                item["worker_id"] = $"{id}/{attempt ?? "legacy"}/{entry.Key}/{client["pid"]}/{client["process_started_unix_ms"]}";
                if (client["monitor"] is JsonObject monitor) item["monitor"] = CompactMonitor(monitor);
                clients.Add(item);
            }
            result["launcher_status"] = launcher["status"]?.DeepClone();
            result["launcher_sampled_unix_ms"] = launcher["sampled_unix_ms"]?.DeepClone();
        } else result["launcher_pending"] = true;
        result["clients"] = clients;
        return result;
    }

    private static JsonObject CompactMonitor(JsonObject value)
    {
        var result = CopyFields(value, "attempt_id", "pid", "process_started_unix_ms", "phase", "state", "state_since",
            "state_since_unix_ms", "last_progress_unix_ms", "role", "workbook",
            "wait_reason", "last_progress_at", "last_progress", "last_event", "last_success", "active_job",
            "uploaded_images", "completed_commands", "sheet_name", "sheet_index", "profile");
        result["events"] = new JsonArray((value["events"] as JsonArray)?.TakeLast(24)
            .Select(item => item?.DeepClone()).ToArray() ?? []);
        return result;
    }

    private static JsonObject CopyFields(JsonObject source, params string[] fields)
    {
        var result = new JsonObject();
        foreach (var field in fields) if (source[field] is JsonNode value) result[field] = value.DeepClone();
        return result;
    }

    private static async Task<JsonObject> ReadControls(string path, JsonObject fallback, JsonArray diagnostics)
    {
        try { return MergeControls(Defaults(), await ReadObject(path)); }
        catch (Exception error) when (error is IOException or JsonException or InvalidOperationException or ArgumentException) {
            diagnostics.Add(new JsonObject { ["file"] = "control.json", ["error"] = error.Message });
            return fallback.DeepClone().AsObject();
        }
    }

    private static async Task<JsonArray> RecentCommands(string directory, string groupId, JsonArray diagnostics)
    {
        var commands = new List<JsonObject>();
        foreach (var file in new DirectoryInfo(directory).EnumerateFiles("*.json").OrderByDescending(file => file.LastWriteTimeUtc)) {
            if ((file.Attributes & FileAttributes.ReparsePoint) != 0) continue;
            var command = await ReadSnapshot(file.FullName, diagnostics);
            if ((string?)command?["group_id"] != groupId) continue;
            commands.Add(command!);
            if (commands.Count == 10) break;
        }
        return new JsonArray(commands.OrderByDescending(command => (long?)command["requested_unix_ms"] ?? 0)
            .Select(command => (JsonNode)command).ToArray());
    }

    private static void CheckClientProcesses(JsonObject launcher)
    {
        if (launcher["clients"] is not JsonObject clients) return;
        foreach (var entry in clients) {
            if (entry.Value is not JsonObject client) continue;
            CheckProcess(client);
        }
    }

    private static void CheckProcess(JsonObject processState)
    {
        bool? alive = null;
        if (processState["pid"] is JsonValue pidValue && pidValue.TryGetValue<int>(out var pid) && pid > 0 &&
            processState["process_started_unix_ms"] is JsonValue stamp && stamp.TryGetValue<long>(out var expectedStart)) {
            try {
                using var process = System.Diagnostics.Process.GetProcessById(pid);
                var actualStart = new DateTimeOffset(process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds();
                alive = !process.HasExited && Math.Abs(actualStart - expectedStart) <= 1;
            } catch (Exception error) when (error is ArgumentException or InvalidOperationException) { alive = false; }
            catch (System.ComponentModel.Win32Exception) { }
        }
        // Quiet protocol traffic does not imply a dead process. Check PID and
        // creation time together, including for the Rust workbook coordinator.
        processState["process_alive"] = alive is null ? null : JsonValue.Create(alive.Value);
        processState["process_checked_at"] = DateTimeOffset.UtcNow.ToString("O");
    }

    private static async Task<JsonObject?> ReadSnapshot(string path, JsonArray diagnostics, long maximum = SnapshotLimit)
    {
        if (!File.Exists(path)) return null;
        try { return await ReadObject(path, maximum); }
        catch (Exception error) when (error is IOException or JsonException or InvalidOperationException) {
            diagnostics.Add(new JsonObject { ["file"] = Path.GetFileName(path), ["error"] = error.Message });
            return null;
        }
    }

    private static async Task<JsonObject> ReadObject(string path, long maximum = SnapshotLimit)
    {
        // Atomic rename is the producer contract. Delete sharing lets a writer
        // replace a snapshot while an HTTP response still reads the previous one.
        await using var stream = new FileStream(path, FileMode.Open, FileAccess.Read,
            FileShare.ReadWrite | FileShare.Delete, 16 * 1024, FileOptions.Asynchronous);
        if (stream.Length > maximum) throw new IOException($"Snapshot exceeds the {maximum / 1024} KiB metadata limit.");
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
