# How does the real Windows EDIT control (the class Explorer renames a file in) answer clicks and keys?
# Run hidden:  powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File tools\edit_probe.ps1 [-Auto]
# Writes %TEMP%\edit-probe.txt (-Auto: the control with SHAutoComplete, as in the Run box: %TEMP%\edit-probe-auto.txt).
# The numbers in the tests of src\bin\wfiles\fmt.rs (Field) come from this probe (Windows 11 26200, 2026-11-26).
# Probe of the real Windows EDIT control (comctl32 v6, the class Explorer's rename box uses).
# The window is created but never shown. Output: plain text facts.
param([switch]$Auto)
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.Application]::EnableVisualStyles()
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Windows.Forms;
public static class EP {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, int m, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, int m, IntPtr w, string l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, int m, IntPtr w, StringBuilder l);
    [DllImport("user32.dll")] public static extern bool SetKeyboardState(byte[] s);
    [DllImport("user32.dll")] public static extern bool GetKeyboardState(byte[] s);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("shlwapi.dll")] public static extern int SHAutoComplete(IntPtr h, uint flags);
    public static IntPtr H;
    public static Form F; public static TextBox T;
    public static void Make(bool auto) {
        F = new Form(); F.ShowInTaskbar = false; F.Width = 900; F.Height = 200;
        T = new TextBox(); T.Width = 800; T.Left = 10; T.Top = 10; T.Font = new System.Drawing.Font("Segoe UI", 9f);
        F.Controls.Add(T);
        IntPtr fh = F.Handle; H = T.Handle;
        if (auto) SHAutoComplete(H, 0x20000000u | 0x80000000u); // AUTOSUGGEST_FORCE_OFF | AUTOAPPEND_FORCE_OFF
    }
    public static string Cls() { var s = new StringBuilder(64); GetClassName(H, s, 64); return s.ToString(); }
    public static void Mods(bool ctrl, bool shift) {
        var k = new byte[256]; GetKeyboardState(k);
        k[0x11] = (byte)(ctrl ? 0x80 : 0); k[0x10] = (byte)(shift ? 0x80 : 0);
        k[0xA2] = k[0x11]; k[0xA0] = k[0x10];
        SetKeyboardState(k);
    }
    public static void Set(string s) { SendMessage(H, 0x000C, IntPtr.Zero, s); }
    public static string Get() { var sb = new StringBuilder(1024); SendMessage(H, 0x000D, (IntPtr)1024, sb); return sb.ToString(); }
    public static void Sel(int a, int b) { SendMessage(H, 0x00B1, (IntPtr)a, (IntPtr)b); }
    public static string GetSel() { long r = (long)SendMessage(H, 0x00B0, IntPtr.Zero, IntPtr.Zero); return (r & 0xFFFF) + ".." + ((r >> 16) & 0xFFFF); }
    public static int SelA() { long r = (long)SendMessage(H, 0x00B0, IntPtr.Zero, IntPtr.Zero); return (int)(r & 0xFFFF); }
    public static int SelB() { long r = (long)SendMessage(H, 0x00B0, IntPtr.Zero, IntPtr.Zero); return (int)((r >> 16) & 0xFFFF); }
    public static void Key(int vk, bool ctrl, bool shift) { Mods(ctrl, shift); SendMessage(H, 0x0100, (IntPtr)vk, IntPtr.Zero); SendMessage(H, 0x0101, (IntPtr)vk, IntPtr.Zero); Mods(false, false); }
    public static void KeyChar(int vk, int ch, bool ctrl, bool shift) { Mods(ctrl, shift); SendMessage(H, 0x0100, (IntPtr)vk, IntPtr.Zero); SendMessage(H, 0x0102, (IntPtr)ch, IntPtr.Zero); SendMessage(H, 0x0101, (IntPtr)vk, IntPtr.Zero); Mods(false, false); }
    public static void Type(string s) { foreach (char c in s) SendMessage(H, 0x0102, (IntPtr)c, IntPtr.Zero); }
    public static int PosX(int i) { long r = (long)SendMessage(H, 0x00D6, (IntPtr)i, IntPtr.Zero); return (short)(r & 0xFFFF); }
    public static int CharFromX(int x) { long r = (long)SendMessage(H, 0x00D7, IntPtr.Zero, (IntPtr)((8 << 16) | (x & 0xFFFF))); return (int)(r & 0xFFFF); }
    static IntPtr LP(int x) { return (IntPtr)((8 << 16) | (x & 0xFFFF)); }
    public static void Click(int x, bool shift) { Mods(false, shift); SendMessage(H, 0x0201, (IntPtr)(1 | (shift ? 4 : 0)), LP(x)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x)); Mods(false, false); }
    public static void Dbl(int x) { SendMessage(H, 0x0201, (IntPtr)1, LP(x)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x)); SendMessage(H, 0x0203, (IntPtr)1, LP(x)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x)); }
    public static void DblDrag(int x, int x2) { SendMessage(H, 0x0201, (IntPtr)1, LP(x)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x)); SendMessage(H, 0x0203, (IntPtr)1, LP(x)); SendMessage(H, 0x0200, (IntPtr)1, LP(x2)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x2)); }
    public static void Drag(int x, int x2) { SendMessage(H, 0x0201, (IntPtr)1, LP(x)); SendMessage(H, 0x0200, (IntPtr)1, LP(x2)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x2)); }
    public static long Msg(int m, long w, long l) { return (long)SendMessage(H, m, (IntPtr)w, (IntPtr)l); }
}
"@
[EP]::Make($Auto.IsPresent)
$o = New-Object System.Collections.Generic.List[string]
function Say($s) { $o.Add($s) }
function Vis($s) { ($s.ToCharArray() | ForEach-Object { if ([int]$_ -lt 32 -or [int]$_ -eq 127) { '<' + [int]$_ + '>' } else { $_ } }) -join '' }
Say ("class=" + [EP]::Cls() + " cuebanner(v6)=" + [EP]::Msg(0x1501, 0, 0) + " wordbreakproc=" + [EP]::Msg(0x00D1, 0, 0) + " auto=" + $Auto.IsPresent)

