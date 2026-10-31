//! Profils enregistrés (%APPDATA%\MonitorMG\config.json) et petites intégrations
//! Windows : raccourcis bureau, couleur d'accent, thème de la barre des tâches, langue.

use crate::display::{self, Range, Res, Snapshot, Tile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::{fs, path::PathBuf, sync::OnceLock, time::SystemTime};
use windows::core::{w, Interface, HSTRING, PCWSTR};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};

#[derive(Serialize, Deserialize, Clone)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub hotkey: Option<String>,
    pub sig: String,
    pub tiles: Vec<Tile>,
    pub snap: Snapshot,
    /// Appliqué dès que les écrans branchés sont exactement `screens`.
    #[serde(default)]
    pub auto: bool,
    #[serde(default)]
    pub screens: BTreeSet<String>,
    /// Réglages à ne pas appliquer : "scale", "hdr" ou "brightness".
    #[serde(default)]
    pub exclude: BTreeSet<String>,
}

/// Réglages qu'un profil peut exclure.
pub const EXTRAS: [&str; 3] = ["scale", "hdr", "brightness"];

impl Profile {
    pub fn extras(&self) -> display::Extras {
        let on = |k: &str| !self.exclude.contains(k);
        display::Extras { scale: on("scale"), hdr: on("hdr"), brightness: on("brightness") }
    }
}

/// Fenêtre « Conserver cet affichage ? » et délai avant retour automatique.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(default)]
pub struct Confirm {
    pub on: bool,
    pub secs: u64,
}

impl Default for Confirm {
    fn default() -> Self {
        Self { on: true, secs: 15 }
    }
}

/// Position et taille de la fenêtre principale (pixels physiques).
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub max: bool,
}

/// Réglage DDC/CI affiché dans le panneau (contraste, volume, entrée…).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Feature {
    pub code: u8,
    /// Plage utilisée par le curseur, pour un réglage continu.
    pub range: Range,
    /// Suit la luminosité : 0 à `stop` % de luminosité parcourent toute la plage.
    pub linked: bool,
    pub stop: u32,
}

impl Default for Feature {
    fn default() -> Self {
        Self { code: 0, range: Range::FULL, linked: false, stop: 100 }
    }
}

/// Codes VCP qui déclenchent une action à chaque écriture (démagnétiser, remettre les
/// réglages d'usine, enregistrer) : jamais affichés comme réglages.
pub const COMMANDS: [u8; 7] = [0x01, 0x04, 0x05, 0x06, 0x08, 0x0A, 0xB0];

/// Préférences d'un écran pour le panneau et DDC/CI, par clé d'écran.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Screen {
    /// Nom dans le panneau ; vide : celui de Windows.
    pub name: String,
    pub hidden: bool,
    /// Aucun échange DDC/CI, pour un écran qui fige le PC quand on l'interroge.
    pub blocked: bool,
    /// Code VCP de la luminosité, s'il n'est pas 0x10.
    pub vcp: Option<u8>,
    pub range: Range,
    pub features: Vec<Feature>,
    /// Codes annoncés par l'écran, lus une fois : la lecture prend plusieurs secondes.
    pub caps: BTreeMap<u8, Vec<u8>>,
}

impl Default for Screen {
    fn default() -> Self {
        Self { name: String::new(), hidden: false, blocked: false, vcp: None, range: Range::FULL, features: Vec::new(), caps: BTreeMap::new() }
    }
}

impl Screen {
    /// Code de la luminosité : celui choisi, sinon 0x10, ou à défaut 0x13 ou 0x6B si
    /// l'écran n'annonce que ceux-là.
    pub fn brightness_code(&self) -> u8 {
        let has = |c: &u8| self.caps.contains_key(c);
        self.vcp.unwrap_or_else(|| match self.caps.is_empty() || has(&0x10) {
            true => 0x10,
            false => [0x13, 0x6B].into_iter().find(has).unwrap_or(0x10),
        })
    }

    /// Réglage à choix (entrée, préréglage couleur…) : l'écran en liste les valeurs.
    /// Les autres sont continus et se règlent en %.
    pub fn discrete(&self, code: u8) -> bool {
        self.caps.get(&code).is_some_and(|v| !v.is_empty())
    }

    pub fn tune(&self) -> display::Tune {
        display::Tune { vcp: self.brightness_code(), range: self.range, blocked: self.blocked }
    }

