# Starts a program on the never-shown desktop "wfiles_hidden" (or -Desktop Default for an --offscreen window on the
# real desktop) and returns at once (prints its PID). The window is shown without activation.
# Nothing appears on the owner's screen. Stop it with `wfm gui quit`.
param([string]$Exe, [string]$ArgLine = "", [string]$Desktop = "wfiles_hidden")
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class HS {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct SI { public int cb; public string r; public string desk; public string title; public int x, y, w, h, cx, cy, fill, flags; public short show, cb2; public IntPtr r2, i, o, e; }
    [StructLayout(LayoutKind.Sequential)] struct PI { public IntPtr hp, ht; public int pid, tid; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern IntPtr CreateDesktop(string n, IntPtr d, IntPtr m, int f, uint a, IntPtr s);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool CreateProcess(string a, StringBuilder c, IntPtr pa, IntPtr ta, bool inh, uint f, IntPtr env, string dir, ref SI si, out PI pi);
    public static string Start(string exe, string args, string desk) {
        if (desk != "Default") {
            IntPtr d = CreateDesktop(desk, IntPtr.Zero, IntPtr.Zero, 0, 0x10000000u, IntPtr.Zero);
            if (d == IntPtr.Zero) return "error=CreateDesktop " + Marshal.GetLastWin32Error();
        }
        // SW_SHOWNOACTIVATE: never takes focus
        var si = new SI(); si.cb = Marshal.SizeOf(si); si.desk = @"WinSta0\" + desk; si.flags = 1; si.show = 4;
        PI pi; if (!CreateProcess(null, new StringBuilder("\"" + exe + "\" " + args), IntPtr.Zero, IntPtr.Zero, false, 0, IntPtr.Zero, null, ref si, out pi)) return "error=CreateProcess " + Marshal.GetLastWin32Error();
        return "pid=" + pi.pid;
    }
}
"@
[HS]::Start($Exe, $ArgLine, $Desktop)
