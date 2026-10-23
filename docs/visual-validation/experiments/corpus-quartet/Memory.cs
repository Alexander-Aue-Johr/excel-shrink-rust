using System;
using System.Runtime.InteropServices;

public static class CaptureMemory
{
    [StructLayout(LayoutKind.Sequential)]
    struct Status
    {
        public uint Length, Load;
        public ulong TotalPhysical, AvailablePhysical, TotalPage, AvailablePage;
        public ulong TotalVirtual, AvailableVirtual, Extended;
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GlobalMemoryStatusEx(ref Status status);
    public static ulong Available
    {
        get
        {
            var info = new Status { Length = (uint)Marshal.SizeOf(typeof(Status)) };
            if (!GlobalMemoryStatusEx(ref info)) throw new System.ComponentModel.Win32Exception();
            return info.AvailablePhysical;
        }
    }
}
