; Boltget's additions to Tauri's NSIS installer (docs/RELEASE.md, "Installer").
;
; Tauri's uninstaller already removes the files, the Start menu and desktop
; shortcuts, the uninstall key and the Run key value "Boltget" that "Start
; with Windows" writes; when "Delete the application data" is ticked, also the
; WebView's data and window state (%LOCALAPPDATA% and %APPDATA%
; \app.boltget.desktop). These hooks remove what it leaves behind. None of
; the removals runs on an update (/UPDATE), which must keep everything.

; The browsers' native messaging host (docs/BROWSER-INTEGRATION.md, "Native
; messaging host registration on Windows") is installed next to Boltget.exe
; (tauri.installer.conf.json) and registers itself: its two manifests beside
; it, and one key per browser under HKCU. Uninstalling removes both, also
; when the host itself is gone. The host name is browser::HOST_NAME in
; crates/ipc.
!define NATIVE_HOST "app.boltget.native_host"

!macro NSIS_HOOK_POSTINSTALL
  ; Also on an update, which may have moved the install folder.
  nsExec::Exec '"$INSTDIR\boltget-native-host.exe" --register'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    nsExec::Exec '"$INSTDIR\boltget-native-host.exe" --unregister'
    Pop $0
    ; The same by hand, should the host be missing or fail.
    DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\${NATIVE_HOST}"
    DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NATIVE_HOST}"
    DeleteRegKey HKCU "Software\Mozilla\NativeMessagingHosts\${NATIVE_HOST}"
    Delete "$INSTDIR\${NATIVE_HOST}.chrome.json"
    Delete "$INSTDIR\${NATIVE_HOST}.firefox.json"
  ${EndIf}
!macroend

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
