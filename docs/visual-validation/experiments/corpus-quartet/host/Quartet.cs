using System.Buffers.Binary;
using System.Diagnostics;
using System.Security.Cryptography;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

// Workers capture independently; only complete digest rounds synchronize the quartet.
sealed class Quartet
{
    static readonly string[] Sides = ["original", "default", "maximum", "too-aggressive"];
    static readonly string[] Variants = ["default", "maximum", "too-aggressive"];
    static readonly TimeSpan PreparationLimit = TimeSpan.FromHours(1);
    // The launcher owns the trial timeout; host barriers must not time out first.
    static readonly TimeSpan CaptureLimit = TimeSpan.FromHours(8);
    readonly object gate = new();
    readonly int[] sheetOrder;
    readonly Dictionary<int, Sheet> sheets;
    readonly string output;
    readonly SemaphoreSlim comparers;
    readonly Dictionary<string, int> completedSheets = Sides.ToDictionary(side => side, _ => 0);
    readonly Profile[] profiles = Variants.Select(name => new Profile(name)).ToArray();
    long evidenceBytes;
    double transferReceiveMilliseconds;
    int refinementCalls;

    sealed class Profile(string name)
    {
        public readonly string Name = name;
        public int Tiles;
        public int HashEqual;
        public long ImageBytes;
        public double CompareMilliseconds;
        public readonly List<object> Differences = [];
    }

    sealed record Tile(string Id, string Range, int Width, int Height, string Hash);

    sealed class Leaf(Dictionary<string, Tile> metadata)
    {
        public readonly Dictionary<string, Tile> Metadata = metadata;
        public readonly Dictionary<string, byte[]> Images = [];
        public readonly TaskCompletionSource<bool> Compared = New<bool>();
        public bool Comparing;
    }

    sealed class Refinement(string parent)
    {
        public readonly string Parent = parent;
        public readonly Dictionary<string, Tile[]> Offers = [];
        public readonly TaskCompletionSource<JsonObject> Ready = New<JsonObject>();
    }

    sealed class Sheet(int index)
    {
        public readonly int Index = index;
        public readonly Dictionary<string, JsonObject> Geometries = [];
        public readonly TaskCompletionSource<JsonObject> GeometryReady = New<JsonObject>();
        public int[][]? Rows;
        public readonly Dictionary<string, Tile[]> RoundOffers = [];
        public readonly TaskCompletionSource<JsonObject> RoundReady = New<JsonObject>();
        public readonly Dictionary<string, Leaf> Leaves = [];
        public Refinement? Refinement;
        public readonly HashSet<string> DoneSides = [];
        public readonly TaskCompletionSource<bool> Done = New<bool>();
    }

    static TaskCompletionSource<T> New<T>() => new(TaskCreationOptions.RunContinuationsAsynchronously);

    public Quartet(int[] indices, string output, SemaphoreSlim comparers)
    {
        if (indices.Length == 0 || indices.Distinct().Count() != indices.Length)
            throw new ArgumentException("Expected distinct worksheet indices.", nameof(indices));
        sheetOrder = indices;
        sheets = indices.ToDictionary(index => index, index => new Sheet(index));
        this.output = output;
        this.comparers = comparers;
    }

    static void CheckSide(string side)
    {
        if (!Sides.Contains(side)) throw new InvalidOperationException("Unknown quartet client.");
    }

    Sheet Selected(int index)
    {
        if (!sheets.TryGetValue(index, out var sheet))
            throw new InvalidOperationException("Unexpected worksheet index.");
        return sheet;
    }

    static int[][] Rows(JsonObject data)
    {
        var rows = data["tile_rows"]!.AsArray().Select(item =>
            item!.AsArray().Select(value => (int)value!).ToArray()).ToArray();
        if (rows.Length == 0 || rows.Length > 4096 || rows.Any(row =>
            row.Length != 2 || row[0] < 0 || row[0] >= row[1] || row[1] > 1048576))
            throw new InvalidOperationException("Invalid tile row plan.");
        for (int i = 1; i < rows.Length; i++)
            if (rows[i - 1][1] > rows[i][0])
                throw new InvalidOperationException("Overlapping tile rows.");
        return rows;
    }

