using System.Collections.Concurrent;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
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

Directory.CreateDirectory(imageRoot);

var strictUtf8 = new UTF8Encoding(false, true);
var expectedNames = File.ReadAllLines(fileListPath, strictUtf8)
    .Select(line => line.Trim())
    .Where(line => line.Length != 0)
    .Select(Path.GetFileName)
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

static ProbeResult AnalyzeProbe(JsonObject payload)
{
    var workbook = payload["workbook"]!.GetValue<string>();
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

    var rightmost = -1;
    var bottommost = -1;

    for (var y = 0; y < beforeImage.Height; y++)
    {
        for (var x = 0; x < beforeImage.Width; x++)
        {
            if (!beforeImage[x, y].Equals(afterImage[x, y]))
            {
                if (x > rightmost) rightmost = x;
                if (y > bottommost) bottommost = y;
            }
        }
    }

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

    return new ProbeResult(
        workbook,
        sheet,
        axis,
        startIndex,
        extentUnits,
        changedPixel,
        changedPoints);
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
            ["workbooks"] = workbooks
        };

        var parent = Path.GetDirectoryName(outputPath);
        if (!string.IsNullOrWhiteSpace(parent))
        {
            Directory.CreateDirectory(parent);
        }

        await File.WriteAllTextAsync(
            outputPath,
            document.ToJsonString(new JsonSerializerOptions { WriteIndented = true }) + "\n",
            strictUtf8);
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
    overflow_probe_height_px = overflowHeightPx
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
            var sheet = image["sheet"]!.GetValue<string>();
            var address = image["address"]!.GetValue<string>();
            var bytes = Convert.FromBase64String(image["base64"]!.GetValue<string>());
            var (width, height) = PngSize(bytes);
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
        Record("server_error", new { message = exception.Message });
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
            Task.Run(() => AnalyzeProbe((JsonObject)payload.DeepClone()))))
        {
            throw new InvalidOperationException($"Duplicate probe: {sheet} / {axis}");
        }

        Record("probe_received", new { workbook, sheet, axis });
        return Results.Json(new { accepted = true });
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
app.MapGet("/commands.html", () => StaticFile("commands.html", "text/html; charset=utf-8"));
app.MapGet("/mode.js", () =>
    StaticFile(
        experiment == "range-image"
            ? "modes/range-image.js"
            : "modes/shape-bounds.js",
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
    string Workbook,
    string Sheet,
    string Axis,
    int StartIndex,
    int ExtentUnits,
    int ChangedPixel,
    double ChangedPoints);
