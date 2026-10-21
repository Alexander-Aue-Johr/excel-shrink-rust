using System.Buffers.Binary;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Microsoft.Extensions.FileProviders;

const int MaxImageBytes = 160 * 1024 * 1024;

var arguments = ReadArguments(args);
var configPath = Path.GetFullPath(Required(arguments, "config"));
var outputDirectory = Path.GetFullPath(Required(arguments, "output"));
var publishedDirectory = Path.GetFullPath(Required(arguments, "published-output"));
var webDirectory = Path.GetFullPath(Required(arguments, "web"));
var resultPath = Path.Combine(outputDirectory, "result.json");
var runId = Required(arguments, "run-id");

if (!File.Exists(configPath) || !Directory.Exists(webDirectory))
    throw new InvalidOperationException("Capture config or web assets are missing.");
Directory.CreateDirectory(outputDirectory);

using var configDocument = JsonDocument.Parse(File.ReadAllBytes(configPath));
var config = configDocument.RootElement;
var mode = RequiredString(config, "mode");
if (mode == "capture") Directory.CreateDirectory(Path.Combine(outputDirectory, "images"));
if (mode is not ("heights" or "capture" or "paired"))
    throw new InvalidOperationException("Invalid capture mode.");
if (!config.TryGetProperty("sheets", out var sheets) || sheets.ValueKind != JsonValueKind.Array)
    throw new InvalidOperationException("Capture config has no sheets array.");
var expectedSheets = new Dictionary<int, string>();
foreach (var sheet in sheets.EnumerateArray())
{
    var index = sheet.GetProperty("index").GetInt32();
    if (index < 1 || !expectedSheets.TryAdd(index, RequiredString(sheet, "name")))
        throw new InvalidOperationException("Invalid or duplicate sheet index.");
}

using var paired = mode == "paired" ? new PairedCapture(Required(arguments, "stream-pipe"), runId, expectedSheets.Keys.ToArray()) : null;
var pairedResults = new Dictionary<string, JsonNode>();
var resultGate = new SemaphoreSlim(1);

var builder = WebApplication.CreateBuilder(args);
var port = arguments.TryGetValue("port", out var configuredPort) ? int.Parse(configuredPort) : 3000;
if (port is < 1024 or > 65535) throw new InvalidOperationException("Invalid localhost port.");
builder.WebHost.UseUrls($"http://localhost:{port}");
builder.WebHost.ConfigureKestrel(server => server.Limits.MaxRequestBodySize = MaxImageBytes * 2L);
var app = builder.Build();
var configSeen = 0;
var originalSeen = 0;
var shrunkSeen = 0;
var files = new PhysicalFileProvider(webDirectory);
app.Use(async (context, next) =>
{
    context.Response.Headers.CacheControl = "no-store";
    await next();
});
app.UseDefaultFiles(new DefaultFilesOptions { FileProvider = files });
app.UseStaticFiles(new StaticFileOptions { FileProvider = files });

app.MapGet("/health", () => Results.Json(new { ok = true, run_id = runId,
    config_seen = Volatile.Read(ref configSeen) != 0,
    original_seen = Volatile.Read(ref originalSeen) != 0,
    shrunk_seen = Volatile.Read(ref shrunkSeen) != 0 }));
app.MapGet("/config", (HttpRequest request) =>
{
    Interlocked.Exchange(ref configSeen, 1);
    var response = JsonNode.Parse(config.GetRawText())!.AsObject();
    if (paired is not null)
    {
        var side = request.Query["side"].ToString();
        if (side is not ("original" or "shrunk")) return Results.BadRequest("Invalid capture side.");
        if (side == "original") Interlocked.Exchange(ref originalSeen, 1);
        else Interlocked.Exchange(ref shrunkSeen, 1);
        response = response[side]!.AsObject();
        response["mode"] = "paired";
        response["side"] = side;
    }
    response["run_id"] = runId;
    return Results.Json(response);
});