    public async Task<JsonObject> Geometry(string side, JsonObject data)
    {
        CheckSide(side);
        Task<JsonObject> task;
        lock (gate)
        {
            var sheet = Selected((int)data["sheet_index"]!);
            if (sheet.Done.Task.IsCompleted || sheet.RoundOffers.Count > 0 ||
                !sheet.Geometries.TryAdd(side, data.DeepClone().AsObject()))
                throw new InvalidOperationException("Duplicate or late geometry.");
            int end = (int)data["end_column"]!, max = (int)data["max_end_column"]!;
            if (end < 0 || end > max || max >= 16384) throw new InvalidOperationException("Invalid column boundary.");
            Rows(data);
            task = sheet.GeometryReady.Task;
            if (sheet.Geometries.Count == Sides.Length)
            {
                var plans = sheet.Geometries.Values.Select(Rows).ToArray();
                var cuts = plans.SelectMany(rows => rows).SelectMany(row => row).Distinct().Order().ToArray();
                var common = new List<int[]>();
                for (int i = 0; i + 1 < cuts.Length; i++)
                {
                    var covered = plans.Select(rows => rows.Any(row =>
                        row[0] <= cuts[i] && cuts[i + 1] <= row[1])).ToArray();
                    if (covered.Any(value => value != covered[0]))
                        throw new InvalidOperationException("Quartet row coverage differs.");
                    if (covered[0]) common.Add([cuts[i], cuts[i + 1]]);
                }
                if (common.Count == 0 || common.Count > 4096)
                    throw new InvalidOperationException("Invalid shared tile plan.");
                int maxColumn = sheet.Geometries.Values.Max(value => (int)value["max_end_column"]!);
                if (sheet.Geometries.Values.Any(value => (int)value["max_end_column"]! != maxColumn))
                    throw new InvalidOperationException("Quartet column limits differ.");
                sheet.Rows = common.ToArray();
                sheet.GeometryReady.SetResult(new JsonObject {
                    ["tile_rows"] = System.Text.Json.JsonSerializer.SerializeToNode(common),
                    ["capture_right_column"] = sheet.Geometries.Values.Max(value => (int)value["end_column"]!)
                });
            }
        }
        return await task.WaitAsync(PreparationLimit);
    }

    public bool Prepared
    {
        get { lock (gate) return sheets.Values.All(sheet => sheet.GeometryReady.Task.IsCompletedSuccessfully); }
    }

    static Tile ReadTile(JsonNode? node, string id)
    {
        var item = node!.AsObject();
        string range = (string)item["range"]!, hash = (string)item["sha256"]!;
        int width = (int)item["width"]!, height = (int)item["height"]!;
        if (range.Length > 80 || !Regex.IsMatch(range, @"^[A-Z]+[1-9][0-9]*:[A-Z]+[1-9][0-9]*$", RegexOptions.CultureInvariant) ||
            width <= 0 || height <= 0 || width > 30000 || height > 30000 ||
            (long)width * height > 32_000_000 ||
            hash.Length != 64 || hash.Any(value => !Uri.IsHexDigit(value)))
            throw new InvalidOperationException("Invalid tile metadata.");
        Rectangle(range);
        return new Tile(id, range, width, height, hash.ToLowerInvariant());
    }

    static (int r0, int c0, int r1, int c1) Rectangle(string address)
    {
        var match = Regex.Match(address, @"^([A-Z]+)([1-9][0-9]*):([A-Z]+)([1-9][0-9]*)$", RegexOptions.CultureInvariant);
        if (!match.Success) throw new InvalidOperationException("Invalid logical range.");
        static int Column(string letters)
        {
            int result = 0;
            foreach (char letter in letters) result = checked(result * 26 + letter - 'A' + 1);
            return result;
        }
        var rect = (r0: int.Parse(match.Groups[2].Value), c0: Column(match.Groups[1].Value),
            r1: int.Parse(match.Groups[4].Value), c1: Column(match.Groups[3].Value));
        if (rect.r0 > rect.r1 || rect.c0 > rect.c1 || rect.r1 > 1048576 || rect.c1 > 16384)
            throw new InvalidOperationException("Logical range exceeds worksheet bounds.");
        return rect;
    }

