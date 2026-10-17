using System.Diagnostics;
using System.Security.Cryptography;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using SixLabors.ImageSharp;
using SixLabors.ImageSharp.PixelFormats;
using SixLabors.ImageSharp.Processing;

// Sequential per-tile export from persisted evidence. No Excel re-capture or giant stitched canvas.
static class FinalImageEndpoints
{
    public static void Map(WebApplication app, string imageRoot, string staticRoot,
        HashSet<string> expectedNames, Func<string, string> safeName, Action<string, object?> record)
    {
        var gate = new SemaphoreSlim(1, 1);
        app.MapPost("/finalize-image", async (HttpRequest request) =>
        {
            var timer = Stopwatch.StartNew();
            string? temporary = null;
            JsonObject? body = null;
            await gate.WaitAsync();
            try
            {
                body = (await JsonNode.ParseAsync(request.Body))?.AsObject()
                    ?? throw new InvalidOperationException("Expected an export object.");
                var workbook = body["workbook"]?.GetValue<string>() ?? "";
                var id = body["source_id"]?.GetValue<string>() ?? "";
                var sha = body["source_sha256"]?.GetValue<string>() ?? "";
                var sheet = body["sheet"]?.GetValue<string>() ?? "";
                var address = body["address"]?.GetValue<string>() ?? "";
                if (!expectedNames.Contains(workbook) || !Regex.IsMatch(id, @"^[A-Za-z0-9_-]{1,94}$") ||
                    !Regex.IsMatch(sha, @"^[a-f0-9]{64}$") || sheet.Length is < 1 or > 31 || address.Length is < 1 or > 100)
                    throw new InvalidOperationException("Invalid final-image identity.");
                var width = body["width"]!.GetValue<int>();
                var sourceWidth = body["source_width"]!.GetValue<int>();
                var sourceHeight = body["source_height"]!.GetValue<int>();
                var x = body["x"]?.GetValue<int>() ?? 0;
                var y = body["y"]?.GetValue<int>() ?? 0;
                var height = body["height"]?.GetValue<int>() ?? sourceHeight;
                var options = JsonNode.Parse(await File.ReadAllTextAsync(Path.Combine(staticRoot, "capture-options.json")));
                var mp = options?["tile_megapixels"]?.GetValue<int>() ?? 16;
                if (mp is not (16 or 32 or 64) || width < 1 || height < 1 || x < 0 || y < 0 || (long)x+width > sourceWidth || (long)y+height > sourceHeight || sourceWidth < 1 || sourceHeight < 1 ||
                    sourceWidth > 30000 || sourceHeight > 30000 || (long)sourceWidth * sourceHeight > mp * 1_000_000L)
                    throw new InvalidOperationException("Native export pixel budget exceeded.");
                var dir = Path.Combine(imageRoot, safeName(workbook));
                var source = Path.Combine(dir, id + ".png");
                if ((File.GetAttributes(dir) & FileAttributes.ReparsePoint) != 0 ||
                    (File.GetAttributes(source) & FileAttributes.ReparsePoint) != 0)
                    throw new InvalidOperationException("Reparse paths are not valid image sources.");
                if (new FileInfo(source).Length > 128L * 1024 * 1024)
                    throw new InvalidOperationException("Source PNG byte budget exceeded.");
                record("final_png_started", new { workbook, sheet, address, source_id = id, x, y, width, height, source_width = sourceWidth, source_height = sourceHeight });
                var bytes = await File.ReadAllBytesAsync(source);
                if (Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant() != sha)
                    throw new InvalidOperationException("Source PNG hash changed.");
                var info = Image.Identify(bytes);
                if (info.Width != sourceWidth || info.Height != sourceHeight)
                    throw new InvalidOperationException("Source PNG dimensions differ from the capture receipt.");
                using var image = Image.Load<Rgba32>(bytes);
                image.Mutate(ctx => ctx.Crop(new Rectangle(x, y, width, height)));
                var outputId = id + "-final";
                var destination = Path.Combine(dir, outputId + ".png");
                temporary = destination + "." + Guid.NewGuid().ToString("N") + ".new";
                await image.SaveAsPngAsync(temporary);
                var outputBytes = await File.ReadAllBytesAsync(temporary);
                File.Move(temporary, destination); // Fail on an unexpected existing final artifact.
                temporary = null;
                var capture = new { id = outputId, workbook, sheet, address,
                    relative_png = (safeName(workbook) + "/" + outputId + ".png"),
                    png_width = width, png_height = height, png_bytes = outputBytes.Length,
                    sha256 = Convert.ToHexString(SHA256.HashData(outputBytes)).ToLowerInvariant(),
                    source_id = id, export_kind = x != 0 || y != 0 || height != sourceHeight ? "native-context-crop" : "native-right-crop", crop_rect_px = new { x, y, width, height } };
                record("final_png_saved", new { capture, crop_ms = timer.Elapsed.TotalMilliseconds });
                return Results.Json(new { capture, crop_ms = timer.Elapsed.TotalMilliseconds });
            }
            catch (Exception error)
            {
                // Preserve the original screenshot. This is not another Excel render failure.
                record("final_png_failed", new { request = body, error = error.ToString() });
                return Results.Json(new { error = error.ToString(), phase = "final-export" }, statusCode: 400);
            }
            finally
            {
                if (temporary is not null) { try { File.Delete(temporary); } catch { } }
                gate.Release();
            }
        });
    }
}
