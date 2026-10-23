using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

// Decode only when the transport has not established byte-for-byte equality.
// Compare exact decoded pixels. Evidence is cropped from these same captures.
static class Pixels
{
    public sealed record Difference(bool Equal, int Width, int Height, int[]? Bounds, int Blocks);

    public static Difference Compare(byte[] left, byte[] right, string evidence)
    {
        using var ls = new MemoryStream(left);
        using var rs = new MemoryStream(right);
        using var a = new Bitmap(ls);
        using var b = new Bitmap(rs);
        if ((long)a.Width * a.Height > 32_000_000 || (long)b.Width * b.Height > 32_000_000 ||
            Math.Max(a.Width, a.Height) > 30_000 || Math.Max(b.Width, b.Height) > 30_000)
            throw new InvalidOperationException("Image exceeds the selected capture budget.");
        if (a.Size != b.Size)
        {
            Directory.CreateDirectory(Path.GetDirectoryName(evidence)!);
            File.WriteAllBytes(evidence + "-original.png", left);
            File.WriteAllBytes(evidence + "-shrunk.png", right);
            return new(false, a.Width, a.Height, null, 0);
        }
        var rect = new Rectangle(0, 0, a.Width, a.Height);
        var ap = a.LockBits(rect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        var bp = b.LockBits(rect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        int x0 = a.Width, y0 = a.Height, x1 = -1, y1 = -1;
        var blocks = new HashSet<(int, int)>();
        try
        {
            var ar = new byte[a.Width * 4];
            var br = new byte[a.Width * 4];
            for (int y = 0; y < a.Height; y++)
            {
                Marshal.Copy(IntPtr.Add(ap.Scan0, y * ap.Stride), ar, 0, ar.Length);
                Marshal.Copy(IntPtr.Add(bp.Scan0, y * bp.Stride), br, 0, br.Length);
                for (int x = 0; x < a.Width; x++)
                {
                    int p = x * 4;
                    if (ar[p] == br[p] && ar[p + 1] == br[p + 1] &&
                        ar[p + 2] == br[p + 2] && ar[p + 3] == br[p + 3]) continue;
                    x0 = Math.Min(x0, x); y0 = Math.Min(y0, y);
                    x1 = Math.Max(x1, x); y1 = Math.Max(y1, y);
                    blocks.Add((x / 256, y / 256));
                }
            }
        }
        finally { a.UnlockBits(ap); b.UnlockBits(bp); }
        if (x1 < 0) return new(true, a.Width, a.Height, null, 0);
        Directory.CreateDirectory(Path.GetDirectoryName(evidence)!);
        var crop = Rectangle.FromLTRB(Math.Max(0, x0 - 8), Math.Max(0, y0 - 8),
            Math.Min(a.Width, x1 + 9), Math.Min(a.Height, y1 + 9));
        using var ac = a.Clone(crop, PixelFormat.Format32bppArgb);
        using var bc = b.Clone(crop, PixelFormat.Format32bppArgb);
        ac.Save(evidence + "-original.png", ImageFormat.Png);
        bc.Save(evidence + "-shrunk.png", ImageFormat.Png);
        return new(false, a.Width, a.Height, [x0, y0, x1 + 1, y1 + 1], blocks.Count);
    }
}
