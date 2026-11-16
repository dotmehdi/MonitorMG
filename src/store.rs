//! Profils enregistrés (%APPDATA%\MonitorMG\config.json) et petites intégrations
//! Windows : raccourcis bureau, couleur d'accent, thème de la barre des tâches, langue.

use crate::display::{self, Range, Res, Snapshot, Tile};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
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
    /// Déclencheurs automatiques, en plus du branchement (`auto`).
    #[serde(default)]
    pub triggers: Triggers,
}

/// Source d'alimentation d'un portable.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PowerSource {
    /// Secteur.
    Ac,
    /// Batterie.
    Dc,
}

/// Déclencheurs d'un profil : il s'applique seul à l'heure dite, au changement
/// d'alimentation ou au lancement d'un programme.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Triggers {
    /// Heure d'application quotidienne, en minutes depuis minuit.
    #[serde(deserialize_with = "lenient")]
    pub at: Option<u32>,
    #[serde(deserialize_with = "lenient")]
    pub power: Option<PowerSource>,
    /// Nom de l'exécutable (« jeu.exe ») dont le lancement applique le profil ; vide : aucun.
    pub exe: String,
}

impl Triggers {
    /// Remet dans les bornes : heure du jour, et nom de fichier seul (ni chemin ni
    /// caractère de contrôle), avec son extension.
    pub fn checked(mut self) -> Self {
        self.at = self.at.filter(|&m| m < 1440);
        let file = self.exe.trim().rsplit(['\\', '/']).next().unwrap_or_default();
        self.exe = file.trim().chars().filter(|c| !c.is_control()).take(80).collect();
        if !self.exe.is_empty() && !self.exe.to_lowercase().ends_with(".exe") {
            self.exe.push_str(".exe");
        }
        self
    }
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
    /// Les deux entrées (valeurs du code 0x60) entre lesquelles `input_hotkey` bascule.
    pub inputs: Option<(u8, u8)>,
    /// Raccourci global de bascule d'entrée ; attribué par `assign_hotkey`, jamais par
    /// l'enregistrement des préférences.
    pub input_hotkey: Option<String>,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            name: String::new(),
            hidden: false,
            blocked: false,
            vcp: None,
            range: Range::FULL,
            features: Vec::new(),
            caps: BTreeMap::new(),
            inputs: None,
            input_hotkey: None,
        }
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

    /// Valeur acceptable pour un réglage à choix (ou l'alimentation) : une de celles que
    /// l'écran annonce. Sans liste annoncée, les valeurs MCCS d'alimentation.
    pub fn allows(&self, code: u8, value: u32) -> bool {
        match self.caps.get(&code).filter(|v| !v.is_empty()) {
            Some(list) => list.iter().any(|&v| u32::from(v) == value),
            None => code != display::VCP_POWER || [1, 4, 5].contains(&value),
        }
    }

    /// Préférences reçues de l'interface, vérifiées. Les capacités restent celles lues
    /// sur l'écran.
    pub fn checked(self, caps: BTreeMap<u8, Vec<u8>>) -> Res<Self> {
        let bad = || t("Plage invalide : le minimum doit rester sous le maximum", "Invalid range: the minimum must stay below the maximum").to_string();
        if !self.range.valid() || self.features.iter().any(|f| !f.range.valid()) {
            return Err(bad());
        }
        if self.features.iter().any(|f| COMMANDS.contains(&f.code)) {
            return Err(t("Ce code est une commande (remise à zéro…), pas un réglage", "This code is a command (reset…), not a setting").into());
        }
        let s = Self { caps, ..self };
        if s.inputs.is_some_and(|(a, b)| a == b || !s.allows(display::VCP_INPUT, a.into()) || !s.allows(display::VCP_INPUT, b.into())) {
            return Err(t("Choisissez deux entrées différentes parmi celles de l'écran", "Pick two different inputs among those of the display").into());
        }
        Ok(s.repaired())
    }

    /// Remet dans les bornes ce qui ne l'est pas : à la lecture du fichier, on corrige
    /// plutôt que de tout refuser.
    pub fn repaired(mut self) -> Self {
        self.name = self.name.trim().chars().filter(|c| !c.is_control()).take(40).collect();
        self.vcp = self.vcp.filter(|&c| c != 0);
        self.inputs = self.inputs.filter(|(a, b)| a != b);
        if !self.range.valid() {
            self.range = Range::FULL;
        }
        let mut seen = BTreeSet::new();
        self.features.retain(|f| f.code != 0 && !COMMANDS.contains(&f.code) && seen.insert(f.code));
        for f in &mut self.features {
            if !f.range.valid() {
                f.range = Range::FULL;
            }
            f.stop = f.stop.clamp(1, 100);
        }
        self
    }
}

