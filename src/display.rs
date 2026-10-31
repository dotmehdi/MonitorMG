//! Moteur d'affichage. Topologie, disposition et profils via l'API CCD
//! (QueryDisplayConfig / SetDisplayConfig) ; résolution, fréquence et orientation
//! via ChangeDisplaySettingsEx, seule API qui accepte une fréquence précise.

use crate::store;
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::Entry, BTreeMap, BTreeSet, HashMap, HashSet};
use std::mem::size_of;
use std::sync::{Mutex, MutexGuard, PoisonError};
use windows::core::{w, BOOL, BSTR, HRESULT, PCWSTR};
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, HANDLE, LPARAM, LUID, POINTL, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::*;

pub type Res<T> = Result<T, String>;

const INVALID: u32 = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
const APPLY: SET_DISPLAY_CONFIG_FLAGS = SET_DISPLAY_CONFIG_FLAGS(
    SDC_APPLY.0 | SDC_USE_SUPPLIED_DISPLAY_CONFIG.0 | SDC_SAVE_TO_DATABASE.0 | SDC_ALLOW_CHANGES.0,
);
fn not_active() -> &'static str {
    store::t("Cet écran n'est pas actif", "This display is not active")
}

/// Une seule modification d'affichage à la fois : fenêtre, menu, raccourci et
/// branchement peuvent agir en même temps.
static WRITE: Mutex<()> = Mutex::new(());

/// Verrou tolérant : une erreur interne sous verrou ne bloque pas les appels suivants.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// Requêtes DisplayConfigGetDeviceInfo. Les valeurs négatives (échelle) ne sont pas
// documentées mais sont celles qu'utilise la page Paramètres de Windows.
const GET_TARGET_NAME: i32 = 2;
const GET_PREFERRED_MODE: i32 = 3;
const GET_ADAPTER_NAME: i32 = 4;
const GET_SOURCE_NAME: i32 = 1;
const GET_COLOR_INFO: i32 = 9;
const SET_COLOR_STATE: i32 = 10;
const GET_COLOR_INFO_2: i32 = 15;
const SET_HDR_STATE: i32 = 16;
const GET_DPI_SCALE: i32 = -3;
const SET_DPI_SCALE: i32 = -4;
const DPI_STEPS: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

#[repr(C)]
struct DpiGet {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    min: i32,
    cur: i32,
    max: i32,
}

#[repr(C)]
struct ColorInfo {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    bits: u32,
    encoding: u32,
    bpc: u32,
}

#[repr(C)]
struct ColorInfo2 {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    bits: u32,
    encoding: u32,
    bpc: u32,
    mode: u32,
}

/// Paquet "SET" à un seul champ (échelle, HDR).
#[repr(C)]
struct SetValue {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    value: i32,
}

#[derive(Serialize, Clone, Debug)]
pub struct Mode {
    pub w: u32,
    pub h: u32,
    pub rates: Vec<u32>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Scale {
    pub current: u32,
    pub recommended: u32,
    pub options: Vec<u32>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Monitor {
    pub key: String,
    pub name: String,
    pub connector: &'static str,
    pub internal: bool,
    pub active: bool,
    pub primary: bool,
    pub mirror_of: Option<String>,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub hz: u32,
    pub rotation: u32,
    pub native: Option<(u32, u32)>,
    pub modes: Vec<Mode>,
    pub scale: Option<Scale>,
    pub hdr: Option<bool>,
    pub brightness: Option<u32>,
}

/// Vignette d'un écran dans un profil, pour l'aperçu.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Tile {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub hz: u32,
    pub primary: bool,
}

/// Configuration CCD brute, sérialisée en hexadécimal pour rester fidèle à l'octet près.
/// Les LUID d'adaptateur changent à chaque démarrage : on garde le chemin du
/// périphérique de chaque adaptateur pour les retrouver au moment d'appliquer.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Snapshot {
    pub paths: Vec<String>,
    pub modes: Vec<String>,
    pub adapters: BTreeMap<String, String>,
    #[serde(default)]
    pub scales: BTreeMap<String, u32>,
    #[serde(default)]
    pub hdr: BTreeMap<String, bool>,
    #[serde(default)]
    pub brightness: BTreeMap<String, u32>,
}

/// Réglages appliqués après la disposition. Un profil peut en exclure certains :
/// le réglage actuel de l'écran est alors conservé.
#[derive(Clone, Copy)]
pub struct Extras {
    pub scale: bool,
    pub hdr: bool,
    pub brightness: bool,
}

impl Extras {
    /// Annuler ou rétablir : la luminosité n'a pas été touchée par la modification.
    pub const UNDO: Extras = Extras { scale: true, hdr: true, brightness: false };
}

#[derive(Clone)]
pub struct Config {
    pub paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    pub modes: Vec<DISPLAYCONFIG_MODE_INFO>,
}

/// Clé stable d'un écran : chemin de l'adaptateur + identifiant du connecteur.
#[derive(Default)]
pub struct Keys(HashMap<u64, String>);

impl Keys {
    pub fn adapter(&mut self, l: LUID) -> String {
        self.0
            .entry(luid64(l))
            .or_insert_with(|| {
                info::<DISPLAYCONFIG_ADAPTER_NAME>(GET_ADAPTER_NAME, l, 0)
                    .map(|n| wstr(&n.adapterDevicePath))
                    .unwrap_or_else(|| luid_hex(l))
            })
            .clone()
    }

    pub fn key(&mut self, l: LUID, target: u32) -> String {
        format!("{}#{target}", self.adapter(l))
    }

    fn of(&mut self, p: &DISPLAYCONFIG_PATH_INFO) -> String {
        self.key(p.targetInfo.adapterId, p.targetInfo.id)
    }
}

// ---------------------------------------------------------------- bas niveau

fn luid64(l: LUID) -> u64 {
    ((l.HighPart as u32 as u64) << 32) | l.LowPart as u64
}

fn luid_hex(l: LUID) -> String {
    format!("{:016x}", luid64(l))
}

pub fn wstr(s: &[u16]) -> String {
    String::from_utf16_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())])
}

fn win_err(code: u32) -> String {
    windows::core::Error::from_hresult(HRESULT::from_win32(code)).message()
}

fn hex<T: Copy>(v: &T) -> String {
    // Les structures CCD n'ont pas d'octets de remplissage : lecture brute sûre.
    let b = unsafe { std::slice::from_raw_parts(v as *const T as *const u8, size_of::<T>()) };
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex<T: Copy>(s: &str) -> Option<T> {
    if s.len() != size_of::<T>() * 2 {
        return None;
    }
    let b = (0..s.len())
        .step_by(2)
        .map(|i| s.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()))
        .collect::<Option<Vec<u8>>>()?;
    Some(unsafe { std::ptr::read_unaligned(b.as_ptr() as *const T) })
}

/// Toutes les structures d'info commencent par l'en-tête : on le remplit puis on interroge.
fn info<T>(kind: i32, adapter: LUID, id: u32) -> Option<T> {
    let mut v: T = unsafe { std::mem::zeroed() };
    let h = &mut v as *mut T as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER;
    unsafe {
        *h = header::<T>(kind, adapter, id);
        (DisplayConfigGetDeviceInfo(h) == 0).then_some(v)
    }
}

fn header<T>(kind: i32, adapter: LUID, id: u32) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: DISPLAYCONFIG_DEVICE_INFO_TYPE(kind),
        size: size_of::<T>() as u32,
        adapterId: adapter,
        id,
    }
}

fn set_value(kind: i32, adapter: LUID, id: u32, value: i32) -> Res<()> {
    let v = SetValue { header: header::<SetValue>(kind, adapter, id), value };
    match unsafe { DisplayConfigSetDeviceInfo(&v.header) } {
        0 => Ok(()),
        e => Err(win_err(e as u32)),
    }
}

