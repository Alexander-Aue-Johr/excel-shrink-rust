using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text.Json.Nodes;
using Microsoft.Extensions.FileProviders;

var settings = JsonNode.Parse(File.ReadAllText(args[0]))!.AsObject();
string output = (string)settings["output"]!, web = (string)settings["web"]!;
Directory.CreateDirectory(output);
int count = (int)settings["pairs"]!;
bool hashFirst = (string)settings["transport"]! == "hash";
var plan = settings["capture"]!.AsObject();
int[] sheets = plan["sheets"]!.AsArray().Select(s => (int)s!["index"]!).ToArray();
var comparers = new SemaphoreSlim(Math.Max(1, Math.Min(Environment.ProcessorCount, count)));
var pairs = Enumerable.Range(0, count).Select(i =>
    new Pair(sheets, hashFirst, Path.Combine(output, $"pair-{i}"), comparers)).ToArray();
var configs = new Dictionary<string, JsonObject>();
for (int i = 0; i < count; i++)
    foreach (var side in new[] { "original", "shrunk" })
    {
        var config = plan.DeepClone().AsObject();
        config["mode"] = "paired"; config["check_heights"] = false; config["side"] = side;
        config["run_id"] = $"{i}-{side}";
        config["workbook"] = $"{i}-{side}.xlsx";
        config["batch_size"] = (int)settings["batch_size"]!;
        config["max_batch_pixels"] = (int)settings["max_batch_pixels"]!;
        config["transport"] = hashFirst ? "hash" : "images";
        configs.Add($"{i}-{side}", config);
    }
var seen = new ConcurrentDictionary<string, bool>();
var active = new ConcurrentDictionary<string, bool>();
var results = new ConcurrentDictionary<string, JsonObject>();
var errors = new ConcurrentQueue<string>();
var start = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
var clock = new Stopwatch();
double? captureSeconds = null;
var appBuilder = WebApplication.CreateBuilder(Array.Empty<string>());
appBuilder.Logging.ClearProviders();
appBuilder.WebHost.UseUrls($"http://localhost:{(int)settings["port"]!}");
appBuilder.WebHost.ConfigureKestrel(k => k.Limits.MaxRequestBodySize = 64 * 1024 * 1024);
var app = appBuilder.Build();
app.Use(async (context, next) =>
{
    context.Response.Headers.CacheControl = "no-store";
    try { await next(); }
    catch (Exception error)
    {
        errors.Enqueue(error.Message);
        context.Response.StatusCode = 400;
        await context.Response.WriteAsync(error.Message);
    }
});
app.UseStaticFiles(new StaticFileOptions { FileProvider = new PhysicalFileProvider(web) });

(string Id, int Index, string Side) Client(string id)
{
    if (!configs.ContainsKey(id)) throw new InvalidOperationException("Unknown client.");
    var parts = id.Split('-', 2);
    return (id, int.Parse(parts[0]), parts[1]);
}
async Task<JsonObject> Body(HttpRequest request) =>
    (await JsonNode.ParseAsync(request.Body))!.AsObject();
(string Id, int Index, string Side) DataClient(string id)
{
    var client = Client(id);
    if (!active.ContainsKey(id)) throw new InvalidOperationException("Capture before start barrier.");
    return client;
}

app.MapGet("/config", (HttpRequest request) =>
{
    var client = Client(request.Query["client"].ToString());
    if (active.ContainsKey(client.Id)) throw new InvalidOperationException("Task pane restarted after capture began.");
    seen[client.Id] = true;
    return Results.Json(configs[client.Id]);
});
app.MapPost("/experiment/start", async (HttpRequest request) =>
{
    var client = Client((string)(await Body(request))["run_id"]!);
    await start.Task.WaitAsync(TimeSpan.FromMinutes(15));
    if (!active.TryAdd(client.Id, true)) throw new InvalidOperationException("Duplicate active task pane.");
    return Results.Json(new { ok = true });
});
app.MapPost("/control/start", () =>
{
    if (seen.Count != count * 2 || start.Task.IsCompleted) return Results.Conflict("Not all clients are ready.");
    clock.Start(); start.SetResult(true); return Results.Json(new { ok = true });
});
app.MapPost("/pair/geometry", async (HttpRequest request) =>
{
    var data = await Body(request); var client = DataClient((string)data["run_id"]!);
    return Results.Json(await pairs[client.Index].Geometry(client.Side, data));
});
app.MapPost("/offer", async (HttpRequest request) =>
{
    var data = await Body(request); var client = DataClient((string)data["run_id"]!);
    return Results.Json(new { upload = await pairs[client.Index].Offer(client.Side, data) });
});
app.MapPost("/image/{id}/{sheet:int}/{tile:int}", async (string id, int sheet, int tile, HttpRequest request) =>
{
    var client = DataClient(id);
    using var memory = new MemoryStream();
    await request.Body.CopyToAsync(memory);
    await pairs[client.Index].Image(client.Side, sheet, tile, memory.ToArray());
    return Results.Json(new { ok = true });
});
app.MapPost("/pair/sheet", async (HttpRequest request) =>
{
    var data = await Body(request); var client = DataClient((string)data["run_id"]!);
    await pairs[client.Index].SheetDone(client.Side, (int)data["sheet_index"]!);
    return Results.Json(new { ok = true });
});
app.MapPost("/result", async (HttpRequest request) =>
{
    var data = await Body(request); var client = Client((string)data["run_id"]!);
    if (data["error"] is not null) errors.Enqueue(client.Id + ": " + data["error"]!.ToJsonString());
    if (!results.TryAdd(client.Id, data)) throw new InvalidOperationException("Duplicate result.");
    if (results.Count == count * 2) captureSeconds = clock.Elapsed.TotalSeconds;
    return Results.Json(new { ok = true });
});
app.MapGet("/health", () => Results.Json(new {
    seen = seen.Keys.Order(), finished = results.Count, errors = errors.ToArray(),
    complete = results.Count == count * 2 && pairs.All(p => p.Complete) && errors.IsEmpty,
    capture_seconds = captureSeconds,
    pairs = pairs.Select((p, i) => p.Snapshot(i)),
    clients = results
}));
await app.RunAsync();
