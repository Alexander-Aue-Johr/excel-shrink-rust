using System.Collections.Concurrent;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;
using SixLabors.ImageSharp;
using SixLabors.ImageSharp.PixelFormats;

static string RequireEnv(string name)
{
    var value = Environment.GetEnvironmentVariable(name);
    if (string.IsNullOrWhiteSpace(value))
    {
        throw new InvalidOperationException($"Missing environment variable: {name}");
    }
    return value;
}

static int EnvInt(string name, int fallback)
{
    var value = Environment.GetEnvironmentVariable(name);
    return int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed)
        ? parsed
        : fallback;
}

static string SafeFileName(string value)
{
    var invalid = Path.GetInvalidFileNameChars().ToHashSet();
    var chars = value.Select(ch => invalid.Contains(ch) ? '_' : ch).ToArray();
    return new string(chars);
}

var experiment = RequireEnv("VV_EXPERIMENT");
var strategy = Environment.GetEnvironmentVariable("VV_STRATEGY") ?? "A";
var hostKind = Environment.GetEnvironmentVariable("VV_HOST_KIND") ?? "dotnet";
var staticRoot = Path.GetFullPath(RequireEnv("VV_STATIC_ROOT"));
var fileListPath = Path.GetFullPath(RequireEnv("VV_FILE_LIST"));
var outputPath = Path.GetFullPath(RequireEnv("VV_OUTPUT_PATH"));
var imageRoot = Path.GetFullPath(RequireEnv("VV_IMAGE_ROOT"));
var batchSize = EnvInt("VV_BATCH_SIZE", 8);
var overflowWidthPx = EnvInt("VV_OVERFLOW_WIDTH_PX", 1920);
var overflowHeightPx = EnvInt("VV_OVERFLOW_HEIGHT_PX", 1080);
// Source Context patch: manual task-pane start configuration.
var waitForStart = string.Equals(
    Environment.GetEnvironmentVariable("VV_WAIT_FOR_START"),
    "1",
    StringComparison.OrdinalIgnoreCase);
// Source Context patch: Office.js diagnostic configuration.
var extendedErrorLogging = string.Equals(
    Environment.GetEnvironmentVariable("VV_EXTENDED_ERROR_LOGGING"),
    "1",
    StringComparison.OrdinalIgnoreCase);
var officeJsDebug = string.Equals(
    Environment.GetEnvironmentVariable("VV_OFFICE_JS_DEBUG"),
    "1",
    StringComparison.OrdinalIgnoreCase);

// Diagnostic run IDs prevent still-open task panes from previous runs contaminating this run.
var isRenderDiagnostic = experiment is "range-image-height-limit" or "range-image-calibration";
var diagnosticRunId = isRenderDiagnostic ? Guid.NewGuid().ToString("N") : null;
var captureRunId = experiment == "range-image" ? (Environment.GetEnvironmentVariable("VV_CAPTURE_RUN_ID") ?? Guid.NewGuid().ToString("N")) : null;
var diagnosticConfigPath = Path.Combine(staticRoot, "diagnostic-config.json");
JsonNode? diagnosticOptions = isRenderDiagnostic && File.Exists(diagnosticConfigPath)
    ? JsonNode.Parse(File.ReadAllText(diagnosticConfigPath)) : null;

Directory.CreateDirectory(imageRoot);

var strictUtf8 = new UTF8Encoding(false, true);
var expectedNames = File.ReadAllLines(fileListPath, strictUtf8)
    .Select(line => line.Trim())
    .Where(line => line.Length != 0)
    .Select(line => Path.GetFileName(line) ?? throw new InvalidOperationException("Invalid workbook path"))
    .ToArray();

var expectedNameSet = new HashSet<string>(
    expectedNames,
    StringComparer.OrdinalIgnoreCase);

var results = new ConcurrentDictionary<string, JsonObject>(
    StringComparer.OrdinalIgnoreCase);
var errors = new ConcurrentQueue<string>();
var events = new ConcurrentQueue<EventEntry>();
var eventSequence = 0L;
var probeTasks = new ConcurrentDictionary<string, Task<ProbeResult>>(
    StringComparer.OrdinalIgnoreCase);
