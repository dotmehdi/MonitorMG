#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Texte formaté dans la langue de l'interface : `tr!("français {x}", "English {x}")`.
/// Pour un texte fixe : `store::t`.
macro_rules! tr {
    ($fr:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if $crate::store::english() { format!($en $(, $a)*) } else { format!($fr $(, $a)*) }
    };
}

mod auto;
mod cli;
mod dim;
mod display;
mod journal;
mod hotkeys;
mod placement;
mod rekey;
mod store;
mod tray;
mod update;
mod views;
mod watch;

use display::{lock, Res};
use journal::Level;
use hotkeys::*;
use tray::*;
use views::*;
use watch::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use store::t;
use tauri::image::Image;
use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::window::{Effect, EffectsBuilder};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent, Wry,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Après un changement de disposition, Windows déplace lui-même les fenêtres : on attend
/// qu'il ait fini avant de les remettre à leur place.
const WINDOWS_SETTLE: Duration = Duration::from_millis(1500);
const NNBSP: char = '\u{202F}';

/// Origine d'une action, affichée dans la vue Activité.
#[derive(Clone, Copy)]
enum Src {
    Window,
    Panel,
    Menu,
    Hotkey,
    Plug,
    Desktop,
    System,
    Schedule,
    Power,
    Program,
    Command,
}

impl Src {
    fn label(self) -> &'static str {
        match self {
            Src::Window => t("Fenêtre", "Window"),
            Src::Panel => t("Panneau", "Panel"),
            Src::Menu => "Menu",
            Src::Hotkey => t("Raccourci", "Shortcut"),
            Src::Plug => t("Branchement", "Connection"),
            Src::Desktop => t("Bureau", "Desktop"),
            Src::System => t("Système", "System"),
            Src::Schedule => t("Planification", "Schedule"),
            Src::Power => t("Alimentation", "Power"),
            Src::Program => t("Programme", "Program"),
            Src::Command => t("Commande", "Command"),
        }
    }

    /// Action demandée par l'utilisateur, qui peut la confirmer ; les automatismes non.
    fn manual(self) -> bool {
        !matches!(self, Src::Plug | Src::System | Src::Schedule | Src::Power | Src::Program)
    }
}

enum Decision {
    Keep,
    Revert,
    Replaced,
}

struct Pending {
    id: u64,
    tx: mpsc::Sender<Decision>,
    snap: display::Snapshot,
}

struct App {
    store: Mutex<store::Store>,
    pending: Mutex<Option<Pending>>,
    /// État d'avant la dernière modification et son libellé, pour « Annuler ».
    undo: Mutex<Option<(display::Snapshot, String)>>,
}

#[derive(Serialize)]
struct ProfileView {
    id: String,
    name: String,
    hotkey: Option<String>,
    tiles: Vec<display::Tile>,
    active: bool,
    auto: bool,
    triggers: store::Triggers,
    extras: Vec<ExtraView>,
}

/// Réglage facultatif d'un profil : appliqué ou non, et valeur enregistrée (vide si
/// le profil date d'avant ce réglage).
#[derive(Serialize)]
struct ExtraView {
    id: &'static str,
    on: bool,
    stored: String,
}

#[derive(Serialize)]
struct View {
    monitors: Vec<display::Monitor>,
    profiles: Vec<ProfileView>,
    topology: Option<&'static str>,
    autostart: bool,
    confirm: store::Confirm,
    identify: store::Identify,
    accent: (String, String),
    undo: Option<String>,
    version: &'static str,
    portable: bool,
    data_dir: String,
    tray_icon: &'static str,
    taskbar_light: bool,
    off_hotkey: Option<String>,
    up_hotkey: Option<String>,
    down_hotkey: Option<String>,
    panel: store::Panel,
    /// Préférences de chaque écran branché, valeurs par défaut comprises : l'interface
    /// n'a pas à les connaître.
    screens: BTreeMap<String, store::Screen>,
    /// Code de luminosité retenu automatiquement pour chaque écran, comme le moteur.
    auto_vcp: BTreeMap<String, u8>,
    schedule: store::Schedule,
    /// Vérification quotidienne des mises à jour, et version plus récente déjà trouvée.
    updates: bool,
    update: Option<String>,
    /// « download », « ready » ou « install » : où en est la version trouvée.
    update_step: Option<&'static str>,
    auto_update: store::AutoUpdate,
}

/// Valeur d'un réglage DDC/CI : en % pour un réglage continu, brute pour un réglage à
/// choix ; `None` si l'écran n'a pas répondu.
#[derive(Serialize)]
struct DdcValue {
    code: u8,
    value: Option<u32>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);
static READING: AtomicBool = AtomicBool::new(false);
static LAST_READ: Mutex<Option<Instant>> = Mutex::new(None);

// ---------------------------------------------------------------- commandes