/// Champ lu avec indulgence : une valeur inconnue prend la valeur par défaut au lieu de
/// rendre tout le fichier illisible.
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    Ok(serde_json::Value::deserialize(d).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
}

/// « Éteindre les écrans » : veille de Windows, signal DDC/CI, ou les deux.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OffMode {
    #[default]
    Soft,
    Hard,
    Both,
}

/// Signal DDC/CI d'extinction (code D6), enregistré sous sa valeur MCCS : 4 veille,
/// 5 arrêt, 6 les deux à la suite.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(try_from = "u32", into = "u32")]
pub enum PowerSignal {
    Standby,
    #[default]
    Off,
    Both,
}

impl TryFrom<u32> for PowerSignal {
    type Error = String;
    fn try_from(v: u32) -> Result<Self, String> {
        match v {
            4 => Ok(Self::Standby),
            5 => Ok(Self::Off),
            6 => Ok(Self::Both),
            _ => Err(format!("signal d'extinction inconnu : {v}")),
        }
    }
}

impl From<PowerSignal> for u32 {
    fn from(p: PowerSignal) -> u32 {
        match p {
            PowerSignal::Standby => 4,
            PowerSignal::Off => 5,
            PowerSignal::Both => 6,
        }
    }
}

impl PowerSignal {
    /// Valeurs D6 à envoyer pour éteindre, dans l'ordre.
    pub fn values(self) -> &'static [u32] {
        match self {
            Self::Standby => &[4],
            Self::Off => &[5],
            Self::Both => &[4, 5],
        }
    }
}

/// Réglages du panneau ouvert d'un clic sur l'icône de notification.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Panel {
    /// Un seul curseur pour tous les écrans.
    pub linked: bool,
    #[serde(deserialize_with = "lenient")]
    pub power: PowerSignal,
    #[serde(deserialize_with = "lenient")]
    pub off: OffMode,
    /// Pas des raccourcis de luminosité, en %.
    pub step: u32,
}

impl Default for Panel {
    fn default() -> Self {
        Self { linked: false, power: PowerSignal::default(), off: OffMode::default(), step: 10 }
    }
}

impl Panel {
    pub fn checked(mut self) -> Self {
        self.step = self.step.clamp(1, 50);
        self
    }
}

/// Luminosité selon l'heure : chaque jour, tous les écrans réglables suivent ces étapes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Schedule {
    pub on: bool,
    /// Transition continue d'une étape à la suivante, au lieu d'un palier à chaque étape.
    pub smooth: bool,
    /// (minutes depuis minuit, luminosité en %), triées par heure.
    pub steps: Vec<(u32, u32)>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self { on: false, smooth: true, steps: Vec::new() }
    }
}

impl Schedule {
    pub const MAX_STEPS: usize = 12;

    /// Étapes dans les bornes, triées, une seule par heure.
    pub fn checked(mut self) -> Self {
        self.steps.retain(|s| s.0 < 1440);
        for s in &mut self.steps {
            s.1 = s.1.min(100);
        }
        self.steps.sort_by_key(|s| s.0);
        self.steps.dedup_by_key(|s| s.0);
        self.steps.truncate(Self::MAX_STEPS);
        self
    }

