using System.Buffers.Binary;
using System.IO.Pipes;
using System.Text.Json.Nodes;

var name = "es-pair-test-" + Guid.NewGuid().ToString("N");
var sheets = new[] { 1, 3, 5 };
using var pair = new PairedCapture(name, "test", sheets);
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
        if ((int?)image["sheet_index"] != index || (int?)image["tile_index"] != 0)
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
    await Task.WhenAll(sides.Select(side => {
        var data = Data(side);
        data["heights"] = new JsonObject { ["sheet_index"] = index, ["runs"] = new JsonArray() };
        return pair.Heights(data);
    }));
    await Task.WhenAll(sides.Select(side => {
        var data = Data(side);
        data["tile_rows"] = new JsonArray(new JsonArray(0, 1));
        return pair.Geometry(data);
    }));
    await Task.WhenAll(sides.Select(side => {
        var data = Data(side);
        data["sheet_name"] = "S" + index;
        data["range"] = "A1:B1";
        data["tile_index"] = 0;
        return pair.Image(data, new byte[] { 1, 2, 3 });
    }));
    await Task.WhenAll(sides.Select(side => pair.SheetDone(Data(side))));
}
if (!pair.Complete) throw new Exception("Incomplete multi-sheet session.");
await pair.Finish(false);
await consume;
Console.WriteLine("PASS: heights, images and completion stay aligned across three worksheets.");

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
