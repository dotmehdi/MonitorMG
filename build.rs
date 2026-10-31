// Commandes exposées à l'interface. Les déclarer ici permet de limiter, fenêtre par
// fenêtre, ce que chaque page peut appeler (voir capabilities/).
const COMMANDS: &[&str] = &[
    "state",
    "set_enabled",
    "set_primary",
    "set_positions",
    "set_mode",
    "set_rotation",
    "set_scale",
    "set_hdr",
    "set_brightness",
    "ddc_caps",
    "ddc_values",
    "ddc_set",
    "ddc_power",
    "set_screen",
    "set_panel",
    "screens_off_now",
    "open_main",
    "flyout_fit",
    "set_topology",
    "undo",
    "activity",
    "open_data_dir",
    "identify_cmd",
    "confirm",
    "profile_save",
    "profile_update",
    "profile_rename",
    "profile_delete",
    "profile_move",
    "profile_apply",
    "profile_hotkey",
    "action_hotkey",
    "profile_auto",
    "profile_include",
    "profile_shortcut",
    "set_autostart",
    "set_confirm",
    "set_tray_icon",
];

fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)))
        .expect("tauri-build");
}
