using System.Drawing;
using System.Drawing.Imaging;
using System.Security.Cryptography;
using System.Text.Json.Nodes;

int assertions = 0;
void Check(bool value, string message) { assertions++; if (!value) throw new Exception(message); }
byte[] Image(Color color)
{
    using var bitmap = new Bitmap(4, 4);
    using (var graphics = Graphics.FromImage(bitmap)) graphics.Clear(Color.White);
    bitmap.SetPixel(2, 1, color);
    using var output = new MemoryStream();
    bitmap.Save(output, ImageFormat.Png);
    return output.ToArray();
}
JsonObject Geometry() => new() { ["sheet_index"] = 1, ["end_column"] = 1,
    ["max_end_column"] = 2, ["tile_rows"] = new JsonArray(new JsonArray(0, 2)) };
JsonObject Offer(byte[] png) => new() { ["sheet_index"] = 1, ["tile_index"] = 0,
    ["range"] = "A1:B2", ["width"] = 4, ["height"] = 4,
    ["sha256"] = Convert.ToHexString(SHA256.HashData(png)).ToLowerInvariant() };
async Task<bool> Rejected(Func<Task> action)
{
    try { await action(); return false; } catch (InvalidOperationException) { return true; }
}
var directory = Path.Combine(Path.GetTempPath(), "es-parallel-protocol-" + Guid.NewGuid().ToString("N"));
try
{
    var white = Image(Color.White);
    var changed = Image(Color.Red);
    var alternate = white.Concat(new byte[] { 0 }).ToArray();
    using var workers = new SemaphoreSlim(2);
    foreach (var (hash, left, right, expectedEqual) in new[] {
        (true, white, white, true), (true, white, alternate, true),
        (true, white, changed, false), (false, white, white, true),
        (false, white, changed, false) })
    {
        var pair = new Pair([1], hash, Path.Combine(directory, Guid.NewGuid().ToString("N")), workers);
        var geometry = pair.Geometry("original", Geometry());
        Check(!geometry.IsCompleted, "Geometry did not wait for B");
        await pair.Geometry("shrunk", Geometry()); await geometry;
        Check(await Rejected(async () => await pair.SheetDone("original", 1)), "Accepted missing tiles");
        var ao = pair.Offer("original", Offer(left));
        Check(!ao.IsCompleted, "Offer did not wait for B");
        bool upload = await pair.Offer("shrunk", Offer(right));
        Check(await ao == upload, "Upload decisions differ");
        Check(upload == (!hash || !left.SequenceEqual(right)), "Incorrect PNG fast path");
        if (upload)
        {
            var ai = pair.Image("original", 1, 0, left);
            Check(!ai.IsCompleted, "Image comparison did not wait for B");
            await pair.Image("shrunk", 1, 0, right); await ai;
        }
        Check(pair.Differences.Count == (expectedEqual ? 0 : 1), "Pixel result differs");
        var done = pair.SheetDone("original", 1);
        Check(!done.IsCompleted, "Sheet completion did not wait for B");
        await pair.SheetDone("shrunk", 1); await done;
        Check(pair.Complete && pair.Tiles == 1, "Incomplete pair marked complete");
        Check(await Rejected(async () => await pair.Offer("original", Offer(white))), "Accepted extra tile");
    }
    // Eight unrelated sessions must not share barriers or completion state.
    var many = Enumerable.Range(0, 8).Select(_ => new Pair([1], true, directory, workers)).ToArray();
    await Task.WhenAll(many.Select(async pair => {
        var a = pair.Geometry("original", Geometry()); await pair.Geometry("shrunk", Geometry()); await a;
        var b = pair.Offer("original", Offer(white)); await pair.Offer("shrunk", Offer(white)); await b;
        var c = pair.SheetDone("original", 1); await pair.SheetDone("shrunk", 1); await c;
    }));
    Check(many.All(pair => pair.Complete), "Eight-pair routing failed");
    Console.WriteLine("PASS " + assertions + " protocol assertions");
}
finally
{
    if (Directory.Exists(directory)) Directory.Delete(directory, true);
}
