using System.Collections.Concurrent;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

var strictUtf8 = new UTF8Encoding(
    encoderShouldEmitUTF8Identifier: false,
    throwOnInvalidBytes: true);

string RequireEnvironment(string name)
{
    var value = Environment.GetEnvironmentVariable(name);
    if (string.IsNullOrWhiteSpace(value))
    {
        throw new InvalidOperationException(
            $"Required environment variable is missing: {name}");
    }

    return value;
}

var outputPath = Path.GetFullPath(
    RequireEnvironment("DRAWING_BOUNDS_OUTPUT_PATH"));
var staticRoot = Path.GetFullPath(
    RequireEnvironment("DRAWING_BOUNDS_STATIC_ROOT"));
var capturePlanPath = Path.GetFullPath(
    RequireEnvironment("DRAWING_BOUNDS_CAPTURE_PLAN_PATH"));
var imageOutputRoot = Path.GetFullPath(
    RequireEnvironment("DRAWING_BOUNDS_IMAGE_OUTPUT_ROOT"));

var imageBatchSize = int.Parse(
    RequireEnvironment("DRAWING_BOUNDS_IMAGE_BATCH_SIZE"),
    System.Globalization.CultureInfo.InvariantCulture);

if (imageBatchSize < 1)
{
    throw new InvalidOperationException(
        "DRAWING_BOUNDS_IMAGE_BATCH_SIZE must be at least 1.");
}

Directory.CreateDirectory(imageOutputRoot);

var planRows = File.ReadAllLines(capturePlanPath, strictUtf8);

if (planRows.Length < 2)
{
    throw new InvalidOperationException(
        $"Capture plan has no data rows: {capturePlanPath}");
}

var expectedHeader =
    "id\tworkbook_path\tworkbook_name\tsheet_name\taddress\t"
    + "used_rows\tused_columns\tshape_count\trelative_png\treason";

if (!string.Equals(planRows[0], expectedHeader, StringComparison.Ordinal))
{
    throw new InvalidOperationException(
        "Unexpected capture-plan header. Expected: " + expectedHeader);
}

var captures = new List<CapturePlan>();

for (var index = 1; index < planRows.Length; index++)
{
    var line = planRows[index];

    if (string.IsNullOrWhiteSpace(line))
    {
        continue;
    }

    var fields = line.Split('\t');

    if (fields.Length != 10)
    {
        throw new InvalidOperationException(
            $"Capture-plan line {index + 1} has {fields.Length} fields; expected 10.");
    }

    captures.Add(
        new CapturePlan(
            fields[0],
            fields[1],
            fields[2],
            fields[3],
            fields[4],
            int.Parse(fields[5], System.Globalization.CultureInfo.InvariantCulture),
            int.Parse(fields[6], System.Globalization.CultureInfo.InvariantCulture),
            int.Parse(fields[7], System.Globalization.CultureInfo.InvariantCulture),
            fields[8],
            fields[9]));
}

if (captures.Count == 0)
{
    throw new InvalidOperationException(
        $"Capture plan contains no captures: {capturePlanPath}");
}

var capturesByWorkbook = captures
    .GroupBy(capture => capture.WorkbookName, StringComparer.OrdinalIgnoreCase)
    .ToDictionary(
        group => group.Key,
        group => group.ToArray(),
        StringComparer.OrdinalIgnoreCase);

var captureByWorkbookAndId = captures
    .ToDictionary(
        capture => capture.WorkbookName + "\n" + capture.Id,
        capture => capture,
        StringComparer.OrdinalIgnoreCase);

var expectedWorkbookNames = captures
    .Select(capture => capture.WorkbookName)
    .Distinct(StringComparer.OrdinalIgnoreCase)
    .ToArray();

var results = new ConcurrentDictionary<string, JsonObject>(
    StringComparer.OrdinalIgnoreCase);
var errors = new ConcurrentQueue<string>();
var clientEvents = new ConcurrentQueue<string>();
var writeGate = new SemaphoreSlim(1, 1);

