# Runs every wfm command once, end to end, inside %TEMP%\wfm-test-e2e, and prints a transcript.
# Settings and the undo journal go to %TEMP%\wfm-test-e2e\_data (WFM_DATA_DIR). Commands that open anything run
# on the never-shown desktop "wfm_hidden". Nothing appears on screen.
param([string]$Wfm = "$PSScriptRoot\..\target\release\wfm.exe")
$ErrorActionPreference = 'Continue'
$Wfm = (Resolve-Path $Wfm).Path
$SaveDlg = Join-Path (Split-Path $Wfm) 'examples\savedlg.exe'
$T = Join-Path $env:TEMP 'wfm-test-e2e'
if (Test-Path $T) { Remove-Item -LiteralPath $T -Recurse -Force }
New-Item -ItemType Directory -Force $T | Out-Null
$env:WFM_DATA_DIR = "$T\_data"
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class E2E {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct SI { public int cb; public string r; public string desk; public string title; public int x, y, w, h, cx, cy, fill, flags; public short show, cb2; public IntPtr r2, i, o, e; }
    [StructLayout(LayoutKind.Sequential)] struct PI { public IntPtr hp, ht; public int pid, tid; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern IntPtr CreateDesktop(string n, IntPtr d, IntPtr m, int f, uint a, IntPtr s);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool CreateProcess(string a, StringBuilder c, IntPtr pa, IntPtr ta, bool inh, uint f, IntPtr env, string dir, ref SI si, out PI pi);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr h, uint ms);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr h, out uint c);
    public static IntPtr Desk;
    public static void Init() { Desk = CreateDesktop("wfm_hidden", IntPtr.Zero, IntPtr.Zero, 0, 0x10000000u, IntPtr.Zero); }
    // Start on the hidden desktop; wait ms (0 = do not wait). Returns "pid=N exit=C".
    public static string Run(string cmd, uint waitMs) {
        var si = new SI(); si.cb = Marshal.SizeOf(si); si.desk = @"WinSta0\wfm_hidden";
        PI pi; if (!CreateProcess(null, new StringBuilder(cmd), IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero, null, ref si, out pi)) return "error " + Marshal.GetLastWin32Error();
        if (waitMs == 0) return "pid=" + pi.pid;
        uint c = 999; if (WaitForSingleObject(pi.hp, waitMs) == 0) GetExitCodeProcess(pi.hp, out c);
        return "pid=" + pi.pid + " exit=" + c;
    }
}
"@
[E2E]::Init()
function Show([string]$text) { $text.Replace($env:TEMP, '%TEMP%') }
function W { param([Parameter(ValueFromRemainingArguments = $true)] [string[]]$A)
    $line = ($A | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
    "> wfm $(Show $line)"
    $out = & $Wfm @A 2>&1 | ForEach-Object { "$_" }
    $code = $LASTEXITCODE
    foreach ($l in $out) { Show $l }
    "(exit $code)"
    ""
}
# Test data
New-Item -ItemType Directory -Force "$T\docs\sub", "$T\dst", "$T\photos" | Out-Null
Set-Content "$T\docs\readme.txt" "hello"
Set-Content "$T\docs\sub\notes.txt" "notes"
[IO.File]::WriteAllBytes("$T\big.bin", (New-Object byte[] (20MB)))
foreach ($n in 'IMG_0001.JPG', 'IMG_0002.JPG', 'IMG_0010.JPG') { Set-Content "$T\photos\$n" $n }

"=== Browse ==="
W help
W ls "$T" --sort size --desc
W ls "$T" --json --top 2
W info "$T\docs" --no-index
W info C:\Windows\System32\notepad.exe
W find setup --top 3
W find readme --in "$T" --engine walk
W drives
W recent --top 3
W pin "$T\docs" --name Docs
W pins
W unpin Docs
W bin --top 0
W devices
W ls \\localhost
"=== Change ==="
W copy "$T\big.bin" "$T\docs" "$T\dst" --json
W copy "$T\docs\readme.txt" "$T\dst\docs" --conflict keep-both
W move "$T\dst\docs" "$T\moved-docs"
W rename "$T\moved-docs" "renamed-docs"
W batch-rename "$T\photos" --pattern '^IMG_(\d+)\.jpg$' --to 'Trip {n:2} ($1).jpg' --dry-run
W batch-rename "$T\photos" --pattern '^IMG_(\d+)\.jpg$' --to 'Trip {n:2} ($1).jpg'
W mkdir "$T\new folder\inner"
W new-file "$T\new folder\inner\empty.txt"
W zip "$T\docs" "$T\big.bin" "$T\out.zip"
W unzip "$T\out.zip" "$T\unzipped"
W delete "$T\big.bin"
$id = ((& $Wfm bin --json | ConvertFrom-Json).items | Where-Object { $_.original -eq "$T\big.bin" } | Select-Object -First 1).id
W bin restore $id
W bin empty
W undo --list
W undo
W undo
W delete "$T\unzipped" --permanent
"=== Open ==="
W open --select "$T\docs\readme.txt"
W open-with "$T\docs\readme.txt"
W menu "$T\docs\readme.txt"
$env:WFM_TEST_MARKER = "$T\opened-marker.txt"
"> wfm open <savedlg.exe>   (wfm runs on the hidden desktop)"
Show ([E2E]::Run("`"$Wfm`" open `"$SaveDlg`"", 15000))
Start-Sleep -Milliseconds 800
"marker written by the opened program: $(Test-Path "$T\opened-marker.txt")"
""
Set-Content "$T\menu-victim.txt" "x"
"> wfm menu %TEMP%\wfm-test-e2e\menu-victim.txt --invoke delete   (on the hidden desktop)"
Show ([E2E]::Run("`"$Wfm`" menu `"$T\menu-victim.txt`" --invoke delete", 20000))
"file gone (in the Recycle Bin): $(-not (Test-Path "$T\menu-victim.txt"))"
& $Wfm bin restore "$T\menu-victim.txt" | Out-Null
""
W eject C:
W reveal "$T\missing-file.txt"
"=== Live ==="
"> wfm watch %TEMP%\wfm-test-e2e\docs --timeout 3 --json   (while a file is added, renamed and deleted)"
$psi = New-Object System.Diagnostics.ProcessStartInfo $Wfm, "watch `"$T\docs`" --timeout 3 --json"
$psi.UseShellExecute = $false; $psi.CreateNoWindow = $true; $psi.RedirectStandardOutput = $true
$p = [System.Diagnostics.Process]::Start($psi)
Start-Sleep -Milliseconds 500
Set-Content "$T\docs\live.txt" "1"; Rename-Item "$T\docs\live.txt" "live2.txt"; Remove-Item "$T\docs\live2.txt"
$p.WaitForExit(); Show $p.StandardOutput.ReadToEnd()
"(exit $($p.ExitCode))"
""
W thumb "$T\docs\readme.txt" --out "$T\readme-icon.png" --size 64 --icon
W shell-ls "$T\docs"
W shell-copy "$T\docs\readme.txt" "$T\dst"
"=== Agents: Save dialog (test app on the hidden desktop) ==="
$dlgCmd = '"' + $SaveDlg + '" "' + "$T\dlg-result.txt" + '" "' + $T + '"'
$started = [E2E]::Run($dlgCmd, 0)
"> savedlg.exe (GetSaveFileNameW) started on wfm_hidden: $started"
Start-Sleep -Seconds 3
W dialog list --desktop wfm_hidden
W dialog save --path "$T\saved by agent.txt" --desktop wfm_hidden
Start-Sleep -Milliseconds 500
"test app wrote: $(Show (Get-Content "$T\dlg-result.txt" -ErrorAction SilentlyContinue))"
""
"=== Errors and usage ==="
W ls "$T\nope"
W frobnicate
W copy "$T\docs"
# Report any Recycle Bin item this run left behind (expected: none).
& $Wfm bin --json | ConvertFrom-Json | ForEach-Object { $_.items } | Where-Object { $_.original -like "$T*" } | ForEach-Object { "(left in the Recycle Bin by this run: $(Show $_.original))" }
