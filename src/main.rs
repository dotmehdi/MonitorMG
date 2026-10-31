#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Texte formaté dans la langue de l'interface : `tr!("français {x}", "English {x}")`.
/// Pour un texte fixe : `store::t`.
macro_rules! tr {
    ($fr:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if $crate::store::english() { format!($en $(, $a)*) } else { format!($fr $(, $a)*) }
    };
}

mod display;
mod journal;
mod store;

use display::{lock, Res};
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
const WEBVIEW2_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
const NNBSP: char = '\u{202F}';
/// Paliers de luminosité du menu de notification.
const BRIGHTNESS_STEPS: [u32; 12] = [0, 5, 15, 25, 35, 45, 55, 65, 75, 85, 95, 100];
/// Propriétaires des raccourcis d'action (les autres sont des id de profil) : éteindre
/// les écrans, monter et baisser la luminosité.
const OFF: &str = "@off";
const UP: &str = "@up";
const DOWN: &str = "@down";
/// Largeur du panneau ouvert d'un clic sur l'icône de notification.
const FLYOUT_W: f64 = 360.0;

/// Origine d'une action, affichée dans la vue Activité.
#[derive(Clone, Copy)]
enum Src {
    Window,
    Panel,
    Menu,
    Hotkey,
    Plug,
    System,
}

impl Src {
    fn label(self) -> &'static str {
        match self {
            Src::Window => t("Fenêtre", "Window"),
            Src::Panel => t("Panneau", "Panel"),
            Src::Menu => "Menu",
            Src::Hotkey => t("Raccourci", "Shortcut"),
            Src::Plug => t("Branchement", "Connection"),
            Src::System => t("Système", "System"),
        }
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
    screens: BTreeMap<String, store::Screen>,
}

