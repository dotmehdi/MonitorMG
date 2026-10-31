; Désinstallation de MonitorMG (hors mise à jour).
; - Retire le lancement avec Windows : sinon Windows tenterait de lancer un fichier disparu.
; - Si l'utilisateur coche « Supprimer les données de l'application », retire aussi
;   les profils et le journal (%APPDATA%\MonitorMG), que le modèle Tauri ignore.

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "MonitorMG"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "MonitorMG"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\MonitorMG"
  ${EndIf}
!macroend