var writeGate = new SemaphoreSlim(1, 1);
var lastActivity = DateTimeOffset.UtcNow;
var journalGate = new object();
var progressGate = new SemaphoreSlim(1,1);
var captureOwners = new ConcurrentDictionary<string,string>(StringComparer.OrdinalIgnoreCase);
Directory.CreateDirectory(Path.GetDirectoryName(outputPath)!);
var journalPath = outputPath + ".events.jsonl";

void Record(string stage, object? detail = null)
{
    lastActivity = DateTimeOffset.UtcNow;
    var detailJson = detail is null
        ? null
        : JsonSerializer.SerializeToNode(detail);
    var entry = new EventEntry(
        Interlocked.Increment(ref eventSequence),
        lastActivity,
        stage,
        detailJson);
    lock (journalGate)
    {
        File.AppendAllText(journalPath, JsonSerializer.Serialize(new { sequence=entry.Sequence,
            timestamp_utc=entry.Timestamp, source="dotnet", stage, detail=detailJson }) + "\n", new UTF8Encoding(false));
    }
    events.Enqueue(entry);
    Console.WriteLine(
        $"[{entry.Timestamp:HH:mm:ss.fff}] {stage}"
        + (detailJson is null ? "" : " " + detailJson.ToJsonString()));
}

static (int Width, int Height) PngSize(byte[] bytes)
{
    if (bytes.Length < 24)
    {
        throw new InvalidOperationException("PNG payload is too short.");
    }
    var signature = new byte[] { 137, 80, 78, 71, 13, 10, 26, 10 };
    if (!bytes.AsSpan(0, 8).SequenceEqual(signature))
    {
        throw new InvalidOperationException("Payload is not PNG.");
    }
    static int ReadInt(byte[] source, int offset) =>
        (source[offset] << 24)
        | (source[offset + 1] << 16)
        | (source[offset + 2] << 8)
        | source[offset + 3];
    return (ReadInt(bytes, 16), ReadInt(bytes, 20));
}

static ProbeResult AnalyzeProbe(JsonObject payload, string imageRoot)
{
    var workbook = payload["workbook"]!.GetValue<string>();
    var probeId = payload["probe_id"]?.GetValue<string>() ?? Guid.NewGuid().ToString("N");
    var sheet = payload["sheet"]!.GetValue<string>();
    var axis = payload["axis"]!.GetValue<string>();
    var startIndex = payload["start_index"]!.GetValue<int>();
    var before = Convert.FromBase64String(payload["before_base64"]!.GetValue<string>());
    var after = Convert.FromBase64String(payload["after_base64"]!.GetValue<string>());
    var totalPoints = payload["total_points"]!.GetValue<double>();
    var boundaries = payload["boundaries_points"]!.AsArray()
        .Select(node => node!.GetValue<double>())
        .ToArray();

    using var beforeImage = Image.Load<Rgba32>(before);
    using var afterImage = Image.Load<Rgba32>(after);

    if (beforeImage.Width != afterImage.Width || beforeImage.Height != afterImage.Height)
    {
        throw new InvalidOperationException(
            $"Probe dimensions differ for {sheet}: "
            + $"{beforeImage.Width}x{beforeImage.Height} vs "
            + $"{afterImage.Width}x{afterImage.Height}");
    }

    var diagnosticDirectory = Path.Combine(
        imageRoot,
        SafeFileName(workbook),
        "_overflow-probes");
    Directory.CreateDirectory(diagnosticDirectory);

    var stem = $"{SafeFileName(probeId)}-{SafeFileName(sheet)}-{SafeFileName(axis)}";
    var beforeFileName = stem + "-before.png";
    var afterFileName = stem + "-after.png";
    var xorFileName = stem + "-xor.png";
    var beforePath = Path.Combine(diagnosticDirectory, beforeFileName);
    var afterPath = Path.Combine(diagnosticDirectory, afterFileName);
    var xorPath = Path.Combine(diagnosticDirectory, xorFileName);

    File.WriteAllBytes(beforePath, before);
    File.WriteAllBytes(afterPath, after);

    using var xorImage = new Image<Rgba32>(beforeImage.Width, beforeImage.Height);
    var rightmost = -1;
    var bottommost = -1;

    for (var y = 0; y < beforeImage.Height; y++)
    {
        for (var x = 0; x < beforeImage.Width; x++)
        {
            var beforePixel = beforeImage[x, y];
            var afterPixel = afterImage[x, y];
            var changed = !beforePixel.Equals(afterPixel);

            xorImage[x, y] = new Rgba32(
                (byte)(beforePixel.R ^ afterPixel.R),
                (byte)(beforePixel.G ^ afterPixel.G),
                (byte)(beforePixel.B ^ afterPixel.B),
                changed ? (byte)255 : (byte)0);

            if (changed)
            {
                if (x > rightmost) rightmost = x;
                if (y > bottommost) bottommost = y;
            }
        }
    }

    xorImage.SaveAsPng(xorPath);

    var changedPixel = axis == "horizontal" ? rightmost : bottommost;
    var pixelExtent = axis == "horizontal" ? beforeImage.Width : beforeImage.Height;
    var changedPoints = changedPixel < 0 || pixelExtent <= 0
        ? -1.0
        : (changedPixel + 1.0) * totalPoints / pixelExtent;

    var extentUnits = 0;
    if (changedPoints >= 0)
    {
        for (var index = 0; index < boundaries.Length; index++)
        {
            extentUnits = index + 1;
            if (boundaries[index] + 0.01 >= changedPoints)
            {
                break;
            }
        }
    }

    string RelativeDiagnostic(string fileName) =>
        Path.Combine(SafeFileName(workbook), "_overflow-probes", fileName)
            .Replace('\\', '/');

    return new ProbeResult(
        workbook,
        probeId,
        sheet,
        axis,
        startIndex,
        extentUnits,
        changedPixel,
        changedPoints,
        RelativeDiagnostic(beforeFileName),
        RelativeDiagnostic(afterFileName),
        RelativeDiagnostic(xorFileName));
}