pub fn query(all: bool) -> Res<Config> {
    let flags = if all { QDC_ALL_PATHS } else { QDC_ONLY_ACTIVE_PATHS };
    loop {
        let (mut np, mut nm) = (0u32, 0u32);
        unsafe { GetDisplayConfigBufferSizes(flags, &mut np, &mut nm) }
            .ok()
            .map_err(|e| e.message())?;
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        let r = unsafe {
            QueryDisplayConfig(flags, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
        };
        if r == ERROR_INSUFFICIENT_BUFFER {
            continue; // un écran est apparu entre les deux appels
        }
        r.ok().map_err(|e| e.message())?;
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        return Ok(Config { paths, modes });
    }
}

fn apply(cfg: &Config, flags: SET_DISPLAY_CONFIG_FLAGS) -> Res<()> {
    let modes = (!cfg.modes.is_empty()).then_some(cfg.modes.as_slice());
    match unsafe { SetDisplayConfig(Some(&cfg.paths), modes, flags) } {
        0 => Ok(()),
        e => Err(win_err(e as u32)),
    }
}

fn is_active(p: &DISPLAYCONFIG_PATH_INFO) -> bool {
    p.flags & DISPLAYCONFIG_PATH_ACTIVE != 0
}

fn src_idx(p: &DISPLAYCONFIG_PATH_INFO) -> u32 {
    unsafe { p.sourceInfo.Anonymous.modeInfoIdx }
}

fn tgt_idx(p: &DISPLAYCONFIG_PATH_INFO) -> u32 {
    unsafe { p.targetInfo.Anonymous.modeInfoIdx }
}

fn source_mode(cfg: &Config, p: &DISPLAYCONFIG_PATH_INFO) -> Option<DISPLAYCONFIG_SOURCE_MODE> {
    cfg.modes
        .get(src_idx(p) as usize)
        .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
        .map(|m| unsafe { m.Anonymous.sourceMode })
}

fn target_hz(cfg: &Config, p: &DISPLAYCONFIG_PATH_INFO) -> u32 {
    cfg.modes
        .get(tgt_idx(p) as usize)
        .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET)
        .map(|m| unsafe { m.Anonymous.targetMode.targetVideoSignalInfo.vSyncFreq })
        .filter(|f| f.Denominator != 0)
        .map_or(0, |f| (f.Numerator as f64 / f.Denominator as f64).round() as u32)
}

fn shift(cfg: &mut Config, dx: i32, dy: i32) {
    for m in cfg.modes.iter_mut().filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE) {
        let s = unsafe { &mut m.Anonymous.sourceMode };
        s.position.x -= dx;
        s.position.y -= dy;
    }
}

/// Windows considère comme principal l'écran placé en (0,0) : il en faut toujours un.
fn ensure_primary(cfg: &mut Config) {
    let pos: Vec<POINTL> = cfg.paths.iter().filter_map(|p| source_mode(cfg, p)).map(|m| m.position).collect();
    if let Some(o) = pos.first().copied() {
        if !pos.iter().any(|p| p.x == 0 && p.y == 0) {
            shift(cfg, o.x, o.y);
        }
    }
}

/// Ne garde que les modes référencés par les chemins restants et renumérote.
fn compact_modes(cfg: &mut Config) {
    let Config { paths, modes: old } = cfg;
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut modes = Vec::new();
    for p in paths.iter_mut() {
        let idxs = unsafe { [&mut p.sourceInfo.Anonymous.modeInfoIdx, &mut p.targetInfo.Anonymous.modeInfoIdx] };
        for idx in idxs {
            if *idx == INVALID {
                continue;
            }
            *idx = match (map.entry(*idx), old.get(*idx as usize)) {
                (Entry::Occupied(e), _) => *e.get(),
                (Entry::Vacant(e), Some(m)) => {
                    modes.push(*m);
                    *e.insert(modes.len() as u32 - 1)
                }
                (Entry::Vacant(_), None) => INVALID,
            };
        }
    }
    cfg.modes = modes;
}

fn connector(t: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY) -> &'static str {
    match t {
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI => "HDMI",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EXTERNAL => "DisplayPort",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_USB_TUNNEL => "USB-C",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI => "DVI",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HD15 => "VGA",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_MIRACAST => store::t("Sans fil", "Wireless"),
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_WIRED => "USB",
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL => store::t("Virtuel", "Virtual"),
        t if is_internal(t) => store::t("Intégré", "Built-in"),
        _ => store::t("Autre", "Other"),
    }
}

fn is_internal(t: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY) -> bool {
    matches!(
        t,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
    )
}

fn gdi_name(adapter: LUID, source: u32) -> Option<Vec<u16>> {
    let n: DISPLAYCONFIG_SOURCE_DEVICE_NAME = info(GET_SOURCE_NAME, adapter, source)?;
    let len = n.viewGdiDeviceName.iter().position(|&c| c == 0)?;
    Some(n.viewGdiDeviceName[..=len].to_vec())
}