    /// Préférences reçues de l'interface, vérifiées. Les capacités restent celles lues
    /// sur l'écran.
    pub fn checked(mut self, caps: BTreeMap<u8, Vec<u8>>) -> Res<Self> {
        let bad = || t("Plage invalide : le minimum doit rester sous le maximum", "Invalid range: the minimum must stay below the maximum").to_string();
        self.name = self.name.trim().chars().filter(|c| !c.is_control()).take(40).collect();
        self.vcp = self.vcp.filter(|&c| c != 0);
        self.caps = caps;
        if !self.range.valid() {
            return Err(bad());
        }
        if self.features.iter().any(|f| COMMANDS.contains(&f.code)) {
            return Err(t("Ce code est une commande (remise à zéro…), pas un réglage", "This code is a command (reset…), not a setting").into());
        }
        let mut seen = BTreeSet::new();
        self.features.retain(|f| f.code != 0 && seen.insert(f.code));
        for f in &mut self.features {
            if !f.range.valid() {
                return Err(bad());
            }
            f.stop = f.stop.clamp(1, 100);
        }
        Ok(self)
    }
}

/// Réglages du panneau ouvert d'un clic sur l'icône de notification.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Panel {
    /// Un seul curseur pour tous les écrans.
    pub linked: bool,
    /// Valeur DDC/CI d'extinction (code D6) : 4 veille, 5 arrêt, 6 les deux.
    pub power: u32,
    /// « Éteindre les écrans » : "soft" (Windows), "hard" (DDC/CI) ou "both".
    pub off: String,
    /// Pas des raccourcis de luminosité, en %.
    pub step: u32,
}

impl Default for Panel {
    fn default() -> Self {
        Self { linked: false, power: 5, off: "soft".into(), step: 10 }
    }
}

impl Panel {
    /// Valeurs D6 à envoyer pour éteindre.
    pub fn power_values(&self) -> &'static [u32] {
        match self.power {
            4 => &[4],
            6 => &[4, 5],
            _ => &[5],
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct Store {
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub confirm: Confirm,
    #[serde(default)]
    pub window: Option<Place>,
    /// Icône choisie pour la zone de notification (vide : celle par défaut).
    #[serde(default)]
    pub tray: String,
    /// Raccourci global qui éteint les écrans.
    #[serde(default)]
    pub off_hotkey: Option<String>,
    /// Raccourcis globaux qui montent ou baissent la luminosité de tous les écrans.
    #[serde(default)]
    pub up_hotkey: Option<String>,
    #[serde(default)]
    pub down_hotkey: Option<String>,
    #[serde(default)]
    pub panel: Panel,
    #[serde(default)]
    pub screens: BTreeMap<String, Screen>,
}

/// Dossier de l'exécutable s'il contient un fichier `portable`.
fn portable_root() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    dir.join("portable").is_file().then_some(dir)
}

pub fn portable() -> bool {
    portable_root().is_some()
}

/// Dossier des données : `%APPDATA%\MonitorMG`, ou `data\` à côté de l'exécutable
/// en version portable.
pub fn data_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| match portable_root() {
        Some(d) => d.join("data"),
        None => PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("MonitorMG"),
    })
    .clone()
}

fn file() -> PathBuf {
    data_dir().join("config.json")
}

impl Store {
    pub fn load() -> Self {
        let f = file();
        let Ok(text) = fs::read_to_string(&f) else { return Self::default() };
        serde_json::from_str(&text).unwrap_or_else(|e| {
            // Fichier abîmé : on le met de côté, daté, plutôt que de l'écraser au prochain enregistrement.
            let ms = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
            let _ = fs::rename(&f, f.with_file_name(format!("config-{ms}.bak.json")));
            crate::journal::write("error", t("Système", "System"), &tr!("Configuration illisible, mise de côté : {e}", "Unreadable configuration, set aside: {e}"));
            Self::default()
        })
    }

    pub fn save(&self) -> Res<()> {
        let f = file();
        let tmp = f.with_extension("tmp");
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        fs::create_dir_all(f.parent().unwrap_or(&f))
            .and_then(|_| fs::write(&tmp, json))
            .and_then(|_| fs::rename(&tmp, &f))
            .map_err(|e| tr!("Enregistrement impossible : {e}", "Can't save: {e}"))
    }