static (int Width, int Height) ReadPngDimensions(byte[] bytes)
{
    ReadOnlySpan<byte> signature =
        stackalloc byte[] { 137, 80, 78, 71, 13, 10, 26, 10 };

    if (bytes.Length < 24 || !bytes.AsSpan(0, 8).SequenceEqual(signature))
    {
        throw new InvalidOperationException(
            "Range.getImage() did not return a valid PNG payload.");
    }

    static int ReadBigEndianInt32(byte[] source, int offset)
    {
        return
            (source[offset] << 24)
            | (source[offset + 1] << 16)
            | (source[offset + 2] << 8)
            | source[offset + 3];
    }

    var width = ReadBigEndianInt32(bytes, 16);
    var height = ReadBigEndianInt32(bytes, 20);

    if (width <= 0 || height <= 0)
    {
        throw new InvalidOperationException(
            $"PNG dimensions are invalid: {width}x{height}");
    }

    return (width, height);
}

string ResolveImagePath(string relativePath)
{
    var fullRoot = Path.GetFullPath(imageOutputRoot)
        .TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
        + Path.DirectorySeparatorChar;

    var full = Path.GetFullPath(
        Path.Combine(imageOutputRoot, relativePath));

    if (!full.StartsWith(fullRoot, StringComparison.OrdinalIgnoreCase))
    {
        throw new InvalidOperationException(
            $"Capture-plan image path escapes the output root: {relativePath}");
    }

    return full;
}

async Task WriteOutputAsync()
{
    await writeGate.WaitAsync();

    try
    {
        var document = new JsonObject();
        var captureArray = new JsonArray();

        double renderTotal = 0.0;
        double persistTotal = 0.0;
        double captureTotal = 0.0;
        var completedCaptureCount = 0;

        foreach (var workbookName in expectedWorkbookNames)
        {
            if (!results.TryGetValue(workbookName, out var payload))
            {
                continue;
            }

            renderTotal += payload["render_total_ms"]?.GetValue<double>() ?? 0.0;
            persistTotal += payload["persist_total_ms"]?.GetValue<double>() ?? 0.0;
            captureTotal += payload["capture_total_ms"]?.GetValue<double>() ?? 0.0;

            if (payload["captures"] is JsonArray workbookCaptures)
            {
                foreach (var capture in workbookCaptures)
                {
                    if (capture is not null)
                    {
                        captureArray.Add(capture.DeepClone());
                        completedCaptureCount++;
                    }
                }
            }
        }

        document["_meta"] = new JsonObject
        {
            ["variant"] = "office-js-range-getimage",
            ["capture_total_ms"] =
                Math.Round(captureTotal, 6, MidpointRounding.AwayFromZero),
            ["render_total_ms"] =
                Math.Round(renderTotal, 6, MidpointRounding.AwayFromZero),
            ["persist_total_ms"] =
                Math.Round(persistTotal, 6, MidpointRounding.AwayFromZero),
            ["timing_scope"] =
                "Range.getImage() rendering plus transfer of Base64 PNG data to the "
                + "local host and persistence to disk. Workbook open/close, add-in "
                + "startup, web-server startup, capture-plan generation, and sideload "
                + "registration are excluded.",
            ["excel_api_requirement_set"] = "1.11",
            ["image_batch_size"] = imageBatchSize,
            ["capture_count"] = completedCaptureCount,
            ["planned_capture_count"] = captures.Count,
            ["completed_workbooks"] = results.Count,
            ["expected_workbooks"] = expectedWorkbookNames.Length,
            ["web_host"] = "ASP.NET Core Kestrel / HTTPS localhost"
        };

        document["captures"] = captureArray;

        var parent = Path.GetDirectoryName(outputPath);
        if (!string.IsNullOrEmpty(parent))
        {
            Directory.CreateDirectory(parent);
        }

        var options = new JsonSerializerOptions
        {
            WriteIndented = true
        };

        await File.WriteAllTextAsync(
            outputPath,
            document.ToJsonString(options) + "\n",
            strictUtf8);
    }
    finally
    {
        writeGate.Release();
    }
}

IResult JsonResponse(object value, int statusCode = 200)
{
    return Results.Json(value, statusCode: statusCode);
}

var builder = WebApplication.CreateBuilder(args);

// Range.getImage() payloads are Base64 encoded and can easily exceed Kestrel's
// default request-body limit for large worksheet ranges or multi-image batches.
builder.WebHost.ConfigureKestrel(options =>
{
    options.Limits.MaxRequestBodySize = null;
});

builder.WebHost.UseUrls(
    "https://localhost:3000",
    "http://127.0.0.1:3001");

var app = builder.Build();