fn devmode(gdi: &[u16], i: u32) -> Option<DEVMODEW> {
    let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    unsafe { EnumDisplaySettingsW(PCWSTR(gdi.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm) }
        .as_bool()
        .then_some(dm)
}

type RawModes = BTreeMap<(u32, u32), Vec<u32>>;

/// Modes du pilote par écran. Les énumérer coûte 50 à 80 ms et ne change qu'au
/// branchement ou à un changement de topologie : on les garde jusque-là.
static MODES: Mutex<BTreeMap<String, RawModes>> = Mutex::new(BTreeMap::new());

pub fn forget_modes() {
    lock(&MODES).clear();
}

fn raw_modes(gdi: &[u16]) -> RawModes {
    let mut map = RawModes::new();
    let mut i = 0;
    while let Some(d) = devmode(gdi, i) {
        i += 1;
        if d.dmBitsPerPel < 24 || d.dmDisplayFrequency <= 1 {
            continue;
        }
        let rates = map.entry((d.dmPelsWidth, d.dmPelsHeight)).or_default();
        if !rates.contains(&d.dmDisplayFrequency) {
            rates.push(d.dmDisplayFrequency);
        }
    }
    map
}

/// Modes proposés par le pilote, groupés par résolution, du plus grand au plus petit.
/// `key` identifie l'écran : un nom GDI (\\.\DISPLAY2) peut passer d'un écran à l'autre.
fn gdi_modes(key: &str, gdi: &[u16], cur: (u32, u32)) -> (Vec<Mode>, u32) {
    let hz = devmode(gdi, ENUM_CURRENT_SETTINGS.0).map_or(0, |d| d.dmDisplayFrequency);
    let map = lock(&MODES).entry(format!("{key}|{}", wstr(gdi))).or_insert_with(|| raw_modes(gdi)).clone();
    // Selon le pilote, la liste suit l'orientation native : on la tourne si besoin.
    let swap = !map.contains_key(&cur) && map.contains_key(&(cur.1, cur.0));
    let mut modes: Vec<Mode> = map
        .into_iter()
        .map(|((w, h), mut rates)| {
            rates.sort_unstable_by(|a, b| b.cmp(a));
            let (w, h) = if swap { (h, w) } else { (w, h) };
            Mode { w, h, rates }
        })
        .collect();
    modes.sort_by_key(|m| std::cmp::Reverse((m.w as u64 * m.h as u64, m.w)));
    (modes, hz)
}

fn scale_info(adapter: LUID, source: u32) -> Option<Scale> {
    let d: DpiGet = info(GET_DPI_SCALE, adapter, source)?;
    // Valeurs relatives à l'échelle recommandée ; le minimum vaut toujours 100 %.
    let rec = -d.min;
    let (cur, max) = (rec + d.cur, (rec + d.max).min(DPI_STEPS.len() as i32 - 1));
    if rec < 0 || rec > max || cur < 0 || cur > max {
        return None;
    }
    Some(Scale {
        current: DPI_STEPS[cur as usize],
        recommended: DPI_STEPS[rec as usize],
        options: DPI_STEPS[..=max as usize].to_vec(),
    })
}

fn apply_scale(adapter: LUID, source: u32, pct: u32) -> Res<()> {
    let d: DpiGet = info(GET_DPI_SCALE, adapter, source).ok_or(store::t("Mise à l'échelle indisponible", "Scaling unavailable"))?;
    let idx = DPI_STEPS.iter().position(|&s| s == pct).ok_or(store::t("Échelle inconnue", "Unknown scale"))? as i32;
    let rel = idx + d.min; // idx - recommandé, avec recommandé = -min
    if rel == d.cur {
        return Ok(());
    }
    set_value(SET_DPI_SCALE, adapter, source, rel)
}

fn hdr_info(adapter: LUID, target: u32) -> Option<bool> {
    if let Some(c) = info::<ColorInfo2>(GET_COLOR_INFO_2, adapter, target) {
        return (c.bits & 1 << 4 != 0).then_some(c.bits & 1 << 5 != 0);
    }
    // Avant Windows 11 24H2 : "advanced color" = HDR.
    let c: ColorInfo = info(GET_COLOR_INFO, adapter, target)?;
    (c.bits & 1 != 0 && c.bits & 1 << 3 == 0).then_some(c.bits & 1 << 1 != 0)
}

fn apply_hdr(adapter: LUID, target: u32, on: bool) -> Res<()> {
    set_value(SET_HDR_STATE, adapter, target, on as i32)
        .or_else(|_| set_value(SET_COLOR_STATE, adapter, target, on as i32))
}

// ---------------------------------------------------------------- lecture

pub fn monitors() -> Res<Vec<Monitor>> {
    let cfg = query(true)?;
    let mut keys = Keys::default();
    let mut seen = HashSet::new();
    let mut sources: HashMap<(u64, u32), String> = HashMap::new();
    let mut out = Vec::new();
    let ordered = cfg.paths.iter().filter(|p| is_active(p)).chain(cfg.paths.iter().filter(|p| !is_active(p)));
    for p in ordered {
        let t = &p.targetInfo;
        if !t.targetAvailable.as_bool() || !seen.insert((luid64(t.adapterId), t.id)) {
            continue;
        }
        let internal = is_internal(t.outputTechnology);
        let name = friendly(t);
        let native = info::<DISPLAYCONFIG_TARGET_PREFERRED_MODE>(GET_PREFERRED_MODE, t.adapterId, t.id)
            .map(|m| (m.width, m.height));
        let mut m = Monitor {
            key: keys.of(p),
            name,
            connector: connector(t.outputTechnology),
            internal,
            active: is_active(p),
            primary: false,
            mirror_of: None,
            x: 0,
            y: 0,
            w: native.map_or(0, |n| n.0),
            h: native.map_or(0, |n| n.1),
            hz: 0,
            rotation: 0,
            native,
            modes: Vec::new(),
            scale: None,
            hdr: None,
            brightness: None,
        };
        if m.active {
            if let Some(sm) = source_mode(&cfg, p) {
                (m.x, m.y, m.w, m.h) = (sm.position.x, sm.position.y, sm.width, sm.height);
            }
            m.rotation = match t.rotation {
                DISPLAYCONFIG_ROTATION_ROTATE90 => 90,
                DISPLAYCONFIG_ROTATION_ROTATE180 => 180,
                DISPLAYCONFIG_ROTATION_ROTATE270 => 270,
                _ => 0,
            };
            let s = &p.sourceInfo;
            match sources.entry((luid64(s.adapterId), s.id)) {
                Entry::Occupied(e) => m.mirror_of = Some(e.get().clone()),
                Entry::Vacant(e) => {
                    e.insert(m.key.clone());
                    m.primary = m.x == 0 && m.y == 0;
                }
            }
            if let Some(gdi) = gdi_name(s.adapterId, s.id) {
                (m.modes, m.hz) = gdi_modes(&m.key, &gdi, (m.w, m.h));
            }
            m.scale = scale_info(s.adapterId, s.id);
            m.hdr = hdr_info(t.adapterId, t.id);
            m.brightness = lock(&LEVELS).get(&m.key).copied();
        }
        out.push(m);
    }
    Ok(out)
}

fn friendly(t: &DISPLAYCONFIG_PATH_TARGET_INFO) -> String {
    info::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(GET_TARGET_NAME, t.adapterId, t.id)
        .map(|n| wstr(&n.monitorFriendlyDeviceName))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| if is_internal(t.outputTechnology) { store::t("Écran intégré", "Built-in display") } else { store::t("Écran", "Display") }.into())
}

/// Nom lisible d'un écran, pour le journal d'activité.
pub fn name(key: &str) -> String {
    let mut keys = Keys::default();
    query(true)
        .ok()
        .and_then(|cfg| cfg.paths.iter().find(|p| keys.of(p) == key).map(|p| friendly(&p.targetInfo)))
        .unwrap_or_else(|| store::t("Écran", "Display").into())
}

pub fn tiles() -> Res<Vec<Tile>> {
    Ok(monitors()?
        .into_iter()
        .filter(|m| m.active && m.mirror_of.is_none())
        .map(|m| Tile { name: m.name, x: m.x, y: m.y, w: m.w, h: m.h, hz: m.hz, primary: m.primary })
        .collect())
}

/// Clés des écrans branchés, actifs ou non.
pub fn connected() -> Res<BTreeSet<String>> {
    let cfg = query(true)?;
    let mut keys = Keys::default();
    Ok(cfg.paths.iter().filter(|p| p.targetInfo.targetAvailable.as_bool()).map(|p| keys.of(p)).collect())
}

/// Empreinte de la configuration active, pour reconnaître le profil en cours.
pub fn signature() -> Res<String> {
    let cfg = query(false)?;
    let mut keys = Keys::default();
    let mut v: Vec<String> = cfg
        .paths
        .iter()
        .map(|p| {
            let m = source_mode(&cfg, p).unwrap_or_default();
            format!("{}@{},{},{}x{},{}", keys.of(p), m.position.x, m.position.y, m.width, m.height, target_hz(&cfg, p))
        })
        .collect();
    v.sort();
    Ok(v.join(";"))
}

/// Préréglage Win+P actif, s'il y en a un.
pub fn topology() -> Option<&'static str> {
    let (mut np, mut nm) = (0u32, 0u32);
    unsafe { GetDisplayConfigBufferSizes(QDC_DATABASE_CURRENT, &mut np, &mut nm) }.ok().ok()?;
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
    let mut topo = DISPLAYCONFIG_TOPOLOGY_ID::default();
    unsafe {
        QueryDisplayConfig(QDC_DATABASE_CURRENT, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), Some(&mut topo))
    }
    .ok()
    .ok()?;
    match topo {
        DISPLAYCONFIG_TOPOLOGY_INTERNAL => Some("internal"),
        DISPLAYCONFIG_TOPOLOGY_CLONE => Some("clone"),
        DISPLAYCONFIG_TOPOLOGY_EXTEND => Some("extend"),
        DISPLAYCONFIG_TOPOLOGY_EXTERNAL => Some("external"),
        _ => None,
    }
}

// ---------------------------------------------------------------- écriture