/// Valeur d'un réglage DDC/CI : en % pour un réglage continu, brute pour un réglage à
/// choix ; `None` si l'écran n'a pas répondu.
#[derive(Serialize)]
struct DdcValue {
    code: u8,
    value: Option<u32>,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static TICK: AtomicU64 = AtomicU64::new(0);
/// Fenêtre invisible de `watch_displays`, qui reçoit aussi l'ordre d'éteindre les écrans.
static WATCHER: AtomicIsize = AtomicIsize::new(0);
static NEXT: AtomicU64 = AtomicU64::new(1);
static SCREENS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
static READING: AtomicBool = AtomicBool::new(false);
static LAST_READ: Mutex<Option<Instant>> = Mutex::new(None);
static DISPLAY_DIRTY: AtomicBool = AtomicBool::new(false);
static OPENING: AtomicBool = AtomicBool::new(false);
/// Fermeture du panneau par perte du focus : le clic sur l'icône qui l'a causée ne doit
/// pas le rouvrir aussitôt.
static FLYOUT_CLOSED: Mutex<Option<Instant>> = Mutex::new(None);

// ---------------------------------------------------------------- commandes

#[tauri::command]
async fn state(app: AppHandle) -> Res<View> {
    let monitors = display::monitors()?;
    read_brightness(&app, false);
    let sig = display::signature().unwrap_or_default();
    let (profiles, confirm, hotkeys, panel, screens) = {
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
                extras: store::EXTRAS
                    .iter()
                    .map(|&id| ExtraView { id, on: !p.exclude.contains(id), stored: stored(&p.snap, id) })
                    .collect(),
            })
            .collect();
        (views, s.confirm, [s.off_hotkey.clone(), s.up_hotkey.clone(), s.down_hotkey.clone()], s.panel.clone(), s.screens.clone())
    };
    let [off_hotkey, up_hotkey, down_hotkey] = hotkeys;
    Ok(View {
        monitors,
        profiles,
        topology: display::topology(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        confirm,
        accent: store::accent(),
        undo: lock(&state_of(&app).undo).as_ref().map(|u| u.1.clone()),
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
    let what = tr!("{} : {w} × {h} à {hz} Hz", "{}: {w} × {h} at {hz} Hz", display::name(&key));
    from_window(&app, &what, true, || display::set_mode(&key, w, h, hz))
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
    let known = store(&app).screens.get(&key).map(|s| s.caps.clone()).filter(|c| !c.is_empty());
    if let (false, Some(c)) = (force, known) {
        return Ok(c);
    }
    let caps = display::vcp_caps(&key)?;
    let mut s = store(&app);
    s.screens.entry(key).or_default().caps = caps.clone();
    s.save()?;
    drop(s);
    retune(&app);
    Ok(caps)
}

/// Valeurs actuelles des réglages affichés dans le panneau.
#[tauri::command]
async fn ddc_values(app: AppHandle, key: String) -> Res<Vec<DdcValue>> {
    let sc = store(&app).screens.get(&key).cloned().unwrap_or_default();
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
    let sc = store(&app).screens.get(&key).cloned().unwrap_or_default();
    if !sc.features.iter().any(|f| f.code == code) {
        return Err(t("Ce réglage n'est pas activé pour cet écran", "This control isn't enabled for this display").into());
    }
    if sc.discrete(code) || code == display::VCP_POWER {
        display::vcp_set(&key, code, value)
    } else {
        display::vcp_level(&key, code, value, feature_range(&sc, code))
    }
}

/// Allume ou éteint un écran par DDC/CI (code D6). Renvoie vrai s'il est allumé.
#[tauri::command]
async fn ddc_power(app: AppHandle, key: String) -> Res<bool> {
    let values = store(&app).panel.power_values();
    let cur = display::vcp_get(&key, &[display::VCP_POWER])?.first().and_then(|x| x.1).map(|x| x.0);
    let name = display::name(&key);
    if cur.is_some_and(|c| c != 1) {
        display::vcp_set(&key, display::VCP_POWER, 1)?;
        note(&app, "ok", Src::Panel, &tr!("{name} allumé", "{name} turned on"));
        return Ok(true);
    }
    display::vcp_set(&key, display::VCP_POWER, values[0])?;
    for &v in &values[1..] {
        let _ = display::vcp_set(&key, display::VCP_POWER, v);
    }
    note(&app, "ok", Src::Panel, &tr!("{name} éteint", "{name} turned off"));
    Ok(false)
}

#[tauri::command]
async fn set_screen(app: AppHandle, key: String, screen: store::Screen) -> Res<()> {
    let mut s = store(&app);
    let caps = s.screens.get(&key).map(|x| x.caps.clone()).unwrap_or_default();
    let screen = screen.checked(caps)?;
    if screen == store::Screen::default() {
        s.screens.remove(&key);
    } else {
        s.screens.insert(key, screen);
    }
    s.save()?;
    drop(s);
    retune(&app);
    read_brightness(&app, true);
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn set_panel(app: AppHandle, panel: store::Panel) -> Res<()> {
    let mut p = panel;
    p.step = p.step.clamp(1, 50);
    if ![4, 5, 6].contains(&p.power) {
        p.power = 5;
    }
    if !["soft", "hard", "both"].contains(&p.off.as_str()) {
        p.off = "soft".into();
    }
    let mut s = store(&app);
    s.panel = p;
    s.save()
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
    if let Some(p) = lock(&state_of(&app).pending).take() {
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
    let mut s = store(&app);
    s.profiles.push(p);
    s.save()?;
    drop(s);
    note(&app, "ok", Src::Window, &tr!("Profil « {name} » enregistré", "Profile “{name}” saved"));
    changed(&app);
    Ok(id)
}

#[tauri::command]
async fn profile_update(app: AppHandle, id: String) -> Res<()> {
    let mut s = store(&app);
    let p = s.get_mut(&id)?;
    let fresh = store::capture(p.name.clone())?;
    let name = p.name.clone();
    (p.sig, p.tiles, p.snap, p.screens) = (fresh.sig, fresh.tiles, fresh.snap, fresh.screens);
    s.save()?;
    drop(s);
    note(&app, "ok", Src::Window, &tr!("Profil « {name} » remplacé par l'affichage actuel", "Profile “{name}” replaced with the current setup"));
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_rename(app: AppHandle, id: String, name: String) -> Res<()> {
    let name = clean_name(&name)?;
    let mut s = store(&app);
    let p = s.get_mut(&id)?;
    let old = std::mem::replace(&mut p.name, name.clone());
    s.save()?;
    drop(s);
    note(&app, "ok", Src::Window, &tr!("Profil « {old} » renommé « {name} »", "Profile “{old}” renamed “{name}”"));
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_delete(app: AppHandle, id: String) -> Res<()> {
    let mut s = store(&app);
    let name = s.get_mut(&id)?.name.clone();
    s.profiles.retain(|p| p.id != id);
    s.save()?;
    drop(s);
    register_hotkeys(&app);
    note(&app, "ok", Src::Window, &tr!("Profil « {name} » supprimé", "Profile “{name}” deleted"));
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_move(app: AppHandle, id: String, to: usize) -> Res<()> {
    let mut s = store(&app);
    let from = s.profiles.iter().position(|p| p.id == id).ok_or(t("Profil introuvable", "Profile not found"))?;
    let p = s.profiles.remove(from);
    let to = to.min(s.profiles.len());
    s.profiles.insert(to, p);
    s.save()?;
    drop(s);
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
    note(&app, "ok", Src::Window, &what);
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
    note(&app, "ok", Src::Window, &what);
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_auto(app: AppHandle, id: String, on: bool) -> Res<()> {
    let mut s = store(&app);
    let p = s.get_mut(&id)?;
    if on && p.screens.is_empty() {
        return Err(t("Profil trop ancien : remplacez-le d'abord par l'affichage actuel", "Profile too old. Replace it with the current setup first").into());
    }
    p.auto = on;
    s.save()?;
    drop(s);
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_include(app: AppHandle, id: String, setting: String, on: bool) -> Res<()> {
    if !store::EXTRAS.contains(&setting.as_str()) {
        return Err(t("Réglage inconnu", "Unknown setting").into());
    }
    let mut s = store(&app);
    let p = s.get_mut(&id)?;
    if on {
        p.exclude.remove(&setting);
    } else {
        p.exclude.insert(setting);
    }
    s.save()?;
    drop(s);
    changed(&app);
    Ok(())
}

#[tauri::command]
async fn profile_shortcut(app: AppHandle, id: String) -> Res<String> {
    let p = store(&app).get_mut(&id)?.clone();
    let path = store::desktop_shortcut(&p)?;
    note(&app, "ok", Src::Window, &tr!("Raccourci du profil « {} » créé sur le bureau", "Desktop shortcut created for profile “{}”", p.name));
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
    let mut s = store(&app);
    s.tray = name;
    s.save()?;
    drop(s);
    if let Some(tray) = app.tray_by_id("tray") {
        let _ = tray.set_icon(Some(tray_icon(&app)));
    }
    Ok(())
}

#[tauri::command]
async fn set_confirm(app: AppHandle, on: bool, secs: u64) -> Res<()> {
    let mut s = store(&app);
    s.confirm = store::Confirm { on, secs: secs.clamp(5, 120) };
    s.save()
}

// ---------------------------------------------------------------- logique

fn state_of(app: &AppHandle) -> &App {
    app.state::<App>().inner()
}

fn store(app: &AppHandle) -> MutexGuard<'_, store::Store> {
    lock(&state_of(app).store)
}

/// Applique les préférences DDC/CI au moteur d'affichage.
fn retune(app: &AppHandle) {
    display::tune(store(app).screens.iter().map(|(k, s)| (k.clone(), s.tune())).collect());
}

fn feature_range(sc: &store::Screen, code: u8) -> display::Range {
    sc.features.iter().find(|f| f.code == code).map_or(display::Range::FULL, |f| f.range)
}

/// Réglages liés à la luminosité : ils la suivent jusqu'à leur seuil, puis restent au maximum.
fn follow(app: &AppHandle, key: &str, pct: u32) {
    let Some(sc) = store(app).screens.get(key).cloned() else { return };
    for f in sc.features.iter().filter(|f| f.linked && !sc.discrete(f.code) && f.code != display::VCP_POWER) {
        let _ = display::vcp_level(key, f.code, pct.min(f.stop) * 100 / f.stop.max(1), f.range);
    }
}

/// Raccourcis de luminosité : tous les écrans réglables montent ou baissent d'un pas.
fn nudge(app: &AppHandle, up: bool) {
    let step = store(app).panel.step;
    let app = app.clone();
    thread::spawn(move || {
        for m in display::monitors().unwrap_or_default().into_iter().filter(|m| m.active) {
            let Some(cur) = m.brightness else { continue };
            let pct = if up { (cur + step).min(100) } else { cur.saturating_sub(step) };
            if pct != cur && display::set_brightness(&m.key, pct).is_ok() {
                follow(&app, &m.key, pct);
            }
        }
        changed(&app);
    });
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

/// Valeurs d'un réglage facultatif enregistrées dans un profil, écran par écran.
fn stored(s: &display::Snapshot, id: &str) -> String {
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
fn note(app: &AppHandle, level: &str, src: Src, text: &str) {
    note_with(app, level, src, text, false);
}

fn note_with(app: &AppHandle, level: &str, src: Src, text: &str, undo: bool) {
    let e = journal::write_with(level, src.label(), text, undo);
    let _ = app.emit("activity", &e);
    if level != "info" {
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
                note_with(app, "ok", src, what, undoable);
            } else {
                note_with(app, "warn", src, &format!("{what}. {warn}"), undoable);
            }
        }
        Err(e) => note(app, "error", src, &tr!("{what} : échec. {e}", "{what}: failed. {e}")),
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
    let (name, snap, extras) = {
        let mut s = store(app);
        let p = s.get_mut(id)?;
        (p.name.clone(), p.snap.clone(), p.extras())
    };
    let what = match src {
        Src::Plug => tr!("Profil « {name} » appliqué au branchement", "Profile “{name}” applied on connect"),
        _ => tr!("Profil « {name} » appliqué", "Profile “{name}” applied"),
    };
    act(app, src, &what, false, || display::restore(&snap, extras).map(missing_text))
}

fn undo_last(app: &AppHandle, src: Src) -> Res<()> {
    let Some((snap, what)) = lock(&state_of(app).undo).take() else {
        return Err(t("Rien à annuler", "Nothing to undo").into());
    };
    act(app, src, &tr!("Annulé : {what}", "Undone: {what}"), false, || display::restore(&snap, display::Extras::UNDO).map(|_| String::new())).map(drop)
}

/// Hors de la fenêtre, une erreur ou un avertissement doit rester visible.
fn outcome(app: &AppHandle, r: Res<String>) {
    match r {
        Err(e) => report(app, &e),
        Ok(m) if !m.is_empty() => report(app, &m),
        _ => {}
    }
}

/// Rafraîchit l'interface et le menu de la zone de notification.
fn changed(app: &AppHandle) {
    let _ = app.emit("changed", ());
    if let Some(tray) = app.tray_by_id("tray") {
        let _ = tray.set_icon(Some(tray_icon(app)));
        if let Ok(menu) = tray_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
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
        let changed = display::read_brightness();
        READING.store(false, Ordering::SeqCst);
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
        return f();
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
                Some(t("sans confirmation", "no confirmation"))
            }
        };
        if let Some(w) = app.get_webview_window("confirm") {
            let _ = w.destroy();
        }
        match why.map(|why| (why, display::restore(&snap, display::Extras::UNDO))) {
            None => note(&app, "info", Src::System, t("Changement conservé", "Change kept")),
            Some((why, Ok(_))) => {
                lock(&state_of(&app).undo).take();
                note(&app, "warn", Src::System, &tr!("Affichage précédent rétabli ({why})", "Previous display settings restored ({why})"));
            }
            Some((_, Err(e))) => {
                note(&app, "error", Src::System, &tr!("Retour à l'affichage précédent impossible. {e}", "Can't restore the previous display settings. {e}"));
                report(&app, &tr!("Retour impossible : {e}", "Can't revert: {e}"));
            }
        }
        changed(&app);
    });
    Ok(out)
}

/// Fenêtre de l'application, avec la langue de l'interface dans `window.LANG` (lue
/// par ui/i18n.js). En version portable, les données de WebView2 restent à côté de l'exécutable.
fn window<'a>(app: &'a AppHandle, label: &str, page: &str) -> WebviewWindowBuilder<'a, Wry, AppHandle> {
    let b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .initialization_script(format!("window.LANG = \"{}\";", t("fr", "en")));
    if store::portable() {
        b.data_directory(store::data_dir().join("webview"))
    } else {
        b
    }
}

fn show_confirm(app: &AppHandle, secs: u64) {
    if let Some(w) = app.get_webview_window("confirm") {
        let _ = w.eval(format!("window.SECS = {secs}; restart();"));
        let _ = w.set_focus();
        return;
    }
    let _ = window(app, "confirm", "confirm.html")
        .initialization_script(format!("window.SECS = {secs};"))
        .title(t("Conserver l'affichage ?", "Keep these display settings?"))
        .inner_size(420.0, 188.0)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .center()
        .build();
}

fn show_main(app: &AppHandle) {
    show_main_at(app, None);
}

/// Ouvre la fenêtre principale, sur une vue donnée (« ddc »…) si besoin.
fn show_main_at(app: &AppHandle, view: Option<String>) {
    let view = view.filter(|v| ["layout", "ddc", "activity", "settings"].contains(&v.as_str()));
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(v) = view {
            let _ = w.emit("show-view", v);
        }
        return;
    }
    if OPENING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    // Hors du thread principal : sous Windows, créer une webview depuis un
    // gestionnaire d'événement synchrone peut bloquer.
    thread::spawn(move || {
        let place = store(&app).window.filter(|p| on_screen(&app, p));
        let start = view.map_or(String::new(), |v| format!("window.START_VIEW = \"{v}\";"));
        let built = window(&app, "main", "index.html")
            .initialization_script(start)
            .title("MonitorMG")
            .inner_size(1120.0, 740.0)
            .min_inner_size(900.0, 620.0)
            .center()
            .visible(place.is_none())
            .transparent(true)
            .effects(EffectsBuilder::new().effect(Effect::Mica).build())
            .build();
        match built {
            Ok(w) => {
                if let Some(p) = place {
                    let _ = w.set_position(PhysicalPosition::new(p.x, p.y));
                    let _ = w.set_size(PhysicalSize::new(p.w, p.h));
                    if p.max {
                        let _ = w.maximize();
                    }
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let (a, w2) = (app.clone(), w.clone());
                w.on_window_event(move |e| {
                    if let WindowEvent::CloseRequested { .. } = e {
                        save_place(&a, &w2);
                    }
                });
            }
            Err(e) => {
                journal::write("error", Src::System.label(), &tr!("Fenêtre impossible à ouvrir : {e}", "Can't open the window: {e}"));
                store::alert(&tr!(
                    "La fenêtre de MonitorMG ne peut pas s'ouvrir. Le composant Microsoft Edge WebView2 est peut-être absent ou endommagé.\n\nInstallez-le depuis {WEBVIEW2_URL} puis relancez MonitorMG.\n\nDétail : {e}",
                    "MonitorMG's window can't open. The Microsoft Edge WebView2 component may be missing or damaged.\n\nInstall it from {WEBVIEW2_URL}, then restart MonitorMG.\n\nDetails: {e}"
                ));
            }
        }
        OPENING.store(false, Ordering::SeqCst);
    });
}

/// Clic sur l'icône de notification : ouvre le panneau, ou le referme s'il est affiché.
fn toggle_flyout(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("flyout") {
        let _ = w.destroy();
        return;
    }
    if lock(&FLYOUT_CLOSED).is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
        return;
    }
    let app = app.clone();
    // Hors du thread principal, comme pour la fenêtre principale.
    thread::spawn(move || show_flyout(&app));
}

/// Panneau des écrans (luminosité, réglages DDC/CI, profils), en bas à droite de
/// l'écran où se trouve le pointeur. Sa page l'ajuste à son contenu ; il se ferme dès
/// qu'il perd le focus.
fn show_flyout(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("flyout") {
        let _ = w.destroy();
    }
    let (w, h) = (FLYOUT_W, 320.0);
    let built = window(app, "flyout", "flyout.html")
        .title("MonitorMG")
        .inner_size(w, h)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build();
    let Ok(win) = built else { return };
    let mon = app
        .cursor_position()
        .ok()
        .and_then(|c| app.monitor_from_point(c.x, c.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(mon) = mon {
        let (wa, sf) = (mon.work_area(), mon.scale_factor());
        let x = wa.position.x + wa.size.width as i32 - (w * sf) as i32;
        let y = wa.position.y + wa.size.height as i32 - (h * sf) as i32;
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
    let _ = win.show();
    let _ = win.set_focus();
    let (a, w2) = (app.clone(), win.clone());
    win.on_window_event(move |e| match e {
        WindowEvent::Focused(false) => {
            *lock(&FLYOUT_CLOSED) = Some(Instant::now());
            let _ = w2.destroy();
        }
        WindowEvent::Destroyed => changed(&a),
        _ => {}
    });
}

/// La barre de titre enregistrée est-elle encore sur un écran branché ?
fn on_screen(app: &AppHandle, p: &store::Place) -> bool {
    let (x, y) = (p.x + p.w as i32 / 2, p.y + 16);
    app.available_monitors().unwrap_or_default().iter().any(|m| {
        let (o, s) = (m.position(), m.size());
        x >= o.x && x < o.x + s.width as i32 && y >= o.y && y < o.y + s.height as i32
    })
}

fn save_place(app: &AppHandle, w: &WebviewWindow) {
    let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) else { return };
    let max = w.is_maximized().unwrap_or(false);
    let mut s = store(app);
    s.window = Some(store::Place { x: pos.x, y: pos.y, w: size.width, h: size.height, max });
    let _ = s.save();
}

/// Affiche le numéro de chaque écran actif, comme le bouton « Identifier » de Windows.
fn identify(app: &AppHandle) -> Res<()> {
    let mons = display::monitors()?;
    let mut labels = Vec::new();
    for (i, m) in mons.iter().enumerate().filter(|(_, m)| m.active && m.mirror_of.is_none()) {
        let label = format!("identify{i}");
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.destroy();
        }
        let data = serde_json::json!({ "n": i + 1, "name": m.name, "res": format!("{} × {}", m.w, m.h) });
        let w = window(app, &label, "identify.html")
            .initialization_script(format!("window.ID = {data};"))
            .title("MonitorMG")
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .resizable(false)
            .visible(false)
            .inner_size(300.0, 220.0)
            .build()
            .map_err(|e| e.to_string())?;
        let _ = w.set_position(PhysicalPosition::new(m.x + 48, m.y + 48));
        let _ = w.show();
        labels.push(label);
    }
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(2800));
        for l in labels {
            if let Some(w) = app.get_webview_window(&l) {
                let _ = w.destroy();
            }
        }
    });
    Ok(())
}

// ---------------------------------------------------------------- raccourcis clavier

/// Raccourcis globaux : (propriétaire, combinaison). Le propriétaire est un id de profil,
/// ou `OFF`, `UP`, `DOWN`.
fn hotkeys(s: &store::Store) -> Vec<(String, String)> {
    let actions = [(OFF, &s.off_hotkey), (UP, &s.up_hotkey), (DOWN, &s.down_hotkey)];
    let actions = actions.into_iter().filter_map(|(o, h)| Some((o.to_string(), h.clone()?)));
    s.profiles.iter().filter_map(|p| Some((p.id.clone(), p.hotkey.clone()?))).chain(actions).collect()
}

fn hotkey_slot<'a>(s: &'a mut store::Store, owner: &str) -> Res<&'a mut Option<String>> {
    Ok(match owner {
        OFF => &mut s.off_hotkey,
        UP => &mut s.up_hotkey,
        DOWN => &mut s.down_hotkey,
        id => &mut s.get_mut(id)?.hotkey,
    })
}

