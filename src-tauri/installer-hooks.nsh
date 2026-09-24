; Install log
;
; NSIS's own LogSet/LogText need a build compiled with NSIS_CONFIG_LOG, which
; Tauri does not ship. Writing the file ourselves through the hooks Tauri does
; expose needs no special build.
;
; This is the only record the install phase leaves: the app stops logging when
; it exits to hand over to the installer, and the update-cover helper only
; watches for a process. The helper's "Open log" button reads this file first.
;
; $LOCALAPPDATA, not $INSTDIR: the installer rewrites the install directory, and
; a log living inside it would be destroyed by the very operation it records.

!define MEHEN_LOG "$LOCALAPPDATA\dev.mehen.app\logs\Mehen-Install.log"

; Append one line. Opens and closes per line so a crash mid-install still
; leaves everything written up to that point on disk.
;
; Branchless on purpose: this macro is inserted into both the installer and the
; uninstaller, which are separate scopes, and a label defined in one cannot be
; resolved from the other. FileSeek/FileWrite/FileClose on the empty handle a
; failed FileOpen yields are no-ops, so a log that cannot be written is skipped
; rather than failing the install.
!macro MEHEN_LOG_LINE Text
  Push $0
  CreateDirectory "$LOCALAPPDATA\dev.mehen.app\logs"
  FileOpen $0 "${MEHEN_LOG}" a
  FileSeek $0 0 END
  FileWrite $0 "${Text}$\r$\n"
  FileClose $0
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; Each install starts a fresh log, so the file always describes the run being
  ; asked about.
  Push $0
  CreateDirectory "$LOCALAPPDATA\dev.mehen.app\logs"
  FileOpen $0 "${MEHEN_LOG}" w
  FileWrite $0 "Mehen install log$\r$\n"
  FileClose $0
  Pop $0

  !insertmacro MEHEN_LOG_LINE "Version:    ${VERSION}"
  !insertmacro MEHEN_LOG_LINE "Target dir: $INSTDIR"

  ; $UpdateMode is only ever assigned 1, never 0, so it is empty on a normal
  ; install. Picks a string and logs once, because the relative jumps a branch
  ; around two expanded macros would need cannot be counted reliably.
  Push $0
  Push $1
  StrCpy $1 "a person (fresh install)"
  ClearErrors
  ${GetOptions} $CMDLINE "/UPDATE" $0
  IfErrors +2 0
    StrCpy $1 "the updater (/UPDATE)"
  ClearErrors
  !insertmacro MEHEN_LOG_LINE "Started by: $1"
  Pop $1
  Pop $0

  !insertmacro MEHEN_LOG_LINE "Files are being copied now"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Comes from the uninstaller of the version being removed.
  !insertmacro MEHEN_LOG_LINE "Removing the previous version"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Last line of a healthy install. A log that stops before this says the
  ; install died partway.
  !insertmacro MEHEN_LOG_LINE "Install finished successfully"
!macroend
