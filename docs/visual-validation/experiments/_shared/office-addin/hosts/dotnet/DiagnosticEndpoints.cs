using System.Collections.Concurrent;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using SixLabors.ImageSharp;
using SixLabors.ImageSharp.PixelFormats;

// Only used by explicit diagnostic experiments. Large hospital images are never decoded here.
static class DiagnosticEndpoints
{
    public static void Map(WebApplication app, string imageRoot, HashSet<string> expectedNames)
    {
        var owners = new ConcurrentDictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var gate = new SemaphoreSlim(1, 1);
        string Workbook(JsonObject body)
        {
            var name = body["workbook"]?.GetValue<string>() ?? "";
            if (!expectedNames.Contains(name)) throw new InvalidOperationException("Unexpected diagnostic workbook");
            if (name != Path.GetFileName(name) || name.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0)
                throw new InvalidOperationException("Invalid workbook filename");
            return name;
        }
        app.MapPost("/diagnostic-claim", async (HttpRequest request) =>
        {
            var body = (await JsonNode.ParseAsync(request.Body))!.AsObject();
            var workbook = Workbook(body);
            var client = body["client_id"]?.GetValue<string>() ?? "";
            if (client.Length is < 1 or > 200) return Results.BadRequest(new { error = "Invalid client ID" });
            var owner = owners.GetOrAdd(workbook, client);
            return Results.Json(new { acquired = string.Equals(owner, client, StringComparison.Ordinal) });
        });
        app.MapPost("/diagnostic-progress", async (HttpRequest request) =>
        {
            var body = (await JsonNode.ParseAsync(request.Body))!.AsObject();
            var workbook = Workbook(body);
            var dir = Path.Combine(imageRoot, "_diagnostics");
            await gate.WaitAsync();
            try
            {
                Directory.CreateDirectory(dir);
                var destination = Path.Combine(dir, workbook + ".progress.json");
                var temporary = destination + ".new";
                await File.WriteAllTextAsync(temporary, body.ToJsonString(new JsonSerializerOptions { WriteIndented = true }), new UTF8Encoding(false));
                File.Move(temporary, destination, overwrite: true);
            }
            finally { gate.Release(); }
            return Results.Json(new { ok = true });
        });
        app.MapPost("/diagnostic-markers", async (HttpRequest request) =>
        {
            try
            {
                var body = (await JsonNode.ParseAsync(request.Body))!.AsObject();
                var workbook = Workbook(body);
                var id = body["id"]?.GetValue<string>() ?? "";
                if (!Regex.IsMatch(id, "^d[0-9]{5,8}$")) throw new InvalidOperationException("Invalid capture ID");
                var axis = body["axis"]?.GetValue<string>();
                if (axis is not ("x" or "y")) throw new InvalidOperationException("Invalid marker axis");
                var path = Path.Combine(imageRoot, workbook, id + ".png");
                var info = Image.Identify(path) ?? throw new InvalidOperationException("Cannot identify marker PNG");
                const long maxPixels = 20_000_000;
                if ((long)info.Width * info.Height > maxPixels)
                    throw new InvalidOperationException("Marker decode budget exceeded; PNG is retained. Limit is 20 million pixels.");
                var definitions = body["markers"]!.AsArray();
                if (definitions.Count is < 1 or > 32) throw new InvalidOperationException("Invalid marker count");
                var palette = definitions.Select(node =>
                {
                    var rgb = node!["rgb"]!.GetValue<string>();
                    if (!Regex.IsMatch(rgb, "^[A-Fa-f0-9]{6}$")) throw new InvalidOperationException("Invalid marker RGB");
                    return (Name: node["name"]!.GetValue<string>(), R: Convert.ToInt32(rgb[..2], 16),
                        G: Convert.ToInt32(rgb[2..4], 16), B: Convert.ToInt32(rgb[4..], 16));
                }).ToArray();
                using var image = Image.Load<Rgba32>(path);
                var length = axis == "y" ? image.Height : image.Width;
                var cross = axis == "y" ? image.Width : image.Height;
                var samples = new[] { 0.70, 0.80, 0.90 }.Select(f => Math.Clamp((int)(f * (cross - 1)), 0, cross - 1)).ToArray();
                var runs = Enumerable.Range(0, palette.Length).Select(_ => new List<int[]>()).ToArray();
                var active = -1; var start = 0;
                for (var position = 0; position <= length; position++)
                {
                    var found = -1;
                    if (position < length)
                    {
                        for (var m = 0; m < palette.Length; m++)
                        {
                            var count = 0;
                            foreach (var s in samples)
                            {
                                var p = axis == "y" ? image[s, position] : image[position, s];
                                if (p.A > 200 && Math.Abs(p.R - palette[m].R) <= 24 && Math.Abs(p.G - palette[m].G) <= 24 && Math.Abs(p.B - palette[m].B) <= 24) count++;
                            }
                            if (count >= 2) { found = m; break; }
                        }
                    }
                    if (found == active) continue;
                    if (active >= 0 && position - start >= 2) runs[active].Add(new[] { start, position - 1 });
                    active = found; start = position;
                }
                var markers = palette.Select((p, i) => new
                {
                    name = p.Name, count = runs[i].Sum(run => run[1] - run[0] + 1),
                    first = runs[i].Count > 0 ? (int?)runs[i][0][0] : null,
                    last = runs[i].Count > 0 ? (int?)runs[i][^1][1] : null,
                    segments = runs[i]
                }).ToArray();
                return Results.Json(new { axis, png_width = image.Width, png_height = image.Height,
                    sample_cross_positions = samples, markers, decoded_by = "ImageSharp host; no browser canvas" });
            }
            catch (Exception exception)
            {
                // This is an analysis error, not a global experiment host/render failure.
                return Results.Json(new { error = exception.ToString() }, statusCode: 400);
            }
        });
    }
}
