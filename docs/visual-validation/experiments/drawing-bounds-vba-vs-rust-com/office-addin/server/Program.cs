using System.Collections.Concurrent;
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
var fileListPath = Path.GetFullPath(
    RequireEnvironment("DRAWING_BOUNDS_FILE_LIST"));
var staticRoot = Path.GetFullPath(
    RequireEnvironment("DRAWING_BOUNDS_STATIC_ROOT"));

var expectedPaths = File.ReadAllLines(fileListPath, strictUtf8)
    .Select(line => line.Trim())
    .Where(line => line.Length != 0)
    .ToArray();

if (expectedPaths.Length == 0)
{
    throw new InvalidOperationException(
        $"File list is empty: {fileListPath}");
}

var expectedNames = expectedPaths
    .Select(path => Path.GetFileName(path))
    .ToArray();

var expectedNameSet = new HashSet<string>(
    expectedNames,
    StringComparer.OrdinalIgnoreCase);

var results = new ConcurrentDictionary<string, JsonObject>(
    StringComparer.OrdinalIgnoreCase);
var errors = new ConcurrentQueue<string>();
var clientEvents = new ConcurrentQueue<string>();
var writeGate = new SemaphoreSlim(1, 1);

static double ReadGeometryTotal(JsonObject payload)
{
    var node = payload["geometry_total_ms"];
    return node is null ? 0.0 : node.GetValue<double>();
}

async Task WriteOutputAsync()
{
    await writeGate.WaitAsync();

    try
    {
        var document = new JsonObject();
        var meta = new JsonObject
        {
            ["variant"] = "office-js-drawing-bounds",
            ["geometry_total_ms"] = 0.0,
            ["timing_scope"] =
                "Office.js batched ShapeCollection property load plus local "
                + "rotation-aware envelope calculation. Workbook open, add-in "
                + "startup, web-server startup, sideload registration, and host "
                + "workbook close are excluded.",
            ["excel_api_requirement_set"] = "1.9",
            ["web_host"] = "ASP.NET Core Kestrel / HTTPS localhost",
            ["completed_workbooks"] = results.Count,
            ["expected_workbooks"] = expectedNames.Length
        };

        document["_meta"] = meta;

        double total = 0.0;

        foreach (var name in expectedNames)
        {
            if (!results.TryGetValue(name, out var payload))
            {
                continue;
            }

            total += ReadGeometryTotal(payload);

            if (payload["sheets"] is not JsonObject sheets)
            {
                throw new InvalidOperationException(
                    $"Result payload for {name} has no sheets object.");
            }

            document[name] = sheets.DeepClone();
        }

        meta["geometry_total_ms"] =
            Math.Round(total, 6, MidpointRounding.AwayFromZero);

        var options = new JsonSerializerOptions
        {
            WriteIndented = true
        };

        var parent = Path.GetDirectoryName(outputPath);
        if (!string.IsNullOrEmpty(parent))
        {
            Directory.CreateDirectory(parent);
        }

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
            : results.Count == expectedNames.Length;

        return JsonResponse(new
        {
            done,
            received = results.Keys.OrderBy(x => x).ToArray(),
            expected = expectedNames,
            errors = errors.ToArray(),
            client_events = clientEvents.ToArray()
        });
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

            if (!expectedNameSet.Contains(workbook))
            {
                return JsonResponse(
                    new
                    {
                        error =
                            $"Unexpected workbook result: {workbook}"
                    },
                    400);
            }

            if (payload["sheets"] is not JsonObject)
            {
                return JsonResponse(
                    new { error = "Result payload has no sheets object." },
                    400);
            }

            results[workbook] =
                (JsonObject)payload.DeepClone();

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
