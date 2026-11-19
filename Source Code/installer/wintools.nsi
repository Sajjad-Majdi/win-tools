; Win Tools installer (one per app): per-user, no admin, Start menu shortcut, uninstaller.
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
InstallDirRegKey HKCU "Software\${APPID}" "InstallDir"
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
  SetOutPath "$INSTDIR"
  ; an older copy may be open: replace it on the next start if the file is in use
  SetOverwrite try
  File "${STAGE}\${EXE}"
  File "${STAGE}\${CLI}"
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
SectionEnd

Section "Uninstall"
  ; ask the open window to quit through its own CLI (no taskkill by name)
  nsExec::Exec '"$INSTDIR\${CLI}" gui quit'
  Sleep 400
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\${CLI}"
  Delete "$INSTDIR\LICENSES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  DeleteRegKey HKCU "Software\${APPID}"
  DeleteRegKey HKCU "${UNINST}"
  ; settings and downloaded AI models in %LOCALAPPDATA%\WinTools are kept (shared by both apps)
SectionEnd
