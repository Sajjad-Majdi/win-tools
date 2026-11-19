# Measures wfm: process start -> exit time and peak working set (RAM), 5 runs each, median.
# Every run is hidden (CreateNoWindow) and its output goes nowhere (like "> nul").
# Also reads (read only) the current working set of the running explorer.exe for comparison.
# Usage: powershell -NoProfile -File bench\measure.ps1 [-Wfm <path to wfm.exe>] [-Runs 5]
param([string]$Wfm = "$PSScriptRoot\..\target\release\wfm.exe", [int]$Runs = 5)
Add-Type -TypeDefinition @"
using System; using System.Diagnostics; using System.Runtime.InteropServices;
public static class WfmMeas {
    [StructLayout(LayoutKind.Sequential)]
    struct PMC { public int cb, PageFaultCount; public UIntPtr Peak, WorkingSet, QPeakPaged, QPaged, QPeakNP, QNP, Pagefile, PeakPagefile; }
    [DllImport("psapi.dll")] static extern bool GetProcessMemoryInfo(IntPtr h, out PMC c, int cb);
    // returns "ms peak_ws_bytes exit_code stdout_bytes"
    public static string Run(string exe, string args) {
        var psi = new ProcessStartInfo(exe, args);
        psi.UseShellExecute = false; psi.CreateNoWindow = true; psi.RedirectStandardOutput = true;
        var sw = Stopwatch.StartNew();
        var p = Process.Start(psi);
        var ms = new System.IO.MemoryStream();
        p.StandardOutput.BaseStream.CopyTo(ms);
        p.WaitForExit();
        double t = sw.Elapsed.TotalMilliseconds;
        PMC c; GetProcessMemoryInfo(p.Handle, out c, Marshal.SizeOf(typeof(PMC)));
        string r = String.Format("{0:F1} {1} {2} {3}", t, (ulong)c.Peak, p.ExitCode, ms.Length);
        p.Dispose();
        return r;
    }
}
"@
$Wfm = (Resolve-Path $Wfm).Path
function Median($xs) { $s = $xs | Sort-Object; $s[[int][math]::Floor($s.Count / 2)] }
function Bench($label, $argLine) {
    $t = @(); $m = @()
    for ($i = 1; $i -le $Runs; $i++) {
        $r = [WfmMeas]::Run($Wfm, $argLine) -split ' '
        $t += [double]$r[0]; $m += [double]$r[1] / 1MB
        "  run $i : {0} ms, peak working set {1:N1} MB, exit {2}, {3} bytes of output" -f $r[0], ([double]$r[1] / 1MB), $r[2], $r[3]
    }
    "{0}: median {1} ms, peak working set median {2:N1} MB" -f $label, (Median $t), (Median $m)
}
"wfm: $Wfm ($([math]::Round((Get-Item $Wfm).Length / 1KB)) KB)"
Bench "wfm ls C:\Windows\System32 --json > nul (start -> exit)" "ls C:\Windows\System32 --json"
$inner = @(); for ($i = 0; $i -lt $Runs; $i++) { $inner += [double]((& $Wfm ls C:\Windows\System32 --json | ConvertFrom-Json).ms) }
"time to list + sort C:\Windows\System32 inside wfm: median $(Median $inner) ms (count: $((& $Wfm ls C:\Windows\System32 --json | ConvertFrom-Json).count) items)"
Bench "wfm find setup --json (start -> exit, Everything)" "find setup --json"
$f = & $Wfm find setup --json | ConvertFrom-Json
"find 'setup': engine $($f.engine), $($f.count) results (top 100), $($f.ms) ms inside wfm"
Bench "wfm help (bare start -> exit)" "help"
$ex = Get-Process explorer -ErrorAction SilentlyContinue
foreach ($e in $ex) { "explorer.exe PID $($e.Id): working set now {0:N1} MB, private {1:N1} MB (read only, not restarted)" -f ($e.WorkingSet64 / 1MB), ($e.PrivateMemorySize64 / 1MB) }
