using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text.Json.Nodes;
using Microsoft.Extensions.FileProviders;

var settings = JsonNode.Parse(File.ReadAllText(args[0]))!.AsObject();
string output = (string)settings["output"]!, web = (string)settings["web"]!;
Directory.CreateDirectory(output);
var plan = settings["capture"]!.AsObject();
var sheets = plan["sheets"]!.AsArray().Select(item => (int)item!["index"]!).ToArray();
var comparers = new SemaphoreSlim(Math.Max(1, Environment.ProcessorCount));
var quartet = new Quartet(sheets, output, comparers);
string[] sides = ["original", "default", "maximum", "too-aggressive"];
var configs = new Dictionary<string, JsonObject>();
foreach (var side in sides)
{
    var config = plan.DeepClone().AsObject();
    config["mode"] = "paired";
    config["check_heights"] = false;
    config["side"] = side;
    config["run_id"] = side;
    config["workbook"] = side + ".xlsx";
    config["batch_size"] = (int)settings["batch_size"]!;
    config["max_batch_pixels"] = (int)settings["max_batch_pixels"]!;
    config["transport"] = settings["transport"]!.DeepClone();
    configs.Add(side, config);
}
var seen = new ConcurrentDictionary<string, bool>();
var preflightActive = new ConcurrentDictionary<string, bool>();
var declaredReady = new ConcurrentDictionary<string, bool>();
var waiting = new ConcurrentDictionary<string, int>();
var active = new ConcurrentDictionary<string, bool>();
var dataStarted = new ConcurrentDictionary<string, bool>();
var results = new ConcurrentDictionary<string, JsonObject>();
var errors = new ConcurrentQueue<string>();
var start = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
var preflight = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
long lastConfig = Stopwatch.GetTimestamp();
int configRequests = 0;
var clock = new Stopwatch();
double captureSeconds = -1;
var builder = WebApplication.CreateBuilder(Array.Empty<string>());
builder.Logging.ClearProviders();
builder.WebHost.UseUrls($"http://localhost:{(int)settings["port"]!}");
builder.WebHost.ConfigureKestrel(k => k.Limits.MaxRequestBodySize = 160 * 1024 * 1024);
var app = builder.Build();
app.Use(async (context, next) =>
{
    context.Response.Headers.CacheControl = "no-store";
    try { await next(); }
    catch (OperationCanceledException) when (
        (context.Request.Path == "/experiment/start" || context.Request.Path == "/experiment/preflight") &&
        context.RequestAborted.IsCancellationRequested) { }
    catch (Exception error)
    {
        errors.Enqueue(error.Message);
        quartet.Fail(error);
        context.Response.StatusCode = 400;
        await context.Response.WriteAsync(error.Message);
    }
});
app.UseStaticFiles(new StaticFileOptions { FileProvider = new PhysicalFileProvider(web) });
string Client(string id) => configs.ContainsKey(id) ? id :
    throw new InvalidOperationException("Unknown quartet client.");
string DataClient(string id)
{
    Client(id);
    if (!active.ContainsKey(id)) throw new InvalidOperationException("Capture before start barrier.");
    dataStarted[id] = true;
    return id;
}
async Task<JsonObject> Body(HttpRequest request) =>
    (await JsonNode.ParseAsync(request.Body))!.AsObject();