#[tauri::command]
async fn state(app: AppHandle) -> Res<View> {
    let monitors = display::monitors()?;
    read_brightness(&app, false);
    let sig = display::signature().unwrap_or_default();
    let (profiles, confirm, identify, hotkeys, panel, mut screens, schedule, (updates, auto_update)) = {
        let s = store(&app);
        let views = s
            .profiles
            .iter()
            .map(|p| ProfileView {
                id: p.id.clone(),
                name: p.name.clone(),
                hotkey: p.hotkey.clone(),
                tiles: p.tiles.clone(),
                active: p.sig == sig,
                auto: p.auto,
                triggers: p.triggers.clone(),
                extras: store::EXTRAS
                    .iter()
                    .map(|&id| ExtraView { id, on: !p.exclude.contains(id), stored: stored(p, id) })
                    .collect(),
            })
            .collect();
        (views, s.confirm, s.identify, [s.off_hotkey.clone(), s.up_hotkey.clone(), s.down_hotkey.clone()], s.panel.clone(), s.by_key(), s.schedule.clone(), (s.updates, s.auto_update))
    };
    let [off_hotkey, up_hotkey, down_hotkey] = hotkeys;
    for m in &monitors {
        screens.entry(m.key.clone()).or_default();
    }
    let auto_vcp = screens.iter().map(|(k, s)| (k.clone(), store::Screen { vcp: None, ..s.clone() }.brightness_code())).collect();
    let undo = lock(&state_of(&app).undo).as_ref().map(|u| u.1.clone());
    Ok(View {
        monitors,
        profiles,
        topology: display::topology(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        confirm,
        identify,
        accent: store::accent(),
        undo,
        version: VERSION,
        portable: store::portable(),
        data_dir: store::data_dir().display().to_string(),
        tray_icon: tray_name(&app),
        taskbar_light: store::light_taskbar(),
        off_hotkey,
        up_hotkey,
        down_hotkey,
        panel,
        screens,
        auto_vcp,
        schedule,
        updates,
        update: update::newer(),
        update_step: update::step(),
        auto_update,
    })
}

#[tauri::command]
async fn set_enabled(app: AppHandle, key: String, on: bool) -> Res<()> {
    let what = format!("{} {}", display::name(&key), if on { t("activé", "enabled") } else { t("désactivé", "disabled") });
    from_window(&app, &what, true, || display::set_enabled(&key, on))
}

#[tauri::command]
async fn set_primary(app: AppHandle, key: String) -> Res<()> {
    let what = tr!("{} devient l'écran principal", "{} is now the main display", display::name(&key));
    from_window(&app, &what, false, || display::set_primary(&key))
}

#[tauri::command]
async fn set_positions(app: AppHandle, list: Vec<(String, i32, i32)>) -> Res<()> {
    from_window(&app, t("Disposition modifiée", "Layout changed"), false, || display::set_positions(&list))
}

#[tauri::command]
async fn set_mode(app: AppHandle, key: String, w: u32, h: u32, hz: u32) -> Res<()> {
    from_window(&app, &mode_label(&display::name(&key), w, h, hz), true, || display::set_mode(&key, w, h, hz))
}

#[tauri::command]
async fn set_rotation(app: AppHandle, key: String, deg: u32) -> Res<()> {
    let label = match deg {
        90 => "portrait",
        180 => t("paysage retourné", "landscape (flipped)"),
        270 => t("portrait retourné", "portrait (flipped)"),
        _ => t("paysage", "landscape"),
    };
    let what = tr!("{} : orientation {label}", "{}: orientation {label}", display::name(&key));
    from_window(&app, &what, true, || display::set_rotation(&key, deg))
}

#[tauri::command]
async fn set_scale(app: AppHandle, key: String, pct: u32) -> Res<()> {
    let what = tr!("{} : mise à l'échelle {pct}{NNBSP}%", "{}: scale {pct}%", display::name(&key));
    from_window(&app, &what, false, || display::set_scale(&key, pct))
}

#[tauri::command]
async fn set_hdr(app: AppHandle, key: String, on: bool) -> Res<()> {
    let what = tr!("{} : HDR {}", "{}: HDR {}", display::name(&key), if on { t("activé", "on") } else { t("désactivé", "off") });
    from_window(&app, &what, false, || display::set_hdr(&key, on))
}

/// Ni journal ni rafraîchissement : le curseur envoie ses valeurs pendant le glissement.
#[tauri::command]
async fn set_brightness(app: AppHandle, key: String, pct: u32) -> Res<()> {
    display::set_brightness(&key, pct)?;
    follow(&app, &key, pct);
    Ok(())
}

/// Codes VCP de l'écran, lus une fois puis gardés dans ses préférences.
#[tauri::command]
async fn ddc_caps(app: AppHandle, key: String, force: bool) -> Res<BTreeMap<u8, Vec<u8>>> {
    let known = store(&app).screen(&key).map(|s| s.caps.clone()).filter(|c| !c.is_empty());
    if let (false, Some(c)) = (force, known) {
        return Ok(c);
    }
    let caps = display::vcp_caps(&key)?;
    save_with(&app, |s| {
        s.screen_mut(&key).caps = caps.clone();
        Ok(())
    })?;
    retune(&app);
    Ok(caps)
}

/// Valeurs actuelles des réglages affichés dans le panneau.
#[tauri::command]
async fn ddc_values(app: AppHandle, key: String) -> Res<Vec<DdcValue>> {
    let sc = store(&app).screen(&key).cloned().unwrap_or_default();
    let codes: Vec<u8> = sc.features.iter().map(|f| f.code).collect();
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    Ok(display::vcp_get(&key, &codes)?
        .into_iter()
        .map(|(code, r)| {
            let value = r.and_then(|(cur, max)| match sc.discrete(code) {
                // Certains écrans renvoient l'entrée avec un octet de poids fort.
                true => Some(cur & 0xFF),
                false if code == display::VCP_POWER => Some(cur),
                false => display::percent((cur, max)).map(|p| feature_range(&sc, code).to_slider(p)),
            });
            DdcValue { code, value }
        })
        .collect())
}

/// Règle un réglage affiché dans le panneau : en % s'il est continu, valeur brute sinon.
/// Les autres codes sont refusés : certains remettent l'écran à zéro.
#[tauri::command]
async fn ddc_set(app: AppHandle, key: String, code: u8, value: u32) -> Res<()> {
    let sc = store(&app).screen(&key).cloned().unwrap_or_default();
    if !sc.features.iter().any(|f| f.code == code) {
        return Err(t("Ce réglage n'est pas activé pour cet écran", "This control isn't enabled for this display").into());
    }
    if sc.discrete(code) || code == display::VCP_POWER {
        if !sc.allows(code, value) {
            return Err(t("L'écran n'annonce pas cette valeur", "The display doesn't offer this value").into());
        }
        Ok(display::vcp_set(&key, code, value)?)
    } else {
        Ok(display::vcp_level(&key, code, value, feature_range(&sc, code))?)
    }
}

/// Allume ou éteint un écran par DDC/CI (code D6). `on` : l'état voulu ; sans lui, on
/// bascule d'après l'état lu. Un état illisible n'éteint jamais : on allume. Renvoie la
/// valeur D6 envoyée en premier (1 : allumé).
#[tauri::command]
async fn ddc_power(app: AppHandle, key: String, on: Option<bool>) -> Res<u32> {
    let (sc, signal) = {
        let s = store(&app);
        (s.screen(&key).cloned().unwrap_or_default(), s.panel.power)
    };
    let on = match on {
        Some(on) => on,
        None => display::vcp_get(&key, &[display::VCP_POWER])?.first().and_then(|x| x.1).is_none_or(|(cur, _)| cur != 1),
    };
    let name = display::name(&key);
    if on {
        display::vcp_set(&key, display::VCP_POWER, 1)?;
        note(&app, Level::Ok, Src::Panel, &tr!("{name} allumé", "{name} turned on"));
        return Ok(1);
    }
    // Seules les valeurs que l'écran annonce (le BenQ EW3270U n'a pas de veille).
    let values: Vec<u32> = signal.values().iter().copied().filter(|&v| sc.allows(display::VCP_POWER, v)).collect();
    let first = *values.first().ok_or(t("L'écran n'annonce pas ce signal d'extinction", "The display doesn't offer this power-off signal"))?;
    display::vcp_set(&key, display::VCP_POWER, first)?;
    for &v in &values[1..] {
        let _ = display::vcp_set(&key, display::VCP_POWER, v);
    }
    note(&app, Level::Ok, Src::Panel, &tr!("{name} éteint", "{name} turned off"));
    Ok(first)
}

/// Enregistre les préférences d'un écran et renvoie celles retenues (nettoyées), pour
/// que l'interface parte toujours de l'état enregistré.
#[tauri::command]
async fn set_screen(app: AppHandle, key: String, screen: store::Screen) -> Res<store::Screen> {
    let kept = save_with(&app, |s| {
        let (caps, input_hotkey, model) = s.screen(&key).map(|x| (x.caps.clone(), x.input_hotkey.clone(), x.model.clone())).unwrap_or_default();
        // Le raccourci de bascule d'entrée passe par `input_hotkey`, qui l'enregistre auprès de Windows.
        let screen = store::Screen { input_hotkey, model, ..screen }.checked(caps)?;
        // Le modèle noté ne compte pas : sans autre réglage, les préférences disparaissent.
        if (store::Screen { model: String::new(), ..screen.clone() }) == store::Screen::default() {
            s.screens.remove(&s.id(&key));
        } else {
            s.screens.insert(s.id(&key), screen.clone());
        }
        Ok(screen)
    })?;
    retune(&app);
    read_brightness(&app, true);
    changed(&app);
    Ok(kept)
}

#[tauri::command]
async fn set_panel(app: AppHandle, panel: store::Panel) -> Res<()> {
    save_with(&app, |s| {
        s.panel = panel.checked();
        Ok(())
    })
}

#[tauri::command]
async fn screens_off_now(app: AppHandle) {
    screens_off(&app, Src::Panel);
}

#[tauri::command]
async fn open_main(app: AppHandle, view: Option<String>) {
    show_main_at(&app, view);
}

/// Le panneau prend la hauteur de son contenu, ancré en bas à droite de son écran.
#[tauri::command]
async fn flyout_fit(window: WebviewWindow, h: f64) -> Res<()> {
    let mon = window.current_monitor().ok().flatten().ok_or(t("Écran introuvable", "Display not found"))?;
    let (wa, sf) = (mon.work_area(), mon.scale_factor());
    let h = h.clamp(120.0, wa.size.height as f64 / sf);
    let (pw, ph) = ((FLYOUT_W * sf) as u32, (h * sf) as u32);
    let _ = window.set_size(PhysicalSize::new(pw, ph));
    let x = wa.position.x + wa.size.width as i32 - pw as i32;
    let y = wa.position.y + wa.size.height as i32 - ph as i32;
    window.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_topology(app: AppHandle, kind: String) -> Res<()> {
    from_window(&app, &tr!("Projection : {}", "Projection: {}", topology_label(&kind)), true, || display::set_topology(&kind))
}

#[tauri::command]
async fn undo(app: AppHandle) -> Res<()> {
    undo_last(&app, Src::Window)
}

#[tauri::command]
async fn activity() -> Vec<journal::Entry> {
    journal::recent(300)
}

#[tauri::command]
async fn open_data_dir() -> Res<()> {
    let dir = store::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let explorer = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into())).join("explorer.exe");
    std::process::Command::new(explorer).arg(dir).spawn().map(drop).map_err(|e| e.to_string())
}

