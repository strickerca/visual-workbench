using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
// Actual host QPC only. No process, input, transport or settings mutation.
public static class VwHeldClockR24 {
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool QueryPerformanceCounter(out long value);
    [DllImport("kernel32.dll",SetLastError=true)] private static extern bool QueryPerformanceFrequency(out long value);
    public static long[] Read(){long counter,frequency;if(!QueryPerformanceCounter(out counter)||!QueryPerformanceFrequency(out frequency)||counter<=0||frequency<=0)throw new Win32Exception();return new[]{counter,frequency};}
}
