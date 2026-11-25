# Second probe of the real EDIT control: where a double-click lands at word edges, word-wise dragging after a
# double-click, Shift+double-click, Undo steps, sideways scrolling. Run hidden like edit_probe.ps1.
# Writes %TEMP%\edit-probe2.txt.
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
    public static int W = 800; public static void Make(bool auto) {
        F = new Form(); F.ShowInTaskbar = false; F.Width = 900; F.Height = 200;
        T = new TextBox(); T.Width = W; T.Left = 10; T.Top = 10; T.Font = new System.Drawing.Font("Segoe UI", 9f);
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
    public static void RClick(int x) { SendMessage(H, 0x0204, (IntPtr)2, LP(x)); }
    public static void ShiftDbl(int x) { Mods(false, true); SendMessage(H, 0x0201, (IntPtr)5, LP(x)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x)); SendMessage(H, 0x0203, (IntPtr)5, LP(x)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x)); Mods(false, false); }
    public static void DblDragHold(int x, int x2) { SendMessage(H, 0x0201, (IntPtr)1, LP(x)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x)); SendMessage(H, 0x0203, (IntPtr)1, LP(x)); SendMessage(H, 0x0200, (IntPtr)1, LP(x2)); }
    public static void MoveUp(int x2) { SendMessage(H, 0x0200, (IntPtr)1, LP(x2)); SendMessage(H, 0x0202, IntPtr.Zero, LP(x2)); }
    public static long Msg(int m, long w, long l) { return (long)SendMessage(H, m, (IntPtr)w, (IntPtr)l); }
}
"@
[EP]::Make($Auto.IsPresent)