async Task WriteOutputAsync()
{
    await writeGate.WaitAsync();
    try
    {
        var workbooks = new JsonObject();
        foreach (var name in expectedNames)
        {
            if (results.TryGetValue(name, out var payload))
            {
                workbooks[name] = payload.DeepClone();
            }
        }

        var document = new JsonObject
        {
            ["_meta"] = new JsonObject
            {
                ["experiment"] = experiment,
                ["strategy"] = strategy,
                ["host_kind"] = hostKind,
                ["batch_size"] = batchSize,
                ["completed_workbooks"] = results.Count,
                ["expected_workbooks"] = expectedNames.Length,
                ["generated_utc"] = DateTimeOffset.UtcNow.ToString("O")
            },
            ["errors"] = JsonSerializer.SerializeToNode(errors.ToArray()),
            ["workbooks"] = workbooks
        };

        var parent = Path.GetDirectoryName(outputPath);
        if (!string.IsNullOrWhiteSpace(parent))
        {
            Directory.CreateDirectory(parent);
        }

        await File.WriteAllTextAsync(
            outputPath + ".new",
            document.ToJsonString(new JsonSerializerOptions { WriteIndented = true }) + "\n",
            strictUtf8);
        File.Move(outputPath + ".new", outputPath, overwrite:true);
    }
    finally
    {
        writeGate.Release();
    }
}

var builder = WebApplication.CreateBuilder(args);
builder.WebHost.ConfigureKestrel(options =>
{
    options.Limits.MaxRequestBodySize = null;
});
builder.WebHost.UseUrls("https://localhost:3000", "http://127.0.0.1:3001");

var app = builder.Build();
await WriteOutputAsync(); // A valid partial result exists before Excel is started.

if (captureRunId is not null)
{
    app.Use(async (context,next) => {
        var path=context.Request.Path.Value ?? "";
        if (context.Request.Method == "POST" && path is ("/capture-claim" or "/capture-progress" or "/image-batch" or "/finalize-image" or "/result" or "/client-event" or "/error" or "/probe-batch"))
        {
            if (!string.Equals(context.Request.Headers["X-VV-Run-Id"].ToString(), captureRunId, StringComparison.Ordinal))
            { context.Response.StatusCode=409; await context.Response.WriteAsJsonAsync(new {error="stale-capture-run"}); return; }
        }
        await next();
    });
}
if (experiment == "range-image") FinalImageEndpoints.Map(app, imageRoot, staticRoot, expectedNameSet, SafeFileName, Record);