    pub fn get_mut(&mut self, id: &str) -> Res<&mut Profile> {
        self.profiles.iter_mut().find(|p| p.id == id).ok_or_else(|| t("Profil introuvable", "Profile not found").into())
    }
}

/// Photographie la configuration actuelle.
pub fn capture(name: String) -> Res<Profile> {
    let ms = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    // Luminosité relue sur l'écran : celle du cache peut dater de quelques secondes.
    display::read_brightness();
    Ok(Profile {
        id: format!("{ms:x}"),
        name,
        hotkey: None,
        sig: display::signature()?,
        tiles: display::tiles()?,
        snap: display::capture()?,
        auto: false,
        screens: display::connected()?,
        exclude: BTreeSet::new(),
    })
}

/// Crée sur le bureau un raccourci qui applique le profil sans ouvrir l'interface.
pub fn desktop_shortcut(p: &Profile) -> Res<String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let file = shortcut_file(&p.name);
    let name = p.name.clone();
    let id = p.id.clone();
    // COM dans un thread dédié, pour ne pas dépendre de l'appartement de l'appelant.
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let r = (|| -> windows::core::Result<String> {
            let desk = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None)?;
            let dir = desk.to_string().unwrap_or_default();
            CoTaskMemFree(Some(desk.0 as _));
            let path = format!("{dir}\\{file}");
            let exe = HSTRING::from(exe.as_os_str());
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&exe)?;
            link.SetArguments(&HSTRING::from(format!("--profile {id}")))?;
            link.SetIconLocation(&exe, 0)?;
            link.SetDescription(&HSTRING::from(tr!("Applique le profil d'affichage {name}", "Applies the display profile {name}")))?;
            link.cast::<IPersistFile>()?.Save(&HSTRING::from(path.as_str()), true)?;
            Ok(path)
        })();
        CoUninitialize();
        r.map_err(|e| e.message())
    })
    .join()
    .unwrap_or_else(|_| Err(t("Création du raccourci interrompue", "Shortcut creation interrupted").into()))
}

/// `Bureau (MonitorMG).lnk` : le suffixe évite d'écraser un raccourci de l'utilisateur
/// et neutralise les noms réservés de Windows (CON, NUL…).
fn shortcut_file(profile: &str) -> String {
    let clean: String = profile.chars().filter(|c| !c.is_control() && !r#"<>:"/\|?*"#.contains(*c)).collect();
    let clean = clean.trim().trim_end_matches('.');
    format!("{} (MonitorMG).lnk", if clean.is_empty() { t("Profil", "Profile") } else { clean })
}

fn reg<T: Default>(key: PCWSTR, value: PCWSTR, kind: REG_ROUTINE_FLAGS) -> Option<T> {
    reg_in(HKEY_CURRENT_USER, key, value, kind)
}

fn reg_in<T: Default>(root: HKEY, key: PCWSTR, value: PCWSTR, kind: REG_ROUTINE_FLAGS) -> Option<T> {
    let mut v = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    unsafe { RegGetValueW(root, key, value, kind, None, Some(&mut v as *mut T as _), Some(&mut size)) }
        .ok()
        .ok()
        .map(|_| v)
}

/// « Windows 25H2 (26200) », pour le journal de diagnostic.
pub fn windows_version() -> String {
    let key = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let s = |v: PCWSTR| reg_in::<[u16; 32]>(HKEY_LOCAL_MACHINE, key, v, RRF_RT_REG_SZ).map(|b| crate::display::wstr(&b));
    format!("Windows {} ({})", s(w!("DisplayVersion")).unwrap_or_default(), s(w!("CurrentBuild")).unwrap_or_default())
}

/// Langue de l'interface : français si Windows est en français, anglais sinon.
/// `MONITORMG_LANG=en` (ou `fr`) force la langue, pour tester sans changer celle de Windows.
pub fn english() -> bool {
    static EN: OnceLock<bool> = OnceLock::new();
    *EN.get_or_init(|| match std::env::var("MONITORMG_LANG") {
        Ok(l) => l == "en",
        // 0x0C : LANG_FRENCH, quelle que soit la variante (France, Canada, Belgique…).
        Err(_) => (unsafe { GetUserDefaultUILanguage() } & 0x3ff) != 0x0c,
    })
}

/// Texte fixe dans la langue de l'interface ; `tr!` pour un texte formaté.
pub fn t(fr: &'static str, en: &'static str) -> &'static str {
    if english() {
        en
    } else {
        fr
    }
}

