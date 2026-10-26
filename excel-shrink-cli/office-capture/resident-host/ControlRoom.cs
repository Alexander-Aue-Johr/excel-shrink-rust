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
    private static readonly string[] ControlKeys = ["target_groups", "paused", "system_reserve_mib", "open_timeout_seconds", "job_timeout_seconds", "equal_preview_width", "capture_tile_megapixels", "capture_batch_tiles", "tall_sheet_threshold", "tall_sheet_edge_tiles", "hide_excel"];

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
            queue["update_ready"] = (bool?)queue["runner_alive"] == false && OutputIdle(root);
            await ReconcileCompletedGroups(root, queue, diagnostics);
            queue["groups"] ??= new JsonArray();
            queue["controls"] = await ReadControls(controlPath, initial, diagnostics);
            var stopPath = Path.Combine(root, "stop-request.json");
            var stop = File.Exists(stopPath) ? await ReadSnapshot(stopPath, diagnostics) : null;
            queue["stop_requested"] = ActiveStopRequested(queue, stop);
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
            await ReconcileCompletedGroups(root, queue, diagnostics);
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
            if (queue is not null) {
                CheckRunnerOwnership(queue, diagnostics);
                await ReconcileCompletedGroups(root, queue, diagnostics);
            }
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
            if (scheduledStatus is "queued" or "preparing" or "restart_pending" or "complete" or "completed" or "failed" or "skipped" or "unchanged" or "differences" or "waiting_cleanup" or "cleanup_blocked" or "restart_waiting_cleanup" or "interrupted" or "retrying_classic") {
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
            merged["client_reset_commands"] = await RecentCommands(Path.Combine(directory, "client-resets"), id, diagnostics);
            var record = await ReadSnapshot(Path.Combine(Directory.GetParent(root)!.FullName, "records", id + ".json"), diagnostics);
            if (record?["cleanup_pending"] is JsonObject cleanup) merged["cleanup_pending"] = cleanup.DeepClone();
            var measurements = new JsonObject();
            foreach (var profile in (record?["results"] as JsonArray)?.OfType<JsonObject>() ?? []) {
                var name = (string?)profile["profile"];
                if (name is null) continue;
                measurements[name] = new JsonObject {
                    ["bytes"] = profile["shrunk_bytes"]?.DeepClone(), ["open_seconds"] = profile["shrunk_open_seconds"]?.DeepClone(),
                    ["memory_bytes"] = profile["shrunk_memory_bytes"]?.DeepClone(), ["shrink_seconds"] = profile["shrink_seconds"]?.DeepClone()
                };
                measurements["original"] ??= new JsonObject {
                    ["bytes"] = profile["original_bytes"]?.DeepClone(), ["open_seconds"] = profile["original_open_seconds"]?.DeepClone(),
                    ["memory_bytes"] = profile["original_memory_bytes"]?.DeepClone()
                };
                // Measurements are populated profile by profile while the run opens files.
                foreach (var field in new[] { "open_seconds", "memory_bytes" })
                    if (measurements["original"]![field] is null) measurements["original"]![field] = profile["original_" + field]?.DeepClone();
                var native = profile["native_startup"];
                if ((string?)native?["method"] == "launch-to-addin-ready-v1"
                    && (displayedAttempt is null || (string?)native?["attempt_id"] == displayedAttempt)) {
                    measurements[name]!["startup"] = NativeStartup(native?["shrunk"], (string?)native?["method"]);
                    measurements["original"]!["startup"] ??= NativeStartup(native?["original"], (string?)native?["method"]);
                }
            }
            // Live samples are attached only to the displayed attempt. Historical
            // COM measurements retain their own fields and are never relabelled.
            if (merged["launcher"]?["clients"] is JsonObject liveClients) {
                foreach (var entry in measurements.ToArray()) {
                    var physical = (string?)merged["aliases"]?[entry.Key] ?? entry.Key;
                    var client = liveClients[physical];
                    if (client is null) continue;
                    var sample = new JsonObject {
                        ["seconds"] = client["startup_seconds"]?.DeepClone(),
                        ["private_bytes"] = client["startup_memory_private_bytes"]?.DeepClone(),
                        ["working_set_bytes"] = client["startup_memory_working_set_bytes"]?.DeepClone(),
                        ["sampled_unix_ms"] = client["startup_sampled_unix_ms"]?.DeepClone(),
                        ["ready_at"] = client["ready_at"]?.DeepClone(),
                        ["pid"] = client["pid"]?.DeepClone(),
                        ["process_started_unix_ms"] = client["process_started_unix_ms"]?.DeepClone(),
                        ["client_id"] = physical
                    };
                    entry.Value!["startup"] = NativeStartup(sample, (string?)client["startup_measurement_method"]);
                }
            }
            merged["measurements"] = measurements;
            merged["id"] = id;
            merged["available"] = detail is not null;
            merged["diagnostics"] = diagnostics;
            return Results.Json(merged);
        });

        app.MapPost("/api/run/stop", async () => {
            await controlGate.WaitAsync();
            try {
                var diagnostics = new JsonArray();
                var queue = await ReadObject(Path.Combine(root, "queue.json"));
                CheckRunnerOwnership(queue, diagnostics);
                if ((bool?)queue["running"] != true || (bool?)queue["runner_alive"] != true)
                    return Results.Conflict(new { error = "There is no verified active runner to stop." });
                if ((bool?)queue["stop_supported"] != true)
                    return Results.Conflict(new { error = "This runner predates orderly stopping. Stop it from the updated corpus configurator." });
                await WriteAtomic(Path.Combine(root, "stop-request.json"), new JsonObject {
                    ["runner_pid"] = queue["runner_pid"]?.DeepClone(),
                    ["runner_started_unix_ms"] = queue["runner_started_unix_ms"]?.DeepClone()
                });
                var current = MergeControls(Defaults(), await ReadObject(controlPath));
                await WriteAtomic(controlPath, MergeControls(current, new JsonObject { ["target_groups"] = 0, ["paused"] = false }));
                return Results.Json(new { ok = true, message = "Stopping after active workbooks and Excel cleanup finish." });
            } finally { controlGate.Release(); }
        });

        app.MapPost("/api/control", async (HttpRequest request) => {
            if (!request.HasJsonContentType()) return Results.StatusCode(StatusCodes.Status415UnsupportedMediaType);
            var patch = (await JsonNode.ParseAsync(request.Body))?.AsObject()
                ?? throw new ArgumentException("Expected a JSON object of control changes.");
            await controlGate.WaitAsync();
            try {
                var current = MergeControls(Defaults(), await ReadObject(controlPath));
                var next = MergeControls(current, patch);
                var queuePath = Path.Combine(root, "queue.json");
                var stopPath = Path.Combine(root, "stop-request.json");
                var queue = File.Exists(stopPath) && File.Exists(queuePath) ? await ReadObject(queuePath) : new JsonObject();
                var stop = File.Exists(stopPath) ? await ReadObject(stopPath) : new JsonObject();
                CheckRunnerOwnership(queue, new JsonArray());
                if (ActiveStopRequested(queue, stop)) {
                    next["target_groups"] = 0;
                    next["paused"] = false;
                }
                await WriteAtomic(controlPath, next);
                return Results.Json(new { ok = true, controls = next });
            } finally { controlGate.Release(); }
        });

        app.MapPost("/api/groups/{id}/restart", async (string id, HttpRequest request) => {
            id = SafeId(id);
            if (!request.HasJsonContentType()) return Results.StatusCode(StatusCodes.Status415UnsupportedMediaType);
            var body = (await JsonNode.ParseAsync(request.Body))?.AsObject()
                ?? throw new ArgumentException("Expected a JSON object.");
            if (body.Any(entry => entry.Key is not "client_id" and not "reason" and not "hard_reset" and not "expected_attempt_id"))
                throw new ArgumentException("Unknown restart option.");
            var hardReset = body["hard_reset"] is null ? false : (bool)body["hard_reset"]!;
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
            await ReconcileCompletedGroups(root, queue, diagnostics);
                if ((bool?)queue["running"] != true || (bool?)queue["runner_alive"] != true)
                    return Results.Conflict(new { error = "Start the corpus runner before requesting a restart." });
                var group = (queue["groups"] as JsonArray)?.OfType<JsonObject>()
                    .FirstOrDefault(group => (string?)group["id"] == id);
                if (group is null) return Results.NotFound(new { error = "Unknown comparison group." });
                if (body["expected_attempt_id"] is not null && (string?)body["expected_attempt_id"] != (string?)group["attempt_id"])
                    return Results.Conflict(new { error = "This Excel group has already been replaced. Refresh the view before retrying." });
                var runId = RequiredString(queue, "run_id");
                if ((string?)group["run_id"] != runId || (bool?)group["restart_eligible"] == false)
                    return Results.Conflict(new { error = "This group belongs to an earlier run; select it in a new corpus run to compare it again." });
                if (clientId is not null) {
                    var launcher = await ReadSnapshot(ConfinedPath(root, $"groups/{id}/launcher.json"), diagnostics);
                    if (launcher?["clients"] is not JsonObject clients || !clients.ContainsKey(clientId))
                        return Results.NotFound(new { error = "Unknown Excel client in this group." });
                    if (hardReset && (bool?)launcher["client_reset_supported"] == true && (bool?)group["active"] == true && (string?)launcher["attempt_id"] == (string?)group["attempt_id"] &&
                        (string?)launcher["status"] == "running" && clients[clientId]?["pid"] is not null && (bool?)clients[clientId]?["closed"] != true) {
                        var resets = ConfinedPath(root, $"groups/{id}/client-resets");
                        Directory.CreateDirectory(resets);
                        foreach (var path in Directory.EnumerateFiles(resets, "*.json")) {
                            var pendingReset = await ReadSnapshot(path, diagnostics);
                            if ((string?)pendingReset?["attempt_id"] == (string?)group["attempt_id"] &&
                                (string?)pendingReset?["client_id"] == clientId && (string?)pendingReset?["status"] is "pending" or "running")
                                return Results.Accepted(value: new { ok = true, restart_scope = "client", command = pendingReset });
                        }
                        var resetId = Guid.NewGuid().ToString("N");
                        var controls = await ReadControls(controlPath, initial, diagnostics);
                        var reset = new JsonObject {
                            ["id"] = resetId, ["type"] = "reset_client", ["client_id"] = clientId, ["group_id"] = id,
                            ["attempt_id"] = group["attempt_id"]?.DeepClone(), ["run_id"] = runId,
                            ["expected_pid"] = clients[clientId]!["pid"]?.DeepClone(),
                            ["expected_started_unix_ms"] = clients[clientId]!["process_started_unix_ms"]?.DeepClone(),
                            ["hide_excel"] = controls["hide_excel"]?.DeepClone(), ["reason"] = reason,
                            ["status"] = "pending", ["requested_unix_ms"] = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()
                        };
                        await WriteAtomic(Path.Combine(resets, resetId + ".json"), reset);
                        return Results.Accepted(value: new { ok = true, restart_scope = "client", command = reset });
                    }
                }
                if (hardReset && clientId is null) throw new ArgumentException("Hard reset requires an Excel client.");
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

    private static JsonObject? NativeStartup(JsonNode? sample, string? method)
    {
        if (method != "launch-to-addin-ready-v1" || sample is not JsonObject data
            || data["seconds"] is null || data["sampled_unix_ms"] is null
            || data["ready_at"] is null || data["pid"] is null || data["process_started_unix_ms"] is null)
            return null;
        var result = (JsonObject)data.DeepClone();
        result["method"] = method;
        return result;
    }

    private static JsonObject Defaults() => new() {
        ["target_groups"] = 1, ["paused"] = false, ["open_timeout_seconds"] = 600,
        ["system_reserve_mib"] = 1024,
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
                if (entry.Key == "system_reserve_mib" && value > 65536)
                    throw new ArgumentException("system_reserve_mib must be between 1 and 65536 MiB.");
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

    // Reconcile presentation only. Never rewrite scheduler files or infer success
    // from a missing process: require a saved result for this exact attempt.
    private static async Task ReconcileCompletedGroups(string root, JsonObject queue, JsonArray diagnostics)
    {
        if (queue["groups"] is not JsonArray groups) return;
        var reconciled = 0;
        foreach (var group in groups.OfType<JsonObject>()) {
            if ((bool?)group["active"] != true || (string?)group["status"] != "waiting_cleanup") continue;
            if ((string?)group["cleanup_pending"]?["state"] == "waiting") continue;
            if ((string?)group["attempt_id"] is not string attempt || attempt.Length == 0) continue;
            var worker = await WorkerGroup(root, group, diagnostics);
            var coordinator = worker["coordinator"];
            var status = (string?)coordinator?["last_event"]?["details"]?["status"];
            if ((string?)coordinator?["phase"] != "finished" ||
                (string?)coordinator?["last_event"]?["kind"] != "record_saved" ||
                (bool?)coordinator?["process_alive"] != false ||
                status is not ("unchanged" or "differences" or "skipped" or "failed")) continue;
            if (worker["clients"] is JsonArray clients && clients.OfType<JsonObject>()
                .Any(client => (bool?)client["process_alive"] != false)) continue;
            group["scheduler_status"] = group["status"]?.DeepClone();
            group["status"] = status;
            group["active"] = false;
            group["completion_reconciled"] = true;
            reconciled++;
        }
        if (reconciled > 0) {
            queue["scheduler_open_groups"] = queue["open_groups"]?.DeepClone();
            queue["open_groups"] = groups.OfType<JsonObject>().Count(g => (bool?)g["active"] == true);
            queue["cleanup_groups"] = groups.OfType<JsonObject>().Count(g =>
                (bool?)g["active"] == true && (string?)g["status"] == "waiting_cleanup");
            diagnostics.Add("Completed workers were still marked active by the scheduler; display reconciled from saved-result and process evidence. Scheduling has not been repaired.");
        }
    }

    private static async Task<JsonObject> WorkerGroup(string root, JsonObject summary, JsonArray diagnostics)
    {
        var id = SafeId(RequiredString(summary, "id"));
        var result = CopyFields(summary, "id", "workbook", "corpus", "status", "active", "attempt_id", "started_unix_ms", "cleanup_pending", "error");
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
                    "stage_since", "requested_at", "opened_at", "hide_excel", "window_mode", "heartbeat_count", "last_heartbeat_at",
                    "active_command", "error", "last_launcher_event", "last_launcher_success", "runtime");
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
            "uploaded_images", "completed_commands", "sheet_name", "sheet_index", "profile", "pane_resources");
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
        if (!Directory.Exists(directory)) return new JsonArray();
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

    private static bool OutputIdle(string root)
    {
        var path = Path.Combine(Directory.GetParent(root)!.FullName, ".run.lock");
        try {
            using var file = new FileStream(path, FileMode.Open, FileAccess.ReadWrite, FileShare.ReadWrite);
            file.Lock(0, 1);
            file.Unlock(0, 1);
            return true;
        } catch (FileNotFoundException) { return true; }
        catch (IOException) { return false; }
        catch (UnauthorizedAccessException) { return false; }
    }

    private static bool ActiveStopRequested(JsonObject queue, JsonObject? stop)
    {
        // A retained request applies only while its runner might still be
        // draining. Saving controls for a stopped/dead runner does not start it.
        return stop is not null && stop["runner_pid"] is not null &&
            (bool?)queue["runner_alive"] != false &&
            ((bool?)queue["running"] == true || (bool?)queue["runner_reported_running"] == true) &&
            JsonNode.DeepEquals(stop["runner_pid"], queue["runner_pid"]) &&
            JsonNode.DeepEquals(stop["runner_started_unix_ms"], queue["runner_started_unix_ms"]);
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
