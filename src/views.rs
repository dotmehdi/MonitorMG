//! Fenêtres de l'application : principale, panneau de la zone de notification,
//! confirmation et identification des écrans.

use super::*;

pub(crate) const WEBVIEW2_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
/// Largeur du panneau ouvert d'un clic sur l'icône de notification.
pub(crate) const FLYOUT_W: f64 = 360.0;
pub(crate) static OPENING: AtomicBool = AtomicBool::new(false);
/// Fermeture du panneau par perte du focus : le clic sur l'icône qui l'a causée ne doit
/// pas le rouvrir aussitôt.
pub(crate) static FLYOUT_CLOSED: Mutex<Option<Instant>> = Mutex::new(None);

/// Fenêtre de l'application, avec la langue de l'interface dans `window.LANG` (lue
/// par ui/i18n.js). En version portable, les données de WebView2 restent à côté de l'exécutable.
pub(crate) fn window<'a>(app: &'a AppHandle, label: &str, page: &str) -> WebviewWindowBuilder<'a, Wry, AppHandle> {
    let b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .initialization_script(format!("window.LANG = \"{}\";", t("fr", "en")));
    if store::portable() {
        b.data_directory(store::data_dir().join("webview"))
    } else {
        b
    }
}

pub(crate) fn show_confirm(app: &AppHandle, secs: u64) {
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

pub(crate) fn show_main(app: &AppHandle) {
    show_main_at(app, None);
}

/// Ouvre la fenêtre principale, sur une vue donnée (« ddc »…) si besoin.
pub(crate) fn show_main_at(app: &AppHandle, view: Option<String>) {
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
        // Copie d'abord : `on_screen` attend le thread principal, jamais sous verrou.
        let saved = store(&app).window;
        let place = saved.filter(|p| on_screen(&app, p));
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
                journal::write(Level::Error, Src::System.label(), &tr!("Fenêtre impossible à ouvrir : {e}", "Can't open the window: {e}"));
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
pub(crate) fn toggle_flyout(app: &AppHandle) {
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
pub(crate) fn show_flyout(app: &AppHandle) {
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
pub(crate) fn on_screen(app: &AppHandle, p: &store::Place) -> bool {
    let (x, y) = (p.x + p.w as i32 / 2, p.y + 16);
    app.available_monitors().unwrap_or_default().iter().any(|m| {
        let (o, s) = (m.position(), m.size());
        x >= o.x && x < o.x + s.width as i32 && y >= o.y && y < o.y + s.height as i32
    })
}

pub(crate) fn save_place(app: &AppHandle, w: &WebviewWindow) {
    let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) else { return };
    let max = w.is_maximized().unwrap_or(false);
    let mut s = store(app);
    s.window = Some(store::Place { x: pos.x, y: pos.y, w: size.width, h: size.height, max });
    let _ = s.save();
}

/// Affiche le numéro de chaque écran actif, comme le bouton « Identifier » de Windows.
pub(crate) fn identify(app: &AppHandle) -> Res<()> {
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