/// Attribue ou retire le raccourci global d'un profil ou de l'extinction des écrans.
fn assign_hotkey(app: &AppHandle, owner: &str, hotkey: Option<String>) -> Res<()> {
    if let Some(h) = &hotkey {
        let sc: Shortcut = h.parse().map_err(|_| t("Combinaison non reconnue", "Key combination not recognized").to_string())?;
        let taken = hotkeys(&store(app)).iter().any(|(o, x)| o != owner && x.parse::<Shortcut>().is_ok_and(|x| x.id() == sc.id()));
        if taken {
            return Err(t("Déjà utilisé par un autre raccourci de MonitorMG", "Already used by another MonitorMG shortcut").into());
        }
    }
    *hotkey_slot(&mut store(app), owner)? = hotkey;
    if register_hotkeys(app).iter().any(|o| o == owner) {
        *hotkey_slot(&mut store(app), owner)? = None;
        register_hotkeys(app);
        return Err(t("Cette combinaison est déjà prise par une autre application", "This combination is already taken by another app").into());
    }
    store(app).save()
}

/// Réenregistre les raccourcis globaux ; renvoie les propriétaires dont la combinaison est refusée.
fn register_hotkeys(app: &AppHandle) -> Vec<String> {
    // Copie d'abord : l'enregistrement passe par le thread principal, qui peut
    // lui-même attendre le verrou des profils (gestionnaire de raccourci).
    let keys = hotkeys(&store(app));
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    keys.into_iter().filter(|(_, h)| gs.register(h.as_str()).is_err()).map(|(id, _)| id).collect()
}

