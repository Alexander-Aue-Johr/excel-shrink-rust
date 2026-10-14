using System;
using System.IO;
using System.Text;
using System.Collections.Generic;
using System.ComponentModel;
using System.Security.Cryptography;
using System.Drawing;
using System.Drawing.Imaging;
using System.Globalization;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

// Offline comparison only. Never builds a giant worksheet bitmap, resizes,
// tolerates a pixel difference, or silently discards an inconsistent seam.
public sealed class VVStripSpec {
    public string Path {get;set;}
    public string Sha256 {get;set;}
    public int X {get;set;}
    public int Y {get;set;}
    public int Width {get;set;}
    public int Height {get;set;}
}
public sealed class VVStripDigestResult {
    public string Algorithm {get;set;}
    public string Sha256 {get;set;}
    public int Width {get;set;}
    public int Height {get;set;}
    public int OverlapRowsChecked {get;set;}
    public int Tiles {get;set;}
}
public static class VVStripDigest {
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true,ExactSpelling=true)]
    static extern SafeFileHandle CreateFileW(string path,uint access,uint share,IntPtr security,uint disposition,uint flags,IntPtr template);
    static FileStream Open(string path) {
        // The caller supplies a validated absolute path; avoid legacy MAX_PATH resolution.
        if(String.IsNullOrEmpty(path)||(!path.StartsWith(@"\\",StringComparison.Ordinal) &&
            !(path.Length>=3 && Char.IsLetter(path[0]) && path[1]==':' && path[2]=='\\')))
            throw new InvalidDataException("Expected an absolute Windows strip path");
        string p=path.Replace('/','\\');
        if(!p.StartsWith(@"\\?\",StringComparison.Ordinal))p=p.StartsWith(@"\\",StringComparison.Ordinal)?@"\\?\UNC\"+p.Substring(2):@"\\?\"+p;
        var handle=CreateFileW(p,0x80000000,1,IntPtr.Zero,3,0x08000000,IntPtr.Zero);
        if(handle.IsInvalid){handle.Dispose();throw new Win32Exception(Marshal.GetLastWin32Error(),"Cannot open strip: "+path);}
        try{return new FileStream(handle,FileAccess.Read,1024*1024,false);}catch{handle.Dispose();throw;}
    }
    static string Hex(byte[] bytes){return BitConverter.ToString(bytes).Replace("-","").ToLowerInvariant();}
    static uint BE(byte[] a,int p){return ((uint)a[p]<<24)|((uint)a[p+1]<<16)|((uint)a[p+2]<<8)|a[p+3];}
    static void CheckHeader(Stream stream,VVStripSpec spec,long maxPixels){
        byte[] h=new byte[24];int n=0,k;
        while(n<24&&(k=stream.Read(h,n,24-n))>0)n+=k;
        byte[] sig={137,80,78,71,13,10,26,10};
        if(n!=24)throw new InvalidDataException("Truncated strip header");
        for(int i=0;i<8;i++)if(h[i]!=sig[i])throw new InvalidDataException("Strip is not a PNG");
        if(Encoding.ASCII.GetString(h,12,4)!="IHDR"||BE(h,16)!=(uint)spec.Width||BE(h,20)!=(uint)spec.Height)
            throw new InvalidDataException("PNG receipt dimensions differ");
        if(spec.Width<=0||spec.Height<=0||spec.Width>30000||spec.Height>30000||maxPixels<1||(long)spec.Width*spec.Height>maxPixels)
            throw new InvalidDataException("Strip decode budget exceeded");
        stream.Position=0;
    }
    public static VVStripDigestResult Read(VVStripSpec[] input,long maxPixels) {
        if(input==null||input.Length==0)throw new InvalidDataException("No final strips");
        var strips=(VVStripSpec[])input.Clone();Array.Sort(strips,delegate(VVStripSpec a,VVStripSpec b){return a.Y.CompareTo(b.Y);});
        int width=strips[0].Width,extent=0,overlaps=0;
        if(strips[0].Y!=0)throw new InvalidDataException("Strip stream does not start at zero");
        foreach(var s in strips){
            if(s.X!=0||s.Width!=width||s.Y<0)throw new InvalidDataException("Horizontal or unequal-width layout is not canonicalized by this validator");
            if(s.Y>extent||s.Y<extent-2)throw new InvalidDataException("Gap or excessive strip overlap");
            int next=checked(s.Y+s.Height);
            if(next<=extent)throw new InvalidDataException("Duplicate or contained strip");
            extent=next;
        }
        int rowBytes=checked(width*4),written=0;
        // Keep two last canonical rows to verify duplicate seam pixels, not merely omit them.
        var tail=new Dictionary<int,byte[]>();
        using(var sha=SHA256.Create()) {
            byte[] prefix=Encoding.ASCII.GetBytes("vv-bgra32-strip-v1\n"+width.ToString(CultureInfo.InvariantCulture)+"x"+extent.ToString(CultureInfo.InvariantCulture)+"\n");
            sha.TransformBlock(prefix,0,prefix.Length,prefix,0);
            foreach(var s in strips) {
                using(var stream=Open(s.Path)) {
                    CheckHeader(stream,s,maxPixels);
                    using(var encoded=SHA256.Create()){
                        if(String.IsNullOrEmpty(s.Sha256)||!String.Equals(Hex(encoded.ComputeHash(stream)),s.Sha256,StringComparison.OrdinalIgnoreCase))
                            throw new InvalidDataException("Final strip file hash differs from its receipt");
                    }
                    stream.Position=0;
                    using(var bitmap=new Bitmap(stream)) {
                        if(bitmap.Width!=s.Width||bitmap.Height!=s.Height)throw new InvalidDataException("Decoded strip size differs");
                        BitmapData data=null;
                        try{
                            data=bitmap.LockBits(new Rectangle(0,0,s.Width,s.Height),ImageLockMode.ReadOnly,PixelFormat.Format32bppArgb);
                            if(Math.Abs((long)data.Stride)<rowBytes)throw new InvalidDataException("Invalid bitmap stride");
                            byte[] row=new byte[rowBytes];
                            for(int y=0;y<s.Height;y++){
                                Marshal.Copy(IntPtr.Add(data.Scan0,checked(y*data.Stride)),row,0,rowBytes);
                                int absolute=checked(s.Y+y);
                                if(absolute<written){
                                    byte[] before;
                                    if(!tail.TryGetValue(absolute,out before))throw new InvalidDataException("Unverifiable overlap");
                                    for(int i=0;i<rowBytes;i++)if(before[i]!=row[i])throw new InvalidDataException("Inconsistent seam pixels at image row "+absolute);
                                    overlaps++;continue;
                                }
                                if(absolute!=written)throw new InvalidDataException("Non-contiguous strip stream");
                                sha.TransformBlock(row,0,rowBytes,row,0);written++;
                                tail[absolute]=(byte[])row.Clone();tail.Remove(absolute-2);
                            }
                        }finally{if(data!=null)bitmap.UnlockBits(data);}
                    }
                }
            }
            if(written!=extent)throw new InvalidDataException("Incomplete canonical image");
            sha.TransformFinalBlock(new byte[0],0,0);
            return new VVStripDigestResult{Algorithm="vv-bgra32-strip-v1",Sha256=Hex(sha.Hash),Width=width,Height=extent,OverlapRowsChecked=overlaps,Tiles=strips.Length};
        }
    }
}