fn enabled_config(key: &str, on: bool) -> Res<Config> {
    let mut cfg = query(true)?;
    let mut keys = Keys::default();
    let mut active: Vec<DISPLAYCONFIG_PATH_INFO> = cfg.paths.iter().filter(|p| is_active(p)).copied().collect();
    let is_on = active.iter().any(|p| keys.of(p) == key);
    if on && !is_on {
        let used: HashSet<(u64, u32)> =
            active.iter().map(|p| (luid64(p.sourceInfo.adapterId), p.sourceInfo.id)).collect();
        // Un chemin inactif vers cet écran dont la source n'est pas déjà prise.
        let mut p = *cfg
            .paths
            .iter()
            .find(|p| {
                p.targetInfo.targetAvailable.as_bool()
                    && !used.contains(&(luid64(p.sourceInfo.adapterId), p.sourceInfo.id))
                    && keys.of(p) == key
            })
            .ok_or(store::t("Cet écran n'est plus disponible", "This display is no longer available"))?;
        p.flags |= DISPLAYCONFIG_PATH_ACTIVE;
        p.sourceInfo.Anonymous.modeInfoIdx = INVALID;
        p.targetInfo.Anonymous.modeInfoIdx = INVALID;
        active.push(p);
    } else if !on && is_on {
        active.retain(|p| keys.of(p) != key);
        if active.is_empty() {
            return Err(store::t("Impossible de désactiver le dernier écran actif", "Can't disable the last active display").into());
        }
    }
    cfg.paths = active;
    compact_modes(&mut cfg);
    ensure_primary(&mut cfg);
    Ok(cfg)
}

pub fn set_enabled(key: &str, on: bool) -> Res<()> {
    let _w = lock(&WRITE);
    forget_modes();
    apply(&enabled_config(key, on)?, APPLY)
}

fn positions_config(list: &[(String, i32, i32)]) -> Res<Config> {
    let mut cfg = query(false)?;
    let mut keys = Keys::default();
    let primary = cfg.paths.iter().map(src_idx).find(|&i| {
        cfg.modes.get(i as usize).is_some_and(|m| unsafe { m.Anonymous.sourceMode.position } == POINTL::default())
    });
    for p in cfg.paths.clone() {
        let k = keys.of(&p);
        if let (Some((_, x, y)), Some(m)) = (list.iter().find(|(key, ..)| *key == k), cfg.modes.get_mut(src_idx(&p) as usize)) {
            m.Anonymous.sourceMode.position = POINTL { x: *x, y: *y };
        }
    }
    // Le principal reste en (0,0) : on recale tout sur lui.
    if let Some(m) = primary.and_then(|i| cfg.modes.get(i as usize)) {
        let o = unsafe { m.Anonymous.sourceMode.position };
        shift(&mut cfg, o.x, o.y);
    }
    Ok(cfg)
}

pub fn set_positions(list: &[(String, i32, i32)]) -> Res<()> {
    let _w = lock(&WRITE);
    apply(&positions_config(list)?, APPLY)
}

pub fn set_primary(key: &str) -> Res<()> {
    let _w = lock(&WRITE);
    let mut cfg = query(false)?;
    let mut keys = Keys::default();
    let p = *cfg.paths.iter().find(|p| keys.of(p) == key).ok_or_else(not_active)?;
    let o = source_mode(&cfg, &p).ok_or_else(not_active)?.position;
    shift(&mut cfg, o.x, o.y);
    apply(&cfg, APPLY)
}

pub fn set_mode(key: &str, w: u32, h: u32, hz: u32) -> Res<()> {
    change(key, |dm| {
        dm.dmPelsWidth = w;
        dm.dmPelsHeight = h;
        dm.dmDisplayFrequency = hz;
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
    })
}

pub fn set_rotation(key: &str, deg: u32) -> Res<()> {
    change(key, |dm| unsafe {
        let o = &mut dm.Anonymous1.Anonymous2.dmDisplayOrientation;
        let new = (deg / 90) % 4;
        if (o.0 + new) % 2 == 1 {
            std::mem::swap(&mut dm.dmPelsWidth, &mut dm.dmPelsHeight);
        }
        *o = DEVMODE_DISPLAY_ORIENTATION(new);
        dm.dmFields = DM_DISPLAYORIENTATION | DM_PELSWIDTH | DM_PELSHEIGHT;
    })
}

fn check(r: DISP_CHANGE) -> Res<()> {
    if r == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else if r == DISP_CHANGE_BADMODE {
        Err(store::t("Ce mode n'est pas pris en charge par l'écran", "The display doesn't support this mode").into())
    } else if r == DISP_CHANGE_RESTART {
        Err(store::t("Windows demande un redémarrage pour ce réglage", "Windows needs a restart for this setting").into())
    } else {
        Err(tr!("Windows a refusé la modification (code {})", "Windows refused the change (code {})", r.0))
    }
}

/// Modifie un écran via l'API GDI. Si sa taille change, les voisins suivent :
/// l'arête tournée vers l'écran principal reste fixe et ce qui est au-delà se décale.
fn change(key: &str, edit: impl FnOnce(&mut DEVMODEW)) -> Res<()> {
    let _w = lock(&WRITE);
    let cfg = query(false)?;
    let mut keys = Keys::default();
    let p = *cfg.paths.iter().find(|p| keys.of(p) == key).ok_or_else(not_active)?;
    let gdi = gdi_name(p.sourceInfo.adapterId, p.sourceInfo.id).ok_or_else(not_active)?;
    let mut dm = devmode(&gdi, ENUM_CURRENT_SETTINGS.0).ok_or_else(not_active)?;
    let me = source_mode(&cfg, &p).ok_or_else(not_active)?;
    let (ow, oh) = (dm.dmPelsWidth as i32, dm.dmPelsHeight as i32);
    edit(&mut dm);
    let (dw, dh) = (dm.dmPelsWidth as i32 - ow, dm.dmPelsHeight as i32 - oh);
    let (mx, my) = (me.position.x, me.position.y);
    let left = mx + ow <= 0; // entièrement à gauche du principal : on ancre son bord droit
    let above = my + oh <= 0;
    let moved = |x: i32, y: i32, w: i32, h: i32| -> (i32, i32) {
        let nx = match (left, dw) {
            (_, 0) => x,
            (true, _) if x + w <= mx => x - dw,
            (false, _) if x >= mx + ow => x + dw,
            _ => x,
        };
        let ny = match (above, dh) {
            (_, 0) => y,
            (true, _) if y + h <= my => y - dh,
            (false, _) if y >= my + oh => y + dh,
            _ => y,
        };
        (nx, ny)
    };
    if left || above {
        let pos = unsafe { &mut dm.Anonymous1.Anonymous2.dmPosition };
        pos.x = if left { mx - dw } else { mx };
        pos.y = if above { my - dh } else { my };
        dm.dmFields |= DM_POSITION;
    }
    let flags = CDS_UPDATEREGISTRY | CDS_NORESET;
    check(unsafe { ChangeDisplaySettingsExW(PCWSTR(gdi.as_ptr()), Some(&dm), None, flags, None) })?;
    if dw != 0 || dh != 0 {
        let mut done = HashSet::from([(luid64(p.sourceInfo.adapterId), p.sourceInfo.id)]);
        for q in &cfg.paths {
            let s = q.sourceInfo;
            let Some(sm) = source_mode(&cfg, q) else { continue };
            if !done.insert((luid64(s.adapterId), s.id)) {
                continue;
            }
            let (x, y) = (sm.position.x, sm.position.y);
            let (nx, ny) = moved(x, y, sm.width as i32, sm.height as i32);
            if (nx, ny) == (x, y) {
                continue;
            }
            let Some(g) = gdi_name(s.adapterId, s.id) else { continue };
            let Some(mut d) = devmode(&g, ENUM_CURRENT_SETTINGS.0) else { continue };
            d.Anonymous1.Anonymous2 = DEVMODEW_0_1 { dmPosition: POINTL { x: nx, y: ny }, ..unsafe { d.Anonymous1.Anonymous2 } };
            d.dmFields = DM_POSITION;
            check(unsafe { ChangeDisplaySettingsExW(PCWSTR(g.as_ptr()), Some(&d), None, flags, None) })?;
        }
    }
    check(unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) })
}