fn on_hotkey(app: &AppHandle, sc: &Shortcut) {
    let owner = hotkeys(&store(app)).into_iter().find(|(_, h)| h.parse::<Shortcut>().is_ok_and(|h| h.id() == sc.id()));
    match owner {
        Some((o, _)) if o == OFF => screens_off(app, Src::Hotkey),
        Some((o, _)) if o == UP || o == DOWN => nudge(app, o == UP),
        Some((id, _)) => {
            let app = app.clone();
            thread::spawn(move || outcome(&app, apply_profile(&app, Src::Hotkey, &id)));
        }
        None => {}
    }
}

/// Éteint les écrans comme la mise en veille de l'affichage de Windows : une touche ou
/// un mouvement de souris les rallume. On attend d'abord que touches et boutons soient
/// relâchés, sinon leur relâchement les rallumerait aussitôt.
/// Au choix, ou en plus, le signal DDC/CI coupe l'alimentation des écrans externes.
fn screens_off(app: &AppHandle, src: Src) {
    note(app, "ok", src, t("Écrans éteints", "Displays turned off"));
    let panel = store(app).panel.clone();
    thread::spawn(move || {
        if panel.off != "soft" {
            display::power_off_all(panel.power_values());
            if panel.off == "hard" {
                return;
            }
        }
        let held = || (1..=0xFE).any(|vk| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0);
        let start = Instant::now();
        while held() && start.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(Duration::from_millis(300));
        let h = HWND(WATCHER.load(Ordering::SeqCst) as *mut _);
        // 2 : écrans éteints (1 : économie d'énergie, -1 : allumés).
        let _ = unsafe { PostMessageW(Some(h), WM_SYSCOMMAND, WPARAM(SC_MONITORPOWER as usize), LPARAM(2)) };
    });
}

