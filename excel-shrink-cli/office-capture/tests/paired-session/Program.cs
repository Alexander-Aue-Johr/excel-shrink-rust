using System.Buffers.Binary;
using System.IO.Pipes;
using System.Text.Json.Nodes;

var name = "es-pair-test-" + Guid.NewGuid().ToString("N");
var sheets = new[] { 1, 3, 5 };
int EndColumn(int sheet) => sheet switch { 1 => 30, 3 => 40, _ => 1 };
using var pair = new PairedCapture(name, "test", sheets);
if (!pair.StartClient("original") || !pair.StartClient("shrunk") || !pair.StartClient("original"))
    throw new Exception("Task pane startup reload was rejected before capture data arrived.");
using var consumer = new NamedPipeClientStream(".", name, PipeDirection.InOut, PipeOptions.Asynchronous);
await consumer.ConnectAsync();
var consume = Task.Run(async () =>
{
    foreach (var index in sheets)
    {
        var height = await ReadFrame(consumer);
        if ((string?)height["kind"] != "heights" || (int?)height["sheet_index"] != index)
            throw new Exception("Wrong height frame order.");
        var body = System.Text.Encoding.UTF8.GetBytes("{\"rows\":[]}");
        await consumer.WriteAsync(new byte[] { 3 });
        await consumer.WriteAsync(BitConverter.GetBytes(body.Length));
        await consumer.WriteAsync(body);
        await consumer.FlushAsync();
        var image = await ReadFrame(consumer);
        if ((int?)image["sheet_index"] != index || (int?)image["tile_index"] != 0 ||
            (int?)image["capture_right_column"] != EndColumn(index))
            throw new Exception("Wrong image frame order.");
        await consumer.WriteAsync(new byte[] { 1 });
        await consumer.FlushAsync();
    }
    var end = new byte[4];
    await consumer.ReadExactlyAsync(end);
    if (BitConverter.ToInt32(end) != 0) throw new Exception("Missing end frame.");
});
foreach (var index in sheets)
{
    JsonObject Data(string side) => new() { ["run_id"] = "test", ["side"] = side, ["sheet_index"] = index };
    var sides = index == 3 ? new[] { "shrunk", "original" } : new[] { "original", "shrunk" };
    var heightTasks = new List<Task<JsonNode>>();
    foreach (var side in sides)
    {
        if (index == 1 && !pair.StartClient(side))
            throw new Exception("The other workbook's data prevented startup reload.");
        var data = Data(side);
        data["heights"] = new JsonObject { ["sheet_index"] = index, ["runs"] = new JsonArray() };
        heightTasks.Add(pair.Heights(data));
        if (pair.StartClient(side)) throw new Exception("Task pane restart after capture data was accepted.");
    }
    await Task.WhenAll(heightTasks);
    var geometry = await Task.WhenAll(sides.Select(side => {
        var data = Data(side);
        data["tile_rows"] = new JsonArray(new JsonArray(0, 1));
        data["max_end_column"] = index == 5 ? 1 : 65;
        data["end_column"] = index switch {
            1 => side == "original" ? 24 : 30,
            3 => side == "original" ? 40 : 20,
            _ => 1
        };
        return pair.Geometry(data);
    }));
    if (geometry.Any(result => (int?)result["capture_right_column"] != EndColumn(index)))
        throw new Exception("Both sides must retain the larger measured boundary.");
    await Task.WhenAll(sides.Select(side => {
        var data = Data(side);
        data["sheet_name"] = "S" + index;
        data["range"] = index switch { 1 => "A1:AE1", 3 => "A1:AO1", _ => "A1:B1" };
        data["tile_index"] = 0;
        return pair.Image(data, new byte[] { 1, 2, 3 });
    }));
    await Task.WhenAll(sides.Select(side => pair.SheetDone(Data(side))));
}
if (!pair.Complete) throw new Exception("Incomplete multi-sheet session.");
await pair.Finish(false);
await consume;
using var invalid = new PairedCapture(name + "-invalid", "test", new[] { 1 });
foreach (var (end, maximum) in new[] { (-1, 65), (66, 65), (65, 16384) })
{
    try
    {
        _ = invalid.Geometry(new JsonObject { ["run_id"] = "test", ["side"] = "original",
            ["sheet_index"] = 1, ["end_column"] = end, ["max_end_column"] = maximum });
        throw new Exception("Invalid column boundary was accepted.");
    }
    catch (InvalidOperationException) { }
}
Console.WriteLine("PASS: heights, negotiated columns and images stay aligned across three worksheets.");

static async Task<JsonObject> ReadFrame(Stream stream)
{
    var header = new byte[16];
    await stream.ReadExactlyAsync(header);
    int Size(int offset) => BinaryPrimitives.ReadInt32LittleEndian(header.AsSpan(offset));
    var metadata = new byte[Size(4)];
    await stream.ReadExactlyAsync(metadata);
    await stream.ReadExactlyAsync(new byte[Size(8) + Size(12)]);
    return JsonNode.Parse(metadata)!.AsObject();
}