pub fn set_scale(key: &str, pct: u32) -> Res<()> {
    let _w = lock(&WRITE);
    let cfg = query(false)?;
    let mut keys = Keys::default();
    let p = cfg.paths.iter().find(|p| keys.of(p) == key).ok_or_else(not_active)?;
    apply_scale(p.sourceInfo.adapterId, p.sourceInfo.id, pct)
}

pub fn set_hdr(key: &str, on: bool) -> Res<()> {
    let _w = lock(&WRITE);
    let cfg = query(false)?;
    let mut keys = Keys::default();
    let p = cfg.paths.iter().find(|p| keys.of(p) == key).ok_or_else(not_active)?;
    apply_hdr(p.targetInfo.adapterId, p.targetInfo.id, on)
}

/// Préréglages équivalents à Win+P.
pub fn set_topology(kind: &str) -> Res<()> {
    let _w = lock(&WRITE);
    forget_modes();
    let f = match kind {
        "extend" => SDC_TOPOLOGY_EXTEND,
        "clone" => SDC_TOPOLOGY_CLONE,
        "internal" => SDC_TOPOLOGY_INTERNAL,
        "external" => SDC_TOPOLOGY_EXTERNAL,
        _ => return Err(store::t("Préréglage inconnu", "Unknown preset").into()),
    };
    match unsafe { SetDisplayConfig(None, None, SET_DISPLAY_CONFIG_FLAGS(SDC_APPLY.0 | f.0)) } {
        0 => Ok(()),
        e => Err(win_err(e as u32)),
    }
}

// ---------------------------------------------------------------- luminosité

pub const VCP_BRIGHTNESS: u8 = 0x10;
pub const VCP_POWER: u8 = 0xD6;
fn no_brightness() -> &'static str {
    store::t("Cet écran ne permet pas de régler la luminosité", "This display doesn't support brightness control")
}

/// Plage réellement utilisée d'un réglage (%) : le curseur va de 0 à 100, l'écran de
/// `0` à `1`. Sert à harmoniser des écrans dont la luminosité ne se ressemble pas.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Range(pub u32, pub u32);

impl Range {
    pub const FULL: Range = Range(0, 100);

    /// Valeur du curseur -> niveau envoyé à l'écran, en %.
    pub fn to_hw(self, pct: u32) -> u32 {
        self.0 + (pct.min(100) * self.1.saturating_sub(self.0) + 50) / 100
    }

    /// Niveau lu sur l'écran -> valeur du curseur.
    pub fn to_slider(self, hw: u32) -> u32 {
        let span = self.1.saturating_sub(self.0);
        if span == 0 {
            return 0;
        }
        ((hw.clamp(self.0, self.1) - self.0) * 100 + span / 2) / span
    }

    pub fn valid(self) -> bool {
        self.0 < self.1 && self.1 <= 100
    }
}

/// Réglages DDC/CI d'un écran tirés de ses préférences : code de la luminosité,
/// plage harmonisée, et écran tenu à l'écart du bus.
#[derive(Clone, Copy)]
pub struct Tune {
    pub vcp: u8,
    pub range: Range,
    pub blocked: bool,
}

const TUNE_DEFAULT: Tune = Tune { vcp: VCP_BRIGHTNESS, range: Range::FULL, blocked: false };
static TUNE: Mutex<BTreeMap<String, Tune>> = Mutex::new(BTreeMap::new());

pub fn tune(map: BTreeMap<String, Tune>) {
    *lock(&TUNE) = map;
}

fn tuning(key: &str) -> Tune {
    lock(&TUNE).get(key).copied().unwrap_or(TUNE_DEFAULT)
}

/// Dernière luminosité connue (%) de chaque écran. DDC/CI est lent (jusqu'à ~50 ms
/// par écran) : `monitors()` lit ce cache, `read_brightness()` le met à jour à part.
static LEVELS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());
/// Un seul échange à la fois avec les écrans ; garde aussi la valeur maximale de chaque
/// code VCP, écran par écran (voir `slot`).
static BUS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

fn slot(key: &str, code: u8) -> String {
    format!("{key}|{code:02x}")
}

/// Écran actif : clé, début de son nom d'instance WMI si c'est un écran intégré, moniteur GDI.
type Target = (String, Option<String>, HMONITOR);

/// \\?\DISPLAY#BOE0812#4&1a2b&0&UID265988#{guid} -> DISPLAY\BOE0812\4&1A2B&0&UID265988_
/// Certains pilotes (AMD) listent aussi les écrans externes dans WMI, avec une valeur fausse.
fn wmi_instance(device_path: &str) -> String {
    let p = device_path.trim_start_matches(r"\\?\");
    let p = p.rsplit_once('#').map_or(p, |(a, _)| a);
    format!("{}_", p.replace('#', "\\")).to_uppercase()
}

fn targets() -> Res<Vec<Target>> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(h, &mut mi as *mut _ as *mut MONITORINFO).as_bool() {
            (*(out.0 as *mut HashMap<String, HMONITOR>)).insert(wstr(&mi.szDevice), h);
        }
        true.into()
    }
    let mut gdi: HashMap<String, HMONITOR> = HashMap::new();
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut gdi as *mut _ as isize)) };
    let cfg = query(false)?;
    let mut keys = Keys::default();
    Ok(cfg
        .paths
        .iter()
        .filter_map(|p| {
            let h = gdi_name(p.sourceInfo.adapterId, p.sourceInfo.id).and_then(|n| gdi.get(&wstr(&n)).copied())?;
            let t = &p.targetInfo;
            let wmi = is_internal(t.outputTechnology).then(|| {
                info::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(GET_TARGET_NAME, t.adapterId, t.id)
                    .map_or_else(String::new, |n| wmi_instance(&wstr(&n.monitorDevicePath)))
            });
            Some((keys.of(p), wmi, h))
        })
        .collect())
}

/// Appelle `f` pour chaque écran externe actif, avec sa poignée DDC/CI.
/// ponytail: les écrans dupliqués sont supposés dans l'ordre des chemins CCD ;
/// comparer les descriptions des écrans physiques si un clone règle le mauvais écran.
fn with_ddc(targets: &[Target], mut f: impl FnMut(&mut BTreeMap<String, u32>, &str, HANDLE)) {
    for (i, (_, _, h)) in targets.iter().enumerate() {
        if targets.iter().position(|t| t.2 == *h) != Some(i) {
            continue; // source déjà traitée
        }
        let mut n = 0;
        if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(*h, &mut n) }.is_err() || n == 0 {
            continue;
        }
        let mut phys = vec![PHYSICAL_MONITOR::default(); n as usize];
        if unsafe { GetPhysicalMonitorsFromHMONITOR(*h, &mut phys) }.is_err() {
            continue;
        }
        for ((key, wmi, _), p) in targets.iter().filter(|t| t.2 == *h).zip(&phys) {
            if wmi.is_none() && !tuning(key).blocked {
                f(&mut lock(&BUS), key, p.hPhysicalMonitor);
            }
        }
        let _ = unsafe { DestroyPhysicalMonitors(&phys) };
    }
}

/// Appelle `f` avec la poignée DDC/CI de l'écran `key`, s'il en a une.
fn on_screen<T>(key: &str, f: impl FnOnce(&mut BTreeMap<String, u32>, HANDLE) -> T) -> Res<T> {
    let ts = targets()?;
    let (mut f, mut out) = (Some(f), None);
    with_ddc(&ts, |max, k, h| {
        if let (true, Some(f)) = (k == key, f.take()) {
            out = Some(f(max, h));
        }
    });
    out.ok_or_else(|| match tuning(key).blocked {
        true => store::t("DDC/CI est bloqué pour cet écran", "DDC/CI is blocked for this display").into(),
        false => store::t("Cet écran ne répond pas en DDC/CI", "This display doesn't respond over DDC/CI").into(),
    })
}