app.MapPost("/capture-claim", async (HttpRequest request) => {
    var body=(await JsonNode.ParseAsync(request.Body))!.AsObject();
    var workbook=body["workbook"]?.GetValue<string>() ?? "";
    var client=body["client_id"]?.GetValue<string>() ?? "";
    if (!expectedNameSet.Contains(workbook) || client.Length is < 1 or > 200) return Results.BadRequest(new {error="Invalid capture claim"});
    var owner=captureOwners.GetOrAdd(workbook,client);
    return Results.Json(new {acquired=string.Equals(owner,client,StringComparison.Ordinal)});
});
app.MapPost("/capture-progress", async (HttpRequest request) => {
    var body=(await JsonNode.ParseAsync(request.Body))!.AsObject();
    var workbook=body["workbook"]?.GetValue<string>() ?? "";
    if (!expectedNameSet.Contains(workbook)) return Results.BadRequest(new {error="Unexpected workbook"});
    var dir=Path.Combine(imageRoot,"_progress"); Directory.CreateDirectory(dir);
    var path=Path.Combine(dir,SafeFileName(workbook)+".progress.json");
    await progressGate.WaitAsync();
    try {
        await File.WriteAllTextAsync(path+".new",body.ToJsonString(new JsonSerializerOptions{WriteIndented=true}),new UTF8Encoding(false));
        File.Move(path+".new",path,overwrite:true);
    } finally {progressGate.Release();}
    return Results.Json(new {ok=true});
});

if (isRenderDiagnostic)
{
    var diagnosticWrites = new HashSet<string>(StringComparer.Ordinal)
    {
        "/client-event", "/client-beacon", "/error", "/result", "/image-batch",
        "/diagnostic-progress", "/diagnostic-claim", "/diagnostic-markers"
    };
    app.Use(async (context, next) =>
    {
        if (diagnosticWrites.Contains(context.Request.Path.Value ?? "") &&
            !string.Equals(context.Request.Headers["X-VV-Run-Id"].ToString(), diagnosticRunId, StringComparison.Ordinal))
        {
            context.Response.StatusCode = 409;
            await context.Response.WriteAsJsonAsync(new { error = "stale-or-untagged-diagnostic-request" });
            return;
        }
        await next();
    });
    DiagnosticEndpoints.Map(app, imageRoot, expectedNameSet);
}

app.Use(async (context, next) =>
{
    context.Response.Headers.CacheControl = "no-store, no-cache, must-revalidate";
    context.Response.Headers.Pragma = "no-cache";
    context.Response.Headers.Expires = "0";
    await next();
});

app.MapGet("/health", () => Results.Json(new { ok = true }));

app.MapGet("/config", () => Results.Json(new
{
    experiment,
    strategy,
    host_kind = hostKind,
    batch_size = batchSize,
    overflow_probe_width_px = overflowWidthPx,
    overflow_probe_height_px = overflowHeightPx,
    wait_for_start = waitForStart,
    extended_error_logging = extendedErrorLogging,
    office_js_debug = officeJsDebug,
    capture_run_id = captureRunId,
    diagnostic_run_id = diagnosticRunId,
    diagnostic_options = diagnosticOptions
}));

app.MapGet("/status", (string? workbook, long? after) =>
{
    var since = after ?? 0;
    var snapshot = events.Where(item => item.Sequence > since).ToArray();
    var done = workbook is null
        ? results.Count == expectedNames.Length
        : results.ContainsKey(workbook);

    return Results.Json(new
    {
        done,
        last_activity_unix_ms = lastActivity.ToUnixTimeMilliseconds(),
        last_sequence = Volatile.Read(ref eventSequence),
        events = snapshot,
        errors = errors.ToArray()
    });
});

app.MapPost("/reset-diagnostics", () =>
{
    while (errors.TryDequeue(out _)) { }
    while (events.TryDequeue(out _)) { }

    // A retry of the same workbook must start with a clean probe namespace.
    // Tasks that were already running may finish in the background, but they
    // are no longer addressable by a later attempt.
    probeTasks.Clear();

    return Results.Json(new { ok = true });
});