// ---------------------------------------------------------------- zone de notification

/// Icônes proposées pour la zone de notification : nom, glyphe clair (barre des tâches
/// sombre), glyphe foncé (barre claire). La première est celle par défaut.
const TRAY_ICONS: [(&str, &[u8], &[u8]); 5] = [
    ("ecrans", include_bytes!("../icons/tray/ecrans-light.png"), include_bytes!("../icons/tray/ecrans-dark.png")),
    ("moniteur", include_bytes!("../icons/tray/moniteur-light.png"), include_bytes!("../icons/tray/moniteur-dark.png")),
    ("double", include_bytes!("../icons/tray/double-light.png"), include_bytes!("../icons/tray/double-dark.png")),
    ("luminosite", include_bytes!("../icons/tray/luminosite-light.png"), include_bytes!("../icons/tray/luminosite-dark.png")),
    ("pastille", include_bytes!("../icons/tray/pastille.png"), include_bytes!("../icons/tray/pastille.png")),
];

fn tray_name(app: &AppHandle) -> &'static str {
    let chosen = store(app).tray.clone();
    TRAY_ICONS.iter().find(|i| i.0 == chosen).unwrap_or(&TRAY_ICONS[0]).0
}

fn tray_icon(app: &AppHandle) -> Image<'static> {
    let name = tray_name(app);
    let (_, light, dark) = TRAY_ICONS.iter().find(|i| i.0 == name).unwrap_or(&TRAY_ICONS[0]);
    Image::from_bytes(if store::light_taskbar() { dark } else { light }).expect("icône de notification intégrée")
}

fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let mons = display::monitors().unwrap_or_default();
    let sig = display::signature().unwrap_or_default();
    let mut menu = MenuBuilder::new(app);

    let profiles = store(app).profiles.clone();
    if profiles.is_empty() {
        menu = menu.item(&MenuItemBuilder::with_id("open", t("Aucun profil enregistré", "No saved profiles")).enabled(false).build(app)?);
    }
    for p in &profiles {
        let item = || CheckMenuItemBuilder::with_id(format!("profile|{}", p.id), &p.name).checked(p.sig == sig);
        let built = match &p.hotkey {
            Some(h) => item().accelerator(h).build(app).or_else(|_| item().build(app))?,
            None => item().build(app)?,
        };
        menu = menu.item(&built);
    }
    let undo = lock(&state_of(app).undo).as_ref().map(|u| u.1.clone());
    if let Some(u) = undo {
        let short: String = u.chars().take(48).collect();
        menu = menu.separator().item(&MenuItemBuilder::with_id("undo", tr!("Annuler : {short}", "Undo: {short}")).build(app)?);
    }
    menu = menu.separator();

    let actives = mons.iter().filter(|m| m.active).count();
    for m in &mons {
        if !m.active {
            menu = menu.item(&MenuItemBuilder::with_id(format!("on|{}", m.key), tr!("Activer {}", "Enable {}", m.name)).build(app)?);
            continue;
        }
        let label = match &m.mirror_of {
            Some(_) => tr!("{}\tdupliqué", "{}\tduplicated", m.name),
            None => format!("{}\t{} × {} · {} Hz", m.name, m.w, m.h, m.hz),
        };
        let mut res = SubmenuBuilder::new(app, t("Résolution", "Resolution"));
        for md in m.modes.iter().take(20) {
            let id = format!("res|{}|{}|{}", m.key, md.w, md.h);
            res = res.item(&CheckMenuItemBuilder::with_id(id, format!("{} × {}", md.w, md.h)).checked(md.w == m.w && md.h == m.h).build(app)?);
        }
        let mut hz = SubmenuBuilder::new(app, t("Fréquence", "Refresh rate"));
        for r in m.modes.iter().find(|md| md.w == m.w && md.h == m.h).map(|md| md.rates.as_slice()).unwrap_or_default() {
            hz = hz.item(&CheckMenuItemBuilder::with_id(format!("hz|{}|{r}", m.key), format!("{r} Hz")).checked(*r == m.hz).build(app)?);
        }
        let mut sub = SubmenuBuilder::new(app, label);
        // Un menu natif ne peut pas contenir de curseur : des paliers, et un panneau
        // flottant pour le réglage fin.
        if let Some(cur) = m.brightness {
            let mut b = SubmenuBuilder::new(app, tr!("Luminosité\t{cur}{NNBSP}%", "Brightness\t{cur}%"));
            for s in BRIGHTNESS_STEPS {
                let id = format!("bright|{}|{s}", m.key);
                b = b.item(&CheckMenuItemBuilder::with_id(id, tr!("{s}{NNBSP}%", "{s}%")).checked(s == cur).build(app)?);
            }
            let fine = MenuItemBuilder::with_id(format!("slider|{}", m.key), t("Réglage précis…", "Fine-tune…")).build(app)?;
            sub = sub.item(&b.separator().item(&fine).build()?).separator();
        }
        let sub = sub
            .item(&res.build()?)
            .item(&hz.build()?)
            .separator()
            .item(&CheckMenuItemBuilder::with_id(format!("primary|{}", m.key), t("Écran principal", "Main display")).checked(m.primary).enabled(!m.primary).build(app)?)
            .item(&MenuItemBuilder::with_id(format!("off|{}", m.key), t("Désactiver", "Disable")).enabled(actives > 1).build(app)?)
            .build()?;
        menu = menu.item(&sub);
    }

    let off = || MenuItemBuilder::with_id("screens_off", t("Éteindre les écrans", "Turn displays off"));
    let off = match store(app).off_hotkey.clone() {
        Some(h) => off().accelerator(&h).build(app).or_else(|_| off().build(app))?,
        None => off().build(app)?,
    };
    let topo = display::topology();
    let mut proj = SubmenuBuilder::new(app, "Projection");
    for (kind, text) in [
        ("extend", t("Étendre", "Extend")),
        ("clone", t("Dupliquer", "Duplicate")),
        ("internal", t("Écran du PC uniquement", "PC screen only")),
        ("external", t("Deuxième écran uniquement", "Second screen only")),
    ] {
        proj = proj.item(&CheckMenuItemBuilder::with_id(format!("topo|{kind}"), text).checked(topo == Some(kind)).build(app)?);
    }
    menu.separator()
        .item(&proj.build()?)
        .item(&MenuItemBuilder::with_id("identify", t("Identifier les écrans", "Identify displays")).build(app)?)
        .item(&off)
        .separator()
        .item(&MenuItemBuilder::with_id("open", t("Ouvrir MonitorMG", "Open MonitorMG")).build(app)?)
        .item(&MenuItemBuilder::with_id("quit", t("Quitter", "Quit")).build(app)?)
        .build()
}

