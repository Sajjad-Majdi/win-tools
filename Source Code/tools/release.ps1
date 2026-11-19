# Builds both apps (Rust Task Manager, Rust File Explorer) from the same source state and writes the shipping files to ..\..\Executables:
#   RustTaskManager-<ver>-setup.exe,  RustTaskManager-<ver>-portable.exe,
#   RustFileExplorer-<ver>-setup.exe, RustFileExplorer-<ver>-portable.exe
# Usage: powershell -File tools\release.ps1 [-Version 0.1.0]
# -Online builds without --offline (CI; the developer's PC uses the local crates mirror and --offline).
# -Only app|fm builds one app (for a single-app update; Executables keeps the other app's current files).
param([string]$Version = "0.2.0", [switch]$Online, [string]$Only = "", [string]$EverythingSetup = "Everything-1.4.1.1032.x64-Setup.exe")
$ErrorActionPreference = "Stop"
$src = Split-Path -Parent $PSScriptRoot
$out = Join-Path (Split-Path -Parent $src) "Executables"
$nsis = @(
  (Join-Path $env:LOCALAPPDATA "tauri\NSIS\makensis.exe"),
  (Join-Path $env:LOCALAPPDATA "tauri\NSIS\Bin\makensis.exe"),
  "C:\Program Files (x86)\NSIS\makensis.exe",
  "C:\Program Files\NSIS\makensis.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $nsis) { throw "makensis.exe (NSIS) not found" }
$cargoArgs = @("build", "--release")
if (-not $Online) { $cargoArgs += "--offline" }
New-Item -ItemType Directory -Force $out | Out-Null

$apps = @(
  @{ Dir = "app"; Exe = "RustTaskManager.exe";  Cli = "wtm.exe"; Name = "Rust Task Manager";  Id = "com.sajjad.wintools.taskman"; File = "RustTaskManager";  Icon = "wintask.ico"; OldExe = "wintask.exe"; OldName = "Win Tools Task Manager" },
  @{ Dir = "fm";  Exe = "RustFileExplorer.exe"; Cli = "wfm.exe"; Name = "Rust File Explorer"; Id = "com.sajjad.wintools.files";   File = "RustFileExplorer"; Icon = "wfiles.ico";  OldExe = "wfiles.exe";  OldName = "Win Tools Files" }
)

if ($Only) { $apps = @($apps | Where-Object { $_.Dir -eq $Only }) }
foreach ($a in $apps) {
  $pkg = Join-Path $src $a.Dir
  Write-Host "== building $($a.Name) $Version"
  Push-Location $pkg
  & cargo @cargoArgs
  if ($LASTEXITCODE -ne 0) { Pop-Location; throw "cargo build failed in $($a.Dir)" }
  Pop-Location
  $bin = Join-Path $pkg "target\release"
  $stage = Join-Path $env:TEMP ("wt-stage-" + $a.Dir)
  Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $stage | Out-Null
  Copy-Item (Join-Path $bin $a.Exe) $stage
  Copy-Item (Join-Path $bin $a.Cli) $stage
  Copy-Item (Join-Path $src "installer\LICENSES.txt") $stage
  $setup = Join-Path $out ("{0}-{1}-setup.exe" -f $a.File, $Version)
  $extra = @()
  if ($a.Dir -eq "fm") {
    # Everything (voidtools, MIT): the File Explorer setup installs it when it is missing. Not in git; CI downloads it.
    $ev = Join-Path $src "installer\third-party\$EverythingSetup"
    if (-not (Test-Path $ev)) { throw "missing $ev (download it from https://www.voidtools.com/)" }
    $sig = Get-AuthenticodeSignature $ev
    if ($sig.Status -ne "Valid" -or $sig.SignerCertificate.Subject -notlike "CN=voidtools*") { throw "Everything setup signature is not voidtools: $($sig.Status)" }
    $extra += "/DEVERYTHING=$ev"
    # the Win+E helper (Windows 11 no longer reads the registry for Win+E)
    Copy-Item (Join-Path $bin "RustFileExplorerWinE.exe") $stage
    $extra += "/DHELPER=RustFileExplorerWinE.exe"
  }
  & $nsis /V2 "/DVERSION=$Version" "/DSTAGE=$stage" "/DOUTFILE=$setup" "/DAPPID=$($a.Id)" "/DAPPNAME=$($a.Name)" `
      "/DEXE=$($a.Exe)" "/DCLI=$($a.Cli)" "/DOLDEXE=$($a.OldExe)" "/DOLDNAME=$($a.OldName)" "/DICON=$(Join-Path $src ('installer\' + $a.Icon))" @extra (Join-Path $src "installer\wintools.nsi")
  if ($LASTEXITCODE -ne 0) { throw "makensis failed for $($a.Name)" }
  Copy-Item (Join-Path $bin $a.Exe) (Join-Path $out ("{0}-{1}-portable.exe" -f $a.File, $Version)) -Force
}
Get-ChildItem $out | Select-Object Name, Length | Format-Table -AutoSize
