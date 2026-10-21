using System.Buffers.Binary;
using System.IO.Pipes;
using System.Text.Json.Nodes;

// Both task panes advance together. Only one worksheet plan and one PNG pair
// are retained; the consumer acknowledges each comparison before the next tile.
sealed class PairedCapture : IDisposable
{
    readonly NamedPipeServerStream pipe;
    readonly Task connected;
    readonly object gate = new();
    readonly string runId;
    readonly int[] sheets;
    int sheetOrdinal;
    int tileIndex;
    JsonObject? firstGeometry;
    string? geometrySide;
    int? captureEndColumn;
    TaskCompletionSource<JsonNode> geometryReady = NewGeometry();
    JsonObject? firstImage;
    byte[]? firstBytes;
    string? imageSide;
    TaskCompletionSource<bool> imageReady = NewImage();
    readonly HashSet<string> completed = new();
    readonly HashSet<string> clients = new();
    bool failed;
    TaskCompletionSource<bool> sheetReady = NewImage();
    JsonObject? firstHeights;
    TaskCompletionSource<JsonNode> heightsReady = NewGeometry();

    static TaskCompletionSource<JsonNode> NewGeometry() => new(TaskCreationOptions.RunContinuationsAsynchronously);
    static TaskCompletionSource<bool> NewImage() => new(TaskCreationOptions.RunContinuationsAsynchronously);

    public PairedCapture(string name, string runId, int[] sheets)
    {
        this.runId = runId;
        this.sheets = sheets;
        pipe = new NamedPipeServerStream(name, PipeDirection.InOut, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
        connected = pipe.WaitForConnectionAsync();
    }

    public void Validate(JsonObject data)
    {
        if ((string?)data["run_id"] != runId) throw new InvalidOperationException("Capture run changed.");
        if ((string?)data["side"] is not ("original" or "shrunk")) throw new InvalidOperationException("Invalid capture side.");
    }

    public bool StartClient(string side)
    {
        if (side is not ("original" or "shrunk")) throw new InvalidOperationException("Invalid capture side.");
        lock (gate) { return clients.Add(side); }
    }

    public async Task<JsonNode> Heights(JsonObject data)
    {
        Validate(data);
        JsonObject? other;
        TaskCompletionSource<JsonNode> ready;
        lock (gate)
        {
            if (failed || sheetOrdinal >= sheets.Length || (int?)data["sheet_index"] != sheets[sheetOrdinal])
                throw new InvalidOperationException($"Unexpected paired row heights from {(string?)data["side"]}: sheet {(int?)data["sheet_index"]}; expected {(sheetOrdinal < sheets.Length ? sheets[sheetOrdinal] : -1)}; stopped={failed}.");
            ready = heightsReady;
            if (ready.Task.IsCompleted) throw new InvalidOperationException("Row heights already compared.");
            other = firstHeights;
            if (other is null) firstHeights = data;
            else if ((string?)other["side"] == (string?)data["side"])
                throw new InvalidOperationException("Duplicate row heights.");
        }
        if (other is not null)
        {
            try
            {
                var original = (string?)data["side"] == "original" ? data : other;
                var shrunk = (string?)data["side"] == "shrunk" ? data : other;
                var metadata = new JsonObject { ["kind"] = "heights", ["sheet_index"] = (int)data["sheet_index"]! };
                byte[] Encode(JsonNode node) => System.Text.Json.JsonSerializer.SerializeToUtf8Bytes(node);
                await WriteFrame(Encode(metadata), Encode(original["heights"]!), Encode(shrunk["heights"]!));
                var ack = new byte[1];
                using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(120));
                await pipe.ReadExactlyAsync(ack, timeout.Token);
                if (ack[0] != 3) throw new InvalidOperationException("Row-height comparison failed.");
                var header = new byte[4];
                await pipe.ReadExactlyAsync(header, timeout.Token);
                var length = BinaryPrimitives.ReadInt32LittleEndian(header);
                if (length < 0 || length > 16 * 1024 * 1024) throw new InvalidOperationException("Invalid height response.");
                var body = new byte[length];
                await pipe.ReadExactlyAsync(body, timeout.Token);
                ready.SetResult(JsonNode.Parse(body) ?? throw new InvalidOperationException("Missing height response."));
            }
            catch (Exception error) { ready.TrySetException(error); throw; }
        }
        return await ready.Task.WaitAsync(TimeSpan.FromSeconds(120));
    }