fn menu_action(app: &AppHandle, id: &str) {
    let mon = |key: &str| display::monitors().ok().and_then(|v| v.into_iter().find(|m| m.key == key)).ok_or(t("Écran introuvable", "Display not found").to_string());
    let parts: Vec<&str> = id.split('|').collect();
    let r: Res<String> = match parts.as_slice() {
        ["profile", pid] => apply_profile(app, Src::Menu, pid),
        ["undo"] => undo_last(app, Src::Menu).map(|()| String::new()),
        ["on", key] => from_menu(app, &tr!("{} activé", "{} enabled", display::name(key)), true, || display::set_enabled(key, true)),
        ["off", key] => from_menu(app, &tr!("{} désactivé", "{} disabled", display::name(key)), true, || display::set_enabled(key, false)),
        ["primary", key] => {
            from_menu(app, &tr!("{} devient l'écran principal", "{} is now the main display", display::name(key)), false, || display::set_primary(key))
        }
        ["res", key, w, h] => mon(key).and_then(|m| {
            let (w, h) = (w.parse().unwrap_or(m.w), h.parse().unwrap_or(m.h));
            let rates = m.modes.iter().find(|md| md.w == w && md.h == h).map(|md| md.rates.clone()).unwrap_or_default();
            // On garde la fréquence actuelle si elle existe dans la nouvelle résolution.
            let hz = if rates.contains(&m.hz) { m.hz } else { rates.first().copied().unwrap_or(m.hz) };
            from_menu(app, &tr!("{} : {w} × {h} à {hz} Hz", "{}: {w} × {h} at {hz} Hz", m.name), true, || display::set_mode(key, w, h, hz))
        }),
        ["hz", key, hz] => mon(key).and_then(|m| {
            let hz = hz.parse().unwrap_or(m.hz);
            from_menu(app, &tr!("{} : {} × {} à {hz} Hz", "{}: {} × {} at {hz} Hz", m.name, m.w, m.h), true, || display::set_mode(key, m.w, m.h, hz))
        }),
        ["topo", kind] => from_menu(app, &tr!("Projection : {}", "Projection: {}", topology_label(kind)), true, || display::set_topology(kind)),
        ["bright", key, pct] => {
            let pct: u32 = pct.parse().unwrap_or(50);
            display::set_brightness(key, pct).map(|()| {
                follow(app, key, pct);
                note(app, "ok", Src::Menu, &tr!("{} : luminosité {pct}{NNBSP}%", "{}: brightness {pct}%", display::name(key)));
                changed(app);
                String::new()
            })
        }
        ["slider", _] => {
            show_flyout(app);
            return;
        }
        ["identify"] => identify(app).map(|()| String::new()),
        ["screens_off"] => {
            screens_off(app, Src::Menu);
            return;
        }
        ["open"] => {
            show_main(app);
            return;
        }
        ["quit"] => {
            journal::write("info", Src::System.label(), t("MonitorMG fermé", "MonitorMG closed"));
            app.exit(0);
            return;
        }
        _ => return,
    };
    outcome(app, r);
}

// ---------------------------------------------------------------- changements d'écran

/// Fenêtre invisible qui écoute les branchements et changements de mode.
/// (Les fenêtres « message-only » ne reçoivent pas ces diffusions.)
fn watch_displays(app: AppHandle) {
    let _ = APP.set(app);
    *lock(&SCREENS) = display::connected().unwrap_or_default();
    thread::spawn(|| unsafe {
        let class = w!("MonitorMGWatcher");
        let inst = GetModuleHandleW(None).unwrap_or_default();
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(watch_proc), hInstance: inst.into(), lpszClassName: class, ..Default::default() });
        if let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE::default(), class, w!(""), WINDOW_STYLE::default(), 0, 0, 0, 0, None, None, Some(inst.into()), None) {
            WATCHER.store(h.0 as isize, Ordering::SeqCst);
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
}

unsafe extern "system" fn watch_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let display = msg == WM_DISPLAYCHANGE || (msg == WM_DEVICECHANGE && wp.0 == DBT_DEVNODES_CHANGED as usize);
    // WM_SETTINGCHANGE est très fréquent : seul le thème (icône, accent) nous concerne.
    let theme = msg == WM_SETTINGCHANGE
        && lp.0 != 0
        && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet");
    if display || theme {
        if display {
            DISPLAY_DIRTY.store(true, Ordering::SeqCst);
        }
        // Windows envoie ces messages en rafale : on attend le calme.
        let t = TICK.fetch_add(1, Ordering::SeqCst) + 1;
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            if TICK.load(Ordering::SeqCst) != t {
                return;
            }
            if let Some(app) = APP.get() {
                if DISPLAY_DIRTY.swap(false, Ordering::SeqCst) {
                    display::forget_modes();
                    plugged(app);
                    read_brightness(app, true);
                }
                changed(app);
            }
        });
    }
    DefWindowProcW(h, msg, wp, lp)
}