    static void ValidatePartition(Tile parent, Tile[] children)
    {
        var outer = Rectangle(parent.Range);
        var pieces = children.Select(child => Rectangle(child.Range)).ToArray();
        long area = 0;
        for (int i = 0; i < pieces.Length; i++)
        {
            var rect = pieces[i];
            if (rect.r0 < outer.r0 || rect.c0 < outer.c0 ||
                rect.r1 > outer.r1 || rect.c1 > outer.c1)
                throw new InvalidOperationException("Refinement child falls outside its parent.");
            area += (long)(rect.r1 - rect.r0 + 1) * (rect.c1 - rect.c0 + 1);
            for (int j = 0; j < i; j++)
            {
                var other = pieces[j];
                if (rect.r0 <= other.r1 && other.r0 <= rect.r1 &&
                    rect.c0 <= other.c1 && other.c0 <= rect.c1)
                    throw new InvalidOperationException("Refinement children overlap.");
            }
        }
        if (area != (long)(outer.r1 - outer.r0 + 1) * (outer.c1 - outer.c0 + 1))
            throw new InvalidOperationException("Refinement children do not cover their parent.");
    }

    static bool Same(Tile left, Tile right) => left.Hash == right.Hash &&
        left.Width == right.Width && left.Height == right.Height;

    static JsonObject Divergent(IEnumerable<string> ids) => new() {
        ["divergent"] = System.Text.Json.JsonSerializer.SerializeToNode(ids.ToArray())
    };

    string[] CompareOffers(Sheet sheet, Dictionary<string, Tile[]> offers)
    {
        var original = offers["original"];
        if (original.Length == 0 || original.Length > 4096 ||
            offers.Values.Any(value => value.Length != original.Length))
            throw new InvalidOperationException("Quartet tile counts differ.");
        foreach (var side in Variants)
        {
            var variant = offers[side];
            var profile = profiles[Array.IndexOf(Variants, side)];
            profile.Tiles += variant.Length;
            for (int i = 0; i < original.Length; i++)
            {
                if (original[i].Id != variant[i].Id || original[i].Range != variant[i].Range)
                    throw new InvalidOperationException("Quartet logical ranges differ.");
                if (Same(original[i], variant[i])) profile.HashEqual++;
            }
        }
        var divergent = new List<string>();
        for (int i = 0; i < original.Length; i++)
        {
            if (Variants.All(side => Same(original[i], offers[side][i]))) continue;
            string id = original[i].Id;
            if (!sheet.Leaves.TryAdd(id, new Leaf(Sides.ToDictionary(side => side, side => offers[side][i]))))
                throw new InvalidOperationException("Duplicate divergent leaf.");
            divergent.Add(id);
        }
        return divergent.ToArray();
    }

    public async Task<JsonObject> Round(string side, JsonObject data)
    {
        CheckSide(side);
        Task<JsonObject> task;
        lock (gate)
        {
            var sheet = Selected((int)data["sheet_index"]!);
            if (!sheet.GeometryReady.Task.IsCompletedSuccessfully || sheet.DoneSides.Count > 0)
                throw new InvalidOperationException("Round before geometry or after completion.");
            var tiles = data["tiles"]!.AsArray();
            if (tiles.Count != sheet.Rows!.Length) throw new InvalidOperationException("Initial tile count differs from plan.");
            var parsed = tiles.Select((node, i) => {
                if ((int)node!["tile_index"]! != i) throw new InvalidOperationException("Invalid initial tile index.");
                return ReadTile(node, $"t{i}");
            }).ToArray();
            if (!sheet.RoundOffers.TryAdd(side, parsed))
                throw new InvalidOperationException("Duplicate initial digest round.");
            task = sheet.RoundReady.Task;
            if (sheet.RoundOffers.Count == Sides.Length)
                sheet.RoundReady.SetResult(new JsonObject {
                    ["divergent"] = System.Text.Json.JsonSerializer.SerializeToNode(
                        CompareOffers(sheet, sheet.RoundOffers).Select(id => int.Parse(id[1..])).ToArray())
                });
        }
        return await task.WaitAsync(CaptureLimit);
    }