#[tauri::command]
async fn identify_cmd(app: AppHandle) -> Res<()> {
    identify(&app)
}

#[tauri::command]
async fn confirm(app: AppHandle, keep: bool) {
    let pending = lock(&state_of(&app).pending).take();
    if let Some(p) = pending {
        let _ = p.tx.send(if keep { Decision::Keep } else { Decision::Revert });
    }
    if let Some(w) = app.get_webview_window("confirm") {
        let _ = w.destroy();
    }
}

#[tauri::command]
async fn profile_save(app: AppHandle, name: String) -> Res<String> {
    let p = store::capture(clean_name(&name)?)?;
    let (id, name) = (p.id.clone(), p.name.clone());
    save_with(&app, |s| {
        s.profiles.push(p);
        Ok(())
    })?;
    note(&app, Level::Ok, Src::Window, &tr!("Profil « {name} » enregistré", "Profile “{name}” saved"));
    changed(&app);
    Ok(id)
}

#[tauri::command]
async fn profile_update(app: AppHandle, id: String) -> Res<()> {
    let name = store(&app).get_mut(&id)?.name.clone();
    // Capture hors verrou : elle relit la luminosité en DDC/CI, parfois plusieurs secondes.
    let fresh = store::capture(name.clone())?;
    save_with(&app, |s| {
        let p = s.get_mut(&id)?;
        if p.windows.is_empty() {
            // Profil d'avant la place des fenêtres : elles restent à demander.
            p.exclude.insert(store::WINDOWS.into());
        }
        (p.sig, p.tiles, p.snap, p.screens, p.windows) = (fresh.sig, fresh.tiles, fresh.snap, fresh.screens, fresh.windows);
        Ok(())
    })?;
    note(&app, Level::Ok, Src::Window, &tr!("Profil « {name} » remplacé par l'affichage actuel", "Profile “{name}” replaced with the current setup"));
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_rename(app: AppHandle, id: String, name: String) -> Res<()> {
    let name = clean_name(&name)?;
    let old = save_with(&app, |s| Ok(std::mem::replace(&mut s.get_mut(&id)?.name, name.clone())))?;
    note(&app, Level::Ok, Src::Window, &tr!("Profil « {old} » renommé « {name} »", "Profile “{old}” renamed “{name}”"));
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_delete(app: AppHandle, id: String) -> Res<()> {
    let name = save_with(&app, |s| {
        let name = s.get_mut(&id)?.name.clone();
        s.profiles.retain(|p| p.id != id);
        Ok(name)
    })?;
    register_hotkeys(&app);
    note(&app, Level::Ok, Src::Window, &tr!("Profil « {name} » supprimé", "Profile “{name}” deleted"));
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_move(app: AppHandle, id: String, to: usize) -> Res<()> {
    save_with(&app, |s| {
        let from = s.profiles.iter().position(|p| p.id == id).ok_or(t("Profil introuvable", "Profile not found"))?;
        let p = s.profiles.remove(from);
        let to = to.min(s.profiles.len());
        s.profiles.insert(to, p);
        Ok(())
    })?;
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_apply(app: AppHandle, id: String, panel: Option<bool>) -> Res<String> {
    apply_profile(&app, if panel == Some(true) { Src::Panel } else { Src::Window }, &id)
}

#[tauri::command]
async fn profile_hotkey(app: AppHandle, id: String, hotkey: Option<String>) -> Res<()> {
    assign_hotkey(&app, &id, hotkey.clone())?;
    let name = store(&app).get_mut(&id)?.name.clone();
    let what = match &hotkey {
        Some(h) => tr!("Raccourci {h} attribué au profil « {name} »", "Shortcut {h} assigned to profile “{name}”"),
        None => tr!("Raccourci du profil « {name} » retiré", "Shortcut removed from profile “{name}”"),
    };
    note(&app, Level::Ok, Src::Window, &what);
    changed(&app);
    Ok(())
}

/// Raccourci d'une action : éteindre les écrans, monter ou baisser la luminosité.
#[tauri::command]
async fn action_hotkey(app: AppHandle, action: String, hotkey: Option<String>) -> Res<()> {
    let label = match action.as_str() {
        OFF => t("l'extinction des écrans", "turning displays off"),
        UP => t("la hausse de luminosité", "brightness up"),
        DOWN => t("la baisse de luminosité", "brightness down"),
        _ => return Err(t("Action inconnue", "Unknown action").into()),
    };
    assign_hotkey(&app, &action, hotkey.clone())?;
    let what = match &hotkey {
        Some(h) => tr!("Raccourci {h} attribué à {label}", "Shortcut {h} assigned to {label}"),
        None => tr!("Raccourci de {label} retiré", "Shortcut for {label} removed"),
    };
    note(&app, Level::Ok, Src::Window, &what);
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_auto(app: AppHandle, id: String, on: bool) -> Res<()> {
    save_with(&app, |s| {
        let p = s.get_mut(&id)?;
        if on && p.screens.is_empty() {
            return Err(t("Profil trop ancien : remplacez-le d'abord par l'affichage actuel", "Profile too old. Replace it with the current setup first").into());
        }
        p.auto = on;
        Ok(())
    })?;
    changed(&app);
    Ok(())
}

/// Déclencheurs d'un profil : heure fixe, changement d'alimentation, lancement d'un programme.
#[tauri::command]
async fn profile_triggers(app: AppHandle, id: String, triggers: store::Triggers) -> Res<()> {
    let name = save_with(&app, |s| {
        let p = s.get_mut(&id)?;
        p.triggers = triggers.checked();
        Ok(p.name.clone())
    })?;
    note(&app, Level::Ok, Src::Window, &tr!("Déclencheurs du profil « {name} » modifiés", "Triggers of profile “{name}” changed"));
    changed(&app);
    Ok(())
}

/// Luminosité selon l'heure : enregistrée, puis appliquée aussitôt si elle est active.
/// Renvoie les étapes retenues (triées, dans les bornes).
#[tauri::command]
async fn set_schedule(app: AppHandle, schedule: store::Schedule) -> Res<store::Schedule> {
    let kept = save_with(&app, |s| {
        s.schedule = schedule.checked();
        Ok(s.schedule.clone())
    })?;
    auto::wake(auto::Wake::Apply);
    changed(&app);
    Ok(kept)
}

/// Vérification quotidienne des mises à jour. Activée, une première a lieu tout de suite.
#[tauri::command]
async fn set_updates(app: AppHandle, on: bool) -> Res<()> {
    save_with(&app, |s| {
        s.updates = on;
        Ok(())
    })?;
    if on {
        thread::spawn(move || update::check_quietly(&app));
    }
    Ok(())
}

/// « Vérifier maintenant » : renvoie la version disponible, s'il y en a une plus récente,
/// et la télécharge ou l'installe ensuite si c'est réglé ainsi.
#[tauri::command]
async fn check_update(app: AppHandle) -> Res<Option<String>> {
    let found = update::check()?;
    changed(&app);
    if found.is_some() {
        thread::spawn(move || update::follow(&app));
    }
    Ok(found)
}

#[tauri::command]
async fn download_update(app: AppHandle) -> Res<()> {
    update::download(&app)
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Res<()> {
    update::install(&app)
}

#[tauri::command]
async fn set_auto_update(app: AppHandle, auto: store::AutoUpdate) -> Res<()> {
    save_with(&app, |s| {
        s.auto_update = auto;
        Ok(())
    })?;
    // Une version déjà trouvée suit les nouveaux réglages sans attendre la prochaine vérification.
    thread::spawn(move || update::follow(&app));
    Ok(())
}

#[tauri::command]
async fn open_releases() -> Res<()> {
    update::open_releases()
}

/// Raccourci qui bascule l'entrée d'un écran entre ses deux entrées choisies.
#[tauri::command]
async fn input_hotkey(app: AppHandle, key: String, hotkey: Option<String>) -> Res<()> {
    assign_hotkey(&app, &format!("{INPUT}{key}"), hotkey.clone())?;
    let name = display::name(&key);
    let what = match &hotkey {
        Some(h) => tr!("Raccourci {h} attribué au changement d'entrée de {name}", "Shortcut {h} assigned to switching the input of {name}"),
        None => tr!("Raccourci de changement d'entrée de {name} retiré", "Input shortcut removed from {name}"),
    };
    note(&app, Level::Ok, Src::Window, &what);
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_include(app: AppHandle, id: String, setting: String, on: bool) -> Res<()> {
    if !store::EXTRAS.contains(&setting.as_str()) {
        return Err(t("Réglage inconnu", "Unknown setting").into());
    }
    save_with(&app, |s| {
        let p = s.get_mut(&id)?;
        if on {
            p.exclude.remove(&setting);
        } else {
            p.exclude.insert(setting);
        }
        Ok(())
    })?;
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_shortcut(app: AppHandle, id: String) -> Res<String> {
    let p = store(&app).get_mut(&id)?.clone();
    let path = store::desktop_shortcut(&p)?;
    note(&app, Level::Ok, Src::Window, &tr!("Raccourci du profil « {} » créé sur le bureau", "Desktop shortcut created for profile “{}”", p.name));
    Ok(path)
}

#[tauri::command]
async fn set_autostart(app: AppHandle, on: bool) -> Res<()> {
    let a = app.autolaunch();
    if on { a.enable() } else { a.disable() }.map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_tray_icon(app: AppHandle, name: String) -> Res<()> {
    if !TRAY_ICONS.iter().any(|i| i.0 == name) {
        return Err(t("Icône inconnue", "Unknown icon").into());
    }
    save_with(&app, |s| {
        s.tray = name;
        Ok(())
    })?;
    if let Some(tray) = app.tray_by_id("tray") {
        let _ = tray.set_icon(Some(tray_icon(&app)));
    }
    Ok(())
}

#[tauri::command]
async fn set_confirm(app: AppHandle, on: bool, secs: u64) -> Res<()> {
    save_with(&app, |s| {
        s.confirm = store::Confirm { on, secs: secs.clamp(5, 120) };
        Ok(())
    })
}

#[tauri::command]
async fn set_identify(app: AppHandle, lit: Option<u32>, dim: u32) -> Res<()> {
    save_with(&app, |s| {
        s.identify = store::Identify { lit, dim }.checked();
        Ok(())
    })
}

// ---------------------------------------------------------------- logique

fn state_of(app: &AppHandle) -> &App {
    app.state::<App>().inner()
}

fn store(app: &AppHandle) -> MutexGuard<'_, store::Store> {
    lock(&state_of(app).store)
}

/// Modifie les réglages et les enregistre, sous le verrou, puis le relâche. Jamais
/// d'appel à Tauri dans `f` : le thread principal peut attendre ce verrou.
fn save_with<T>(app: &AppHandle, f: impl FnOnce(&mut store::Store) -> Res<T>) -> Res<T> {
    let mut s = store(app);
    let out = f(&mut s)?;
    let saved = s.save();
    drop(s);
    saved.map(|()| out)
}

/// « AORUS FO32U2 : 3840 × 2160 à 200 Hz », pour le journal.
fn mode_label(name: &str, w: u32, h: u32, hz: u32) -> String {
    tr!("{name} : {w} × {h} à {hz} Hz", "{name}: {w} × {h} at {hz} Hz")
}

/// Applique les préférences DDC/CI au moteur d'affichage.
fn retune(app: &AppHandle) {
    display::tune(store(app).by_key().iter().map(|(k, s)| (k.clone(), s.tune())).collect());
}

/// Rattache les profils aux écrans dont Windows a changé l'identifiant de connecteur et
/// les préférences DDC/CI aux écrans branchés (`rekey.rs`), l'enregistre et le consigne.
/// Renvoie les écrans branchés.
fn follow_screens(app: &AppHandle) -> Vec<display::Seen> {
    let Ok(seen) = display::seen() else { return Vec::new() };
    let (moved, saved) = {
        let mut s = store(app);
        let moved = rekey::run(&mut s, &seen);
        let saved = if moved.any() { s.save() } else { Ok(()) };
        (moved, saved)
    };
    if !moved.keys.is_empty() {
        note(app, Level::Ok, Src::System, &rekey_text(&moved, &seen));
    }
    if let Err(e) = saved {
        note(app, Level::Error, Src::System, &e);
    }
    if moved.screens {
        retune(app);
        register_hotkeys(app);
    }
    seen
}

/// Même rattachement, MonitorMG fermé : raccourci bureau ou ligne de commande.
fn follow_screens_alone(s: &mut store::Store, src: &str) {
    let Ok(seen) = display::seen() else { return };
    let moved = rekey::run(s, &seen);
    if moved.any() {
        if let Err(e) = s.save() {
            journal::write(Level::Error, src, &e);
        }
    }
    if !moved.keys.is_empty() {
        journal::write(Level::Ok, src, &rekey_text(&moved, &seen));
    }
}

fn rekey_text(moved: &rekey::Moved, seen: &[display::Seen]) -> String {
    let names = rekey::describe(moved, seen);
    tr!(
        "Windows a changé l'identifiant de connecteur de {names} : profils rattachés",
        "Windows changed the connector ID of {names}: profiles reattached"
    )
}

fn feature_range(sc: &store::Screen, code: u8) -> display::Range {
    sc.features.iter().find(|f| f.code == code).map_or(display::Range::FULL, |f| f.range)
}

/// Réglages liés à la luminosité : ils la suivent jusqu'à leur seuil, puis restent au maximum.
fn follow(app: &AppHandle, key: &str, pct: u32) {
    let Some(sc) = store(app).screen(key).cloned() else { return };
    follow_with(&sc, key, pct);
}

fn follow_with(sc: &store::Screen, key: &str, pct: u32) {
    for f in sc.features.iter().filter(|f| f.linked && !sc.discrete(f.code) && f.code != display::VCP_POWER) {
        let _ = display::vcp_level(key, f.code, pct.min(f.stop) * 100 / f.stop.max(1), f.range);
    }
}

fn clean_name(name: &str) -> Res<String> {
    let n: String = name.trim().chars().filter(|c| !c.is_control()).take(40).collect();
    if n.is_empty() {
        Err(t("Donnez un nom au profil", "Give the profile a name").into())
    } else {
        Ok(n)
    }
}

fn topology_label(kind: &str) -> &'static str {
    match kind {
        "extend" => t("étendre", "extend"),
        "clone" => t("dupliquer", "duplicate"),
        "internal" => t("écran du PC uniquement", "PC screen only"),
        "external" => t("deuxième écran uniquement", "second screen only"),
        _ => t("inconnue", "unknown"),
    }
}

/// Valeurs d'un réglage facultatif enregistrées dans un profil, écran par écran ; pour
/// les fenêtres, leur nombre.
fn stored(p: &store::Profile, id: &str) -> String {
    if id == store::WINDOWS {
        return match p.windows.len() {
            0 => String::new(),
            1 => t("1 fenêtre", "1 window").into(),
            n => tr!("{n} fenêtres", "{n} windows"),
        };
    }
    let s = &p.snap;
    let pct = |v: &u32| tr!("{v}{NNBSP}%", "{v}%");
    let list: Vec<(&String, String)> = match id {
        "scale" => s.scales.iter().map(|(k, v)| (k, pct(v))).collect(),
        "hdr" => s.hdr.iter().map(|(k, &on)| (k, if on { t("activé", "on") } else { t("désactivé", "off") }.to_string())).collect(),
        _ => s.brightness.iter().map(|(k, v)| (k, pct(v))).collect(),
    };
    // Avec plusieurs écrans, chaque valeur dit à quel écran elle appartient.
    match list.as_slice() {
        [(_, v)] => v.clone(),
        _ => list.iter().map(|(k, v)| format!("{} {v}", display::name(k))).collect::<Vec<_>>().join(", "),
    }
}

fn missing_text(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => t("Un écran du profil n'est pas branché", "One display from the profile is not connected").into(),
        n => tr!("{n} écrans du profil ne sont pas branchés", "{n} displays from the profile are not connected"),
    }
}

/// Consigne une entrée dans l'activité, la transmet à l'interface et met à jour
/// l'infobulle de l'icône de notification.
fn note(app: &AppHandle, level: Level, src: Src, text: &str) {
    note_with(app, level, src, text, false);
}

fn note_with(app: &AppHandle, level: Level, src: Src, text: &str, undo: bool) {
    let e = journal::write_with(level, src.label(), text, undo);
    let _ = app.emit("activity", &e);
    if level != Level::Info {
        if let Some(t) = app.tray_by_id("tray") {
            let short: String = text.chars().take(100).collect();
            let _ = t.set_tooltip(Some(format!("MonitorMG\n{short}")));
        }
    }
}

/// Applique une modification d'affichage, la consigne dans l'activité et la rend
/// annulable. `risky` : la fenêtre de confirmation protège d'un écran noir.
/// Renvoie un avertissement éventuel (écrans absents d'un profil).
fn act(app: &AppHandle, src: Src, what: &str, risky: bool, f: impl FnOnce() -> Res<String>) -> Res<String> {
    let before = display::capture().ok();
    let r = match (&before, risky) {
        (Some(snap), true) => guarded(app, snap.clone(), f),
        _ => f(),
    };
    match &r {
        Ok(warn) => {
            let undoable = before.is_some();
            *lock(&state_of(app).undo) = before.map(|s| (s, what.to_string()));
            if warn.is_empty() {
                note_with(app, Level::Ok, src, what, undoable);
            } else {
                note_with(app, Level::Warn, src, &format!("{what}. {warn}"), undoable);
            }
        }
        Err(e) => note(app, Level::Error, src, &tr!("{what} : échec. {e}", "{what}: failed. {e}")),
    }
    changed(app);
    r
}

fn from_window(app: &AppHandle, what: &str, risky: bool, f: impl FnOnce() -> Res<()>) -> Res<()> {
    act(app, Src::Window, what, risky, || f().map(|()| String::new())).map(drop)
}

fn from_menu(app: &AppHandle, what: &str, risky: bool, f: impl FnOnce() -> Res<()>) -> Res<String> {
    act(app, Src::Menu, what, risky, || f().map(|()| String::new()))
}

fn apply_profile(app: &AppHandle, src: Src, id: &str) -> Res<String> {
    // Écrans renumérotés par Windows depuis le dernier branchement : rattachés d'abord.
    follow_screens(app);
    let p = store(app).get_mut(id)?.clone();
    let extras = p.extras();
    let windows = p.placed_windows().to_vec();
    let (name, snap) = (p.name, p.snap);
    let what = match src {
        Src::Plug => tr!("Profil « {name} » appliqué au branchement", "Profile “{name}” applied on connect"),
        Src::Schedule => tr!("Profil « {name} » appliqué à l'heure prévue", "Profile “{name}” applied on schedule"),
        Src::Power => tr!("Profil « {name} » appliqué au changement d'alimentation", "Profile “{name}” applied on power change"),
        Src::Program => tr!("Profil « {name} » appliqué au lancement de son programme", "Profile “{name}” applied when its program started"),
        _ => tr!("Profil « {name} » appliqué", "Profile “{name}” applied"),
    };
    // Un profil peut laisser les écrans noirs : confirmé comme un changement de mode.
    // En attendant, Windows ne l'enregistre pas : rebrancher les écrans rend la
    // disposition d'avant, au lieu de réappliquer celle qui ne s'affiche pas.
    let guard = src.manual() && store(app).confirm.on;
    let r = act(app, src, &what, guard, || display::restore(&snap, extras, !guard).map(missing_text));
    if r.is_ok() && !windows.is_empty() {
        let app = app.clone();
        thread::spawn(move || {
            thread::sleep(WINDOWS_SETTLE);
            let n = placement::restore(&windows);
            note(&app, Level::Info, Src::System, &tr!("{n} fenêtre(s) remise(s) en place", "{n} window(s) put back in place"));
        });
    }
    r
}

fn undo_last(app: &AppHandle, src: Src) -> Res<()> {
    let Some((snap, what)) = lock(&state_of(app).undo).take() else {
        return Err(t("Rien à annuler", "Nothing to undo").into());
    };
    act(app, src, &tr!("Annulé : {what}", "Undone: {what}"), false, || display::restore(&snap, display::Extras::UNDO, true).map(|_| String::new())).map(drop)
}

/// Hors de la fenêtre, une erreur ou un avertissement doit rester visible.
fn outcome(app: &AppHandle, r: Res<String>) {
    match r {
        Err(e) => report(app, &e),
        Ok(m) if !m.is_empty() => report(app, &m),
        _ => {}
    }
}

/// Un thread qui exécute `f` une fois le calme revenu : chaque signal envoyé repousse
/// l'exécution de `delay`, une rafale ne la déclenche qu'une fois.
fn debounce(delay: Duration, f: impl Fn() + Send + 'static) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        while rx.recv().is_ok() {
            while rx.recv_timeout(delay).is_ok() {}
            f();
        }
    });
    tx
}

/// Relit la luminosité en arrière-plan. DDC/CI est lent et chaque lecture occupe le
/// bus de l'écran : au plus une lecture toutes les 5 s, sauf au branchement.
fn read_brightness(app: &AppHandle, force: bool) {
    {
        let mut last = lock(&LAST_READ);
        if !force && last.is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
            return;
        }
        if READING.swap(true, Ordering::SeqCst) {
            return;
        }
        *last = Some(Instant::now());
    }
    let app = app.clone();
    thread::spawn(move || {
        // Drapeau remis même si la relecture panique : sinon plus aucune relecture.
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                READING.store(false, Ordering::SeqCst);
            }
        }
        let changed = {
            let _done = Done;
            display::read_brightness()
        };
        if changed {
            self::changed(&app); // le menu de notification aussi, pas seulement la fenêtre
        }
    });
}