/// Consigne les écrans branchés ou débranchés et, pour un dock, applique le premier
/// profil automatique dont les écrans correspondent exactement.
fn plugged(app: &AppHandle) {
    let Ok(now) = display::connected() else { return };
    let before = std::mem::replace(&mut *lock(&SCREENS), now.clone());
    if before == now {
        return;
    }
    for k in now.difference(&before) {
        note(app, "ok", Src::System, &tr!("{} branché", "{} connected", display::name(k)));
    }
    match before.difference(&now).count() {
        0 => {}
        1 => note(app, "ok", Src::System, t("Un écran a été débranché", "A display was disconnected")),
        n => note(app, "ok", Src::System, &tr!("{n} écrans ont été débranchés", "{n} displays were disconnected")),
    }
    let found = store(app).profiles.iter().find(|p| p.auto && p.screens == now).map(|p| (p.id.clone(), p.sig.clone()));
    let Some((id, sig)) = found else { return };
    // Windows a souvent déjà rétabli cet affichage : on évite un clignotement.
    if display::signature().is_ok_and(|s| s == sig) {
        return;
    }
    // Action automatique : consignée dans l'activité, sans boîte de dialogue.
    let _ = apply_profile(app, Src::Plug, &id);
}

// ---------------------------------------------------------------- démarrage

/// Raccourci bureau : applique le profil sans ouvrir l'interface.
fn apply_from_desktop(id: &str) {
    let desk = t("Bureau", "Desktop");
    let s = store::Store::load();
    let Some(p) = s.profiles.iter().find(|p| p.id == id || p.name == id) else {
        let e = tr!("Profil « {id} » introuvable", "Profile “{id}” not found");
        journal::write("error", desk, &e);
        store::alert(&e);
        return;
    };
    let what = tr!("Profil « {} » appliqué", "Profile “{}” applied", p.name);
    match display::restore(&p.snap, p.extras()) {
        Ok(0) => {
            journal::write("ok", desk, &what);
        }
        Ok(n) => {
            let m = format!("{what}. {}", missing_text(n));
            journal::write("warn", desk, &m);
            store::alert(&m);
        }
        Err(e) => {
            journal::write("error", desk, &tr!("{what} : échec. {e}", "{what}: failed. {e}"));
            store::alert(&tr!("Le profil « {} » n'a pas pu être appliqué.\n\n{e}", "Profile “{}” couldn't be applied.\n\n{e}", p.name));
        }
    }
}

fn log_startup() {
    let mode = if store::portable() { " (portable)" } else { "" };
    journal::write("info", Src::System.label(), &tr!("MonitorMG {VERSION}{mode} démarré, {}", "MonitorMG {VERSION}{mode} started, {}", store::windows_version()));
    if let Ok(ms) = display::monitors() {
        let list: Vec<String> = ms
            .iter()
            .map(|m| match m.active {
                true => format!("{} {}×{} {} Hz ({})", m.name, m.w, m.h, m.hz, m.connector),
                false => tr!("{} désactivé ({})", "{} disabled ({})", m.name, m.connector),
            })
            .collect();
        journal::write("info", Src::System.label(), &tr!("Écrans : {}", "Displays: {}", list.join(", ")));
    }
}

fn main() {
    // Une erreur interne laisse une trace dans le journal, même sans console.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        journal::write("error", Src::System.label(), &tr!("Erreur interne : {info}", "Internal error: {info}"));
        default_hook(info);
    }));

    let args: Vec<String> = std::env::args().collect();
    if let Some(id) = args.iter().position(|a| a == "--profile").and_then(|i| args.get(i + 1)) {
        apply_from_desktop(id);
        return;
    }
    let hidden = args.iter().any(|a| a == "--minimized");

    let built = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show_main(app)))
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
            profile_include,
            profile_shortcut,
            set_autostart,
            set_confirm,
            set_tray_icon,
        ])
        .setup(move |app| {
            let h = app.handle().clone();
            log_startup();
            TrayIconBuilder::with_id("tray")
                .icon(tray_icon(&h))
                .tooltip("MonitorMG")
                .menu(&tray_menu(&h)?)
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
                journal::write("warn", Src::System.label(), &tr!("{} raccourci(s) clavier refusé(s) par Windows", "{} keyboard shortcut(s) refused by Windows", refused.len()));
            }
            retune(&h);
            read_brightness(&h, true);
            watch_displays(h.clone());
            if !hidden {
                show_main(&h);
            }
            Ok(())
        })
        .build(tauri::generate_context!());

    match built {
        // Fermer la fenêtre ne quitte pas : l'application reste dans la zone de notification.
        Ok(app) => app.run(|_, e| {
            if let RunEvent::ExitRequested { code: None, api, .. } = e {
                api.prevent_exit();
            }
        }),
        Err(e) => {
            journal::write("error", Src::System.label(), &tr!("Démarrage impossible : {e}", "Can't start: {e}"));
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
    fn missing_screens_are_worded() {
        assert_eq!(missing_text(0), "");
        assert_eq!(missing_text(2), tr!("2 écrans du profil ne sont pas branchés", "2 displays from the profile are not connected"));
    }
}
