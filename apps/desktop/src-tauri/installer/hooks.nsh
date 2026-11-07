; Boltget's additions to Tauri's NSIS installer (docs/RELEASE.md, "Installer").
;
; Tauri's uninstaller already removes the files, the Start menu and desktop
; shortcuts, the uninstall key and the Run key value "Boltget" that "Start
; with Windows" writes; when "Delete the application data" is ticked, also the
; WebView's data and window state (%LOCALAPPDATA% and %APPDATA%
; \app.boltget.desktop). These hooks remove what it leaves behind. None of
; them runs on an update (/UPDATE), which must keep everything.

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    ; The install location Tauri remembers for the next install, which
    ; otherwise outlives the app (HKCU\Software\Boltget contributors\Boltget).
    DeleteRegKey HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
    ; Windows' per-app notification state for the app id the Start menu
    ; shortcut registered (app.boltget.desktop).
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\${BUNDLEID}"
  ${EndIf}

  ; The downloads list and settings live in the engine's data directory,
  ; %LOCALAPPDATA%\Boltget, shared with the boltget CLI: removed only when the
  ; user asks for it. Downloaded files are never touched.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$LOCALAPPDATA\Boltget"
  ${EndIf}
!macroend
