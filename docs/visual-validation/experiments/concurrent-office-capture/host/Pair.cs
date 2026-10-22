using System.Diagnostics;
using System.Security.Cryptography;
using System.Text.Json.Nodes;

sealed class Pair
{
    readonly object gate = new();
    readonly int[] sheets;
    readonly bool hashFirst;
    readonly string output;
    readonly SemaphoreSlim comparers;
    int sheet, tile;
    int[][]? rows;
    JsonObject? geometry;
    string? geometrySide;
    TaskCompletionSource<JsonObject> geometryReady = New<JsonObject>();
    readonly HashSet<string> completed = [];
    TaskCompletionSource<bool> sheetReady = New<bool>();
    readonly Dictionary<string, JsonObject> offers = [];
    TaskCompletionSource<bool> offerReady = New<bool>();
    readonly Dictionary<string, byte[]> images = [];
    TaskCompletionSource<bool> imageReady = New<bool>();
    public readonly List<object> Differences = [];
    public int Tiles { get; private set; }
    public int HashEqual { get; private set; }
    public long ImageBytes { get; private set; }
    public double CompareMilliseconds { get; private set; }
    public bool Complete { get { lock (gate) return sheet == sheets.Length; } }
    public object Snapshot(int index)
    {
        lock (gate) return new { index, Tiles, HashEqual, ImageBytes, CompareMilliseconds,
            Differences = Differences.ToArray(), Complete = sheet == sheets.Length };
    }

    static TaskCompletionSource<T> New<T>() => new(TaskCreationOptions.RunContinuationsAsynchronously);
    public Pair(int[] sheets, bool hashFirst, string output, SemaphoreSlim comparers)
    { this.sheets = sheets; this.hashFirst = hashFirst; this.output = output; this.comparers = comparers; }

    void Current(int index, int? expectedTile = null)
    {
        if (sheet >= sheets.Length || index != sheets[sheet] || (expectedTile is not null && expectedTile != tile))
            throw new InvalidOperationException("Unexpected sheet or tile; incomplete comparisons are not results.");
    }

    public Task<JsonObject> Geometry(string side, JsonObject data)
    {
        lock (gate)
        {
            Current((int)data["sheet_index"]!);
            int end = (int)data["end_column"]!, max = (int)data["max_end_column"]!;
            if (end < 0 || end > max || max >= 16384) throw new InvalidOperationException("Invalid column boundary.");
            var right = ReadRows(data);
            if (geometry is null) { geometry = data; geometrySide = side; }
            else
            {
                if (side == geometrySide || max != (int)geometry["max_end_column"]!)
                    throw new InvalidOperationException("Duplicate or incompatible geometry.");
                var left = ReadRows(geometry);
                var cuts = left.Concat(right).SelectMany(p => p).Distinct().Order().ToArray();
                var merged = new List<int[]>();
                for (int i = 0; i + 1 < cuts.Length; i++)
                {
                    bool Covers(int[][] source) => source.Any(p => p[0] <= cuts[i] && cuts[i + 1] <= p[1]);
                    if (Covers(left) != Covers(right)) throw new InvalidOperationException("Row coverage differs.");
                    if (Covers(left)) merged.Add([cuts[i], cuts[i + 1]]);
                }
                rows = merged.ToArray();
                if (rows.Length == 0 || rows.Length > 4096) throw new InvalidOperationException("Invalid shared plan.");
                geometryReady.SetResult(new JsonObject {
                    ["tile_rows"] = System.Text.Json.JsonSerializer.SerializeToNode(rows),
                    ["capture_right_column"] = Math.Max(end, (int)geometry["end_column"]!)
                });
            }
            return geometryReady.Task.WaitAsync(TimeSpan.FromSeconds(120));
        }
    }

    static int[][] ReadRows(JsonObject data)
    {
        var spans = data["tile_rows"]!.AsArray().Select(n => n!.AsArray().Select(v => (int)v!).ToArray()).ToArray();
        if (spans.Length == 0 || spans.Length > 4096) throw new InvalidOperationException("Invalid tile plan.");
        for (int i = 0; i < spans.Length; i++)
            if (spans[i].Length != 2 || spans[i][0] < 0 || spans[i][0] >= spans[i][1] ||
                spans[i][1] > 1048576 || (i > 0 && spans[i - 1][1] > spans[i][0]))
                throw new InvalidOperationException("Invalid tile bounds.");
        return spans;
    }