app.Use(
    async (context, next) =>
    {
        context.Response.Headers.CacheControl =
            "no-store, no-cache, must-revalidate";
        context.Response.Headers.Pragma = "no-cache";
        context.Response.Headers.Expires = "0";
        await next();
    });

app.MapGet(
    "/health",
    () => JsonResponse(new { ok = true }));

app.MapPost(
    "/reset-diagnostics",
    () =>
    {
        while (errors.TryDequeue(out _))
        {
        }

        while (clientEvents.TryDequeue(out _))
        {
        }

        return JsonResponse(new { ok = true });
    });

app.MapGet(
    "/status",
    (string? workbook) =>
    {
        var done = workbook is not null
            ? results.ContainsKey(workbook)
            : results.Count == expectedWorkbookNames.Length;

        return JsonResponse(new
        {
            done,
            received = results.Keys.OrderBy(x => x).ToArray(),
            expected = expectedWorkbookNames,
            errors = errors.ToArray(),
            client_events = clientEvents.ToArray()
        });
    });

app.MapGet(
    "/capture-plan",
    (string workbook) =>
    {
        if (!capturesByWorkbook.TryGetValue(workbook, out var workbookCaptures))
        {
            return JsonResponse(
                new { error = $"No capture plan for workbook: {workbook}" },
                404);
        }

        return JsonResponse(
            new
            {
                workbook,
                batch_size = imageBatchSize,
                captures = workbookCaptures.Select(
                    capture => new
                    {
                        id = capture.Id,
                        sheet = capture.SheetName,
                        address = capture.Address,
                        relative_png = capture.RelativePng,
                        used_rows = capture.UsedRows,
                        used_columns = capture.UsedColumns,
                        shape_count = capture.ShapeCount,
                        reason = capture.Reason
                    })
            });
    });

app.MapPost(
    "/image-batch",
    async (HttpRequest request) =>
    {
        try
        {
            var node = await JsonNode.ParseAsync(request.Body);

            if (node is not JsonObject payload)
            {
                return JsonResponse(
                    new { error = "Image-batch payload must be a JSON object." },
                    400);
            }

            var workbook = payload["workbook"]?.GetValue<string>();
            if (string.IsNullOrWhiteSpace(workbook))
            {
                return JsonResponse(
                    new { error = "Image-batch payload has no workbook name." },
                    400);
            }

            if (payload["images"] is not JsonArray images)
            {
                return JsonResponse(
                    new { error = "Image-batch payload has no images array." },
                    400);
            }

            var persisted = new JsonArray();

            foreach (var imageNode in images)
            {
                if (imageNode is not JsonObject image)
                {
                    throw new InvalidOperationException(
                        "Image-batch array item must be an object.");
                }

                var id = image["id"]?.GetValue<string>();
                var base64 = image["base64"]?.GetValue<string>();

                if (string.IsNullOrWhiteSpace(id)
                    || string.IsNullOrWhiteSpace(base64))
                {
                    throw new InvalidOperationException(
                        "Image-batch item requires id and base64.");
                }

                var key = workbook + "\n" + id;
                if (!captureByWorkbookAndId.TryGetValue(key, out var plan))
                {
                    throw new InvalidOperationException(
                        $"Unexpected capture id {id} for workbook {workbook}.");
                }

                var bytes = Convert.FromBase64String(base64);
                var (width, height) = ReadPngDimensions(bytes);
                var imagePath = ResolveImagePath(plan.RelativePng);
                var parent = Path.GetDirectoryName(imagePath);

                if (!string.IsNullOrEmpty(parent))
                {
                    Directory.CreateDirectory(parent);
                }

                await File.WriteAllBytesAsync(imagePath, bytes);

                var sha256 = Convert.ToHexString(
                    SHA256.HashData(bytes)).ToLowerInvariant();

                persisted.Add(
                    new JsonObject
                    {
                        ["id"] = plan.Id,
                        ["workbook"] = plan.WorkbookName,
                        ["sheet"] = plan.SheetName,
                        ["address"] = plan.Address,
                        ["relative_png"] = plan.RelativePng,
                        ["used_rows"] = plan.UsedRows,
                        ["used_columns"] = plan.UsedColumns,
                        ["shape_count"] = plan.ShapeCount,
                        ["reason"] = plan.Reason,
                        ["png_bytes"] = bytes.Length,
                        ["png_width"] = width,
                        ["png_height"] = height,
                        ["sha256"] = sha256
                    });
            }

            return JsonResponse(new { captures = persisted });
        }
        catch (Exception exception)
        {
            errors.Enqueue(exception.ToString());
            return JsonResponse(
                new { error = exception.ToString() },
                400);
        }
    });