/// Erreur d'une action lancée hors de la fenêtre (menu, raccourci clavier). Fenêtre
/// ouverte, l'activité l'affiche déjà ; sinon une boîte de dialogue la rend visible.
fn report(app: &AppHandle, msg: &str) {
    if app.get_webview_window("main").is_none() {
        store::alert(msg);
    }
}

/// Applique un réglage risqué (écran noir possible). Sans confirmation dans le
/// délai choisi, on revient à `snap`, comme le fait Windows.
/// L'utilisateur peut couper cette sécurité dans les paramètres.
fn guarded<T>(app: &AppHandle, snap: display::Snapshot, f: impl FnOnce() -> Res<T>) -> Res<T> {
    let c = store(app).confirm;
    if !c.on {
        // Confirmation coupée entre-temps : rien à attendre, Windows l'enregistre.
        let out = f();
        display::persist();
        return out;
    }
    let mut pending = lock(&state_of(app).pending);
    // Une confirmation déjà en cours garde l'état d'origine comme point de retour.
    let snap = pending.as_ref().map_or(snap, |p| p.snap.clone());
    let out = f()?;
    if let Some(p) = pending.take() {
        let _ = p.tx.send(Decision::Replaced);
    }
    let (tx, rx) = mpsc::channel();
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    *pending = Some(Pending { id, tx, snap: snap.clone() });
    drop(pending);
    show_confirm(app, c.secs);
    let app = app.clone();
    thread::spawn(move || {
        let why = match rx.recv_timeout(Duration::from_secs(c.secs)) {
            Ok(Decision::Keep) => None,
            Ok(Decision::Revert) => Some(t("à votre demande", "at your request")),
            Ok(Decision::Replaced) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut p = lock(&state_of(&app).pending);
                if p.as_ref().map(|p| p.id) != Some(id) {
                    return;
                }
                p.take();
                drop(p);
                Some(t("sans confirmation", "no confirmation"))
            }
        };
        if let Some(w) = app.get_webview_window("confirm") {
            let _ = w.destroy();
        }
        match why.map(|why| (why, display::restore(&snap, display::Extras::UNDO, true))) {
            None => {
                display::persist();
                note(&app, Level::Info, Src::System, t("Changement conservé", "Change kept"));
            }
            Some((why, Ok(_))) => {
                lock(&state_of(&app).undo).take();
                note(&app, Level::Warn, Src::System, &tr!("Affichage précédent rétabli ({why})", "Previous display settings restored ({why})"));
            }
            Some((_, Err(e))) => {
                note(&app, Level::Error, Src::System, &tr!("Retour à l'affichage précédent impossible. {e}", "Can't restore the previous display settings. {e}"));
                report(&app, &tr!("Retour impossible : {e}", "Can't revert: {e}"));
            }
        }
        changed(&app);
    });
    Ok(out)
}

