using System.Buffers.Binary;
using System.Collections.Concurrent;
using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Microsoft.Extensions.FileProviders;

if (args.Length == 2 && args[0] == "--control-room") { await ControlRoom.RunAsync(args[1]); return; }
if (args.Length != 1) throw new ArgumentException("Expected the resident host settings JSON path.");
var settings = JsonNode.Parse(File.ReadAllBytes(args[0]))!.AsObject();
var session = Path.GetFullPath((string)settings["session"]!);
var web = Path.GetFullPath((string)settings["web"]!);
var port = (int)settings["port"]!;
if (port is < 1024 or > 65535 || !Directory.Exists(web)) throw new ArgumentException("Invalid host settings.");
Directory.CreateDirectory(session);
var clients = new ConcurrentDictionary<string, ClientState>(StringComparer.Ordinal);
var controlRoot = Environment.GetEnvironmentVariable("EXCEL_SHRINK_CONTROL_ROOM_DIR");
var lastControls = new JsonObject { ["paused"] = true };
var primaryClients = Directory.Exists(Path.Combine(session, "clients"))
    ? Directory.GetFiles(Path.Combine(session, "clients"), "*.json").Select(Path.GetFileNameWithoutExtension).ToArray()
    : Array.Empty<string>();
var builder = WebApplication.CreateBuilder();
// Idle resident clients poll frequently; request-by-request logging would grow
// without bound during a corpus run. Errors remain visible in logs and health.
builder.Logging.SetMinimumLevel(LogLevel.Warning);
builder.WebHost.UseUrls($"http://localhost:{port}");
builder.WebHost.ConfigureKestrel(server => server.Limits.MaxRequestBodySize = 160 * 1024 * 1024);
var app = builder.Build();
app.Use(async (context, next) => {
    context.Response.Headers.CacheControl = "no-store";
    try { await next(); }
    catch (Exception error) when (error is ArgumentException or InvalidOperationException or JsonException or IOException or FormatException) {
        if (context.Response.HasStarted) throw;
        context.Response.StatusCode = 400;
        await context.Response.WriteAsync(error.Message);
    }
});
app.UseStaticFiles(new StaticFileOptions { FileProvider = new PhysicalFileProvider(web) });

app.MapGet("/health", () => {
    var snapshots = clients.ToDictionary(pair => pair.Key, pair => pair.Value.Snapshot());
    return Results.Json(new { ok = true, seen = snapshots.Keys.Order().ToArray(),
        errors = snapshots.Where(pair => pair.Value.Error is not null)
            .Select(pair => new { client_id = pair.Key, code = pair.Value.Error!["code"], message = pair.Value.Error["message"] }),
        clients = snapshots.ToDictionary(pair => pair.Key, pair => new {
            seen_at = pair.Value.SeenAt, last_activity_at = pair.Value.LastActivityAt,
            last_heartbeat_at = pair.Value.LastHeartbeatAt,
            data_started = pair.Value.DataStarted, active_command = pair.Value.ActiveCommand,
            command_count = pair.Value.CommandCount, heartbeat_count = pair.Value.HeartbeatCount,
            error = pair.Value.Error, monitor = pair.Value.Monitor
        }) });
});

app.MapGet("/config", async (HttpRequest request) => {
    var id = SafeId(request.Query["client"].ToString());
    var configPath = Path.Combine(session, "clients", id + ".json");
    if (!File.Exists(configPath)) return Results.NotFound("Unknown resident client.");
    var config = (await ReadObject(configPath));
    if ((string?)config["client_id"] != id) throw new ArgumentException("Client configuration identity differs.");
    var client = clients.GetOrAdd(id, _ => new ClientState());
    await client.Gate.WaitAsync();
    try {
        if (client.DataStarted) {
            client.Error ??= Error("TASKPANE_RESTARTED", "The resident task pane restarted after receiving a job.");
            client.Telemetry.Restarted();
            if (client.ActiveCommand is not null) {
                await WriteResponse(id, client.ActiveCommand, new JsonObject { ["ok"] = false, ["error"] = client.Error.DeepClone() });
            }
            return Results.Conflict("Resident capture task pane restarted.");
        }
        client.Token = Guid.NewGuid().ToString("N");
        client.ConfiguredAt = client.LastActivityAt = DateTimeOffset.UtcNow;
        client.Telemetry.Configured();
        config["client_token"] = client.Token;
        return Results.Json(config);
    } finally { client.Gate.Release(); }
});