    /// Luminosité prévue à `minute` (depuis minuit). Paliers : celle de la dernière étape
    /// passée, la veille au soir s'il le faut. Transitions : interpolée vers la suivante.
    pub fn target(&self, minute: u32) -> Option<u32> {
        let (first, last) = (self.steps.first()?, self.steps.last()?);
        let i = self.steps.iter().rposition(|s| s.0 <= minute);
        let cur = i.map_or(last, |i| &self.steps[i]);
        if !self.smooth || self.steps.len() < 2 {
            return Some(cur.1);
        }
        let next = i.and_then(|i| self.steps.get(i + 1)).unwrap_or(first);
        // Durées modulo un jour : l'étape du soir rejoint celle du matin par minuit.
        let span = f64::from((next.0 + 1440 - cur.0) % 1440).max(1.0);
        let done = f64::from((minute + 1440 - cur.0) % 1440);
        let (a, b) = (f64::from(cur.1), f64::from(next.1));
        Some((a + (b - a) * done / span).round() as u32)
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
    #[serde(default)]
    pub schedule: Schedule,
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
        let s: Store = serde_json::from_str(&text).unwrap_or_else(|e| {
            // Fichier abîmé : on le met de côté, daté, plutôt que de l'écraser au prochain enregistrement.
            let ms = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
            let _ = fs::rename(&f, f.with_file_name(format!("config-{ms}.bak.json")));
            crate::journal::write(crate::journal::Level::Error, t("Système", "System"), &tr!("Configuration illisible, mise de côté : {e}", "Unreadable configuration, set aside: {e}"));
            Self::default()
        });
        s.sanitized()
    }

    /// Remet dans les bornes ce qui vient du fichier : une plage inversée fausserait les
    /// calculs de luminosité, un délai nul annulerait chaque changement aussitôt.
    fn sanitized(mut self) -> Self {
        self.confirm.secs = self.confirm.secs.clamp(5, 120);
        self.panel = self.panel.checked();
        for sc in self.screens.values_mut() {
            *sc = std::mem::take(sc).repaired();
        }
        for p in &mut self.profiles {
            p.triggers = std::mem::take(&mut p.triggers).checked();
        }
        self.schedule = std::mem::take(&mut self.schedule).checked();
        self
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
        triggers: Triggers::default(),
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
    fn bad_values_in_the_file_are_repaired() {
        let json = r#"{"confirm":{"secs":0},"panel":{"power":9,"off":"nope","step":0},
            "screens":{"a":{"range":[80,20],"features":[{"code":18,"range":[60,10],"stop":0},{"code":4},{"code":18}]}}}"#;
        let s = serde_json::from_str::<Store>(json).unwrap().sanitized();
        assert_eq!((s.confirm.secs, s.panel.power, s.panel.off, s.panel.step), (5, PowerSignal::Off, OffMode::Soft, 1));
        let a = &s.screens["a"];
        assert_eq!(a.range, Range::FULL);
        assert_eq!(a.features, [Feature { code: 18, range: Range::FULL, linked: false, stop: 1 }]);
    }