$texts = @(
  'New Text Document.txt',
  'my-file_name v2.final.tar.gz',
  'a(b)c [d] {e}+f=g,h;i&j@k#l',
  'one  two   three',
  "it's 50% off!~x",
  ([string][char]0x06AF + [char]0x0632 + [char]0x0627 + [char]0x0631 + [char]0x0634 + ' ' + [char]0x0646 + [char]0x0647 + [char]0x0627 + [char]0x06CC + [char]0x06CC + '.docx'),
  ('ab ' + [char]0x0633 + [char]0x0644 + [char]0x0627 + [char]0x0645 + ' cd.txt')
)
foreach ($t in $texts) {
  [EP]::Set($t)
  $n = $t.Length
  Say ("TEXT [" + $t + "] len=" + $n)
  $r = @(); for ($i = 0; $i -le $n; $i++) { [EP]::Sel($i, $i); [EP]::Key(0x27, $true, $false); $r += ("" + $i + ">" + [EP]::SelB()) }
  Say ("  ctrl+right: " + ($r -join ' '))
  $r = @(); for ($i = 0; $i -le $n; $i++) { [EP]::Sel($i, $i); [EP]::Key(0x25, $true, $false); $r += ("" + $i + ">" + [EP]::SelB()) }
  Say ("  ctrl+left:  " + ($r -join ' '))
  $r = @(); for ($i = 0; $i -lt $n; $i++) {
    $x0 = [EP]::PosX($i); $x1 = [EP]::PosX($i + 1); if ($i -eq $n - 1) { $x1 = $x0 + 4 }
    $x = [int](($x0 + $x1) / 2); if ($x1 -lt $x0) { $x = [int](($x0 + $x1) / 2) }
    [EP]::Sel(0, 0); [EP]::Dbl($x); $r += ("" + $i + ":" + [EP]::GetSel())
  }
  Say ("  dblclick at char: " + ($r -join ' '))
  $r = @(); for ($i = 0; $i -le $n; $i++) { [EP]::Set($t); [EP]::Sel($i, $i); [EP]::KeyChar(0x08, 0x7F, $true, $false); $r += ("" + $i + ">[" + (Vis ([EP]::Get())) + "]@" + [EP]::SelB()) }
  Say ("  ctrl+backspace: " + ($r -join ' '))
  $r = @(); for ($i = 0; $i -le $n; $i++) { [EP]::Set($t); [EP]::Sel($i, $i); [EP]::Key(0x2E, $true, $false); $r += ("" + $i + ">[" + (Vis ([EP]::Get())) + "]@" + [EP]::SelB()) }
  Say ("  ctrl+delete: " + ($r -join ' '))
}
# plain arrows inside Persian (logical or visual?)
$t = $texts[6]; [EP]::Set($t)
$r = @(); for ($i = 0; $i -le $t.Length; $i++) { [EP]::Sel($i, $i); [EP]::Key(0x25, $false, $false); $r += ("" + $i + ">" + [EP]::SelB()) }
Say ("mixed text, Left from each index: " + ($r -join ' '))
$r = @(); for ($i = 0; $i -le $t.Length; $i++) { $r += ("" + $i + "@" + [EP]::PosX($i)) }
Say ("mixed text, x of each index: " + ($r -join ' '))
$r = @(); for ($x = 0; $x -le 90; $x += 3) { [EP]::Click($x, $false); $r += ("" + $x + ">" + [EP]::SelB()) }
Say ("mixed text, click at x -> caret: " + ($r -join ' '))