// ---------------------------------------------------------------- démarrage

/// MonitorMG tourne-t-il déjà ? Même repère que le greffon d'instance unique.
fn running(identifier: &str) -> bool {
    let wide = |s: String| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (class, title) = (wide(format!("{identifier}-sic")), wide(format!("{identifier}-siw")));
    unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) }.is_ok_and(|h| !h.is_invalid())
}

/// Second lancement. Un raccourci bureau (`--profile`) applique son profil ici, avec les
/// réglages DDC/CI, le journal, l'annulation et le verrou du bus ; sinon on montre la fenêtre.
fn second_instance(app: &AppHandle, args: Vec<String>) {
    let Some(Ok(cmd)) = cli::parse(&args) else { return show_main(app) };
    let app = app.clone();
    // Le message arrive sur le thread principal : le travail se fait ailleurs.
    thread::spawn(move || cli::run(&app, cmd));
}

/// Raccourci bureau quand MonitorMG ne tourne pas : applique le profil puis quitte.
fn apply_from_desktop(id: &str) {
    let desk = Src::Desktop.label();
    let mut s = store::Store::load();
    follow_screens_alone(&mut s, desk);
    // Préférences DDC/CI comprises : écran bloqué, plage et code de la luminosité.
    display::tune(s.by_key().iter().map(|(k, sc)| (k.clone(), sc.tune())).collect());
    let Some(p) = s.profiles.iter().find(|p| p.id == id || p.name == id) else {
        let e = tr!("Profil « {id} » introuvable", "Profile “{id}” not found");
        journal::write(Level::Error, desk, &e);
        store::alert(&e);
        return;
    };
    let what = tr!("Profil « {} » appliqué", "Profile “{}” applied", p.name);
    let r = display::restore(&p.snap, p.extras(), true);
    if r.is_ok() && !p.placed_windows().is_empty() {
        thread::sleep(WINDOWS_SETTLE);
        placement::restore(p.placed_windows());
    }
    match r {
        Ok(0) => {
            journal::write(Level::Ok, desk, &what);
        }
        Ok(n) => {
            let m = format!("{what}. {}", missing_text(n));
            journal::write(Level::Warn, desk, &m);
            store::alert(&m);
        }
        Err(e) => {
            journal::write(Level::Error, desk, &tr!("{what} : échec. {e}", "{what}: failed. {e}"));
            store::alert(&tr!("Le profil « {} » n'a pas pu être appliqué.\n\n{e}", "Profile “{}” couldn't be applied.\n\n{e}", p.name));
        }
    }
}