app.MapPost("/pair/geometry", async (HttpRequest request) =>
{
    if (paired is null) return Results.BadRequest("Not a paired capture.");
    try { return Results.Json(await paired.Geometry((await JsonNode.ParseAsync(request.Body))!.AsObject())); }
    catch (Exception error) { return Results.BadRequest(error.Message); }
});
app.MapPost("/pair/heights", async (HttpRequest request) =>
{
    if (paired is null) return Results.BadRequest("Not a paired capture.");
    try { return Results.Json(await paired.Heights((await JsonNode.ParseAsync(request.Body))!.AsObject())); }
    catch (Exception error) { return Results.BadRequest(error.Message); }
});
app.MapPost("/pair/sheet", async (HttpRequest request) =>
{
    if (paired is null) return Results.BadRequest("Not a paired capture.");
    try { await paired.SheetDone((await JsonNode.ParseAsync(request.Body))!.AsObject()); return Results.Json(new {ok = true}); }
    catch (Exception error) { return Results.BadRequest(error.Message); }
});
app.MapPost("/pair/image", async (HttpRequest request) =>
{
    if (paired is null) return Results.BadRequest("Not a paired capture.");
    try
    {
        var data = (await JsonNode.ParseAsync(request.Body))!.AsObject();
        paired.Validate(data);
        var index = (int)data["sheet_index"]!;
        if (!expectedSheets.TryGetValue(index, out var name) || name != (string?)data["sheet_name"])
            return Results.BadRequest("Unknown worksheet.");
        var base64 = (string)data["base64"]!;
        if (base64.Length > MaxImageBytes * 4L / 3L + 4) return Results.BadRequest("Image is too large.");
        var image = Convert.FromBase64String(base64);
        if (image.Length > MaxImageBytes) return Results.BadRequest("Image is too large.");
        var (width, height) = CheckPng(image);
        if (width > 30_000 || height > 30_000 || (long)width * height > 32_000_000)
            return Results.BadRequest("Paired image exceeds the tile budget.");
        data.Remove("base64");
        await paired.Image(data, image);
        return Results.Json(new {ok = true});
    }
    catch (Exception error) { return Results.BadRequest(error.Message); }
});

app.MapPost("/image", async (HttpRequest request) =>
{
    if (mode != "capture") return Results.BadRequest("Images are not accepted in height mode.");
    JsonDocument payload;
    try { payload = await JsonDocument.ParseAsync(request.Body); }
    catch (JsonException) { return Results.BadRequest("Invalid image JSON."); }
    using (payload)
    {
        try
        {
            var data = payload.RootElement;
            if (RequiredString(data, "run_id") != runId) return Results.Conflict("Capture run changed.");
            var sheetIndex = data.GetProperty("sheet_index").GetInt32();
            var sheetName = RequiredString(data, "sheet_name");
            if (!expectedSheets.TryGetValue(sheetIndex, out var expectedName) || expectedName != sheetName)
                return Results.BadRequest("Unknown worksheet.");
            var range = RequiredString(data, "range");
            if (range.Length > 32 || !Regex.IsMatch(range, @"\A[A-Z]+[1-9][0-9]*(?::[A-Z]+[1-9][0-9]*)?\z", RegexOptions.CultureInvariant))
                return Results.BadRequest("Invalid capture range.");
            var tileIndex = data.GetProperty("tile_index").GetInt32();
            if (tileIndex < 0 || tileIndex > 1_000_000) return Results.BadRequest("Invalid tile index.");
            var base64 = RequiredString(data, "base64");
            const string prefix = "data:image/png;base64,";
            if (base64.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)) base64 = base64[prefix.Length..];
            if (base64.Length > MaxImageBytes * 4L / 3L + 4) return Results.BadRequest("Image is too large.");
            var image = Convert.FromBase64String(base64);
            if (image.Length > MaxImageBytes) return Results.BadRequest("Image is too large.");
            var (width, height) = CheckPng(image);
            var name = $"s{sheetIndex:D4}-t{tileIndex:D5}.png";
            var relativePath = Path.Combine("images", name);
            var localPath = Path.Combine(outputDirectory, relativePath);
            await WriteNewFile(localPath, image);
            return Results.Json(new { path = Path.Combine(publishedDirectory, relativePath), width, height });
        }
        catch (Exception error) when (error is FormatException or InvalidOperationException or KeyNotFoundException)
        {
            return Results.BadRequest(error.Message);
        }
    }
});

app.MapPost("/result", async (HttpRequest request) =>
{
    JsonDocument payload;
    try { payload = await JsonDocument.ParseAsync(request.Body); }
    catch (JsonException) { return Results.BadRequest("Invalid result JSON."); }
    using (payload)
    {
        try
        {
            var data = payload.RootElement;
            if (RequiredString(data, "run_id") != runId) return Results.Conflict("Capture run changed.");
            if (paired is not null)
            {
                var node = JsonNode.Parse(data.GetRawText())!.AsObject();
                paired.Validate(node);
                await resultGate.WaitAsync();
                try
                {
                    if (File.Exists(resultPath)) return Results.Conflict("Result already received.");
                    if (!pairedResults.TryAdd((string)node["side"]!, node)) return Results.Conflict("Duplicate paired result.");
                    bool error = node["error"] is not null;
                    if (error || pairedResults.Count == 2)
                    {
                        if (!error && !paired.Complete) throw new InvalidOperationException("Paired capture has incomplete worksheets.");
                        var result = new JsonObject { ["run_id"] = runId, ["heights"] = new JsonArray(), ["captures"] = new JsonArray() };
                        if (error) result["error"] = node["error"]!.DeepClone();
                        await WriteResultAtomically(resultPath, System.Text.Encoding.UTF8.GetBytes(result.ToJsonString()));
                        await paired.Finish(error);
                    }
                    return Results.Json(new {ok = true});
                }
                finally { resultGate.Release(); }
            }
            var field = mode == "heights" ? "heights" : "captures";
            if (!data.TryGetProperty(field, out var entries) || entries.ValueKind != JsonValueKind.Array)
                return Results.BadRequest($"Result has no {field} array.");
            if (File.Exists(resultPath)) return Results.Conflict("Result already received.");
            await WriteResultAtomically(resultPath, JsonSerializer.SerializeToUtf8Bytes(data));
            return Results.Json(new { ok = true });
        }
        catch (Exception error) when (error is InvalidOperationException or KeyNotFoundException or IOException)
        {
            return Results.BadRequest(error.Message);
        }
    }
});