# selection + arrows
[EP]::Set('New Text Document.txt')
[EP]::Sel(0, 17); [EP]::Key(0x25, $false, $false); Say ("sel 0..17 caret end, Left -> " + [EP]::GetSel())
[EP]::Sel(0, 17); [EP]::Key(0x27, $false, $false); Say ("sel 0..17 caret end, Right -> " + [EP]::GetSel())
[EP]::Sel(4, 8); [EP]::Key(0x25, $false, $false); Say ("sel 4..8, Left -> " + [EP]::GetSel())
[EP]::Sel(4, 8); [EP]::Key(0x27, $false, $false); Say ("sel 4..8, Right -> " + [EP]::GetSel())
[EP]::Sel(8, 4); [EP]::Key(0x25, $false, $false); Say ("sel 8..4 (caret at 4?), Left -> " + [EP]::GetSel())
[EP]::Sel(4, 8); [EP]::Key(0x25, $true, $false); Say ("sel 4..8, Ctrl+Left -> " + [EP]::GetSel())
[EP]::Sel(4, 8); [EP]::Key(0x27, $true, $false); Say ("sel 4..8, Ctrl+Right -> " + [EP]::GetSel())
[EP]::Sel(4, 8); [EP]::Key(0x24, $false, $false); Say ("sel 4..8, Home -> " + [EP]::GetSel())
[EP]::Sel(4, 8); [EP]::Key(0x23, $false, $false); Say ("sel 4..8, End -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x24, $false, $true); Say ("caret 6, Shift+Home -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x23, $false, $true); Say ("caret 6, Shift+End -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x25, $true, $true); Say ("caret 6, Ctrl+Shift+Left -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x27, $true, $true); Say ("caret 6, Ctrl+Shift+Right -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x26, $false, $false); Say ("caret 6, Up -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x28, $false, $false); Say ("caret 6, Down -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x26, $false, $true); Say ("caret 6, Shift+Up -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x28, $false, $true); Say ("caret 6, Shift+Down -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x24, $true, $false); Say ("caret 6, Ctrl+Home -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::Key(0x23, $true, $false); Say ("caret 6, Ctrl+End -> " + [EP]::GetSel())
[EP]::Sel(6, 6); [EP]::KeyChar(0x41, 1, $true, $false); Say ("caret 6, Ctrl+A -> " + [EP]::GetSel() + " text=[" + (Vis ([EP]::Get())) + "]")
[EP]::Sel(4, 8); [EP]::Key(0x08, $true, $false); Say ("sel 4..8, Ctrl+Backspace keydown only -> [" + (Vis ([EP]::Get())) + "] " + [EP]::GetSel())
[EP]::Set('New Text Document.txt'); [EP]::Sel(4, 8); [EP]::KeyChar(0x08, 0x7F, $true, $false); Say ("sel 4..8, Ctrl+Backspace (+char 7F) -> [" + (Vis ([EP]::Get())) + "] " + [EP]::GetSel())
[EP]::Set('New Text Document.txt'); [EP]::Sel(4, 8); [EP]::Key(0x2E, $true, $false); Say ("sel 4..8, Ctrl+Delete -> [" + (Vis ([EP]::Get())) + "] " + [EP]::GetSel())
[EP]::Set('New Text Document.txt'); [EP]::Sel(4, 8); [EP]::Key(0x2E, $false, $true); Say ("sel 4..8, Shift+Delete (cut) -> [" + (Vis ([EP]::Get())) + "] " + [EP]::GetSel())
[EP]::Set('New Text Document.txt'); [EP]::Sel(4, 4); [EP]::Key(0x2E, $false, $true); Say ("caret 4, Shift+Delete -> [" + (Vis ([EP]::Get())) + "] " + [EP]::GetSel())
[EP]::Set('New Text Document.txt'); [EP]::Sel(6, 6); [EP]::Key(0x71, $false, $false); Say ("caret 6, F2 -> [" + (Vis ([EP]::Get())) + "] " + [EP]::GetSel())

# undo
[EP]::Set('abc def'); [EP]::Msg(0x00CD, 0, 0) | Out-Null; [EP]::Sel(7, 7)
[EP]::Type('XYZ'); Say ("typed XYZ -> [" + [EP]::Get() + "] canundo=" + [EP]::Msg(0x00C6, 0, 0))
[EP]::KeyChar(0x5A, 0x1A, $true, $false); Say ("Ctrl+Z -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel() + " canundo=" + [EP]::Msg(0x00C6, 0, 0))
[EP]::KeyChar(0x5A, 0x1A, $true, $false); Say ("Ctrl+Z again -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
[EP]::KeyChar(0x59, 0x19, $true, $false); Say ("Ctrl+Y -> [" + (Vis ([EP]::Get())) + "] sel=" + [EP]::GetSel())
[EP]::Set('abc def'); [EP]::Msg(0x00CD, 0, 0) | Out-Null; [EP]::Sel(7, 7)
[EP]::Type('XY'); [EP]::Sel(0, 0); [EP]::Type('Q'); Say ("typed XY at end, moved home, typed Q -> [" + [EP]::Get() + "]")
[EP]::KeyChar(0x5A, 0x1A, $true, $false); Say ("Ctrl+Z -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
[EP]::KeyChar(0x5A, 0x1A, $true, $false); Say ("Ctrl+Z again -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
[EP]::Set('abc def'); [EP]::Msg(0x00CD, 0, 0) | Out-Null; [EP]::Sel(0, 3); [EP]::Type('Q'); [EP]::Key(0x08, $false, $false); [EP]::Msg(0x0102, 8, 0) | Out-Null
Say ("sel 0..3 typed Q then Backspace -> [" + [EP]::Get() + "]")
[EP]::KeyChar(0x5A, 0x1A, $true, $false); Say ("Ctrl+Z -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
[EP]::KeyChar(0x5A, 0x1A, $true, $false); Say ("Ctrl+Z again -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
[EP]::Set('abc def'); [EP]::Msg(0x00CD, 0, 0) | Out-Null; [EP]::Sel(3, 3); [EP]::Msg(0x0102, 8, 0) | Out-Null; [EP]::Msg(0x0102, 8, 0) | Out-Null; [EP]::Type('ZZ')
Say ("caret 3, 2 backspaces, typed ZZ -> [" + [EP]::Get() + "]")
[EP]::KeyChar(0x5A, 0x1A, $true, $false); Say ("Ctrl+Z -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
[EP]::Key(0x08, $false, $false);
[EP]::Set('abc def'); [EP]::Sel(0, 7); [EP]::Key(0x08, $false, $true); Say ("Alt-less: sel all, WM_UNDO after settext canundo=" + [EP]::Msg(0x00C6, 0, 0))

# mouse
[EP]::Set('New Text Document.txt')
$xT = [int](([EP]::PosX(5) + [EP]::PosX(6)) / 2); $xD = [int](([EP]::PosX(12) + [EP]::PosX(13)) / 2)
[EP]::Sel(0, 17); [EP]::Click($xT, $false); Say ("all selected, click in 'Text' -> " + [EP]::GetSel())
[EP]::Sel(2, 2); [EP]::Click($xD, $true); Say ("caret 2, shift+click at ~12 -> " + [EP]::GetSel())
[EP]::Sel(4, 8); [EP]::Click($xD, $true); Say ("sel 4..8, shift+click at ~12 -> " + [EP]::GetSel())
[EP]::Sel(4, 8); [EP]::Click([EP]::PosX(1), $true); Say ("sel 4..8, shift+click at 1 -> " + [EP]::GetSel())
[EP]::Sel(0, 0); [EP]::Drag($xT, $xD); Say ("drag from ~5 to ~12 -> " + [EP]::GetSel())
[EP]::Sel(0, 0); [EP]::Drag($xD, $xT); Say ("drag from ~12 to ~5 -> " + [EP]::GetSel())
[EP]::Sel(0, 0); [EP]::DblDrag($xT, $xD); Say ("dblclick in Text then drag to ~12 -> " + [EP]::GetSel())
[EP]::Sel(0, 0); [EP]::DblDrag($xD, [EP]::PosX(1)); Say ("dblclick in Document then drag to 1 -> " + [EP]::GetSel())
[EP]::Sel(0, 0); [EP]::Dbl([EP]::PosX(3) + 1); Say ("dblclick on the space after New -> " + [EP]::GetSel())
[EP]::Sel(0, 0); [EP]::Dbl([EP]::PosX(17) + 1); Say ("dblclick on the dot -> " + [EP]::GetSel())
[EP]::Sel(0, 0); [EP]::Dbl(700); Say ("dblclick far right of the text -> " + [EP]::GetSel())
$o | Set-Content -Encoding UTF8 -Path (Join-Path $env:TEMP ($(if ($Auto) { 'edit-probe-auto.txt' } else { 'edit-probe.txt' })))
[EP]::F.Dispose()
