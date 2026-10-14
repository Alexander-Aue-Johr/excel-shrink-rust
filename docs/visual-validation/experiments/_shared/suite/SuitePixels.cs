using System;
using System.IO;
using System.Text;
using System.Security.Cryptography;
using System.Globalization;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

// Optional offline PNG verification for the comparison-suite worker.
// Compiled only AFTER the timed case, never by the suite controller.
// Windows GDI+ (System.Drawing.dll), not WindowsBase/PresentationCore/WPF.
// GDI+ returns 32bpp ARGB pixels in BGRA byte order on Windows.
public sealed class VVPixelDigest {
    public string Algorithm {get;set;}
    public string Sha256 {get;set;}
    public int Width {get;set;}
    public int Height {get;set;}
}
public static class VVSuitePixels {
    static uint BE(byte[] a,int p){return ((uint)a[p]<<24)|((uint)a[p+1]<<16)|((uint)a[p+2]<<8)|a[p+3];}
    public static VVPixelDigest Read(string path,long maxPixels) {
        using(var stream=File.OpenRead(path))return ReadStream(stream,maxPixels);
    }
    // Caller owns this stream; keep it open throughout GDI+ decode and LockBits.
    public static VVPixelDigest ReadStream(Stream stream,long maxPixels) {
        if(stream==null || !stream.CanRead || !stream.CanSeek)throw new ArgumentException("A seekable input stream is required.");
        stream.Position=0;
        var h=new byte[24];int got=0,n;
        while(got<h.Length&&(n=stream.Read(h,got,h.Length-got))>0)got+=n;
        var sig=new byte[]{137,80,78,71,13,10,26,10};
        if(got!=24)throw new InvalidDataException("Truncated PNG header");
        for(int i=0;i<8;i++)if(h[i]!=sig[i])throw new InvalidDataException("Not PNG");
        if(Encoding.ASCII.GetString(h,12,4)!="IHDR")throw new InvalidDataException("Missing IHDR");
        uint width=BE(h,16),height=BE(h,20);
        if(width==0||height==0||width>int.MaxValue/4||height>int.MaxValue||
           maxPixels<=0||(ulong)width*height>(ulong)maxPixels)
            throw new InvalidDataException("Offline pixel verification budget exceeded");
        stream.Position=0;
        using(var bitmap=new Bitmap(stream)) {
            if(bitmap.Width!=(int)width||bitmap.Height!=(int)height)
                throw new InvalidDataException("PNG dimensions changed during GDI+ decode");
            var rect=new Rectangle(0,0,(int)width,(int)height);
            BitmapData data=null;
            try {
                data=bitmap.LockBits(rect,ImageLockMode.ReadOnly,PixelFormat.Format32bppArgb);
                int rowBytes=checked((int)width*4);
                if(Math.Abs((long)data.Stride)<rowBytes)
                    throw new InvalidDataException("Unexpected GDI+ row stride");
                var row=new byte[rowBytes];
                using(var sha=SHA256.Create()) {
                    // Versioned canonical pixel digest: do not confuse it with a
                    // prior WPF decoder's hash or with encoded PNG byte hashes.
                    byte[] prefix=Encoding.ASCII.GetBytes("vv-bgra32-gdiplus-v1\n"+
                        width.ToString(CultureInfo.InvariantCulture)+"x"+
                        height.ToString(CultureInfo.InvariantCulture)+"\n");
                    sha.TransformBlock(prefix,0,prefix.Length,prefix,0);
                    for(int y=0;y<(int)height;y++) {
                        var scanLine=IntPtr.Add(data.Scan0,checked(y*data.Stride));
                        Marshal.Copy(scanLine,row,0,rowBytes);
                        sha.TransformBlock(row,0,row.Length,row,0);
                    }
                    sha.TransformFinalBlock(new byte[0],0,0);
                    return new VVPixelDigest{Algorithm="vv-bgra32-gdiplus-v1",
                        Sha256=BitConverter.ToString(sha.Hash).Replace("-","").ToLowerInvariant(),
                        Width=(int)width,Height=(int)height};
                }
            } finally {
                if(data!=null)bitmap.UnlockBits(data);
            }
        }
    }
}