    async Task WriteFrame(byte[] metadata, byte[] original, byte[] shrunk)
    {
        await connected.WaitAsync(TimeSpan.FromSeconds(30));
        var header = new byte[16];
        BinaryPrimitives.WriteInt32LittleEndian(header, 12 + metadata.Length + original.Length + shrunk.Length);
        BinaryPrimitives.WriteInt32LittleEndian(header.AsSpan(4), metadata.Length);
        BinaryPrimitives.WriteInt32LittleEndian(header.AsSpan(8), original.Length);
        BinaryPrimitives.WriteInt32LittleEndian(header.AsSpan(12), shrunk.Length);
        await pipe.WriteAsync(header);
        await pipe.WriteAsync(metadata);
        await pipe.WriteAsync(original);
        await pipe.WriteAsync(shrunk);
        await pipe.FlushAsync();
    }

    public Task<JsonNode> Geometry(JsonObject data)
    {
        Validate(data);
        lock (gate)
        {
            if (failed || sheetOrdinal >= sheets.Length || (int?)data["sheet_index"] != sheets[sheetOrdinal])
                throw new InvalidOperationException("Unexpected paired worksheet.");
            var requestedEnd = ReadEndColumn(data);
            var side = (string)data["side"]!;
            if (firstGeometry is null)
            {
                firstGeometry = data;
                geometrySide = side;
                return geometryReady.Task.WaitAsync(TimeSpan.FromSeconds(120));
            }
            if (side == geometrySide) throw new InvalidOperationException("Duplicate worksheet geometry.");
            if ((int?)firstGeometry["max_end_column"] != (int?)data["max_end_column"])
                throw new InvalidOperationException("Paired capture column limits differ.");
            var endColumn = Math.Max(ReadEndColumn(firstGeometry), requestedEnd);
            var left = ReadRows(firstGeometry);
            var right = ReadRows(data);
            var cuts = left.Concat(right).SelectMany(p => p).Distinct().Order().ToArray();
            var shared = new JsonArray();
            for (int i = 0; i + 1 < cuts.Length; i++)
            {
                bool Covers(int[][] spans) => spans.Any(p => p[0] <= cuts[i] && cuts[i + 1] <= p[1]);
                if (Covers(left) != Covers(right)) throw new InvalidOperationException("Paired worksheet row coverage differs.");
                if (Covers(left)) shared.Add(new JsonArray(cuts[i], cuts[i + 1]));
            }
            if (shared.Count is 0 or > 4096) throw new InvalidOperationException("Invalid paired tile count.");
            var response = new JsonObject { ["tile_rows"] = shared, ["capture_right_column"] = endColumn };
            captureEndColumn = endColumn;
            var ready = geometryReady;
            firstGeometry = null;
            tileIndex = 0;
            ready.SetResult(response);
            return ready.Task;
        }
    }

    static int ReadEndColumn(JsonObject data)
    {
        var end = (int?)data["end_column"];
        var maximum = (int?)data["max_end_column"];
        if (end is null || maximum is null || end < 0 || end > maximum || maximum >= 16384)
            throw new InvalidOperationException("Invalid paired capture column boundary.");
        return end.Value;
    }

    static int[][] ReadRows(JsonObject data)
    {
        var rows = data["tile_rows"]?.AsArray() ?? throw new InvalidOperationException("Missing tile rows.");
        if (rows.Count is 0 or > 4096) throw new InvalidOperationException("Invalid tile count.");
        var spans = rows.Select(n => n!.AsArray().Select(v => v!.GetValue<int>()).ToArray()).ToArray();
        for (int i = 0; i < spans.Length; i++)
            if (spans[i].Length != 2 || spans[i][0] < 0 || spans[i][1] > 1048576 || spans[i][0] >= spans[i][1] ||
                (i > 0 && spans[i - 1][1] > spans[i][0])) throw new InvalidOperationException("Invalid tile rows.");
        return spans;
    }

