; Lanes NSIS installer hooks (bundle.windows.nsis.installerHooks in tauri.conf.json).
;
; The installer is per-machine and runs elevated, so the uninstaller may
; remove the elevated scheduled task (\Lanes\Elevated) itself.

!macro NSIS_HOOK_PREUNINSTALL
  ; Updates run the old uninstaller with /UPDATE. Keep the task then: the new
  ; lanes.exe lands at the same protected path, so the task stays valid.
  ${If} $UpdateMode <> 1
    ; Runs before the files are deleted, while lanes.exe still exists. The
    ; helper removes the task and the \Lanes folder if it is empty; exit codes
    ; are listed in platform/src/admin/mod.rs (exit_code). A failure must not
    ; block uninstalling, so the result is only logged.
    DetailPrint "Removing the Lanes admin-mode task"
    nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --admin-task remove'
    Pop $0
    DetailPrint "Admin-mode task removal finished with: $0"

    ; Autostart is a per-user HKCU Run value. The app deletes it itself when
    ; autostart is turned off; here we can only reach the user running the
    ; uninstaller (HKCU of the elevated process is the same account under UAC).
    ; Other users' values stay behind and point at a missing file, which
    ; Windows ignores.
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Lanes"
  ${EndIf}
!macroend