app.MapGet("/config", (HttpRequest request) =>
{
    var id = Client(request.Query["client"].ToString());
    if (dataStarted.ContainsKey(id)) throw new InvalidOperationException("Task pane restarted after capture began.");
    Interlocked.Exchange(ref lastConfig, Stopwatch.GetTimestamp());
    Interlocked.Increment(ref configRequests);
    seen[id] = true;
    return Results.Json(configs[id]);
});
app.MapPost("/experiment/start", async (HttpRequest request) =>
{
    var id = Client((string)(await Body(request))["run_id"]!);
    if (!declaredReady.ContainsKey(id)) throw new InvalidOperationException("Worker did not declare capture readiness.");
    waiting.AddOrUpdate(id, 1, (_, n) => n + 1);
    try
    {
        await start.Task.WaitAsync(request.HttpContext.RequestAborted);
        if (!active.TryAdd(id, true) && dataStarted.ContainsKey(id))
            throw new InvalidOperationException("Duplicate active task pane.");
        return Results.Json(new { ok = true });
    }
    finally
    {
        waiting.AddOrUpdate(id, 0, (_, n) => Math.Max(0, n - 1));
    }
});
app.MapPost("/experiment/preflight", async (HttpRequest request) =>
{
    var id = Client((string)(await Body(request))["run_id"]!);
    if (!seen.ContainsKey(id)) throw new InvalidOperationException("Preflight before add-in configuration.");
    await preflight.Task.WaitAsync(request.HttpContext.RequestAborted);
    preflightActive[id] = true;
    return Results.Json(new { ok = true });
});
app.MapPost("/control/preflight", () =>
{
    if (seen.Count != sides.Length || preflight.Task.IsCompleted ||
        Stopwatch.GetElapsedTime(Volatile.Read(ref lastConfig)).TotalMilliseconds < 2000)
        return Results.Conflict("Add-in startup is not quiet yet.");
    preflight.SetResult(true);
    return Results.Json(new { ok = true });
});
app.MapPost("/ready", async (HttpRequest request) =>
{
    var id = Client((string)(await Body(request))["run_id"]!);
    if (!preflightActive.ContainsKey(id) || !quartet.Prepared || start.Task.IsCompleted)
        throw new InvalidOperationException("Worker declared readiness before workbook preflight completed.");
    declaredReady[id] = true;
    return Results.Json(new { ok = true });
});
app.MapPost("/control/start", () =>
{
    if (waiting.Count(item => item.Value > 0) != sides.Length || declaredReady.Count != sides.Length ||
        !quartet.Prepared || start.Task.IsCompleted)
        return Results.Conflict("Not all clients are ready.");
    clock.Start(); start.SetResult(true);
    return Results.Json(new { ok = true });
});
app.MapPost("/pair/geometry", async (HttpRequest request) =>
{
    var data = await Body(request);
    var id = Client((string)data["run_id"]!);
    if (!preflightActive.ContainsKey(id)) throw new InvalidOperationException("Geometry before preflight release.");
    return Results.Json(await quartet.Geometry(id, data));
});
app.MapPost("/round", async (HttpRequest request) =>
{
    var data = await Body(request);
    return Results.Json(await quartet.Round(DataClient((string)data["run_id"]!), data));
});
app.MapPost("/refine", async (HttpRequest request) =>
{
    var data = await Body(request);
    return Results.Json(await quartet.Refine(DataClient((string)data["run_id"]!), data));
});
app.MapPost("/evidence/{id}/{sheet:int}/{leaf}", async (string id, int sheet, string leaf, HttpRequest request) =>
{
    DataClient(id);
    using var memory = new MemoryStream();
    var receive = Stopwatch.StartNew();
    await request.Body.CopyToAsync(memory);
    quartet.RecordTransferReceive(receive.Elapsed.TotalMilliseconds);
    await quartet.Evidence(id, sheet, leaf, request.Headers["X-Sha256"].ToString(), memory.ToArray());
    return Results.Json(new { ok = true });
});
app.MapPost("/pair/sheet", async (HttpRequest request) =>
{
    var data = await Body(request);
    await quartet.SheetDone(DataClient((string)data["run_id"]!), (int)data["sheet_index"]!);
    return Results.Json(new { ok = true });
});
app.MapPost("/result", async (HttpRequest request) =>
{
    var data = await Body(request);
    var id = Client((string)data["run_id"]!);
    if (data["error"] is not null) errors.Enqueue(id + ": " + data["error"]!.ToJsonString());
    else if (!quartet.ClientComplete(id)) throw new InvalidOperationException("Result before all sheet comparisons completed.");
    if (!results.TryAdd(id, data)) throw new InvalidOperationException("Duplicate result.");
    if (data["error"] is not null) quartet.Fail(new InvalidOperationException(id + " reported capture failure."));
    if (results.Count == sides.Length)
        Interlocked.CompareExchange(ref captureSeconds, clock.Elapsed.TotalSeconds, -1);
    return Results.Json(new { ok = true });
});
app.MapGet("/health", () => Results.Json(new {
    seen = seen.Keys.Order(), ready = waiting.Where(item => item.Value > 0).Select(item => item.Key).Order(),
    ready_declared = declaredReady.Keys.Order(), prepared = quartet.Prepared,
    preflight_released = preflight.Task.IsCompletedSuccessfully,
    started = start.Task.IsCompletedSuccessfully, finished = results.Count, errors = errors.ToArray(),
    complete = Volatile.Read(ref captureSeconds) >= 0 && results.Count == sides.Length &&
        quartet.Complete && errors.IsEmpty,
    capture_seconds = Volatile.Read(ref captureSeconds),
    config_requests = Volatile.Read(ref configRequests),
    config_quiet_ms = Stopwatch.GetElapsedTime(Volatile.Read(ref lastConfig)).TotalMilliseconds,
    quartet = quartet.Snapshot(), clients = results
}));
await app.RunAsync();
