using System;
using System.IO;
using System.IO.Pipes;
using System.Threading.Tasks;

public static class PairRelay
{
    // Keep binary image traffic off disk and separate from PowerShell diagnostics.
    public static Task Start(string name)
    {
        return Task.Run(() => {
            using (var pipe = new NamedPipeClientStream(".", name, PipeDirection.InOut))
            {
                pipe.Connect(30000);
                var output = Console.OpenStandardOutput();
                var input = Console.OpenStandardInput();
                var buffer = new byte[65536];
                while (true)
                {
                    var header = new byte[4];
                    ReadExactly(pipe, header, 4);
                    var length = BitConverter.ToInt32(header, 0);
                    if (length < 0 || length > 340 * 1024 * 1024) throw new IOException("Invalid image frame.");
                    output.Write(header, 0, 4);
                    int left = length;
                    while (left > 0)
                    {
                        int count = pipe.Read(buffer, 0, Math.Min(left, buffer.Length));
                        if (count == 0) throw new EndOfStreamException();
                        output.Write(buffer, 0, count);
                        left -= count;
                    }
                    output.Flush();
                    if (length == 0) return;
                    int ack = input.ReadByte();
                    if (ack < 0) throw new EndOfStreamException("Pixel consumer closed.");
                    pipe.WriteByte((byte)ack);
                    if (ack == 3)
                    {
                        // Row-height comparisons return the rows needing evidence.
                        var responseHeader = new byte[4];
                        ReadExactly(input, responseHeader, 4);
                        int responseLength = BitConverter.ToInt32(responseHeader, 0);
                        if (responseLength < 0 || responseLength > 16 * 1024 * 1024)
                            throw new IOException("Invalid height response.");
                        pipe.Write(responseHeader, 0, 4);
                        while (responseLength > 0)
                        {
                            int count = input.Read(buffer, 0, Math.Min(responseLength, buffer.Length));
                            if (count == 0) throw new EndOfStreamException();
                            pipe.Write(buffer, 0, count);
                            responseLength -= count;
                        }
                    }
                    pipe.Flush();
                }
            }
        });
    }
    static void ReadExactly(Stream stream, byte[] bytes, int count)
    {
        int offset = 0;
        while (offset < count)
        {
            int read = stream.Read(bytes, offset, count - offset);
            if (read == 0) throw new EndOfStreamException();
            offset += read;
        }
    }
}