app.MapPost(
    "/result",
    async (HttpRequest request) =>
    {
        try
        {
            var node = await JsonNode.ParseAsync(request.Body);

            if (node is not JsonObject payload)
            {
                return JsonResponse(
                    new { error = "Result payload must be a JSON object." },
                    400);
            }

            var workbook = payload["workbook"]?.GetValue<string>();
            if (string.IsNullOrWhiteSpace(workbook))
            {
                return JsonResponse(
                    new { error = "Result payload has no workbook name." },
                    400);
            }

            if (!capturesByWorkbook.ContainsKey(workbook))
            {
                return JsonResponse(
                    new { error = $"Unexpected workbook result: {workbook}" },
                    400);
            }

            if (payload["captures"] is not JsonArray)
            {
                return JsonResponse(
                    new { error = "Result payload has no captures array." },
                    400);
            }

            results[workbook] = (JsonObject)payload.DeepClone();
            await WriteOutputAsync();

            return JsonResponse(new { ok = true });
        }
        catch (Exception exception)
        {
            errors.Enqueue(exception.ToString());
            return JsonResponse(
                new { error = exception.ToString() },
                400);
        }
    });

app.MapPost(
    "/error",
    async (HttpRequest request) =>
    {
        using var reader = new StreamReader(
            request.Body,
            Encoding.UTF8,
            detectEncodingFromByteOrderMarks: true);

        errors.Enqueue(await reader.ReadToEndAsync());
        return JsonResponse(new { ok = true });
    });

app.MapPost(
    "/client-event",
    async (HttpRequest request) =>
    {
        using var reader = new StreamReader(
            request.Body,
            Encoding.UTF8,
            detectEncodingFromByteOrderMarks: true);

        var body = await reader.ReadToEndAsync();
        clientEvents.Enqueue(
            $"{DateTimeOffset.UtcNow:O} {body}");
        return JsonResponse(new { ok = true });
    });

app.MapGet(
    "/client-beacon",
    (HttpRequest request) =>
    {
        var stage = request.Query["stage"].ToString();
        var detail = request.Query["detail"].ToString();

        clientEvents.Enqueue(
            $"{DateTimeOffset.UtcNow:O} BEACON {stage} {detail}");

        return Results.NoContent();
    });

app.MapGet(
    "/taskpane.html",
    () =>
    {
        clientEvents.Enqueue(
            $"{DateTimeOffset.UtcNow:O} GET /taskpane.html");
        return Results.File(
            Path.Combine(staticRoot, "taskpane.html"),
            "text/html; charset=utf-8");
    });

app.MapGet(
    "/bootstrap.js",
    () =>
    {
        clientEvents.Enqueue(
            $"{DateTimeOffset.UtcNow:O} GET /bootstrap.js");
        return Results.File(
            Path.Combine(staticRoot, "bootstrap.js"),
            "application/javascript; charset=utf-8");
    });

app.MapGet(
    "/taskpane.js",
    () =>
    {
        clientEvents.Enqueue(
            $"{DateTimeOffset.UtcNow:O} GET /taskpane.js");
        return Results.File(
            Path.Combine(staticRoot, "taskpane.js"),
            "application/javascript; charset=utf-8");
    });

app.MapGet(
    "/commands.html",
    () =>
    {
        clientEvents.Enqueue(
            $"{DateTimeOffset.UtcNow:O} GET /commands.html");
        return Results.File(
            Path.Combine(staticRoot, "commands.html"),
            "text/html; charset=utf-8");
    });

app.MapGet(
    "/icon-32.png",
    () => Results.Bytes(
        Convert.FromBase64String(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwC"
            + "AAAAC0lEQVR42mP8/x8AAusB9Wl2XK0AAAAASUVORK5CYII="),
        "image/png"));

app.Lifetime.ApplicationStarted.Register(
    () =>
    {
        Console.WriteLine(
            "READY https://localhost:3000 http://127.0.0.1:3001");
    });

await app.RunAsync();

sealed record CapturePlan(
    string Id,
    string WorkbookPath,
    string WorkbookName,
    string SheetName,
    string Address,
    int UsedRows,
    int UsedColumns,
    int ShapeCount,
    string RelativePng,
    string Reason);
