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

/// Durée d'affichage des numéros.
const ID_SHOW: Duration = Duration::from_millis(2800);
/// Des écrans dupliqués se relaient sur leur carte commune : chacun ce temps-là, fondu compris.
const ID_STEP: Duration = Duration::from_millis(2500);

/// Carte d'identification d'un écran et de ceux qui le dupliquent (même image, même
/// carte). `at` : l'écran désigné, qui reste lumineux pendant que les autres s'assombrissent.
#[derive(Serialize)]
struct Card {
    #[serde(skip)]
    label: String,
    nums: Vec<usize>,
    names: Vec<String>,
    res: String,
    at: Option<usize>,
    /// Clé de chaque écran et luminosité matérielle à rétablir, s'il en a une.
    #[serde(skip)]
    levels: Vec<(String, Option<u32>)>,
}

/// Affiche le numéro de chaque écran actif, comme le bouton « Identifier » de Windows.
pub(crate) fn identify(app: &AppHandle) -> Res<()> {
    // Une à la fois : la suivante prendrait un écran assombri pour réglé ainsi.
    static SHOWING: AtomicBool = AtomicBool::new(false);
    let mons = display::monitors()?;
    if SHOWING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let mut cards = Vec::new();
    let mut failed = Ok(());
    for (i, m) in mons.iter().enumerate().filter(|(_, m)| m.active && m.mirror_of.is_none()) {
        let group: Vec<(usize, &display::Monitor)> =
            mons.iter().enumerate().filter(|(_, o)| o.key == m.key || o.mirror_of.as_ref() == Some(&m.key)).collect();
        // Un voile logiciel couvrirait l'image commune : seule la luminosité matérielle distingue.
        let levels: Vec<(String, Option<u32>)> = group.iter().map(|(_, o)| (o.key.clone(), o.brightness.filter(|_| !o.soft))).collect();
        let card = Card {
            label: format!("identify{i}"),
            nums: group.iter().map(|(j, _)| j + 1).collect(),
            names: group.iter().map(|(_, o)| o.name.clone()).collect(),
            res: format!("{} × {}", m.w, m.h),
            at: (levels.len() > 1 && levels.iter().any(|l| l.1.is_some())).then_some(0),
            levels,
        };
        if let Some(w) = app.get_webview_window(&card.label) {
            let _ = w.destroy();
        }
        let built = window(app, &card.label, "identify.html")
            .initialization_script(format!("window.ID = {};", serde_json::to_string(&card).unwrap_or_default()))
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
            .build();
        match built {
            Ok(w) => {
                let _ = w.set_position(PhysicalPosition::new(m.x + 48, m.y + 48));
                let _ = w.show();
                cards.push(card);
            }
            Err(e) => {
                failed = Err(e.to_string());
                break;
            }
        }
    }
    let look = store(app).identify;
    let app = app.clone();
    thread::spawn(move || {
        let start = Instant::now();
        relay(&app, &mut cards, look);
        thread::sleep(ID_SHOW.saturating_sub(start.elapsed()));
        for c in &cards {
            if let Some(w) = app.get_webview_window(&c.label) {
                let _ = w.destroy();
            }
        }
        SHOWING.store(false, Ordering::SeqCst);
    });
    failed
}

/// Écrans dupliqués : chacun à son tour est désigné sur la carte, à la luminosité choisie
/// dans les paramètres (`look`), pendant que les autres s'assombrissent. À la fin, chacun
/// retrouve la sienne.
fn relay(app: &AppHandle, cards: &mut [Card], look: store::Identify) {
    let steps = cards.iter().filter(|c| c.at.is_some()).map(|c| c.nums.len()).max().unwrap_or(0);
    // Luminosité d'origine de chaque écran concerné, et celle qu'il a maintenant.
    let orig: Vec<(String, u32)> =
        cards.iter().filter(|c| c.at.is_some()).flat_map(|c| &c.levels).filter_map(|(k, l)| Some((k.clone(), (*l)?))).collect();
    let mut now: BTreeMap<String, u32> = orig.iter().cloned().collect();
    for step in 0..steps {
        let started = Instant::now();
        let mut to = Vec::new();
        for c in cards.iter_mut().filter(|c| c.at.is_some()) {
            let at = step % c.nums.len();
            if step > 0 {
                c.at = Some(at);
                if let (Some(w), Ok(js)) = (app.get_webview_window(&c.label), serde_json::to_string(&*c)) {
                    let _ = w.eval(format!("show({js})"));
                }
            }
            // L'écran désigné d'abord : jamais tout le groupe dans le noir.
            for i in std::iter::once(at).chain((0..c.levels.len()).filter(|&i| i != at)) {
                if let (key, Some(level)) = &c.levels[i] {
                    to.push((key.clone(), if i == at { look.lit.unwrap_or(*level) } else { look.dim }));
                }
            }
        }
        fade(&mut now, &to);
        thread::sleep(ID_STEP.saturating_sub(started.elapsed()));
    }
    fade(&mut now, &orig);
}

/// Un saut de luminosité surprend l'œil : quelques paliers l'adoucissent. Les écritures
/// DDC/CI passent une à une par le verrou du bus, environ 50 ms chacune.
/// ponytail: durée fixe ; en faire un réglage si elle paraît lente ou brusque sur certains écrans.
const FADE: Duration = Duration::from_millis(700);
const FADE_STEPS: u32 = 6;

/// Mène chaque écran de sa luminosité actuelle (`now`) à celle visée, ensemble et par paliers.
fn fade(now: &mut BTreeMap<String, u32>, to: &[(String, u32)]) {
    let moves: Vec<(&str, u32, u32)> =
        to.iter().filter_map(|(k, t)| now.get(k).filter(|&&f| f != *t).map(|&f| (k.as_str(), f, *t))).collect();
    if moves.is_empty() {
        return;
    }
    for n in 1..=FADE_STEPS {
        let tick = Instant::now();
        for &(key, from, to) in &moves {
            let v = tier(from, to, n);
            if n == FADE_STEPS {
                if settle(key, v) {
                    now.insert(key.into(), v);
                }
            } else if v != tier(from, to, n - 1) {
                let _ = display::set_brightness(key, v);
            }
        }
        if n < FADE_STEPS {
            thread::sleep((FADE / FADE_STEPS).saturating_sub(tick.elapsed()));
        }
    }
}

/// Palier `n` sur `FADE_STEPS` d'un fondu de `from` à `to`.
fn tier(from: u32, to: u32, n: u32) -> u32 {
    let (f, t) = (i64::from(from), i64::from(to));
    (f + (t - f) * i64::from(n) / i64::from(FADE_STEPS)) as u32
}

/// Le dernier palier compte : un raté du bus DDC/CI laisserait l'écran sombre. Quelques essais.
fn settle(key: &str, level: u32) -> bool {
    for _ in 0..3 {
        if display::set_brightness(key, level).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_tiers_end_exactly_on_target() {
        let tiers = |from, to| (0..=FADE_STEPS).map(|n| tier(from, to, n)).collect::<Vec<_>>();
        assert_eq!(tiers(60, 0), [60, 50, 40, 30, 20, 10, 0]);
        assert_eq!(tiers(0, 60), [0, 10, 20, 30, 40, 50, 60]);
        // Petit écart : des paliers se répètent (non renvoyés), le dernier tombe juste.
        assert_eq!(tiers(19, 14), [19, 19, 18, 17, 16, 15, 14]);
    }
}