    #[test]
    fn power_signal_keeps_its_file_format() {
        let p: Panel = serde_json::from_str(r#"{"power":4,"off":"both"}"#).unwrap();
        assert_eq!((p.power, p.off, p.power.values()), (PowerSignal::Standby, OffMode::Both, &[4][..]));
        assert_eq!(serde_json::to_value(&p).unwrap()["power"], 4);
        assert_eq!(PowerSignal::Both.values(), [4, 5]);
    }

    #[test]
    fn only_announced_values_are_allowed() {
        let s = Screen { caps: BTreeMap::from([(0x60, vec![0x0F, 0x11]), (0xD6, vec![1, 5])]), ..Default::default() };
        assert!(s.allows(0x60, 0x11) && !s.allows(0x60, 0x03));
        assert!(s.allows(0xD6, 5) && !s.allows(0xD6, 4));
        // Capacités inconnues : valeurs MCCS d'alimentation seulement.
        let none = Screen::default();
        assert!(none.allows(0xD6, 4) && !none.allows(0xD6, 7));
    }

    #[test]
    fn schedule_gives_the_brightness_of_the_hour() {
        let steps = vec![(8 * 60, 80), (22 * 60, 20)];
        let paliers = Schedule { on: true, smooth: false, steps: steps.clone() };
        assert_eq!(paliers.target(7 * 60), Some(20)); // avant la première étape : celle de la veille
        assert_eq!(paliers.target(8 * 60), Some(80));
        assert_eq!(paliers.target(21 * 60 + 59), Some(80));
        assert_eq!(paliers.target(23 * 60), Some(20));
        let doux = Schedule { smooth: true, ..paliers };
        assert_eq!(doux.target(8 * 60), Some(80));
        assert_eq!(doux.target(15 * 60), Some(50)); // à mi-chemin entre 8 h et 22 h
        assert_eq!(doux.target(22 * 60), Some(20));
        assert_eq!(doux.target(3 * 60), Some(50)); // à mi-chemin par minuit, de 22 h à 8 h
        assert_eq!(Schedule::default().target(600), None);
        assert_eq!(Schedule { steps: vec![(600, 40)], ..Schedule::default() }.target(0), Some(40));
    }

    #[test]
    fn schedule_steps_are_checked() {
        let s = Schedule { on: true, smooth: true, steps: vec![(1300, 250), (60, 10), (60, 90), (2000, 5)] }.checked();
        assert_eq!(s.steps, [(60, 10), (1300, 100)]);
        let many = Schedule { steps: (0..30).map(|i| (i * 10, 50)).collect(), ..Schedule::default() }.checked();
        assert_eq!(many.steps.len(), Schedule::MAX_STEPS);
    }

    #[test]
    fn triggers_are_checked_and_read_leniently() {
        let t = Triggers { at: Some(1500), power: Some(PowerSource::Ac), exe: r" C:\Jeux\Jeu.EXE ".into() }.checked();
        assert_eq!((t.at, t.power, t.exe.as_str()), (None, Some(PowerSource::Ac), "Jeu.EXE"));
        assert_eq!(Triggers { exe: "bloc-notes".into(), ..Default::default() }.checked().exe, "bloc-notes.exe");
        assert_eq!(Triggers::default().checked(), Triggers::default());
        // Valeurs inconnues dans le fichier : le déclencheur est ignoré, pas le fichier.
        let p: Profile = serde_json::from_str(
            r#"{"id":"1","name":"x","sig":"","tiles":[],"snap":{"paths":[],"modes":[],"adapters":{}},"triggers":{"at":"midi","power":"solaire","exe":"a.exe"}}"#,
        )
        .unwrap();
        assert_eq!(p.triggers, Triggers { at: None, power: None, exe: "a.exe".into() });
        let p: Triggers = serde_json::from_str(r#"{"at":480,"power":"dc"}"#).unwrap();
        assert_eq!((p.at, p.power), (Some(480), Some(PowerSource::Dc)));
    }

    #[test]
    fn input_pairs_are_checked() {
        let caps = BTreeMap::from([(0x60, vec![0x0F, 0x11])]);
        let s = Screen { inputs: Some((0x0F, 0x11)), ..Default::default() };
        assert_eq!(s.checked(caps.clone()).unwrap().inputs, Some((0x0F, 0x11)));
        assert!(Screen { inputs: Some((0x0F, 0x0F)), ..Default::default() }.checked(caps.clone()).is_err());
        assert!(Screen { inputs: Some((0x0F, 0x03)), ..Default::default() }.checked(caps).is_err());
        assert_eq!(Screen { inputs: Some((3, 3)), ..Default::default() }.repaired().inputs, None);
    }

    #[test]
    fn old_configs_get_default_settings() {
        let s: Store = serde_json::from_str(r#"{"profiles":[]}"#).unwrap();
        assert!(s.confirm.on);
        assert_eq!(s.confirm.secs, 15);
        let s: Store = serde_json::from_str(r#"{"confirm":{"on":false}}"#).unwrap();
        assert_eq!((s.confirm.on, s.confirm.secs), (false, 15));
        assert_eq!((s.panel.power, s.panel.step, s.panel.off), (PowerSignal::Off, 10, OffMode::Soft));
        let s: Store = serde_json::from_str(r#"{"screens":{"a":{"hidden":true,"features":[{"code":18}]}}}"#).unwrap();
        let a = &s.screens["a"];
        assert!(a.hidden && a.range == Range::FULL && a.features[0].stop == 100);
        assert_eq!((a.inputs, &a.input_hotkey, &s.schedule), (None, &None, &Schedule::default()));
    }
}
