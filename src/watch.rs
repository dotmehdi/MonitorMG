//! Surveillance des branchements, des changements d'affichage, du thème, de
//! l'alimentation et de la sortie de veille.

use super::*;

pub(crate) static APP: OnceLock<AppHandle> = OnceLock::new();
/// Signale un message système au thread qui attend la fin de la rafale.
static WATCH: OnceLock<mpsc::Sender<()>> = OnceLock::new();
/// Fenêtre invisible de `watch_displays`, qui reçoit aussi l'ordre d'éteindre les écrans.
pub(crate) static WATCHER: AtomicIsize = AtomicIsize::new(0);
pub(crate) static SCREENS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
pub(crate) static DISPLAY_DIRTY: AtomicBool = AtomicBool::new(false);

/// Fenêtre invisible qui écoute les branchements et changements de mode.
/// (Les fenêtres « message-only » ne reçoivent pas ces diffusions.)
pub(crate) fn watch_displays(app: AppHandle) {
    let _ = APP.set(app);
    *lock(&SCREENS) = display::connected().unwrap_or_default();
    // Windows envoie ces messages en rafale : un seul thread attend le calme.
    let _ = WATCH.set(debounce(Duration::from_millis(500), || {
        let Some(app) = APP.get() else { return };
        if DISPLAY_DIRTY.swap(false, Ordering::SeqCst) {
            display::forget_modes();
            plugged(app);
            read_brightness(app, true);
            // Un écran déplacé, débranché ou réactivé : son voile suit.
            dim::refresh();
        }
        changed(app);
    }));
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
    if msg == dim::WM_VEILS {
        dim::sync();
        return LRESULT(0);
    }
    let display = msg == WM_DISPLAYCHANGE || (msg == WM_DEVICECHANGE && wp.0 == DBT_DEVNODES_CHANGED as usize);
    // WM_SETTINGCHANGE est très fréquent : seul le thème (icône, accent) nous concerne.
    let theme = msg == WM_SETTINGCHANGE
        && lp.0 != 0
        && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet");
    if display || theme {
        if display {
            DISPLAY_DIRTY.store(true, Ordering::SeqCst);
        }
        if let Some(tx) = WATCH.get() {
            let _ = tx.send(());
        }
    }
    if msg == WM_POWERBROADCAST {
        match wp.0 as u32 {
            PBT_APMPOWERSTATUSCHANGE => auto::wake(auto::Wake::Power),
            PBT_APMRESUMEAUTOMATIC => auto::wake(auto::Wake::Resume),
            _ => {}
        }
    }
    DefWindowProcW(h, msg, wp, lp)
}

/// Consigne les écrans branchés ou débranchés et, pour un dock, applique le premier
/// profil automatique dont les écrans correspondent exactement.
pub(crate) fn plugged(app: &AppHandle) {
    let Ok(now) = display::connected() else { return };
    let before = std::mem::replace(&mut *lock(&SCREENS), now.clone());
    if before == now {
        return;
    }
    for k in now.difference(&before) {
        note(app, Level::Ok, Src::System, &tr!("{} branché", "{} connected", display::name(k)));
    }
    match before.difference(&now).count() {
        0 => {}
        1 => note(app, Level::Ok, Src::System, t("Un écran a été débranché", "A display was disconnected")),
        n => note(app, Level::Ok, Src::System, &tr!("{n} écrans ont été débranchés", "{n} displays were disconnected")),
    }
    // Pilote réinstallé, écran rebranché ailleurs : Windows a pu renuméroter les connecteurs.
    follow_screens(app);
    let found = store(app).profiles.iter().find(|p| p.auto && p.screens == now).map(|p| (p.id.clone(), p.sig.clone()));
    let Some((id, sig)) = found else { return };
    // Windows a souvent déjà rétabli cet affichage : on évite un clignotement.
    if display::signature().is_ok_and(|s| s == sig) {
        return;
    }
    // Action automatique : consignée dans l'activité, sans boîte de dialogue.
    let _ = apply_profile(app, Src::Plug, &id);
}