app.Run();

static Dictionary<string, string> ReadArguments(string[] args)
{
    if (args.Length % 2 != 0) throw new ArgumentException("Expected named host arguments.");
    var result = new Dictionary<string, string>(StringComparer.Ordinal);
    for (var i = 0; i < args.Length; i += 2)
    {
        if (!args[i].StartsWith("--", StringComparison.Ordinal) || !result.TryAdd(args[i][2..], args[i + 1]))
            throw new ArgumentException("Invalid host argument.");
    }
    return result;
}

static string Required(Dictionary<string, string> arguments, string key) =>
    arguments.TryGetValue(key, out var value) && !string.IsNullOrWhiteSpace(value)
        ? value : throw new ArgumentException($"Missing --{key}.");

static string RequiredString(JsonElement value, string name) =>
    value.GetProperty(name).GetString() is { Length: > 0 } text
        ? text : throw new InvalidOperationException($"Missing {name}.");

static async Task WriteNewFile(string path, byte[] bytes)
{
    await using var file = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None);
    await file.WriteAsync(bytes);
    file.Flush(flushToDisk: true);
}

static async Task WriteResultAtomically(string path, byte[] bytes)
{
    var pending = path + "." + Guid.NewGuid().ToString("N") + ".new";
    try
    {
        await WriteNewFile(pending, bytes);
        File.Move(pending, path);
    }
    finally
    {
        if (File.Exists(pending)) File.Delete(pending);
    }
}

static (int Width, int Height) CheckPng(byte[] png)
{
    const int maxImagePixels = 120_000_000;
    ReadOnlySpan<byte> signature = stackalloc byte[] { 137, 80, 78, 71, 13, 10, 26, 10 };
    if (png.Length < 45 || !png.AsSpan(0, 8).SequenceEqual(signature))
        throw new InvalidOperationException("Image is not a PNG.");
    var offset = 8;
    var width = 0;
    var height = 0;
    var imageDataSeen = false;
    var endSeen = false;
    while (offset + 12 <= png.Length)
    {
        var length = BinaryPrimitives.ReadUInt32BigEndian(png.AsSpan(offset, 4));
        if (length > png.Length - offset - 12) throw new InvalidOperationException("Truncated PNG chunk.");
        var chunk = png.AsSpan(offset + 4, checked((int)length + 4));
        var expectedCrc = BinaryPrimitives.ReadUInt32BigEndian(png.AsSpan(offset + 8 + (int)length, 4));
        if (PngCrc(chunk) != expectedCrc) throw new InvalidOperationException("Invalid PNG checksum.");
        var type = png.AsSpan(offset + 4, 4);
        if (offset == 8)
        {
            if (!type.SequenceEqual("IHDR"u8) || length != 13) throw new InvalidOperationException("Missing PNG header.");
            var encodedWidth = BinaryPrimitives.ReadUInt32BigEndian(png.AsSpan(offset + 8, 4));
            var encodedHeight = BinaryPrimitives.ReadUInt32BigEndian(png.AsSpan(offset + 12, 4));
            if (encodedWidth is 0 or > int.MaxValue || encodedHeight is 0 or > int.MaxValue
                || (long)encodedWidth * encodedHeight > maxImagePixels)
                throw new InvalidOperationException("Invalid PNG dimensions.");
            width = (int)encodedWidth;
            height = (int)encodedHeight;
            if (png[offset + 18] != 0 || png[offset + 19] != 0 || png[offset + 20] != 0)
                throw new InvalidOperationException("Unsupported PNG encoding.");
        }
        else if (type.SequenceEqual("IHDR"u8)) throw new InvalidOperationException("Duplicate PNG header.");
        if (type.SequenceEqual("IDAT"u8)) imageDataSeen = true;
        if (type.SequenceEqual("IEND"u8))
        {
            if (length != 0 || offset + 12 != png.Length) throw new InvalidOperationException("Invalid PNG ending.");
            endSeen = true;
            break;
        }
        offset += checked((int)length + 12);
    }
    if (!imageDataSeen || !endSeen) throw new InvalidOperationException("Incomplete PNG.");
    return (width, height);
}

static uint PngCrc(ReadOnlySpan<byte> bytes)
{
    var crc = uint.MaxValue;
    foreach (var value in bytes)
    {
        crc ^= value;
        for (var bit = 0; bit < 8; bit++)
            crc = (crc >> 1) ^ ((crc & 1) == 0 ? 0u : 0xedb88320u);
    }
    return ~crc;
}