pub fn light_taskbar() -> bool {
    reg::<u32>(w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), w!("SystemUsesLightTheme"), RRF_RT_REG_DWORD)
        == Some(1)
}

/// Accent Windows : teinte claire (thème sombre) et teinte foncée (thème clair),
/// comme les applications WinUI. AccentPalette = 8 couleurs RGBA, de la plus claire à la plus foncée.
pub fn accent() -> (String, String) {
    let rgb = |b: &[u8]| format!("#{:02x}{:02x}{:02x}", b[0], b[1], b[2]);
    match reg::<[u8; 32]>(w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"), w!("AccentPalette"), RRF_RT_REG_BINARY) {
        Some(p) => (rgb(&p[4..]), rgb(&p[16..])),
        None => ("#4cc2ff".into(), "#005fb8".into()),
    }
}

pub fn alert(msg: &str) {
    unsafe { MessageBoxW(None, &HSTRING::from(msg), w!("MonitorMG"), MB_OK | MB_ICONWARNING) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_names_are_safe() {
        assert_eq!(shortcut_file("Bureau"), "Bureau (MonitorMG).lnk");
        assert_eq!(shortcut_file("Jeu: 240/Hz?"), "Jeu 240Hz (MonitorMG).lnk");
        assert_eq!(shortcut_file("CON"), "CON (MonitorMG).lnk");
        assert_eq!(shortcut_file(" ...\t"), format!("{} (MonitorMG).lnk", t("Profil", "Profile")));
    }

    #[test]
    fn profile_extras_follow_exclusions() {
        // Profil antérieur aux réglages facultatifs : tout s'applique, rien n'est exclu.
        let mut p: Profile = serde_json::from_str(r#"{"id":"1","name":"x","sig":"","tiles":[],"snap":{"paths":[],"modes":[],"adapters":{}}}"#).unwrap();
        assert!(p.extras().brightness && p.snap.brightness.is_empty());
        p.exclude.insert("brightness".into());
        let x = p.extras();
        assert!(!x.brightness && x.scale && x.hdr);
    }

    #[test]
    fn screens_pick_their_brightness_code() {
        let mut s = Screen::default();
        assert_eq!(s.brightness_code(), 0x10); // capacités inconnues
        s.caps = BTreeMap::from([(0x12, vec![]), (0x6B, vec![]), (0x60, vec![0x0F, 0x11])]);
        assert_eq!(s.brightness_code(), 0x6B);
        assert!(s.discrete(0x60) && !s.discrete(0x12) && !s.discrete(0x99));
        s.vcp = Some(0x13);
        assert_eq!(s.brightness_code(), 0x13);
    }

    #[test]
    fn screen_prefs_are_checked() {
        let caps = BTreeMap::from([(0x10, vec![])]);
        let f = |code, range| Feature { code, range, ..Default::default() };
        let s = Screen { name: " Gauche\n".into(), vcp: Some(0), features: vec![f(0x12, Range::FULL), f(0x12, Range(0, 50)), f(0, Range::FULL)], ..Default::default() };
        let s = s.checked(caps.clone()).unwrap();
        assert_eq!((s.name.as_str(), s.vcp, s.features.len(), &s.caps), ("Gauche", None, 1, &caps));
        assert!(Screen { range: Range(60, 40), ..Default::default() }.checked(caps.clone()).is_err());
        assert!(Screen { features: vec![f(0x62, Range(10, 10))], ..Default::default() }.checked(caps.clone()).is_err());
        assert!(Screen { features: vec![f(0x04, Range::FULL)], ..Default::default() }.checked(caps).is_err());
    }

    #[test]
    fn old_configs_get_default_settings() {
        let s: Store = serde_json::from_str(r#"{"profiles":[]}"#).unwrap();
        assert!(s.confirm.on);
        assert_eq!(s.confirm.secs, 15);
        let s: Store = serde_json::from_str(r#"{"confirm":{"on":false}}"#).unwrap();
        assert_eq!((s.confirm.on, s.confirm.secs), (false, 15));
        assert_eq!((s.panel.power, s.panel.step, s.panel.off.as_str()), (5, 10, "soft"));
        let s: Store = serde_json::from_str(r#"{"screens":{"a":{"hidden":true,"features":[{"code":18}]}}}"#).unwrap();
        let a = &s.screens["a"];
        assert!(a.hidden && a.range == Range::FULL && a.features[0].stop == 100);
    }
}
