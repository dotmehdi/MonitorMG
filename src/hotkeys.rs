//! Raccourcis clavier globaux : profils, extinction des écrans, luminosité, bascule d'entrée.

use super::*;

/// Propriétaires des raccourcis d'action (les autres sont des id de profil) : éteindre
/// les écrans, monter et baisser la luminosité.
pub(crate) const OFF: &str = "@off";
pub(crate) const UP: &str = "@up";
pub(crate) const DOWN: &str = "@down";
/// Préfixe du propriétaire d'une bascule d'entrée, suivi de la clé de l'écran.
pub(crate) const INPUT: &str = "@input|";

/// Raccourcis globaux : (propriétaire, combinaison). Le propriétaire est un id de profil,
/// `OFF`, `UP`, `DOWN`, ou `INPUT` suivi d'une clé d'écran.
pub(crate) fn hotkeys(s: &store::Store) -> Vec<(String, String)> {
    let actions = [(OFF, &s.off_hotkey), (UP, &s.up_hotkey), (DOWN, &s.down_hotkey)];
    let actions = actions.into_iter().filter_map(|(o, h)| Some((o.to_string(), h.clone()?)));
    let inputs = s.screens.iter().filter_map(|(k, sc)| Some((format!("{INPUT}{k}"), sc.input_hotkey.clone()?)));
    s.profiles.iter().filter_map(|p| Some((p.id.clone(), p.hotkey.clone()?))).chain(actions).chain(inputs).collect()
}

pub(crate) fn hotkey_slot<'a>(s: &'a mut store::Store, owner: &str) -> Res<&'a mut Option<String>> {
    Ok(match owner {
        OFF => &mut s.off_hotkey,
        UP => &mut s.up_hotkey,
        DOWN => &mut s.down_hotkey,
        o if o.starts_with(INPUT) => &mut s.screens.entry(o[INPUT.len()..].to_string()).or_default().input_hotkey,
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
        Some((o, _)) if o.starts_with(INPUT) => {
            let app = app.clone();
            thread::spawn(move || toggle_input(&app, &o[INPUT.len()..]));
        }
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

/// Nom MCCS d'une entrée (code 0x60), pour le journal ; les mêmes que dans `ui/vcp.js`.
fn input_name(v: u8) -> String {
    match v {
        0x01 => "VGA 1",
        0x02 => "VGA 2",
        0x03 => "DVI 1",
        0x04 => "DVI 2",
        0x0F => "DisplayPort 1",
        0x10 => "DisplayPort 2",
        0x11 => "HDMI 1",
        0x12 => "HDMI 2",
        0x1B => "USB-C",
        _ => return format!("0x{v:02X}"),
    }
    .into()
}

/// Entrée vers laquelle basculer : la seconde si l'écran affiche la première, sinon la
/// première (état illisible ou autre entrée compris).
fn next_input((a, b): (u8, u8), current: Option<u32>) -> u8 {
    if current == Some(u32::from(a)) {
        b
    } else {
        a
    }
}

/// Raccourci de bascule d'entrée : un KVM logiciel pour un écran branché à deux machines.
/// Pas de confirmation, c'est le but ; le journal garde la trace.
pub(crate) fn toggle_input(app: &AppHandle, key: &str) {
    let Some(pair) = store(app).screens.get(key).and_then(|sc| sc.inputs) else { return };
    let name = display::name(key);
    // Certains écrans renvoient l'entrée avec un octet de poids fort.
    let current = display::vcp_get(key, &[display::VCP_INPUT]).ok().and_then(|v| v.first().and_then(|x| x.1)).map(|(cur, _)| cur & 0xFF);
    let to = next_input(pair, current);
    match display::vcp_set(key, display::VCP_INPUT, to.into()) {
        Ok(()) => note(app, Level::Ok, Src::Hotkey, &tr!("{name} : entrée {}", "{name}: input {}", input_name(to))),
        Err(e) => {
            let m = tr!("{name} : changement d'entrée impossible. {e}", "{name}: can't switch input. {e}");
            note(app, Level::Error, Src::Hotkey, &m);
            report(app, &m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_toggle_picks_the_other_input() {
        assert_eq!(next_input((0x0F, 0x11), Some(0x0F)), 0x11);
        assert_eq!(next_input((0x0F, 0x11), Some(0x11)), 0x0F);
        // Entrée illisible ou autre : on va vers la première.
        assert_eq!(next_input((0x0F, 0x11), None), 0x0F);
        assert_eq!(next_input((0x0F, 0x11), Some(0x03)), 0x0F);
        assert_eq!((input_name(0x11), input_name(0x99)), ("HDMI 1".to_string(), "0x99".to_string()));
    }

    #[test]
    fn input_hotkeys_are_listed_with_their_screen() {
        let mut s = store::Store { off_hotkey: Some("Ctrl+Alt+KeyO".into()), ..Default::default() };
        s.screens.insert("écran-a".into(), store::Screen { input_hotkey: Some("Ctrl+Alt+KeyI".into()), ..Default::default() });
        s.screens.insert("écran-b".into(), store::Screen::default());
        let keys = hotkeys(&s);
        assert_eq!(keys, [(OFF.to_string(), "Ctrl+Alt+KeyO".to_string()), (format!("{INPUT}écran-a"), "Ctrl+Alt+KeyI".to_string())]);
        *hotkey_slot(&mut s, &format!("{INPUT}écran-b")).unwrap() = Some("F9".into());
        assert_eq!(s.screens["écran-b"].input_hotkey.as_deref(), Some("F9"));
    }
}