app.MapPost("/client-event", async (HttpRequest request) =>
{
    var node = await JsonNode.ParseAsync(request.Body);
    var stage = node?["stage"]?.GetValue<string>() ?? "client_event";
    Record(stage, node?["detail"]);
    return Results.Json(new { ok = true });
});

app.MapGet("/client-beacon", (HttpRequest request) =>
{
    var stage = request.Query["stage"].ToString();
    var detailRaw = request.Query["detail"].ToString();
    JsonNode? detail = null;
    if (!string.IsNullOrWhiteSpace(detailRaw))
    {
        try { detail = JsonNode.Parse(detailRaw); } catch { }
    }
    Record(string.IsNullOrWhiteSpace(stage) ? "beacon" : stage, detail);
    return Results.NoContent();
});

app.MapPost("/image-batch", async (HttpRequest request) =>
{
    try
    {
        var payload = await JsonNode.ParseAsync(request.Body) as JsonObject
            ?? throw new InvalidOperationException("Image batch must be an object.");
        var workbook = payload["workbook"]?.GetValue<string>()
            ?? throw new InvalidOperationException("Image batch has no workbook.");
        if (!expectedNameSet.Contains(workbook)) throw new InvalidOperationException("Unexpected image workbook");
        var batchIndex = payload["batch_index"]?.GetValue<int>() ?? 0;
        var images = payload["images"]?.AsArray()
            ?? throw new InvalidOperationException("Image batch has no images.");

        var persisted = new JsonArray();
        var workbookDirectory = Path.Combine(imageRoot, SafeFileName(workbook));
        Directory.CreateDirectory(workbookDirectory);

        foreach (var node in images)
        {
            var image = node!.AsObject();
            var id = image["id"]!.GetValue<string>();
            if (!System.Text.RegularExpressions.Regex.IsMatch(id, @"^[A-Za-z0-9_-]{1,100}$")) throw new InvalidOperationException("Unsafe capture ID");
            var sheet = image["sheet"]!.GetValue<string>();
            var address = image["address"]!.GetValue<string>();
            var base64 = image["base64"]?.GetValue<string>() ?? "";
            var base64Prefix = base64[..Math.Min(32, base64.Length)];

            Record("image_batch_item_received", new
            {
                workbook,
                batch_index = batchIndex,
                id,
                sheet,
                address,
                base64_chars = base64.Length,
                base64_prefix = base64Prefix
            });

            byte[] bytes;
            try
            {
                bytes = Convert.FromBase64String(base64);
            }
            catch (Exception exception)
            {
                Record("image_payload_invalid", new
                {
                    workbook,
                    batch_index = batchIndex,
                    id,
                    sheet,
                    address,
                    base64_chars = base64.Length,
                    base64_prefix = base64Prefix,
                    reason = "base64-decode-failed",
                    exception = exception.Message
                });
                throw new InvalidOperationException(
                    $"Invalid image base64 for workbook={JsonSerializer.Serialize(workbook)}, " +
                    $"batch={batchIndex}, id={id}, sheet={JsonSerializer.Serialize(sheet)}, " +
                    $"address={address}, base64_chars={base64.Length}.",
                    exception);
            }

            int width;
            int height;
            try
            {
                (width, height) = PngSize(bytes);
            }
            catch (Exception exception)
            {
                var bytePrefix = Convert.ToHexString(
                    bytes.AsSpan(0, Math.Min(24, bytes.Length)))
                    .ToLowerInvariant();
                Record("image_payload_invalid", new
                {
                    workbook,
                    batch_index = batchIndex,
                    id,
                    sheet,
                    address,
                    base64_chars = base64.Length,
                    decoded_bytes = bytes.Length,
                    base64_prefix = base64Prefix,
                    byte_prefix_hex = bytePrefix,
                    reason = exception.Message
                });
                throw new InvalidOperationException(
                    $"Invalid PNG payload for workbook={JsonSerializer.Serialize(workbook)}, " +
                    $"batch={batchIndex}, id={id}, sheet={JsonSerializer.Serialize(sheet)}, " +
                    $"address={address}, base64_chars={base64.Length}, " +
                    $"decoded_bytes={bytes.Length}, byte_prefix_hex={bytePrefix}: " +
                    exception.Message,
                    exception);
            }

            Record("image_payload_validated", new
            {
                workbook,
                batch_index = batchIndex,
                id,
                sheet,
                address,
                base64_chars = base64.Length,
                decoded_bytes = bytes.Length,
                png_width = width,
                png_height = height
            });

            var fileName = id + ".png";
            var path = Path.Combine(workbookDirectory, fileName);
            await File.WriteAllBytesAsync(path, bytes);
            var sha = Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();

            persisted.Add(new JsonObject
            {
                ["id"] = id,
                ["workbook"] = workbook,
                ["sheet"] = sheet,
                ["address"] = address,
                ["relative_png"] = Path.Combine(SafeFileName(workbook), fileName)
                    .Replace('\\', '/'),
                ["png_bytes"] = bytes.Length,
                ["png_width"] = width,
                ["png_height"] = height,
                ["sha256"] = sha
            });
        }

        Record("image_batch_persisted", new { workbook, batch_index=batchIndex, captures=persisted });
        if (experiment == "range-image") await WriteOutputAsync();
        Record("image_batch_received", new
        {
            workbook,
            batch_index = batchIndex,
            image_count = images.Count
        });

        return Results.Json(new { captures = persisted });
    }
    catch (Exception exception)
    {
        errors.Enqueue(exception.ToString());
        await WriteOutputAsync();
        Record("server_error", new
        {
            message = exception.Message,
            exception = exception.ToString()
        });
        return Results.Json(new { error = exception.ToString() }, statusCode: 400);
    }
});