fn log_startup() {
    let mode = if store::portable() { " (portable)" } else { "" };
    journal::write(Level::Info, Src::System.label(), &tr!("MonitorMG {VERSION}{mode} démarré, {}", "MonitorMG {VERSION}{mode} started, {}", store::windows_version()));
    if let Ok(ms) = display::monitors() {
        let list: Vec<String> = ms
            .iter()
            .map(|m| match m.active {
                true => format!("{} {}×{} {} Hz ({})", m.name, m.w, m.h, m.hz, m.connector),
                false => tr!("{} désactivé ({})", "{} disabled ({})", m.name, m.connector),
            })
            .collect();
        journal::write(Level::Info, Src::System.label(), &tr!("Écrans : {}", "Displays: {}", list.join(", ")));
    }
}

fn main() {
    // Une erreur interne laisse une trace dans le journal, même sans console.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        journal::write(Level::Error, Src::System.label(), &tr!("Erreur interne : {info}", "Internal error: {info}"));
        default_hook(info);
    }));

    let args: Vec<String> = std::env::args().collect();
    let context = tauri::generate_context!();
    match cli::parse(&args) {
        Some(Err(e)) => return cli::refuse(&e),
        // L'état se lit ici même : la sortie standard est celle de ce processus.
        Some(Ok(cli::Cmd::Json)) => return cli::print_state(),
        // MonitorMG déjà lancé : le greffon d'instance unique lui transmet la commande.
        Some(Ok(cmd)) if !running(&context.config().identifier) => return cli::alone(cmd),
        _ => {}
    }
    let hidden = args.iter().any(|a| a == "--minimized");

    let built = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| second_instance(app, args)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, e| {
                    if e.state == ShortcutState::Pressed {
                        on_hotkey(app, sc);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .manage(App { store: Mutex::new(store::Store::load()), pending: Mutex::new(None), undo: Mutex::new(None) })
        .invoke_handler(tauri::generate_handler![
            state,
            set_enabled,
            set_primary,
            set_positions,
            set_mode,
            set_rotation,
            set_scale,
            set_hdr,
            set_brightness,
            ddc_caps,
            ddc_values,
            ddc_set,
            ddc_power,
            set_screen,
            set_panel,
            screens_off_now,
            open_main,
            flyout_fit,
            set_topology,
            undo,
            activity,
            open_data_dir,
            identify_cmd,
            confirm,
            profile_save,
            profile_update,
            profile_rename,
            profile_delete,
            profile_move,
            profile_apply,
            profile_hotkey,
            action_hotkey,
            profile_auto,
            profile_triggers,
            set_schedule,
            input_hotkey,
            set_updates,
            check_update,
            open_releases,
            download_update,
            install_update,
            set_auto_update,
            profile_include,
            profile_shortcut,
            set_autostart,
            set_confirm,
            set_identify,
            set_tray_icon,
        ])
        .setup(move |app| {
            let h = app.handle().clone();
            log_startup();
            follow_screens(&h);
            TrayIconBuilder::with_id("tray")
                .icon(tray_icon(&h))
                .tooltip("MonitorMG")
                .menu(&tray_menu(&h, &MenuData::read(&h))?)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| {
                    let (app, id) = (app.clone(), e.id().0.clone());
                    thread::spawn(move || menu_action(&app, &id));
                })
                .on_tray_icon_event(|tray, e| match e {
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => toggle_flyout(tray.app_handle()),
                    TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => show_main(tray.app_handle()),
                    _ => {}
                })
                .build(app)?;
            let refused = register_hotkeys(&h);
            if !refused.is_empty() {
                journal::write(Level::Warn, Src::System.label(), &tr!("{} raccourci(s) clavier refusé(s) par Windows", "{} keyboard shortcut(s) refused by Windows", refused.len()));
            }
            retune(&h);
            display::enable_soft();
            read_brightness(&h, true);
            watch_displays(h.clone());
            auto::start(h.clone());
            auto::watch_programs(h.clone());
            update::watch(h.clone());
            if !hidden {
                show_main(&h);
            }
            Ok(())
        })
        .build(context);

    match built {
        // Fermer la fenêtre ne quitte pas : l'application reste dans la zone de notification.
        Ok(app) => app.run(|_, e| {
            if let RunEvent::ExitRequested { code: None, api, .. } = e {
                api.prevent_exit();
            }
        }),
        Err(e) => {
            journal::write(Level::Error, Src::System.label(), &tr!("Démarrage impossible : {e}", "Can't start: {e}"));
            store::alert(&tr!("MonitorMG n'a pas pu démarrer.\n\n{e}", "MonitorMG couldn't start.\n\n{e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_are_cleaned() {
        assert_eq!(clean_name("  Bureau\n "), Ok("Bureau".into()));
        assert!(clean_name("   ").is_err());
        assert_eq!(clean_name(&"x".repeat(60)).map(|n| n.chars().count()), Ok(40));
    }

    #[test]
    fn only_user_actions_wait_for_confirmation() {
        assert!(Src::Window.manual() && Src::Menu.manual() && Src::Hotkey.manual() && Src::Desktop.manual());
        assert!(!Src::Plug.manual() && !Src::Schedule.manual() && !Src::Power.manual() && !Src::Program.manual());
    }

    #[test]
    fn missing_screens_are_worded() {
        assert_eq!(missing_text(0), "");
        assert_eq!(missing_text(2), tr!("2 écrans du profil ne sont pas branchés", "2 displays from the profile are not connected"));
    }
}
