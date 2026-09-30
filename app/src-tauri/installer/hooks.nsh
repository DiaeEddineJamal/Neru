; Neru's installer hooks, included near the top of Tauri's NSIS template.
;
; Look: warm parchment behind the header and the welcome/finish pages (matching the paper-cut art
; from tools/build_installer_art.py), dark ink text, the header art on the right, Segoe UI, and a
; smooth progress bar. These must be defined before the pages, which is why they live here.
!define MUI_BGCOLOR "F4EEE3"
!define MUI_TEXTCOLOR "1F1E1B"
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADER_TRANSPARENT_TEXT
!define MUI_INSTFILESPAGE_COLORS "1F1E1B F4EEE3"
!define MUI_INSTFILESPAGE_PROGRESSBAR "smooth"
!define MUI_WELCOMEPAGE_TITLE "Welcome to Neru"
!define MUI_WELCOMEPAGE_TEXT "A calm, local-first coding agent that reads your project, proposes changes you review line by line, and never acts without asking.$\r$\n$\r$\nThis installs the Neru app and the neru command for your terminal.$\r$\n$\r$\nClick Next to continue."
!define MUI_FINISHPAGE_TITLE "Neru is ready"
!define MUI_FINISHPAGE_TEXT "Open a folder, connect a model, and ask Neru to build, fix or explain something.$\r$\n$\r$\nIn any new terminal, type neru to work with it there too."
SetFont "Segoe UI" 9

; PATH: puts `neru` (the terminal CLI) on the user's PATH: a small neru.cmd launcher in $INSTDIR\bin,
; and that folder appended to the per-user Path. Uninstalling removes both.
; Tauri's installer template already includes StrFunc and declares ${StrLoc}.
!include "LogicLib.nsh"
!include "StrFunc.nsh"
${UnStrRep}

!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$INSTDIR\bin"
  FileOpen $0 "$INSTDIR\bin\neru.cmd" w
  FileWrite $0 "@echo off$\r$\n$\"%~dp0..\neru-cli.exe$\" %*$\r$\n"
  FileClose $0
  ReadRegStr $1 HKCU "Environment" "Path"
  ${StrLoc} $2 "$1" "$INSTDIR\bin" ">"
  ${If} $2 == ""
    ${If} $1 == ""
      WriteRegExpandStr HKCU "Environment" "Path" "$INSTDIR\bin"
    ${Else}
      WriteRegExpandStr HKCU "Environment" "Path" "$1;$INSTDIR\bin"
    ${EndIf}
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ReadRegStr $1 HKCU "Environment" "Path"
  ${UnStrRep} $1 "$1" ";$INSTDIR\bin" ""
  ${UnStrRep} $1 "$1" "$INSTDIR\bin" ""
  WriteRegExpandStr HKCU "Environment" "Path" "$1"
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  Delete "$INSTDIR\bin\neru.cmd"
  RMDir "$INSTDIR\bin"
!macroend