    public async Task<JsonObject> Refine(string side, JsonObject data)
    {
        CheckSide(side);
        Task<JsonObject> task;
        lock (gate)
        {
            var sheet = Selected((int)data["sheet_index"]!);
            string parent = (string)data["parent_id"]!;
            if (!sheet.RoundReady.Task.IsCompletedSuccessfully ||
                !sheet.Leaves.TryGetValue(parent, out var leaf) || leaf.Comparing ||
                leaf.Images.Count > 0 || leaf.Compared.Task.IsCompleted || sheet.DoneSides.Count > 0)
                throw new InvalidOperationException("Unexpected refinement parent.");
            var parts = data["parts"]!.AsArray();
            if (parts.Count is < 1 or > 4) throw new InvalidOperationException("Refinement needs one to four parts.");
            var parsed = parts.Select(node => {
                string id = (string)node!["id"]!;
                if (!id.StartsWith(parent + ".", StringComparison.Ordinal) ||
                    id.Length != parent.Length + 2 || id[^1] is < '0' or > '3')
                    throw new InvalidOperationException("Invalid refinement child ID.");
                return ReadTile(node, id);
            }).ToArray();
            if (parsed.Select(item => item.Id).Distinct().Count() != parsed.Length)
                throw new InvalidOperationException("Duplicate refinement child ID.");
            ValidatePartition(leaf.Metadata[side], parsed);
            sheet.Refinement ??= new Refinement(parent);
            var pending = sheet.Refinement;
            if (pending.Parent != parent || !pending.Offers.TryAdd(side, parsed))
                throw new InvalidOperationException("Refinement calls are out of order.");
            task = pending.Ready.Task;
            if (pending.Offers.Count == Sides.Length)
            {
                var divergent = CompareOffers(sheet, pending.Offers);
                sheet.Leaves.Remove(parent);
                sheet.Refinement = null;
                refinementCalls++;
                pending.Ready.SetResult(Divergent(divergent));
            }
        }
        return await task.WaitAsync(CaptureLimit);
    }