app.MapGet("/heartbeat", async (HttpRequest request) => {
    var (_, client) = Authenticate(request.Query["client"].ToString(), request);
    await client.Gate.WaitAsync();
    try { client.LastHeartbeatAt = client.LastActivityAt = DateTimeOffset.UtcNow; client.HeartbeatCount++; }
    finally { client.Gate.Release(); }
    return Results.Json(new { ok = true });
});

app.MapGet("/job", async (HttpRequest request) => {
    var (id, client) = Authenticate(request.Query["client"].ToString(), request);
    await client.Gate.WaitAsync();
    try {
        if (client.Error is not null) return Results.Conflict(client.Error.ToJsonString());
        client.LastActivityAt = DateTimeOffset.UtcNow;
        // An incidental poll must not overwrite the observed state of work
        // already in flight, including while dispatch of new work is paused.
        if (client.ActiveCommand is not null) return Results.Conflict("A command is already in progress.");
        if (!string.IsNullOrEmpty(controlRoot)) {
            if (primaryClients.Any(peer => peer is null || !clients.ContainsKey(peer))) {
                client.Telemetry.Transition("waiting_for_peers", "primary_addins", "Waiting for the other primary add-ins to connect.");
                return Results.Json(new { idle = true, waiting_for_peers = true });
            }
            var controlsPath = Path.Combine(controlRoot, "control.json");
            try {
                var latest = await ReadObject(controlsPath);
                if (latest["paused"] is JsonValue flag && flag.TryGetValue<bool>(out _)) lastControls = latest;
            } catch (Exception error) when (error is IOException or JsonException or InvalidOperationException) {
                // A transient read cannot silently resume a deliberately paused run.
            }
            if ((bool?)lastControls["paused"] == true) {
                client.Telemetry.Transition("paused", "capture_pause", "Capture dispatch is paused by the user.");
                return Results.Json(new { idle = true, paused = true });
            }
        }
        // Excel can reload a newly inserted task pane during workbook startup.
        // Let each client settle independently before its first irreversible job.
        if (!client.DataStarted && DateTimeOffset.UtcNow - client.ConfiguredAt < TimeSpan.FromSeconds(2)) {
            client.Telemetry.Transition("settling", "startup_quiet_period", "Waiting for the startup quiet period.");
            return Results.Json(new { idle = true, settling = true });
        }
        var directory = Path.Combine(session, "commands", id);
        if (!Directory.Exists(directory)) {
            client.Telemetry.Transition("waiting_for_job", "coordinator", "Waiting for the next coordinator command.");
            return Results.Json(new { idle = true });
        }
        foreach (var path in Directory.EnumerateFiles(directory, "*.json").Order(StringComparer.Ordinal)) {
            var commandId = SafeId(Path.GetFileNameWithoutExtension(path));
            if (File.Exists(ResponsePath(id, commandId))) continue;
            var command = await ReadObject(path);
            if ((string?)command["id"] != commandId) throw new ArgumentException("Command filename and id differ.");
            if ((string?)command["kind"] is not ("capture" or "plan" or "capture_images" or "images" or "release")) throw new ArgumentException("Unknown resident command.");
            client.ActiveCommand = commandId;
            client.ActivePayload = command;
            client.DataStarted = true;
            client.CommandCount++;
            client.Telemetry.Started(command);
            if (!string.IsNullOrEmpty(controlRoot)) {
                await WriteAtomic(Path.Combine(session,"dispatched",id,commandId+".json"),
                    JsonSerializer.SerializeToUtf8Bytes(new { dispatched_unix_ms=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() }));
            }
            return Results.Json(command);
        }
        client.Telemetry.Transition("waiting_for_job", "coordinator", "Waiting for the next coordinator command.");
        return Results.Json(new { idle = true });
    } finally { client.Gate.Release(); }
});