$o = New-Object System.Collections.Generic.List[string]
function Say($s) { $o.Add($s) }
[EP]::Set('New Text Document.txt')
foreach ($p in @(@(4,-1),@(4,1),@(9,-1),@(9,1),@(3,-1),@(3,1),@(0,0),@(17,-1),@(17,1))) {
  $x = [EP]::PosX($p[0]) + $p[1]; [EP]::Sel(0,0); [EP]::Click($x,$false); $c = [EP]::SelB(); [EP]::Sel(0,0); [EP]::Dbl($x)
  Say ("dbl at x(idx " + $p[0] + ")" + $p[1] + ": single click caret=" + $c + " dbl sel=" + [EP]::GetSel())
}
[EP]::Sel(0,0); [EP]::Dbl([EP]::PosX(6)); $a=[EP]::GetSel(); [EP]::Key(0x27,$false,$true); Say ("dbl in Text: " + $a + " then Shift+Right -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::Dbl([EP]::PosX(6)); [EP]::Key(0x25,$false,$true); Say ("dbl in Text then Shift+Left -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::DblDrag([EP]::PosX(12), [EP]::PosX(1)); $a=[EP]::GetSel(); [EP]::Key(0x27,$false,$true); Say ("dbl in Document drag to 1: " + $a + " then Shift+Right -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::DblDrag([EP]::PosX(1), [EP]::PosX(12)); $a=[EP]::GetSel(); [EP]::Key(0x25,$false,$true); Say ("dbl in New drag to 12: " + $a + " then Shift+Left -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::DblDrag([EP]::PosX(6), [EP]::PosX(7)); Say ("dbl in Text drag 1 char inside same word -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::DblDrag([EP]::PosX(6), [EP]::PosX(9)+1); Say ("dbl in Text drag to just inside Document -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::DblDrag([EP]::PosX(6), [EP]::PosX(9)-1); Say ("dbl in Text drag to x(9)-1 -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::DblDrag([EP]::PosX(6), [EP]::PosX(4)-1); Say ("dbl in Text drag to x(4)-1 -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::DblDrag([EP]::PosX(6), [EP]::PosX(3)-1); Say ("dbl in Text drag to x(3)-1 -> " + [EP]::GetSel())
[EP]::Sel(0,0); [EP]::DblDragHold([EP]::PosX(12), [EP]::PosX(1)); [EP]::MoveUp([EP]::PosX(6)); Say ("dbl in Document drag to 1 then back to 6 -> " + [EP]::GetSel())
[EP]::Sel(1,1); [EP]::ShiftDbl([EP]::PosX(12)); Say ("caret 1, shift+dblclick in Document -> " + [EP]::GetSel())
[EP]::Sel(20,20); [EP]::ShiftDbl([EP]::PosX(6)); Say ("caret 20, shift+dblclick in Text -> " + [EP]::GetSel())
[EP]::Sel(4,8); [EP]::RClick([EP]::PosX(12)); Say ("sel 4..8, right button down at 12 -> " + [EP]::GetSel())
[EP]::Sel(4,8); [EP]::Click([EP]::PosX(12), $false); [EP]::Click([EP]::PosX(12), $false); Say ("two separate clicks at 12 -> " + [EP]::GetSel())
# typing over a selection, then undo
[EP]::Set('New Text Document.txt'); [EP]::Msg(0x00CD,0,0) | Out-Null; [EP]::Sel(0,17); [EP]::Type('abc')
Say ("sel stem typed abc -> [" + [EP]::Get() + "]"); [EP]::KeyChar(0x5A,0x1A,$true,$false); Say ("  Ctrl+Z -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
[EP]::Key(0x08,$false,$false)
# Alt+Backspace = undo ?
[EP]::Set('abc'); [EP]::Msg(0x00CD,0,0) | Out-Null; [EP]::Sel(3,3); [EP]::Type('d'); [EP]::Msg(0x0304,0,0) | Out-Null; Say ("WM_UNDO -> [" + [EP]::Get() + "]")
# paste / cut via messages keep undo
[EP]::Set('abc def'); [EP]::Msg(0x00CD,0,0) | Out-Null; [EP]::Sel(0,3); [EP]::Msg(0x0303,0,0) | Out-Null; Say ("WM_CLEAR sel 0..3 -> [" + [EP]::Get() + "] canundo=" + [EP]::Msg(0x00C6,0,0)); [EP]::Msg(0x0304,0,0) | Out-Null; Say ("  undo -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
# delete key forward x2 then undo
[EP]::Set('abc def'); [EP]::Msg(0x00CD,0,0) | Out-Null; [EP]::Sel(1,1); [EP]::Key(0x2E,$false,$false); [EP]::Key(0x2E,$false,$false); Say ("2x Delete at 1 -> [" + [EP]::Get() + "]"); [EP]::Msg(0x0304,0,0) | Out-Null; Say ("  undo -> [" + [EP]::Get() + "] sel=" + [EP]::GetSel())
# scrolling
[EP]::T.Width = 100
$t = 'The quick brown fox jumps over the lazy dog again and again.txt'; [EP]::Set($t)
$cw = [EP]::Msg(0x00B2, 0, 0)
Say ("narrow control: width=100 firstvisible(EM_GETFIRSTVISIBLELINE)=" + [EP]::Msg(0x00CE,0,0))
[EP]::Sel(0,0); $r=@(); for ($i=0; $i -lt $t.Length; $i++) { [EP]::Key(0x27,$false,$false); $r += ("" + [EP]::SelB() + ":fv" + [EP]::Msg(0x00CE,0,0) + ":x" + [EP]::PosX([EP]::SelB())) }
Say ("Right x n: caret:firstvisiblechar:caretx  " + ($r -join ' '))
$r=@(); for ($i=0; $i -lt $t.Length; $i++) { [EP]::Key(0x25,$false,$false); $r += ("" + [EP]::SelB() + ":fv" + [EP]::Msg(0x00CE,0,0) + ":x" + [EP]::PosX([EP]::SelB())) }
Say ("Left x n: " + ($r -join ' '))
[EP]::Key(0x23,$false,$false); Say ("End: fv=" + [EP]::Msg(0x00CE,0,0) + " caretx=" + [EP]::PosX($t.Length - 1) + " x(last char)")
[EP]::Key(0x24,$false,$false); Say ("Home: fv=" + [EP]::Msg(0x00CE,0,0))
[EP]::Sel(0, 40); Say ("EM_SETSEL(0,40) (no scrollcaret): fv=" + [EP]::Msg(0x00CE,0,0))
[EP]::Key(0x23,$false,$false); [EP]::Click(50,$false); Say ("at End, click at x=50: caret=" + [EP]::SelB() + " fv=" + [EP]::Msg(0x00CE,0,0))
$rc = New-Object byte[] 16; 
$o | Set-Content -Encoding UTF8 -Path (Join-Path $env:TEMP 'edit-probe2.txt')
[EP]::F.Dispose()
