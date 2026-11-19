# Builds both Win Tools apps from the same source state and writes the shipping files to ..\..\Executables:
#   WinTools-TaskManager-<ver>-setup.exe, WinTools-TaskManager-<ver>-portable.exe,
#   WinTools-Files-<ver>-setup.exe,       WinTools-Files-<ver>-portable.exe
# Usage: powershell -File tools\release.ps1 [-Version 0.1.0]
# -Online builds without --offline (CI; the developer's PC uses the local crates mirror and --offline).
param([string]$Version = "0.1.0", [switch]$Online)
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
  @{ Dir = "app"; Exe = "wintask.exe"; Cli = "wtm.exe"; Name = "Win Tools Task Manager"; Id = "com.sajjad.wintools.taskman"; File = "WinTools-TaskManager"; Icon = "wintask.ico" },
  @{ Dir = "fm";  Exe = "wfiles.exe";  Cli = "wfm.exe"; Name = "Win Tools Files";        Id = "com.sajjad.wintools.files";   File = "WinTools-Files";       Icon = "wfiles.ico" }
)

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
  & $nsis /V2 "/DVERSION=$Version" "/DSTAGE=$stage" "/DOUTFILE=$setup" "/DAPPID=$($a.Id)" "/DAPPNAME=$($a.Name)" `
      "/DEXE=$($a.Exe)" "/DCLI=$($a.Cli)" "/DICON=$(Join-Path $src ('installer\' + $a.Icon))" (Join-Path $src "installer\wintools.nsi")
  if ($LASTEXITCODE -ne 0) { throw "makensis failed for $($a.Name)" }
  Copy-Item (Join-Path $bin $a.Exe) (Join-Path $out ("{0}-{1}-portable.exe" -f $a.File, $Version)) -Force
}
Get-ChildItem $out | Select-Object Name, Length | Format-Table -AutoSize