    public async Task Evidence(string side, int sheetIndex, string leafId, string headerHash, byte[] png)
    {
        CheckSide(side);
        Task<bool> task;
        byte[][]? images = null;
        Leaf leaf;
        lock (gate)
        {
            var sheet = Selected(sheetIndex);
            if (!sheet.RoundReady.Task.IsCompletedSuccessfully ||
                !sheet.Leaves.TryGetValue(leafId, out leaf!) || leaf.Comparing ||
                sheet.DoneSides.Count > 0)
                throw new InvalidOperationException("Unrequested evidence image.");
            var metadata = leaf.Metadata[side];
            if (png.Length < 24 || png.Length > 160 * 1024 * 1024 ||
                !png.AsSpan(0, 8).SequenceEqual(new byte[] { 137, 80, 78, 71, 13, 10, 26, 10 }) ||
                BinaryPrimitives.ReadInt32BigEndian(png.AsSpan(16, 4)) != metadata.Width ||
                BinaryPrimitives.ReadInt32BigEndian(png.AsSpan(20, 4)) != metadata.Height ||
                Convert.ToHexString(SHA256.HashData(png)).ToLowerInvariant() != metadata.Hash ||
                !string.Equals(headerHash, metadata.Hash, StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("Evidence does not match offered image digest and dimensions.");
            if (!leaf.Images.TryAdd(side, png)) throw new InvalidOperationException("Duplicate evidence image.");
            evidenceBytes += png.Length;
            task = leaf.Compared.Task;
            if (leaf.Images.Count == Sides.Length)
            {
                leaf.Comparing = true;
                images = Sides.Select(name => leaf.Images[name]).ToArray();
            }
        }
        if (images is not null)
        {
            await comparers.WaitAsync();
            try
            {
                var timer = Stopwatch.StartNew();
                var outcomes = await Task.Run(() => Variants.Select((profile, i) =>
                    Pixels.Compare(images[0], images[i + 1], Path.Combine(output, profile,
                        $"s{sheetIndex}-{leafId}"))).ToArray());
                lock (gate)
                {
                    for (int i = 0; i < Variants.Length; i++)
                    {
                        var profile = profiles[i];
                        profile.ImageBytes += images[0].Length + images[i + 1].Length;
                        profile.CompareMilliseconds += timer.Elapsed.TotalMilliseconds / Variants.Length;
                        if (!outcomes[i].Equal) profile.Differences.Add(new {
                            sheet_index = sheetIndex,
                            tile_id = leafId,
                            tile_index = int.Parse(leafId.Split('.')[0][1..]),
                            range = leaf.Metadata[Variants[i]].Range,
                            difference = outcomes[i], evidence = $"s{sheetIndex}-{leafId}"
                        });
                    }
                    leaf.Images.Clear();
                    leaf.Compared.SetResult(true);
                }
            }
            catch (Exception error) { leaf.Compared.TrySetException(error); throw; }
            finally { comparers.Release(); }
        }
        await task.WaitAsync(CaptureLimit);
    }

    public async Task SheetDone(string side, int sheetIndex)
    {
        CheckSide(side);
        Task task;
        lock (gate)
        {
            var sheet = Selected(sheetIndex);
            if (!sheet.RoundReady.Task.IsCompletedSuccessfully || sheet.Refinement is not null ||
                sheet.Leaves.Values.Any(leaf => !leaf.Compared.Task.IsCompletedSuccessfully) ||
                completedSheets[side] >= sheets.Count || sheetOrder[completedSheets[side]] != sheetIndex ||
                !sheet.DoneSides.Add(side))
                throw new InvalidOperationException("Worksheet comparisons are incomplete or out of order.");
            completedSheets[side]++;
            task = sheet.Done.Task;
            if (sheet.DoneSides.Count == Sides.Length) sheet.Done.SetResult(true);
        }
        await task.WaitAsync(CaptureLimit);
    }

    public bool ClientComplete(string side)
    {
        lock (gate) return completedSheets[side] == sheets.Count;
    }

    public bool Complete
    {
        get { lock (gate) return sheets.Values.All(sheet => sheet.Done.Task.IsCompletedSuccessfully); }
    }

    public void RecordTransferReceive(double milliseconds)
    {
        lock (gate) transferReceiveMilliseconds += milliseconds;
    }

    public void Fail(Exception error)
    {
        lock (gate)
        {
            foreach (var sheet in sheets.Values)
            {
                sheet.GeometryReady.TrySetException(error);
                sheet.RoundReady.TrySetException(error);
                sheet.Refinement?.Ready.TrySetException(error);
                foreach (var leaf in sheet.Leaves.Values) leaf.Compared.TrySetException(error);
                sheet.Done.TrySetException(error);
            }
        }
    }

    public object Snapshot()
    {
        lock (gate) return new {
            complete = sheets.Values.All(sheet => sheet.Done.Task.IsCompletedSuccessfully),
            prepared = sheets.Values.All(sheet => sheet.GeometryReady.Task.IsCompletedSuccessfully),
            refinement_calls = refinementCalls,
            evidence_bytes = evidenceBytes,
            transfer_receive_ms = transferReceiveMilliseconds,
            comparisons = profiles.Select((profile, index) => new {
                profile = profile.Name,
                result = new {
                    index,
                    profile.Tiles,
                    profile.HashEqual,
                    profile.ImageBytes,
                    profile.CompareMilliseconds,
                    Differences = profile.Differences.ToArray(),
                    Complete = sheets.Values.All(sheet => sheet.Done.Task.IsCompletedSuccessfully)
                }
            }).ToArray()
        };
    }
}