app.MapPost("/probe-batch", async (HttpRequest request) =>
{
    try
    {
        var payload = await JsonNode.ParseAsync(request.Body) as JsonObject
            ?? throw new InvalidOperationException("Probe payload must be an object.");
        var workbook = payload["workbook"]!.GetValue<string>();
        var sheet = payload["sheet"]!.GetValue<string>();
        var axis = payload["axis"]!.GetValue<string>();
        var key = workbook + "\n" + sheet + "\n" + axis;

        if (!probeTasks.TryAdd(
            key,
            Task.Run(() => AnalyzeProbe((JsonObject)payload.DeepClone(), imageRoot))))
        {
            // Multiple Office task panes or a late request from a previous
            // retry can submit the same logical probe. The first accepted
            // probe owns the result; later identical keys are idempotent.
            Record("probe_duplicate_ignored", new { workbook, sheet, axis });
            return Results.Json(new { accepted = true, duplicate = true });
        }

        Record("probe_received", new { workbook, sheet, axis });
        return Results.Json(new { accepted = true, duplicate = false });
    }
    catch (Exception exception)
    {
        errors.Enqueue(exception.ToString());
        return Results.Json(new { error = exception.ToString() }, statusCode: 400);
    }
});

app.MapGet("/probe-status", async (string workbook) =>
{
    var relevant = probeTasks
        .Where(pair => pair.Key.StartsWith(workbook + "\n", StringComparison.OrdinalIgnoreCase))
        .ToArray();

    var completed = relevant.Count(pair => pair.Value.IsCompleted);
    var done = relevant.Length > 0 && completed == relevant.Length;
    var resultArray = new JsonArray();

    if (done)
    {
        foreach (var pair in relevant)
        {
            // Match the Rust host: completed probes are one-shot state.
            // Removing them here also prevents a later retry of the same
            // workbook/sheet/axis from colliding with stale completed tasks.
            probeTasks.TryRemove(pair.Key, out _);

            var result = await pair.Value;
            resultArray.Add(JsonSerializer.SerializeToNode(result));
        }
    }

    return Results.Json(new
    {
        done,
        completed,
        total = relevant.Length,
        results = resultArray
    });
});

