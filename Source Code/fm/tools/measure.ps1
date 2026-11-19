# Measures start time (launch -> first visible top-level window) and memory of a GUI exe, on a never-shown desktop,
# so nothing appears on the owner's screen. Runs N times, prints one line per run and the median.
# Usage: powershell -File tools/measure.ps1 -Exe <full path> [-ArgLine "..."] [-Runs 5] [-Settle 3] [-Desktop Default]
# -Desktop Default: the real desktop (give --offscreen in -ArgLine so the window stays off every monitor). Needed for
# the true memory: on a never-shown desktop DirectComposition falls back to WARP (about 8 MB more).
param([string]$Exe, [string]$ArgLine = "", [int]$Runs = 5, [int]$Settle = 3, [string]$Desktop = "wfiles_measure")
Add-Type -TypeDefinition @"
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
public static class Meas {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO { public int cb; public string lpReserved; public string lpDesktop; public string lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2; public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError; }
    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }
    [StructLayout(LayoutKind.Sequential)]
    struct PMC { public int cb, PageFaultCount; public UIntPtr Peak, WorkingSet, QPeakPaged, QPaged, QPeakNP, QNP, Pagefile, PeakPagefile, Private; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateDesktop(string name, IntPtr dev, IntPtr dm, int flags, uint access, IntPtr sa);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags,
        IntPtr env, string dir, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr h, uint code);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("psapi.dll")] static extern bool GetProcessMemoryInfo(IntPtr h, out PMC c, int cb);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumDesktopWindows(IntPtr d, EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out int pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    static int target; static bool found;
    static bool HasWindow(IntPtr desk, int pid) {
        target = pid; found = false;
        EnumDesktopWindows(desk, (h, l) => { int p; GetWindowThreadProcessId(h, out p); if (p == target && IsWindowVisible(h)) { found = true; return false; } return true; }, IntPtr.Zero);
        return found;
    }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr OpenDesktop(string name, int flags, bool inherit, uint access);
    public static string Run(string exe, string args, int settleSec, string deskName) {
        IntPtr desk = deskName == "Default" ? OpenDesktop("Default", 0, false, 0x0041u) : CreateDesktop(deskName, IntPtr.Zero, IntPtr.Zero, 0, 0x10000000u, IntPtr.Zero);
        if (desk == IntPtr.Zero) return "error=CreateDesktop " + Marshal.GetLastWin32Error();
        var si = new STARTUPINFO(); si.cb = Marshal.SizeOf(si); si.lpDesktop = "WinSta0\\" + deskName;
        si.dwFlags = 1; si.wShowWindow = 4;
        PROCESS_INFORMATION pi;
        var cmd = new StringBuilder("\"" + exe + "\" " + args);
        var sw = Stopwatch.StartNew();
        if (!CreateProcess(null, cmd, IntPtr.Zero, IntPtr.Zero, false, 0, IntPtr.Zero, null, ref si, out pi))
            return "error=CreateProcess " + Marshal.GetLastWin32Error();
        double ms = -1;
        while (sw.ElapsedMilliseconds < 15000) {
            if (HasWindow(desk, pi.dwProcessId)) { ms = sw.Elapsed.TotalMilliseconds; break; }
            System.Threading.Thread.Sleep(1);
        }
        System.Threading.Thread.Sleep(settleSec * 1000);
        PMC c; GetProcessMemoryInfo(pi.hProcess, out c, Marshal.SizeOf(typeof(PMC)));
        TerminateProcess(pi.hProcess, 0); CloseHandle(pi.hThread); CloseHandle(pi.hProcess);
        return string.Format("window_ms={0:F0} ws_mb={1:F1} private_mb={2:F1}", ms, (ulong)c.WorkingSet / 1048576.0, (ulong)c.Private / 1048576.0);
    }
}
"@
$res = @()
for ($i = 1; $i -le $Runs; $i++) {
    $r = [Meas]::Run($Exe, $ArgLine, $Settle, $Desktop); "run $i : $r"
    if ($r -match "window_ms=(\S+) ws_mb=(\S+) private_mb=(\S+)") { $res += ,@([double]$matches[1], [double]$matches[2], [double]$matches[3]) }
    Start-Sleep -Milliseconds 500
}
if ($res.Count) {
    $med = { param($k) ($res | ForEach-Object { $_[$k] } | Sort-Object)[[int][math]::Floor($res.Count / 2)] }
    "median: window_ms=$(& $med 0) ws_mb=$(& $med 1) private_mb=$(& $med 2)"
}