    public Task<bool> Offer(string side, JsonObject data)
    {
        lock (gate)
        {
            Current((int)data["sheet_index"]!, (int)data["tile_index"]!);
            if (rows is null || tile >= rows.Length || !offers.TryAdd(side, data))
                throw new InvalidOperationException("Unexpected image offer.");
            var ready = offerReady;
            if (offers.Count == 2)
            {
                var a = offers["original"]; var b = offers["shrunk"];
                if ((string?)a["range"] != (string?)b["range"])
                    throw new InvalidOperationException("Capture ranges differ.");
                foreach (var item in offers.Values)
                {
                    int w = (int)item["width"]!, h = (int)item["height"]!;
                    if (w <= 0 || h <= 0 || w > 30000 || h > 30000 || (long)w * h > 32_000_000)
                        throw new InvalidOperationException("Invalid image size.");
                    var hash = (string?)item["sha256"];
                    if (hashFirst && (hash?.Length != 64 || !hash.All(Uri.IsHexDigit)))
                        throw new InvalidOperationException("Missing or invalid PNG digest.");
                }
                bool equal = hashFirst && (string?)a["sha256"] == (string?)b["sha256"] &&
                    (int)a["width"]! == (int)b["width"]! && (int)a["height"]! == (int)b["height"]!;
                if (equal) { HashEqual++; Advance(); }
                ready.SetResult(!equal);
            }
            return ready.Task.WaitAsync(TimeSpan.FromSeconds(120));
        }
    }

    void Advance()
    {
        Tiles++; tile++;
        offers.Clear(); images.Clear();
        offerReady = New<bool>(); imageReady = New<bool>();
    }

    public async Task Image(string side, int index, int ordinal, byte[] bytes)
    {
        TaskCompletionSource<bool> ready;
        byte[]? a = null, b = null;
        JsonObject metadata;
        lock (gate)
        {
            Current(index, ordinal);
            if (offers.Count != 2 || !offerReady.Task.IsCompletedSuccessfully || !offerReady.Task.Result)
                throw new InvalidOperationException("Unrequested image.");
            metadata = offers[side];
            if (bytes.Length < 24 || bytes.Length > 64 * 1024 * 1024 ||
                !bytes.AsSpan(0, 8).SequenceEqual(new byte[] { 137, 80, 78, 71, 13, 10, 26, 10 }) ||
                System.Buffers.Binary.BinaryPrimitives.ReadInt32BigEndian(bytes.AsSpan(16, 4)) != (int)metadata["width"]! ||
                System.Buffers.Binary.BinaryPrimitives.ReadInt32BigEndian(bytes.AsSpan(20, 4)) != (int)metadata["height"]!)
                throw new InvalidOperationException("PNG differs from offered dimensions.");
            if (hashFirst && Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant() != (string?)metadata["sha256"])
                throw new InvalidOperationException("Image does not match offered digest.");
            if (!images.TryAdd(side, bytes)) throw new InvalidOperationException("Duplicate image.");
            ImageBytes += bytes.Length;
            ready = imageReady;
            if (images.Count == 2) { a = images["original"]; b = images["shrunk"]; }
        }
        if (a is not null)
        {
            await comparers.WaitAsync();
            try
            {
                var timer = Stopwatch.StartNew();
                var difference = await Task.Run(() => Pixels.Compare(a, b!, Path.Combine(output, $"s{index}-t{ordinal}")));
                lock (gate)
                {
                    CompareMilliseconds += timer.Elapsed.TotalMilliseconds;
                    if (!difference.Equal) Differences.Add(new { sheet_index = index, tile_index = ordinal,
                        range = (string?)metadata["range"], difference, evidence = $"s{index}-t{ordinal}" });
                    Advance();
                }
                ready.SetResult(true);
            }
            catch (Exception error) { ready.TrySetException(error); throw; }
            finally { comparers.Release(); }
        }
        await ready.Task.WaitAsync(TimeSpan.FromSeconds(120));
    }

    public Task<bool> SheetDone(string side, int index)
    {
        lock (gate)
        {
            Current(index);
            if (rows is null || tile != rows.Length || offers.Count != 0 || !completed.Add(side))
                throw new InvalidOperationException("Incomplete or duplicate worksheet completion.");
            var ready = sheetReady;
            if (completed.Count == 2)
            {
                sheet++; tile = 0; rows = null; geometry = null; geometrySide = null;
                completed.Clear(); sheetReady = New<bool>(); geometryReady = New<JsonObject>();
                ready.SetResult(true);
            }
            return ready.Task.WaitAsync(TimeSpan.FromSeconds(120));
        }
    }
}
