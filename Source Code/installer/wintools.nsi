; Installer for Rust Task Manager / Rust File Explorer (one per app): per-user, no admin, Start menu shortcut, uninstaller.
; OLDEXE / OLDNAME: the 0.1.0 names (Win Tools ...). An upgrade removes that copy; files still in use go at the next restart.
; "Set as default" is NOT done here: it stays a button inside the app that the user clicks.
; Built by tools\release.ps1, which passes: APPID, APPNAME, EXE, CLI, ICON, VERSION, STAGE, OUTFILE.
Unicode true
SetCompressor /SOLID lzma
SetCompressorDictSize 32
RequestExecutionLevel user

!include "MUI2.nsh"

!ifndef VERSION
  !error "VERSION not defined"
!endif

Name "${APPNAME}"
Caption "${APPNAME} ${VERSION} Setup"
BrandingText "${APPNAME} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APPNAME}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "FileDescription" "${APPNAME} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Sajjad"
VIAddVersionKey "LegalCopyright" "Sajjad"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APPNAME}"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!define UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}"

Section "${APPNAME}"
  ReadRegStr $R0 HKCU "Software\${APPID}" "InstallDir"
  SetOutPath "$INSTDIR"
!ifdef HELPER
  ; the Win+E helper (Rust File Explorer): the running copy stops through its own --quit, the new one starts below
  IfFileExists "$INSTDIR\${HELPER}" 0 +3
  nsExec::Exec '"$INSTDIR\${HELPER}" --quit'
  Sleep 300
  Delete "$INSTDIR\${HELPER}.old"
  IfFileExists "$INSTDIR\${HELPER}" 0 +2
  Rename "$INSTDIR\${HELPER}" "$INSTDIR\${HELPER}.old"
  File "${STAGE}\${HELPER}"
  Delete /REBOOTOK "$INSTDIR\${HELPER}.old"
!endif
  ; an older copy may be open: a running exe cannot be overwritten, but it can be renamed. Move it aside so the new
  ; files always land; the old copies are deleted now if free, else at the next restart.
  Delete "$INSTDIR\${EXE}.old"
  Delete "$INSTDIR\${CLI}.old"
  IfFileExists "$INSTDIR\${EXE}" 0 +2
  Rename "$INSTDIR\${EXE}" "$INSTDIR\${EXE}.old"
  IfFileExists "$INSTDIR\${CLI}" 0 +2
  Rename "$INSTDIR\${CLI}" "$INSTDIR\${CLI}.old"
  SetOverwrite on
  File "${STAGE}\${EXE}"
  File "${STAGE}\${CLI}"
  Delete /REBOOTOK "$INSTDIR\${EXE}.old"
  Delete /REBOOTOK "$INSTDIR\${CLI}.old"
  File "${STAGE}\LICENSES.txt"
  SetOverwrite on
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0
  WriteRegStr HKCU "Software\${APPID}" "InstallDir" "$INSTDIR"

  WriteRegStr HKCU "${UNINST}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST}" "Publisher" "Sajjad"
  WriteRegStr HKCU "${UNINST}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINST}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKCU "${UNINST}" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKCU "${UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST}" "NoRepair" 1
!ifdef HELPER
  ; Win+E was given to Rust File Explorer ("Set as default"): start the new helper now
  ReadRegStr $R2 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "RustFileExplorerWinE"
  StrCmp $R2 "" +2
  Exec '"$INSTDIR\${HELPER}"'
!endif

!ifdef OLDEXE
  ; 0.1.0 used another exe name and folder: remove that copy (an open window is left alone; in-use files go on restart)
  Delete "$SMPROGRAMS\${OLDNAME}.lnk"
  Delete /REBOOTOK "$INSTDIR\${OLDEXE}"
  StrCmp $R0 "" old_done
  StrCmp $R0 $INSTDIR old_done
  Delete /REBOOTOK "$R0\${OLDEXE}"
  Delete /REBOOTOK "$R0\${CLI}"
  Delete /REBOOTOK "$R0\LICENSES.txt"
  Delete /REBOOTOK "$R0\uninstall.exe"
  RMDir /REBOOTOK "$R0"
  old_done:
!endif
SectionEnd

!ifdef EVERYTHING
; Everything (voidtools, MIT) gives Rust File Explorer instant whole-PC search and folder sizes.
; Installed only when no Everything is found. Its own setup asks for administrator rights (it installs a service).
Section "Everything search (voidtools)"
  ReadRegStr $R1 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Everything" "UninstallString"
  StrCmp $R1 "" 0 ev_done
  SetRegView 64
  ReadRegStr $R1 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Everything" "UninstallString"
  SetRegView 32
  StrCmp $R1 "" 0 ev_done
  IfFileExists "$PROGRAMFILES64\Everything\Everything.exe" ev_done
  IfFileExists "$PROGRAMFILES32\Everything\Everything.exe" ev_done
  IfFileExists "$LOCALAPPDATA\Programs\Everything\Everything.exe" ev_done
  InitPluginsDir
  File "/oname=$PLUGINSDIR\EverythingSetup.exe" "${EVERYTHING}"
  DetailPrint "Installing Everything search (voidtools)..."
  ; ShellExecute so Windows shows the UAC prompt (a plain Exec of an elevated setup fails)
  ExecShellWait "open" "$PLUGINSDIR\EverythingSetup.exe" "/S"
  IfFileExists "$PROGRAMFILES64\Everything\Everything.exe" 0 ev_done
  Exec '"$PROGRAMFILES64\Everything\Everything.exe" -startup'
  ev_done:
SectionEnd
!endif

Section "Uninstall"
  ; ask the open window to quit through its own CLI (no taskkill by name)
  nsExec::Exec '"$INSTDIR\${CLI}" gui quit'
  Sleep 400
!ifdef HELPER
  nsExec::Exec '"$INSTDIR\${HELPER}" --quit'
  Sleep 300
  Delete "$INSTDIR\${HELPER}"
  Delete "$INSTDIR\${HELPER}.old"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "RustFileExplorerWinE"
!endif
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\${CLI}"
  Delete "$INSTDIR\${EXE}.old"
  Delete "$INSTDIR\${CLI}.old"
  Delete "$INSTDIR\LICENSES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  DeleteRegKey HKCU "Software\${APPID}"
  DeleteRegKey HKCU "${UNINST}"
  ; settings and downloaded AI models in %LOCALAPPDATA%\WinTools are kept (shared by both apps)
SectionEnd