app.MapPost("/image/{id}/{commandId}/{sheet:int}/{tile:int}", async (string id, string commandId, int sheet, int tile, HttpRequest request) => {
    var (_, client) = Authenticate(id, request);
    commandId = SafeId(commandId);
    await client.Gate.WaitAsync();
    try {
        var imageFirst = (string?)client.ActivePayload?["kind"] == "capture_images";
        if (client.ActiveCommand != commandId || (!imageFirst && (string?)client.ActivePayload?["kind"] != "images"))
            return Results.Conflict("Image does not belong to the active evidence command.");
        if (client.Error is not null) return Results.Conflict("Resident client has failed.");
        var expected = (client.ActivePayload!["tiles"] as JsonArray ?? throw new ArgumentException("No requested evidence tiles."))
            .SingleOrDefault(item => (int?)item?["sheet_index"] == sheet && (int?)item?["tile_index"] == tile)
            ?? throw new ArgumentException("Unrequested evidence tile.");
        var sha = (string?)expected["sha256"];
        if (!imageFirst && (sha is null || !Regex.IsMatch(sha, "\\A[0-9a-f]{64}\\z") || request.Headers["X-Sha256"].ToString() != sha))
            throw new ArgumentException("Image checksum header differs from request.");
        var bytes = await ReadImage(request);
        var (width, height) = PngSize(bytes);
        var receivedSha = Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();
        if (width != (int?)expected["width"] || height != (int?)expected["height"] || (!imageFirst && receivedSha != sha))
            throw new ArgumentException("Evidence image differs from the offered checksum or dimensions.");
        sha = receivedSha;
        var path = Path.Combine(session, "images", id, commandId, $"s{sheet:D4}-t{tile:D5}.png");
        if (File.Exists(path)) throw new ArgumentException("Duplicate evidence image.");
        await WriteAtomic(path, bytes);
        client.LastActivityAt = DateTimeOffset.UtcNow;
        client.Telemetry.ImageReceived(expected);
        var receipt = expected.DeepClone().AsObject();
        receipt["ok"] = true;
        receipt["image_path"] = path;
        receipt["sha256"] = sha;
        receipt["png_bytes"] = bytes.Length;
        receipt["received_unix_ms"] = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        if (imageFirst) {
            // The orchestrator consumes this atomically published receipt while
            // the client is still capturing or uploading the rest of its batch.
            await WriteAtomic(Path.Combine(session, "receipts", id, commandId, $"s{sheet:D4}-t{tile:D5}.json"),
                JsonSerializer.SerializeToUtf8Bytes(receipt));
        }
        return Results.Json(receipt);
    } finally { client.Gate.Release(); }
});

app.MapPost("/result/{id}/{commandId}", async (string id, string commandId, HttpRequest request) => {
    var (_, client) = Authenticate(id, request);
    commandId = SafeId(commandId);
    var result = (await JsonNode.ParseAsync(request.Body))?.AsObject() ?? throw new ArgumentException("Missing result.");
    await client.Gate.WaitAsync();
    try {
        if (client.Error is not null || client.ActiveCommand != commandId) return Results.Conflict("Command is no longer active.");
        var ok = (bool?)result["ok"] ?? throw new ArgumentException("Missing result status.");
        if (!ok) client.Error = result["error"]?.DeepClone().AsObject() ?? Error("CAPTURE_FAILED", "Capture failed.");
        if (ok && (string?)client.ActivePayload?["kind"] is "images" or "capture_images") {
            var imageFirst = (string?)client.ActivePayload?["kind"] == "capture_images";
            var requested = client.ActivePayload!["tiles"]!.AsArray();
            var images = result["images"]?.AsArray() ?? throw new ArgumentException("Missing evidence receipts.");
            if (images.Count != requested.Count) throw new ArgumentException("Incomplete evidence result.");
            var used = new HashSet<string>();
            foreach (var image in images) {
                var index = (int)image!["sheet_index"]!;
                var tile = (int)image["tile_index"]!;
                var expected = requested.SingleOrDefault(item => (int?)item?["sheet_index"] == index && (int?)item?["tile_index"] == tile)
                    ?? throw new ArgumentException("Unrequested evidence receipt.");
                var path = Path.Combine(session, "images", id, commandId, $"s{index:D4}-t{tile:D5}.png");
                if (!used.Add(path) || !File.Exists(path) || (string?)image["image_path"] != path ||
                    (string?)image["sha256"] != (imageFirst ? Convert.ToHexString(SHA256.HashData(await File.ReadAllBytesAsync(path))).ToLowerInvariant() : (string?)expected["sha256"]) ||
                    (int?)image["width"] != (int?)expected["width"] || (int?)image["height"] != (int?)expected["height"])
                    throw new ArgumentException("Invalid evidence receipt.");
            }
        }
        await WriteResponse(id, commandId, result);
        client.Telemetry.Finished(ok, client.Error);
        client.ActiveCommand = null;
        client.ActivePayload = null;
        client.LastActivityAt = DateTimeOffset.UtcNow;
        return Results.Json(new { ok = true });
    } finally { client.Gate.Release(); }
});