/// (valeur, maximum) d'un code VCP.
pub type Vcp = (u32, u32);

/// Lit un code VCP. Le bus DDC/CI échoue parfois, surtout quand un autre logiciel
/// interroge l'écran au même moment : un second essai, 50 ms après.
fn ddc_read(h: HANDLE, code: u8) -> Option<Vcp> {
    (0..2).find_map(|i| {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let (mut cur, mut max) = (0, 0);
        let ok = unsafe { GetVCPFeatureAndVCPFeatureReply(h, code, None, &mut cur, Some(&mut max)) } != 0;
        // Certains écrans annoncent 65535 mais n'acceptent que 0 à 100.
        let max = if max == 0xFFFF && cur <= 100 { 100 } else { max };
        (ok && max > 0).then_some((cur.min(max), max))
    })
}

/// Écran intégré d'un portable : la luminosité passe par WMI. `None` sur un PC de bureau.
fn wmi<T: Send + 'static>(f: impl FnOnce(&IWbemServices) -> Option<T> + Send + 'static) -> Option<T> {
    // COM dans un thread dédié, pour ne pas dépendre de l'appartement de l'appelant.
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let r = (|| {
            let loc: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;
            let none = BSTR::new();
            let svc = loc.ConnectServer(&BSTR::from(r"ROOT\WMI"), &none, &none, &none, 0, &none, None).ok()?;
            // 10, 0 : RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE
            CoSetProxyBlanket(&svc, 10, 0, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE).ok()?;
            f(&svc)
        })();
        CoUninitialize();
        r
    })
    .join()
    .ok()
    .flatten()
}

/// Objet WMI de l'écran dont le nom d'instance commence par `id`.
fn wmi_find(svc: &IWbemServices, class: &str, id: &str) -> Option<IWbemClassObject> {
    let flags = WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY;
    let q = format!("SELECT * FROM {class} WHERE Active=TRUE");
    let rows = unsafe { svc.ExecQuery(&BSTR::from("WQL"), &BSTR::from(q.as_str()), flags, None) }.ok()?;
    loop {
        let (mut row, mut n) = ([None], 0);
        unsafe { rows.Next(WBEM_INFINITE, &mut row, &mut n) }.ok().ok()?;
        let o = row[0].take()?;
        let mut v = VARIANT::default();
        unsafe { o.Get(w!("InstanceName"), 0, &mut v, None, None) }.ok()?;
        if BSTR::try_from(&v).is_ok_and(|s| s.to_string().to_uppercase().starts_with(id)) {
            return Some(o);
        }
    }
}

fn wmi_brightness(id: String) -> Option<u32> {
    wmi(move |svc| {
        let o = wmi_find(svc, "WmiMonitorBrightness", &id)?;
        let mut v = VARIANT::default();
        unsafe { o.Get(w!("CurrentBrightness"), 0, &mut v, None, None) }.ok()?;
        u32::try_from(&v).ok()
    })
}

fn wmi_set_brightness(id: String, pct: u32) -> Res<()> {
    wmi(move |svc| unsafe {
        let o = wmi_find(svc, "WmiMonitorBrightnessMethods", &id)?;
        let mut path = VARIANT::default();
        o.Get(w!("__PATH"), 0, &mut path, None, None).ok()?;
        let mut class = None;
        svc.GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), WBEM_FLAG_RETURN_WBEM_COMPLETE, None, Some(&mut class), None).ok()?;
        let mut sig = None;
        class?.GetMethod(w!("WmiSetBrightness"), 0, &mut sig, std::ptr::null_mut()).ok()?;
        let args = sig?.SpawnInstance(0).ok()?;
        // WMI convertit les chaînes vers le type déclaré du paramètre.
        args.Put(w!("Timeout"), 0, &VARIANT::from("0"), CIM_UINT32.0).ok()?;
        args.Put(w!("Brightness"), 0, &VARIANT::from(pct.to_string().as_str()), CIM_UINT8.0).ok()?;
        let path = BSTR::try_from(&path).ok()?;
        svc.ExecMethod(&path, &BSTR::from("WmiSetBrightness"), WBEM_FLAG_RETURN_WBEM_COMPLETE, None, &args, None, None).ok()
    })
    .ok_or_else(|| store::t("Windows a refusé le réglage de la luminosité", "Windows refused the brightness setting").into())
}

/// Relit la luminosité de chaque écran actif (lent). Renvoie vrai si elle a changé.
/// Chaque valeur est notée sous le verrou du bus : le cache suit le dernier échange.
/// Une lecture ratée garde la valeur connue : un écran qui a déjà répondu reste réglable,
/// sans quoi la luminosité disparaîtrait des menus au moindre raté du bus.
pub fn read_brightness() -> bool {
    let Ok(ts) = targets() else { return false };
    let before = lock(&LEVELS).clone();
    let note = |k: &str, v: Option<u32>| {
        if let Some(v) = v {
            lock(&LEVELS).insert(k.into(), v);
        }
    };
    for (k, id, _) in &ts {
        if let Some(id) = id {
            let range = tuning(k).range;
            let _bus = lock(&BUS);
            note(k, wmi_brightness(id.clone()).map(|v| range.to_slider(v)));
        }
    }
    with_ddc(&ts, |max, k, h| {
        let tu = tuning(k);
        let r = ddc_read(h, tu.vcp);
        if let Some((_, m)) = r {
            max.insert(slot(k, tu.vcp), m);
        }
        note(k, r.map(|(cur, m)| tu.range.to_slider((cur * 100 + m / 2) / m)));
    });
    let mut l = lock(&LEVELS);
    l.retain(|k, _| ts.iter().any(|t| t.0 == *k));
    *l != before
}

fn refused() -> String {
    store::t("L'écran n'a pas accepté le réglage", "The display didn't accept the setting").into()
}

/// Règle un code VCP continu à `pct` % de son maximum.
fn ddc_level(max: &mut BTreeMap<String, u32>, key: &str, h: HANDLE, code: u8, pct: u32) -> Res<()> {
    let s = slot(key, code);
    match max.get(&s).copied().or_else(|| ddc_read(h, code).map(|x| x.1)) {
        Some(m) if unsafe { SetVCPFeature(h, code, (pct.min(100) * m + 50) / 100) } != 0 => {
            max.insert(s, m);
            Ok(())
        }
        Some(_) => Err(refused()),
        None => Err(store::t("L'écran ne répond pas à ce réglage", "The display doesn't respond to this setting").into()),
    }
}

pub fn set_brightness(key: &str, pct: u32) -> Res<()> {
    let pct = pct.min(100);
    let tu = tuning(key);
    let ts = targets()?;
    let (_, wmi, _) = ts.iter().find(|t| t.0 == key).ok_or_else(not_active)?;
    let r = match wmi {
        Some(id) => {
            let _bus = lock(&BUS);
            wmi_set_brightness(id.clone(), tu.range.to_hw(pct))
        }
        None => match on_screen(key, |max, h| ddc_level(max, key, h, tu.vcp, tu.range.to_hw(pct))) {
            Ok(Err(e)) if e == refused() => Err(e),
            Ok(Ok(())) => Ok(()),
            _ => Err(no_brightness().into()),
        },
    };
    if r.is_ok() {
        lock(&LEVELS).insert(key.into(), pct);
    }
    r
}

// ---------------------------------------------------------------- autres commandes DDC/CI

