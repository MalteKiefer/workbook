; Tauri's own NSIS template (CreateOrUpdateDesktopShortcut /
; CreateOrUpdateStartMenuShortcut in installer.nsi) skips creating both
; shortcuts whenever $UpdateMode or $NoShortcutMode is set -- this covers
; a silent update (the in-app updater re-runs the installer with /UPDATE)
; and any install NSIS resolves as an update from a prior install still on
; the machine. Verified against the real upstream template
; (tauri-apps/tauri, crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi,
; tag tauri-v2.11.5).
;
; This hook runs unconditionally after every install AND every update (see
; NSIS_HOOK_POSTINSTALL's call site, after shortcut/registry setup in the
; Install section) and just (re)creates both shortcuts every time, so an
; admin always ends up with a Desktop icon and a Start Menu entry no matter
; which path the built-in logic took.
!macro NSIS_HOOK_POSTINSTALL
  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"

  ; Mirrors installer.nsi's own STARTMENUFOLDER branch exactly, so this
  ; keeps working correctly if `bundle.windows.nsis.startMenuFolder` is
  ; ever set in tauri.conf.json (nested under that folder) or left unset
  ; (directly under Start Menu\Programs, no subfolder) -- it is currently
  ; unset, so the !else branch is the one that actually runs today.
  !if "${STARTMENUFOLDER}" != ""
    CreateDirectory "$SMPROGRAMS\$AppStartMenuFolder"
    CreateShortcut "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  !else
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  !endif
!macroend