app.Run();

string ResponsePath(string id, string command) => Path.Combine(session, "responses", id, command + ".json");
async Task WriteResponse(string id, string command, JsonObject response) {
    var path = ResponsePath(id, command);
    if (!File.Exists(path)) await WriteAtomic(path, JsonSerializer.SerializeToUtf8Bytes(response));
}
(string, ClientState) Authenticate(string id, HttpRequest request) {
    id = SafeId(id);
    if (!clients.TryGetValue(id, out var client) || client.Token != request.Headers["X-Client-Token"].ToString())
        throw new ArgumentException("Unknown or retired resident client session.");
    return (id, client);
}
static string SafeId(string value) {
    if (!Regex.IsMatch(value, "\\A[a-zA-Z0-9][a-zA-Z0-9_-]{0,95}\\z")) throw new ArgumentException("Invalid resident identifier.");
    return value;
}
static async Task<JsonObject> ReadObject(string path) {
    using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
    return (await JsonNode.ParseAsync(stream))!.AsObject();
}
static JsonObject Error(string code, string message) => new() { ["code"] = code, ["message"] = message };
static async Task WriteAtomic(string path, byte[] bytes) {
    Directory.CreateDirectory(Path.GetDirectoryName(path)!);
    var temporary = path + "." + Guid.NewGuid().ToString("N") + ".pending";
    try {
        await File.WriteAllBytesAsync(temporary, bytes);
        File.Move(temporary, path);
    } finally { if (File.Exists(temporary)) File.Delete(temporary); }
}
static async Task<byte[]> ReadImage(HttpRequest request) {
    const int maximum = 160 * 1024 * 1024;
    using var memory = new MemoryStream();
    var buffer = new byte[64 * 1024];
    int count;
    while ((count = await request.Body.ReadAsync(buffer)) != 0) {
        if (memory.Length + count > maximum) throw new ArgumentException("Evidence image exceeds the byte budget.");
        await memory.WriteAsync(buffer.AsMemory(0, count));
    }
    return memory.ToArray();
}
static (int Width, int Height) PngSize(byte[] bytes) {
    byte[] signature = [137, 80, 78, 71, 13, 10, 26, 10];
    if (bytes.Length < 45 || !bytes.AsSpan(0, 8).SequenceEqual(signature) ||
        !bytes.AsSpan(12, 4).SequenceEqual("IHDR"u8) || !bytes.AsSpan(bytes.Length - 8, 4).SequenceEqual("IEND"u8))
        throw new ArgumentException("Invalid or truncated evidence PNG.");
    int width = BinaryPrimitives.ReadInt32BigEndian(bytes.AsSpan(16, 4));
    int height = BinaryPrimitives.ReadInt32BigEndian(bytes.AsSpan(20, 4));
    if (width < 1 || height < 1 || width > 30000 || height > 30000 || (long)width * height > 32000000)
        throw new ArgumentException("Evidence PNG exceeds the tile budget.");
    return (width, height);
}

sealed class ClientState {
    public readonly WorkerTelemetry Telemetry = new();
    public readonly SemaphoreSlim Gate = new(1);
    public string Token = "";
    public readonly DateTimeOffset SeenAt = DateTimeOffset.UtcNow;
    public DateTimeOffset LastActivityAt = DateTimeOffset.UtcNow;
    public DateTimeOffset? LastHeartbeatAt;
    public DateTimeOffset ConfiguredAt = DateTimeOffset.UtcNow;
    public bool DataStarted;
    public string? ActiveCommand;
    public JsonObject? ActivePayload;
    public JsonObject? Error;
    public long CommandCount;
    public long HeartbeatCount;
    public SnapshotData Snapshot() {
        Gate.Wait();
        try { return new(SeenAt, LastActivityAt, LastHeartbeatAt, DataStarted, ActiveCommand, CommandCount, HeartbeatCount, Error?.DeepClone().AsObject(), Telemetry.Snapshot()); }
        finally { Gate.Release(); }
    }
}
sealed record SnapshotData(DateTimeOffset SeenAt, DateTimeOffset LastActivityAt, DateTimeOffset? LastHeartbeatAt, bool DataStarted,
    string? ActiveCommand, long CommandCount, long HeartbeatCount, JsonObject? Error, JsonObject Monitor);
