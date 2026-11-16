//! Automatismes : luminosité selon l'heure, et profils appliqués à heure fixe, au
//! changement d'alimentation ou au lancement d'un programme.

use super::*;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// Ce qui réveille le thread des automatismes avant la minute suivante.
pub(crate) enum Wake {
    /// Réglages modifiés : la luminosité prévue pour cette heure est appliquée.
    Apply,
    /// Sortie de veille : de même, une fois les écrans de nouveau joignables.
    Resume,
    /// La source d'alimentation a peut-être changé.
    Power,
}

static WAKE: OnceLock<mpsc::Sender<Wake>> = OnceLock::new();

pub(crate) fn wake(w: Wake) {
    if let Some(tx) = WAKE.get() {
        let _ = tx.send(w);
    }
}

/// (jour du mois, minutes depuis minuit, secondes), à l'heure locale.
fn now() -> (u16, u32, u32) {
    let t = unsafe { GetLocalTime() };
    (t.wDay, u32::from(t.wHour) * 60 + u32::from(t.wMinute), u32::from(t.wSecond))
}

fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn power_source() -> Option<store::PowerSource> {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s) }.ok()?;
    match s.ACLineStatus {
        0 => Some(store::PowerSource::Dc),
        1 => Some(store::PowerSource::Ac),
        _ => None,
    }
}

/// Une échéance (minutes depuis minuit) tombe-t-elle maintenant ? `prev` : la dernière
/// minute traitée, le même jour. Un tic en retard d'une minute ou deux rattrape les
/// échéances sautées ; après une veille, rien n'est rattrapé.
fn due(prev: Option<u32>, minute: u32, at: u32) -> bool {
    match prev {
        Some(p) if p < minute && minute - p <= 2 => p < at && at <= minute,
        _ => at == minute,
    }
}

#[derive(Default)]
struct State {
    /// Dernière minute traitée : (jour du mois, minute).
    minute: Option<(u16, u32)>,
    power: Option<store::PowerSource>,
    /// Luminosité envoyée à chaque écran par la planification : un écart révèle un réglage manuel.
    sent: BTreeMap<String, u32>,
    /// Écrans réglés à la main depuis : laissés tranquilles jusqu'à l'étape suivante.
    held: BTreeSet<String>,
}

impl State {
    /// Règle les écrans réglables à `pct`, sauf ceux tenus à l'écart. Vrai si l'un a changé.
    fn set(&mut self, app: &AppHandle, pct: u32) -> bool {
        let mut any = false;
        for m in display::monitors().unwrap_or_default().into_iter().filter(|m| m.active) {
            let Some(cur) = m.brightness else { continue };
            if self.held.contains(&m.key) {
                continue;
            }
            if cur == pct {
                // Déjà à la bonne valeur : notée quand même, pour reconnaître un réglage manuel ensuite.
                self.sent.insert(m.key, pct);
            } else if display::set_brightness(&m.key, pct).is_ok() {
                follow(app, &m.key, pct);
                self.sent.insert(m.key, pct);
                any = true;
            }
        }
        if any {
            changed(app);
        }
        any
    }

    /// Démarrage, réveil, réglages modifiés : la luminosité prévue pour cette heure, partout.
    fn apply_now(&mut self, app: &AppHandle) {
        self.held.clear();
        self.sent.clear();
        let schedule = store(app).schedule.clone();
        if !schedule.on {
            return;
        }
        if let Some(pct) = schedule.target(now().1) {
            if self.set(app, pct) {
                note(app, Level::Info, Src::Schedule, &tr!("Luminosité selon l'heure : {pct}{NNBSP}%", "Brightness by time of day: {pct}%"));
            }
        }
    }

    /// Chaque minute : étapes de luminosité et profils à heure fixe.
    fn tick(&mut self, app: &AppHandle) {
        let (day, minute, _) = now();
        if self.minute == Some((day, minute)) {
            return;
        }
        let prev = self.minute.replace((day, minute)).filter(|p| p.0 == day).map(|p| p.1);
        self.brightness(app, prev, minute);
        let timed: Vec<String> = store(app).profiles.iter().filter(|p| p.triggers.at.is_some_and(|at| due(prev, minute, at))).map(|p| p.id.clone()).collect();
        for id in timed {
            // Action automatique : consignée dans l'activité, sans boîte de dialogue.
            let _ = apply_profile(app, Src::Schedule, &id);
        }
    }

