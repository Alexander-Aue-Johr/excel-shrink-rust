using System.Drawing;
using System.Drawing.Imaging;
using System.Security.Cryptography;
using System.Text.Json.Nodes;

byte[] Image(Color color)
{
    using var bitmap = new Bitmap(4, 4);
    using (var graphics = Graphics.FromImage(bitmap)) graphics.Clear(Color.White);
    bitmap.SetPixel(2, 1, color);
    using var output = new MemoryStream();
    bitmap.Save(output, ImageFormat.Png);
    return output.ToArray();
}

string Hash(byte[] png) => Convert.ToHexString(SHA256.HashData(png)).ToLowerInvariant();
JsonObject Geometry(int sheet = 1) => new() {
    ["sheet_index"] = sheet, ["end_column"] = 1,
    ["max_end_column"] = 2, ["tile_rows"] = new JsonArray(new JsonArray(0, 2))
};
JsonObject Tile(byte[] png, string range, int? index = null, string? id = null)
{
    var tile = new JsonObject { ["range"] = range, ["width"] = 4, ["height"] = 4,
        ["sha256"] = Hash(png) };
    if (index is not null) tile["tile_index"] = index.Value;
    if (id is not null) tile["id"] = id;
    return tile;
}
JsonObject Round(byte[] png) => new() {
    ["sheet_index"] = 1, ["tiles"] = new JsonArray(Tile(png, "A1:B2", index: 0))
};
JsonObject Refine(byte[] source, byte[] changed, bool isChanged) => new() {
    ["sheet_index"] = 1, ["parent_id"] = "t0",
    ["parts"] = new JsonArray(
        Tile(source, "A1:A1", id: "t0.0"), Tile(source, "B1:B1", id: "t0.1"),
        Tile(isChanged ? changed : source, "A2:A2", id: "t0.2"),
        Tile(source, "B2:B2", id: "t0.3"))
};
void Check(bool value, string message)
{
    if (!value) throw new Exception(message);
}

string[] sides = ["original", "default", "maximum", "too-aggressive"];
var directory = Path.Combine(Path.GetTempPath(), "es-quartet-tests-" + Guid.NewGuid().ToString("N"));
try
{
    var source = Image(Color.White);
    var changed = Image(Color.Red);
    using var workers = new SemaphoreSlim(3);
    var quartet = new Quartet([1], directory, workers);
    var firstGeometry = quartet.Geometry("original", Geometry());
    Check(!firstGeometry.IsCompleted && !quartet.Prepared, "Geometry did not wait for every side");
    await Task.WhenAll(sides.Skip(1).Select(side => quartet.Geometry(side, Geometry())));
    await firstGeometry;
    Check(quartet.Prepared, "Quartet did not complete preflight");

    var firstRound = quartet.Round("original", Round(source));
    Check(!firstRound.IsCompleted, "Initial digest round released too early");
    var rounds = sides.Skip(1).Select(side => quartet.Round(side,
        Round(side == "too-aggressive" ? changed : source))).ToArray();
    await Task.WhenAll(rounds);
    var root = await firstRound;
    Check(root["divergent"]!.AsArray().Select(item => (int)item!).SequenceEqual([0]),
        "Wrong initial divergent tile");

    var firstRefinement = quartet.Refine("original", Refine(source, changed, false));
    Check(!firstRefinement.IsCompleted, "Refinement released too early");
    var refinements = sides.Skip(1).Select(side => quartet.Refine(side,
        Refine(source, changed, side == "too-aggressive"))).ToArray();
    await Task.WhenAll(refinements);
    var children = await firstRefinement;
    Check(children["divergent"]!.AsArray().Select(item => (string)item!).SequenceEqual(["t0.2"]),
        "Wrong divergent refinement child");

    var firstEvidence = quartet.Evidence("original", 1, "t0.2", Hash(source), source);
    Check(!firstEvidence.IsCompleted, "Evidence comparison released too early");
    var images = sides.Skip(1).Select(side => quartet.Evidence(side, 1, "t0.2",
        Hash(side == "too-aggressive" ? changed : source),
        side == "too-aggressive" ? changed : source)).ToArray();
    await Task.WhenAll(images);
    await firstEvidence;
    await Task.WhenAll(sides.Select(side => quartet.SheetDone(side, 1)));
    Check(quartet.Complete && sides.All(quartet.ClientComplete), "Quartet did not finish");
    var snapshot = System.Text.Json.JsonSerializer.SerializeToNode(quartet.Snapshot())!;
    var comparisons = snapshot["comparisons"]!.AsArray();
    Check(comparisons.Count == 3, "Missing profile comparison");
    Check(comparisons[0]!["result"]!["Differences"]!.AsArray().Count == 0, "False default difference");
    Check(comparisons[1]!["result"]!["Differences"]!.AsArray().Count == 0, "False maximum difference");
    Check(comparisons[2]!["result"]!["Differences"]!.AsArray().Count == 1,
        "Too-aggressive difference was lost");
    Check(File.Exists(Path.Combine(directory, "too-aggressive", "s1-t0.2-original.png")),
        "Original evidence image is missing");
    Check(File.Exists(Path.Combine(directory, "too-aggressive", "s1-t0.2-shrunk.png")),
        "Changed evidence image is missing");
    var recapturedEqual = new Quartet([1], Path.Combine(directory, "equal"), workers);
    await Task.WhenAll(sides.Select(side => recapturedEqual.Geometry(side, Geometry())));
    await Task.WhenAll(sides.Select(side => recapturedEqual.Round(side,
        Round(side == "too-aggressive" ? changed : source))));
    var equalReplies = await Task.WhenAll(sides.Select(side => recapturedEqual.Refine(side,
        Refine(source, changed, false))));
    Check(equalReplies.All(reply => reply["divergent"]!.AsArray().Count == 0),
        "Equal recapture should finish a divergent parent without evidence");
    await Task.WhenAll(sides.Select(side => recapturedEqual.SheetDone(side, 1)));
    Check(recapturedEqual.Complete, "Equal recapture did not finish");
    Console.WriteLine("PASS quartet preflight, digest round, refinement, evidence and completion");
}
finally
{
    if (Directory.Exists(directory)) Directory.Delete(directory, true);
}