app.MapPost("/result", async (HttpRequest request) =>
{
    try
    {
        var payload = await JsonNode.ParseAsync(request.Body) as JsonObject
            ?? throw new InvalidOperationException("Result payload must be an object.");
        var workbook = payload["workbook"]?.GetValue<string>()
            ?? throw new InvalidOperationException("Result has no workbook.");

        if (!expectedNameSet.Contains(workbook))
        {
            throw new InvalidOperationException($"Unexpected workbook result: {workbook}");
        }

        results[workbook] = (JsonObject)payload.DeepClone();
        await WriteOutputAsync();
        Record("result_received", new { workbook });
        return Results.Json(new { ok = true });
    }
    catch (Exception exception)
    {
        errors.Enqueue(exception.ToString());
        return Results.Json(new { error = exception.ToString() }, statusCode: 400);
    }
});

app.MapPost("/error", async (HttpRequest request) =>
{
    using var reader = new StreamReader(request.Body, Encoding.UTF8);
    var body = await reader.ReadToEndAsync();
    errors.Enqueue(body);
    Record("client_error", new { message = body });
    await WriteOutputAsync();
    return Results.Json(new { ok = true });
});

IResult StaticFile(string relative, string contentType)
{
    var path = Path.Combine(staticRoot, relative);
    return Results.File(path, contentType);
}

app.MapGet("/taskpane.html", () =>
{
    Record("taskpane_requested");
    return StaticFile("taskpane.html", "text/html; charset=utf-8");
});
app.MapGet("/taskpane.css", () => StaticFile("taskpane.css", "text/css; charset=utf-8"));
app.MapGet("/bootstrap.js", () => StaticFile("bootstrap.js", "application/javascript; charset=utf-8"));
app.MapGet("/shell.js", () => StaticFile("shell.js", "application/javascript; charset=utf-8"));
app.MapGet("/safe-capture.js", () => StaticFile("safe-capture.js", "application/javascript; charset=utf-8"));
app.MapGet("/capture-policy.js", () => StaticFile("capture-policy.js", "application/javascript; charset=utf-8"));
app.MapGet("/selective-overflow.js", () => StaticFile("selective-overflow.js", "application/javascript; charset=utf-8"));
app.MapGet("/capture-options.json", () => StaticFile("capture-options.json", "application/json; charset=utf-8"));
app.MapGet("/render-diagnostics.js", () => StaticFile("render-diagnostics.js", "application/javascript; charset=utf-8"));
app.MapGet("/row-height-diagnostics.js", () => StaticFile("row-height-diagnostics.js", "application/javascript; charset=utf-8"));
app.MapGet("/commands.html", () => StaticFile("commands.html", "text/html; charset=utf-8"));
app.MapGet("/visual-plan.json", () => StaticFile("visual-plan.json", "application/json; charset=utf-8"));
app.MapGet("/mode.js", () =>
    StaticFile(
        experiment switch
        {
            "range-image" => "modes/range-image.js",
            "range-image-height-limit" => "modes/range-image-height-limit.js",
            "range-image-calibration" => "modes/range-image-calibration.js",
            _ => "modes/shape-bounds.js"
        },
        "application/javascript; charset=utf-8"));
app.MapGet("/icon-32.png", () => Results.Bytes(
    Convert.FromBase64String(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwC"
        + "AAAAC0lEQVR42mP8/x8AAusB9Wl2XK0AAAAASUVORK5CYII="),
    "image/png"));

app.Lifetime.ApplicationStarted.Register(() =>
{
    Record("server_ready", new
    {
        https = "https://localhost:3000",
        http = "http://127.0.0.1:3001",
        experiment,
        strategy
    });
});

await app.RunAsync();

sealed record EventEntry(
    long Sequence,
    DateTimeOffset Timestamp,
    string Stage,
    JsonNode? Detail);

sealed record ProbeResult(
    [property: JsonPropertyName("workbook")] string Workbook,
    [property: JsonPropertyName("probe_id")] string ProbeId,
    [property: JsonPropertyName("sheet")] string Sheet,
    [property: JsonPropertyName("axis")] string Axis,
    [property: JsonPropertyName("start_index")] int StartIndex,
    [property: JsonPropertyName("extent_units")] int ExtentUnits,
    [property: JsonPropertyName("changed_pixel")] int ChangedPixel,
    [property: JsonPropertyName("changed_points")] double ChangedPoints,
    [property: JsonPropertyName("before_png")] string BeforePng,
    [property: JsonPropertyName("after_png")] string AfterPng,
    [property: JsonPropertyName("xor_png")] string XorPng);