    fn brightness(&mut self, app: &AppHandle, prev: Option<u32>, minute: u32) {
        let schedule = store(app).schedule.clone();
        let Some(pct) = schedule.target(minute).filter(|_| schedule.on) else {
            self.held.clear();
            self.sent.clear();
            return;
        };
        if let Some(step) = schedule.steps.iter().rev().find(|s| due(prev, minute, s.0)) {
            // Une étape commence : tous les écrans la suivent, réglés à la main ou non.
            self.held.clear();
            if self.set(app, pct) {
                note(app, Level::Ok, Src::Schedule, &tr!("Luminosité {pct}{NNBSP}% (étape de {})", "Brightness {pct}% ({} step)", clock(step.0)));
            }
        } else if schedule.smooth {
            // Entre deux étapes, la transition s'arrête pour un écran réglé à la main.
            for m in display::monitors().unwrap_or_default() {
                if let (Some(cur), Some(&sent)) = (m.brightness, self.sent.get(&m.key)) {
                    if cur.abs_diff(sent) > 2 {
                        self.held.insert(m.key);
                    }
                }
            }
            self.set(app, pct);
        }
    }

    fn power_changed(&mut self, app: &AppHandle) {
        let now = power_source();
        if now == self.power {
            return;
        }
        self.power = now;
        let Some(source) = now else { return };
        let id = store(app).profiles.iter().find(|p| p.triggers.power == Some(source)).map(|p| p.id.clone());
        if let Some(id) = id {
            let _ = apply_profile(app, Src::Power, &id);
        }
    }
}

/// Lance le thread des automatismes : il se réveille à chaque minute, et sur `wake`.
pub(crate) fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel();
    if WAKE.set(tx).is_err() {
        return;
    }
    thread::spawn(move || {
        let mut st = State { power: power_source(), ..State::default() };
        st.apply_now(&app);
        loop {
            let wait = Duration::from_secs(u64::from(60 - now().2.min(59)));
            match rx.recv_timeout(wait) {
                Ok(Wake::Apply) => st.apply_now(&app),
                Ok(Wake::Resume) => {
                    // Au réveil, les écrans mettent quelques secondes à répondre en DDC/CI.
                    thread::sleep(Duration::from_secs(8));
                    st.apply_now(&app);
                }
                Ok(Wake::Power) => st.power_changed(&app),
                Err(mpsc::RecvTimeoutError::Timeout) => st.tick(&app),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

/// Noms des exécutables en cours, en minuscules. Vide si la liste est illisible.
fn running_programs() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snap, &mut e).is_ok();
        while more {
            out.insert(display::wstr(&e.szExeFile).to_lowercase());
            more = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Programmes qui viennent d'apparaître parmi ceux attendus : (exécutable en minuscules, id du profil).
fn launched<'a>(wanted: &'a [(String, String)], before: &BTreeSet<String>, now: &BTreeSet<String>) -> Vec<&'a str> {
    wanted.iter().filter(|(exe, _)| now.contains(exe) && !before.contains(exe)).map(|(_, id)| id.as_str()).collect()
}

/// Applique un profil au lancement de son programme. La liste des processus n'est lue,
/// toutes les 5 s, que si un profil a ce déclencheur ; rien ne se passe à la fermeture.
pub(crate) fn watch_programs(app: AppHandle) {
    thread::spawn(move || {
        let mut before: Option<BTreeSet<String>> = None;
        loop {
            thread::sleep(Duration::from_secs(5));
            let wanted: Vec<(String, String)> =
                store(&app).profiles.iter().filter(|p| !p.triggers.exe.is_empty()).map(|p| (p.triggers.exe.to_lowercase(), p.id.clone())).collect();
            if wanted.is_empty() {
                before = None;
                continue;
            }
            let now = running_programs();
            if now.is_empty() {
                continue;
            }
            // Première lecture : un programme déjà ouvert ne déclenche rien.
            if let Some(b) = &before {
                for id in launched(&wanted, b, &now) {
                    let _ = apply_profile(&app, Src::Program, id);
                }
            }
            before = Some(now);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_fire_once_and_catch_up_a_late_tick() {
        assert!(due(Some(599), 600, 600));
        assert!(!due(Some(600), 601, 600));
        // Tic en retard d'une minute : l'échéance sautée est rattrapée.
        assert!(due(Some(599), 601, 600));
        // Après une veille (ou au premier tic), seule la minute exacte compte.
        assert!(!due(Some(300), 700, 600));
        assert!(due(None, 600, 600) && !due(None, 601, 600));
        assert_eq!(clock(22 * 60 + 5), "22:05");
    }

    #[test]
    fn only_newly_started_programs_trigger() {
        let wanted = vec![("jeu.exe".to_string(), "1".to_string()), ("montage.exe".to_string(), "2".to_string())];
        let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(launched(&wanted, &set(&["explorer.exe"]), &set(&["explorer.exe", "jeu.exe"])), ["1"]);
        assert!(launched(&wanted, &set(&["jeu.exe"]), &set(&["jeu.exe"])).is_empty());
        assert!(launched(&wanted, &set(&["jeu.exe"]), &set(&[])).is_empty());
    }
}
