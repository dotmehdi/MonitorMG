//! Raccourcis clavier globaux : profils, extinction des écrans, luminosité.

use super::*;

/// Propriétaires des raccourcis d'action (les autres sont des id de profil) : éteindre
/// les écrans, monter et baisser la luminosité.
pub(crate) const OFF: &str = "@off";
pub(crate) const UP: &str = "@up";
pub(crate) const DOWN: &str = "@down";

/// Raccourcis globaux : (propriétaire, combinaison). Le propriétaire est un id de profil,
/// ou `OFF`, `UP`, `DOWN`.
pub(crate) fn hotkeys(s: &store::Store) -> Vec<(String, String)> {
    let actions = [(OFF, &s.off_hotkey), (UP, &s.up_hotkey), (DOWN, &s.down_hotkey)];
    let actions = actions.into_iter().filter_map(|(o, h)| Some((o.to_string(), h.clone()?)));
    s.profiles.iter().filter_map(|p| Some((p.id.clone(), p.hotkey.clone()?))).chain(actions).collect()
}

pub(crate) fn hotkey_slot<'a>(s: &'a mut store::Store, owner: &str) -> Res<&'a mut Option<String>> {
    Ok(match owner {
        OFF => &mut s.off_hotkey,
        UP => &mut s.up_hotkey,
        DOWN => &mut s.down_hotkey,
        id => &mut s.get_mut(id)?.hotkey,
    })
}

/// Attribue ou retire le raccourci global d'un profil ou de l'extinction des écrans.
pub(crate) fn assign_hotkey(app: &AppHandle, owner: &str, hotkey: Option<String>) -> Res<()> {
    if let Some(h) = &hotkey {
        let sc: Shortcut = h.parse().map_err(|_| t("Combinaison non reconnue", "Key combination not recognized").to_string())?;
        let taken = hotkeys(&store(app)).iter().any(|(o, x)| o != owner && x.parse::<Shortcut>().is_ok_and(|x| x.id() == sc.id()));
        if taken {
            return Err(t("Déjà utilisé par un autre raccourci de MonitorMG", "Already used by another MonitorMG shortcut").into());
        }
    }
    let previous = std::mem::replace(hotkey_slot(&mut store(app), owner)?, hotkey);
    if register_hotkeys(app).iter().any(|o| o == owner) {
        // Refusé par Windows : l'ancien raccourci reprend sa place, il n'est pas perdu.
        *hotkey_slot(&mut store(app), owner)? = previous;
        register_hotkeys(app);
        return Err(t("Cette combinaison est déjà prise par une autre application", "This combination is already taken by another app").into());
    }
    store(app).save()
}

/// Réenregistre les raccourcis globaux ; renvoie les propriétaires dont la combinaison est refusée.
pub(crate) fn register_hotkeys(app: &AppHandle) -> Vec<String> {
    // Copie d'abord : l'enregistrement passe par le thread principal, qui peut
    // lui-même attendre le verrou des profils (gestionnaire de raccourci).
    let keys = hotkeys(&store(app));
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    keys.into_iter().filter(|(_, h)| gs.register(h.as_str()).is_err()).map(|(id, _)| id).collect()
}

pub(crate) fn on_hotkey(app: &AppHandle, sc: &Shortcut) {
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
pub(crate) fn screens_off(app: &AppHandle, src: Src) {
    note(app, Level::Ok, src, t("Écrans éteints", "Displays turned off"));
    let (mode, signal, screens) = {
        let s = store(app);
        (s.panel.off, s.panel.power, s.screens.clone())
    };
    thread::spawn(move || {
        if mode != store::OffMode::Soft {
            display::power_off(&power_targets(signal, &screens));
            if mode == store::OffMode::Hard {
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


/// Écrans externes actifs qui reçoivent le signal d'extinction DDC/CI, avec leurs valeurs :
/// seulement ceux qui annoncent l'alimentation (D6), et seulement les valeurs annoncées.
/// Un écran dont les commandes n'ont jamais été lues reçoit le signal choisi.
fn power_targets(signal: store::PowerSignal, screens: &BTreeMap<String, store::Screen>) -> BTreeMap<String, Vec<u32>> {
    display::monitors()
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.active && !m.internal)
        .filter_map(|m| {
            let sc = screens.get(&m.key).cloned().unwrap_or_default();
            if !sc.caps.is_empty() && !sc.caps.contains_key(&display::VCP_POWER) {
                return None;
            }
            let values: Vec<u32> = signal.values().iter().copied().filter(|&v| sc.allows(display::VCP_POWER, v)).collect();
            (!values.is_empty()).then_some((m.key, values))
        })
        .collect()
}

/// Raccourcis de luminosité : tous les écrans réglables montent ou baissent d'un pas.
pub(crate) fn nudge(app: &AppHandle, up: bool) {
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
