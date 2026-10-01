; Oxplay Windows installer (NSIS Modern UI 2), adapted from the model project's
; per-user installer: installs to %LOCALAPPDATA%\Programs\Oxplay without elevation.
; Build: makensis -DVERSION=<semver> -DDIST_DIR=<staged payload> -DOUTPUT_FILE=<setup.exe> installer.nsi

Unicode True
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!define PRODUCT_NAME "Oxplay"
!define PRODUCT_PUBLISHER "Oxplay contributors"
!define PRODUCT_WEB_SITE "https://github.com/ViceVerse-cz/oxplay"
!define APP_EXE "oxplay.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"

!ifndef VERSION
  !error "Pass -DVERSION=<semantic version>"
!endif
!ifndef DIST_DIR
  !error "Pass -DDIST_DIR=<staged Windows payload>"
!endif
!ifndef OUTPUT_FILE
  !define OUTPUT_FILE "oxplay-${VERSION}-setup.exe"
!endif

Name "${PRODUCT_NAME} ${VERSION}"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT_NAME}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"

!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${DIST_DIR}\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ${PRODUCT_NAME}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "English"

; Labels are function-local; the macro is inserted once per function.
!macro RequireClosed MESSAGE
  ${Do}
    nsExec::Exec 'powershell -NoProfile -NonInteractive -Command "if (Get-Process oxplay -ErrorAction SilentlyContinue) { exit 1 } else { exit 0 }"'
    Pop $0
    ${If} $0 == 0
      ${Break}
    ${EndIf}
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${MESSAGE}" IDRETRY retry_closed
    Abort
    retry_closed:
  ${Loop}
!macroend

Function .onInit
  ${If} ${RunningX64}
    SetRegView 64
  ${Else}
    MessageBox MB_ICONSTOP "${PRODUCT_NAME} requires 64-bit Windows."
    Abort
  ${EndIf}
  !insertmacro RequireClosed "${PRODUCT_NAME} is running. Close it before continuing."
FunctionEnd

Section "MainSection" SEC01
  SetOutPath "$INSTDIR"
  SetOverwrite on
  File /r "${DIST_DIR}\*.*"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE},0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "${PRODUCT_WEB_SITE}"
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"
SectionEnd

Function un.onInit
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}
  !insertmacro RequireClosed "${PRODUCT_NAME} is running. Close it before uninstalling."
FunctionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\${PRODUCT_NAME}.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  ; Remove only files this installer wrote; never recursively delete a user-chosen folder.
  !ifdef UNINSTALL_INCLUDE
    !include "${UNINSTALL_INCLUDE}"
  !else
  Delete "$INSTDIR\oxplay.exe"
  Delete "$INSTDIR\libmpv-2.dll"
  Delete "$INSTDIR\yt-dlp.exe"
  Delete "$INSTDIR\deno.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\BUILD-INFO.json"
  Delete "$INSTDIR\licenses\deno-LICENSE.md"
  Delete "$INSTDIR\licenses\yt-dlp-LICENSE"
  Delete "$INSTDIR\licenses\yt-dlp-THIRD_PARTY_LICENSES.txt"
  RMDir "$INSTDIR\licenses"
  !endif
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