/// Codes VCP annoncés par l'écran dans sa chaîne de capacités MCCS, avec les valeurs
/// permises des réglages à choix. Lent : une à trois secondes.
pub fn vcp_caps(key: &str) -> Res<BTreeMap<u8, Vec<u8>>> {
    on_screen(key, |_, h| {
        (0..2).find_map(|i| {
            if i > 0 {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            let mut n = 0;
            if unsafe { GetCapabilitiesStringLength(h, &mut n) } == 0 || n == 0 {
                return None;
            }
            let mut buf = vec![0u8; n as usize];
            (unsafe { CapabilitiesRequestAndCapabilitiesReply(h, &mut buf) } != 0).then(|| parse_caps(&String::from_utf8_lossy(&buf)))
        })
    })?
    .filter(|c| !c.is_empty())
    .ok_or_else(|| store::t("L'écran n'a pas décrit ses commandes", "The display didn't describe its controls").into())
}

/// `...vcp(02 10 12 14(05 08 0B) 60(0F 11) D6(01 05))...` -> {0x02: [], 0x14: [5, 8, 11], ...}
/// Certains écrans collent les codes sans espace (`vcp(101214)`) : on lit par paires.
pub fn parse_caps(s: &str) -> BTreeMap<u8, Vec<u8>> {
    let mut out: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
    let Some(i) = s.to_ascii_lowercase().find("vcp(") else { return out };
    let (mut depth, mut last, mut tok) = (1, None, String::new());
    let mut flush = |tok: &mut String, depth: i32| {
        if let Ok(v) = u8::from_str_radix(tok, 16) {
            match (depth, last) {
                (1, _) => {
                    out.entry(v).or_default();
                    last = Some(v);
                }
                (2, Some(l)) => out.entry(l).or_default().push(v),
                _ => {}
            }
        }
        tok.clear();
    };
    for c in s[i + 4..].chars() {
        if c.is_ascii_hexdigit() {
            tok.push(c);
            if tok.len() == 2 {
                flush(&mut tok, depth);
            }
            continue;
        }
        flush(&mut tok, depth);
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    // La sourdine n'a que deux valeurs, que l'écran ne liste pas toujours.
    if let Some(v) = out.get_mut(&0x8D).filter(|v| v.is_empty()) {
        *v = vec![1, 2];
    }
    out
}

/// Valeur actuelle et maximum de chaque code, `None` s'il ne répond pas.
pub fn vcp_get(key: &str, codes: &[u8]) -> Res<Vec<(u8, Option<Vcp>)>> {
    on_screen(key, |max, h| {
        codes
            .iter()
            .map(|&c| {
                let r = ddc_read(h, c);
                if let Some((_, m)) = r {
                    max.insert(slot(key, c), m);
                }
                (c, r)
            })
            .collect()
    })
}

/// Envoie une valeur brute (réglage à choix, alimentation).
pub fn vcp_set(key: &str, code: u8, value: u32) -> Res<()> {
    on_screen(key, |_, h| unsafe { SetVCPFeature(h, code, value) } != 0)?.then_some(()).ok_or_else(refused)
}

/// Règle un code continu à `pct` % de sa plage.
pub fn vcp_level(key: &str, code: u8, pct: u32, range: Range) -> Res<()> {
    on_screen(key, |max, h| ddc_level(max, key, h, code, range.to_hw(pct)))?
}

/// Extinction matérielle : envoie les valeurs d'alimentation (D6) à chaque écran externe.
pub fn power_off_all(values: &[u32]) {
    let Ok(ts) = targets() else { return };
    with_ddc(&ts, |_, _, h| {
        for &v in values {
            let _ = unsafe { SetVCPFeature(h, VCP_POWER, v) };
        }
    });
}

// ---------------------------------------------------------------- profils

pub fn capture() -> Res<Snapshot> {
    let cfg = query(false)?;
    let mut keys = Keys::default();
    let mut s = Snapshot {
        paths: cfg.paths.iter().map(hex).collect(),
        modes: cfg.modes.iter().map(hex).collect(),
        ..Default::default()
    };
    for p in &cfg.paths {
        for l in [p.sourceInfo.adapterId, p.targetInfo.adapterId] {
            s.adapters.insert(luid_hex(l), keys.adapter(l));
        }
        let k = keys.of(p);
        if let Some(sc) = scale_info(p.sourceInfo.adapterId, p.sourceInfo.id) {
            s.scales.insert(k.clone(), sc.current);
        }
        if let Some(h) = hdr_info(p.targetInfo.adapterId, p.targetInfo.id) {
            s.hdr.insert(k.clone(), h);
        }
        // Dernière valeur connue : relire DDC/CI ici ralentirait chaque action.
        if let Some(&b) = lock(&LEVELS).get(&k) {
            s.brightness.insert(k, b);
        }
    }
    Ok(s)
}

/// Reconstruit la config d'un profil avec les LUID du démarrage en cours.
/// Les écrans absents sont ignorés ; renvoie leur nombre.
fn rebuild(s: &Snapshot) -> Res<(Config, usize)> {
    let corrupt = store::t("Profil illisible", "Unreadable profile");
    let cur = query(true)?;
    let mut keys = Keys::default();
    let mut luids: HashMap<String, LUID> = HashMap::new();
    let mut available = HashSet::new();
    for p in &cur.paths {
        for l in [p.sourceInfo.adapterId, p.targetInfo.adapterId] {
            luids.insert(keys.adapter(l), l);
        }
        if p.targetInfo.targetAvailable.as_bool() {
            available.insert((luid64(p.targetInfo.adapterId), p.targetInfo.id));
        }
    }
    let remap = |l: LUID| s.adapters.get(&luid_hex(l)).and_then(|path| luids.get(path)).copied();
    let modes = s.modes.iter().map(|h| unhex(h).ok_or(corrupt)).collect::<Result<Vec<DISPLAYCONFIG_MODE_INFO>, _>>()?;
    let mut cfg = Config { paths: Vec::new(), modes };
    let mut missing = HashSet::new();
    for h in &s.paths {
        let mut p: DISPLAYCONFIG_PATH_INFO = unhex(h).ok_or(corrupt)?;
        match (remap(p.sourceInfo.adapterId), remap(p.targetInfo.adapterId)) {
            (Some(a), Some(b)) if available.contains(&(luid64(b), p.targetInfo.id)) => {
                p.sourceInfo.adapterId = a;
                p.targetInfo.adapterId = b;
                cfg.paths.push(p);
            }
            _ => {
                missing.insert((luid64(p.targetInfo.adapterId), p.targetInfo.id));
            }
        }
    }
    if cfg.paths.is_empty() {
        return Err(store::t("Aucun écran de ce profil n'est branché", "None of this profile's displays are connected").into());
    }
    compact_modes(&mut cfg);
    for m in cfg.modes.iter_mut() {
        if let Some(l) = remap(m.adapterId) {
            m.adapterId = l;
        }
    }
    ensure_primary(&mut cfg);
    Ok((cfg, missing.len()))
}

/// Retire les modes cible (`all = false`) ou tous les modes pour laisser Windows choisir.
fn strip(cfg: &mut Config, all: bool) {
    for p in cfg.paths.iter_mut() {
        p.targetInfo.Anonymous.modeInfoIdx = INVALID;
        if all {
            p.sourceInfo.Anonymous.modeInfoIdx = INVALID;
        }
    }
    compact_modes(cfg);
}

pub fn restore(s: &Snapshot, x: Extras) -> Res<usize> {
    let _w = lock(&WRITE);
    forget_modes();
    let (mut cfg, missing) = rebuild(s)?;
    // Config exacte, puis timings choisis par Windows, puis topologie seule.
    apply(&cfg, APPLY)
        .or_else(|_| {
            strip(&mut cfg, false);
            apply(&cfg, APPLY)
        })
        .or_else(|_| {
            strip(&mut cfg, true);
            apply(&cfg, APPLY)
        })?;
    let now = query(false)?;
    let mut keys = Keys::default();
    for p in &now.paths {
        let k = keys.of(p);
        if let (true, Some(&pct)) = (x.scale, s.scales.get(&k)) {
            let _ = apply_scale(p.sourceInfo.adapterId, p.sourceInfo.id, pct);
        }
        let (a, t) = (p.targetInfo.adapterId, p.targetInfo.id);
        if let (true, Some(&on)) = (x.hdr, s.hdr.get(&k)) {
            if hdr_info(a, t).is_some_and(|cur| cur != on) {
                let _ = apply_hdr(a, t, on);
            }
        }
        if let (true, Some(&pct)) = (x.brightness, s.brightness.get(&k)) {
            if lock(&LEVELS).get(&k) != Some(&pct) {
                let _ = set_brightness(&k, pct);
            }
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALIDATE: SET_DISPLAY_CONFIG_FLAGS =
        SET_DISPLAY_CONFIG_FLAGS(SDC_VALIDATE.0 | SDC_USE_SUPPLIED_DISPLAY_CONFIG.0 | SDC_ALLOW_CHANGES.0);

    #[test]
    fn hex_roundtrip() {
        let l = LUID { LowPart: 0xdead_beef, HighPart: -2 };
        let back: LUID = unhex(&hex(&l)).unwrap();
        assert_eq!((back.LowPart, back.HighPart), (l.LowPart, l.HighPart));
        assert!(unhex::<LUID>("abc").is_none());
    }

    #[test]
    fn caps_are_parsed() {
        let s = "(prot(monitor)type(LCD)model(AORUS)cmds(01 02 03)vcp(02 10 12 14(05 08 0B) 60( 0F 11 12) 62 D6(01 05) DF)mccs_ver(2.1))";
        let c = parse_caps(s);
        assert_eq!(c.keys().copied().collect::<Vec<_>>(), [0x02, 0x10, 0x12, 0x14, 0x60, 0x62, 0xD6, 0xDF]);
        assert_eq!(c[&0x14], [5, 8, 11]);
        assert_eq!(c[&0x60], [0x0F, 0x11, 0x12]);
        assert!(c[&0x12].is_empty());
        // Codes collés, majuscules, valeurs imbriquées ignorées.
        assert_eq!(parse_caps("vcp(10 8D)")[&0x8D], [1, 2]);
        let c = parse_caps("VCP(101214(0102)62)");
        assert_eq!(c.keys().copied().collect::<Vec<_>>(), [0x10, 0x12, 0x14, 0x62]);
        assert_eq!(c[&0x14], [1, 2]);
        assert!(parse_caps("(prot(monitor))").is_empty());
    }

    #[test]
    fn ranges_round_trip() {
        let r = Range(20, 80);
        assert_eq!((r.to_hw(0), r.to_hw(100), r.to_hw(50)), (20, 80, 50));
        // 60 niveaux réels pour 100 positions du curseur : à un cran près.
        for p in 0..=100 {
            assert!(r.to_slider(r.to_hw(p)).abs_diff(p) <= 1, "{p}");
        }
        assert_eq!((r.to_slider(5), r.to_slider(95)), (0, 100));
        assert_eq!(Range::FULL.to_hw(42), 42);
        assert!(!Range(50, 50).valid() && !Range(0, 101).valid() && Range(0, 100).valid());
    }

    #[test]
    fn wmi_instance_name() {
        let path = r"\\?\DISPLAY#GBT3214#5&9d93ce2&1&UID272#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        assert!(r"DISPLAY\GBT3214\5&9d93ce2&1&UID272_0".to_uppercase().starts_with(&wmi_instance(path)));
        assert!(!r"DISPLAY\GBT3214\5&9d93ce2&1&UID2720_0".to_uppercase().starts_with(&wmi_instance(path)));
    }

    /// Change vraiment la luminosité du premier écran réglable, puis la rétablit.
    /// `cargo test live_brightness_write -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_brightness_write() {
        let pause = || std::thread::sleep(std::time::Duration::from_millis(1500));
        let level = |k: &str| lock(&LEVELS).get(k).copied();
        read_brightness();
        let (key, before) = lock(&LEVELS).iter().next().map(|(k, v)| (k.clone(), *v)).expect("aucun écran réglable");
        set_brightness(&key, 30).unwrap();
        pause();
        read_brightness(); // relu sur l'écran, pas dans le cache
        let during = level(&key);
        set_brightness(&key, before).unwrap();
        pause();
        read_brightness();
        let after = level(&key);
        println!("avant {before} %, pendant {during:?}, après {after:?}");
        assert_eq!((during, after), (Some(30), Some(before)));
    }

    /// Lit la configuration réelle et fait valider par Windows chaque opération,
    /// sans rien appliquer (SDC_VALIDATE).
    #[test]
    fn live_configs_validate() {
        let time = |label: &str, f: &dyn Fn()| {
            let t = std::time::Instant::now();
            f();
            println!("durée {label} : {:?}", t.elapsed());
        };
        time("monitors (1er appel)", &|| drop(monitors()));
        time("monitors (modes en cache)", &|| drop(monitors()));
        time("signature", &|| drop(signature()));
        time("topology", &|| {
            let _ = topology();
        });
        time("capture", &|| drop(capture()));
        time("modes (pilote)", &|| {
            for p in &query(false).unwrap().paths {
                let gdi = gdi_name(p.sourceInfo.adapterId, p.sourceInfo.id).unwrap();
                drop(raw_modes(&gdi));
            }
        });
        let mons = monitors().unwrap();
        for m in &mons {
            println!("{} | {} | actif={} principal={} {}x{}@{} ({},{}) rot={} échelle={:?} hdr={:?} modes={}",
                m.name, m.connector, m.active, m.primary, m.w, m.h, m.hz, m.x, m.y, m.rotation,
                m.scale.as_ref().map(|s| (s.current, s.recommended)), m.hdr, m.modes.len());
        }
        let active: Vec<_> = mons.iter().filter(|m| m.active).collect();
        assert!(!active.is_empty());
        assert!(active.iter().filter(|m| m.primary).count() == 1);
        assert_eq!(connected().unwrap(), mons.iter().map(|m| m.key.clone()).collect());

        let snap = capture().unwrap();
        let (cfg, missing) = rebuild(&snap).unwrap();
        assert_eq!(missing, 0);
        apply(&cfg, VALIDATE).expect("profil courant");

        let pos: Vec<_> = active.iter().map(|m| (m.key.clone(), m.x, m.y)).collect();
        apply(&positions_config(&pos).unwrap(), VALIDATE).expect("positions");

        for m in &mons {
            let cfg = enabled_config(&m.key, !m.active);
            match (m.active, active.len()) {
                (true, 1) => assert!(cfg.is_err()),
                _ => apply(&cfg.unwrap(), VALIDATE).unwrap_or_else(|e| panic!("bascule {}: {e}", m.name)),
            }
        }
        println!("signature: {}", signature().unwrap());
        println!("préréglage: {:?}", topology());
        // Lecture seule : rien n'est modifié sur les écrans.
        let t = std::time::Instant::now();
        read_brightness();
        println!("luminosité: {:?} en {:?}", lock(&LEVELS), t.elapsed());
        for p in &query(false).unwrap().paths {
            let n: DISPLAYCONFIG_TARGET_DEVICE_NAME = info(GET_TARGET_NAME, p.targetInfo.adapterId, p.targetInfo.id).unwrap();
            let id = wmi_instance(&wstr(&n.monitorDevicePath));
            println!("WMI {id}: {:?}", wmi_brightness(id.clone()));
        }
        // Capacités et valeurs DDC/CI, en lecture seule.
        for m in mons.iter().filter(|m| m.active && !m.internal) {
            let t = std::time::Instant::now();
            let caps = vcp_caps(&m.key);
            println!("capacités {} en {:?} : {caps:x?}", m.name, t.elapsed());
            if let Ok(c) = caps {
                let codes: Vec<u8> = c.keys().copied().filter(|&k| [0x10, 0x12, 0x14, 0x60, 0x62, 0xD6].contains(&k)).collect();
                println!("valeurs : {:x?}", vcp_get(&m.key, &codes));
            }
        }
    }
}