    public async Task Image(JsonObject data, byte[] png)
    {
        Validate(data);
        TaskCompletionSource<bool> ready;
        JsonObject? other;
        byte[]? bytes;
        lock (gate)
        {
            if (failed || sheetOrdinal >= sheets.Length || (int?)data["sheet_index"] != sheets[sheetOrdinal] || (int?)data["tile_index"] != tileIndex)
                throw new InvalidOperationException("Unexpected paired image.");
            if (captureEndColumn is null) throw new InvalidOperationException("Paired image arrived before geometry.");
            data["capture_right_column"] = captureEndColumn.Value;
            ready = imageReady;
            if (firstImage is null)
            {
                firstImage = data;
                firstBytes = png;
                imageSide = (string)data["side"]!;
                other = null;
                bytes = null;
            }
            else
            {
                if ((string?)data["side"] == imageSide || (string?)data["range"] != (string?)firstImage["range"] ||
                    (string?)data["sheet_name"] != (string?)firstImage["sheet_name"])
                    throw new InvalidOperationException("Paired image identity differs.");
                other = firstImage;
                bytes = firstBytes;
                firstImage = null;
                firstBytes = null;
            }
        }
        if (other is not null)
        {
            try
            {
                var original = (string?)data["side"] == "original" ? png : bytes!;
                var shrunk = (string?)data["side"] == "shrunk" ? png : bytes!;
                var metadata = System.Text.Json.JsonSerializer.SerializeToUtf8Bytes(data);
                await WriteFrame(metadata, original, shrunk);
                var ack = new byte[1];
                using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(120));
                await pipe.ReadExactlyAsync(ack, timeout.Token);
                if (ack[0] != 1) throw new InvalidOperationException("Pixel comparison failed.");
                lock (gate) { tileIndex++; imageReady = NewImage(); }
                ready.SetResult(true);
            }
            catch (Exception error) { ready.TrySetException(error); throw; }
        }
        await ready.Task.WaitAsync(TimeSpan.FromSeconds(120));
    }

    public Task SheetDone(JsonObject data)
    {
        Validate(data);
        lock (gate)
        {
            if (failed || sheetOrdinal >= sheets.Length || (int?)data["sheet_index"] != sheets[sheetOrdinal])
                throw new InvalidOperationException("Unexpected completed worksheet.");
            var side = (string)data["side"]!;
            if (!completed.Add(side)) throw new InvalidOperationException("Duplicate worksheet completion.");
            var ready = sheetReady;
            if (completed.Count == 2)
            {
                sheetOrdinal++;
                completed.Clear();
                geometryReady = NewGeometry();
                captureEndColumn = null;
                sheetReady = NewImage();
                firstHeights = null;
                heightsReady = NewGeometry();
                ready.SetResult(true);
            }
            return ready.Task.WaitAsync(TimeSpan.FromSeconds(120));
        }
    }

    public bool Complete { get { lock (gate) { return sheetOrdinal == sheets.Length; } } }

    public async Task Finish(bool error)
    {
        lock (gate) { failed |= error; }
        if (error)
        {
            geometryReady.TrySetException(new InvalidOperationException("Paired capture stopped."));
            sheetReady.TrySetException(new InvalidOperationException("Paired capture stopped."));
            imageReady.TrySetException(new InvalidOperationException("Paired capture stopped."));
            heightsReady.TrySetException(new InvalidOperationException("Paired capture stopped."));
        }
        await connected.WaitAsync(TimeSpan.FromSeconds(30));
        await pipe.WriteAsync(new byte[4]);
        await pipe.FlushAsync();
    }
    public void Dispose() => pipe.Dispose();
}
