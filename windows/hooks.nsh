; Installation et désinstallation de MonitorMG.
; - Mise à jour lancée par MonitorMG sans garder les données (/FRESH) : profils, réglages
;   et journal (%APPDATA%\MonitorMG) sont effacés une fois les fichiers installés. L'ancienne
;   version est fermée à ce stade, la nouvelle n'a pas encore démarré.
; - Désinstallation (hors mise à jour) : retire le lancement avec Windows, sinon Windows
;   tenterait de lancer un fichier disparu.
; - Si l'utilisateur coche « Supprimer les données de l'application », retire aussi les
;   profils et le journal, que le modèle Tauri ignore.
; ${PRODUCTNAME} vaut « MonitorMG » ; une version d'essai a son propre nom, et donc ses
; propres données et son propre lancement avec Windows (voir docs/DEVELOPPEMENT.md).

!macro NSIS_HOOK_POSTINSTALL
  Push $0
  ${GetOptions} $CMDLINE "/FRESH" $0
  ${IfNot} ${Errors}
    SetShellVarContext current
    RmDir /r "$APPDATA\${PRODUCTNAME}"
  ${EndIf}
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\${PRODUCTNAME}"
  ${EndIf}
!macroend
