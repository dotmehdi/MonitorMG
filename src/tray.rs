//! Icône et menu de la zone de notification.

use super::*;

/// Paliers de luminosité du menu de notification.
pub(crate) const BRIGHTNESS_STEPS: [u32; 12] = [0, 5, 15, 25, 35, 45, 55, 65, 75, 85, 95, 100];

/// Icônes proposées pour la zone de notification : nom, glyphe clair (barre des tâches
/// sombre), glyphe foncé (barre claire). La première est celle par défaut.
pub(crate) const TRAY_ICONS: [(&str, &[u8], &[u8]); 5] = [
    ("ecrans", include_bytes!("../icons/tray/ecrans-light.png"), include_bytes!("../icons/tray/ecrans-dark.png")),
    ("moniteur", include_bytes!("../icons/tray/moniteur-light.png"), include_bytes!("../icons/tray/moniteur-dark.png")),
    ("double", include_bytes!("../icons/tray/double-light.png"), include_bytes!("../icons/tray/double-dark.png")),
    ("luminosite", include_bytes!("../icons/tray/luminosite-light.png"), include_bytes!("../icons/tray/luminosite-dark.png")),
    ("pastille", include_bytes!("../icons/tray/pastille.png"), include_bytes!("../icons/tray/pastille.png")),
];

pub(crate) fn tray_name(app: &AppHandle) -> &'static str {
    let chosen = store(app).tray.clone();
    TRAY_ICONS.iter().find(|i| i.0 == chosen).unwrap_or(&TRAY_ICONS[0]).0
}

pub(crate) fn tray_icon(app: &AppHandle) -> Image<'static> {
    let name = tray_name(app);
    let (_, light, dark) = TRAY_ICONS.iter().find(|i| i.0 == name).unwrap_or(&TRAY_ICONS[0]);
    Image::from_bytes(if store::light_taskbar() { dark } else { light }).expect("icône de notification intégrée")
}

/// Ce que montre le menu de notification. Lu d'abord, sous de courts verrous : la
/// construction du menu, elle, attend le thread principal et ne doit tenir aucun verrou.
#[derive(Serialize)]
pub(crate) struct MenuData {
    mons: Vec<display::Monitor>,
    /// id, nom, raccourci, profil actuel
    profiles: Vec<(String, String, Option<String>, bool)>,
    undo: Option<String>,
    off_key: Option<String>,
    topo: Option<&'static str>,
}

impl MenuData {
    pub(crate) fn read(app: &AppHandle) -> Self {
        let sig = display::signature().unwrap_or_default();
        let (profiles, off_key) = {
            let s = store(app);
            (s.profiles.iter().map(|p| (p.id.clone(), p.name.clone(), p.hotkey.clone(), p.sig == sig)).collect(), s.off_hotkey.clone())
        };
        let undo = lock(&state_of(app).undo).as_ref().map(|u| u.1.clone());
        Self { mons: display::monitors().unwrap_or_default(), profiles, undo, off_key, topo: display::topology() }
    }
}

static MENU: OnceLock<mpsc::Sender<()>> = OnceLock::new();
/// Empreinte du menu et de l'icône affichés.
static SHOWN: Mutex<String> = Mutex::new(String::new());

/// Rafraîchit l'interface tout de suite, l'icône et le menu de notification une fois le
/// calme revenu. Une rafale d'événements ne reconstruit le menu qu'une fois, et seulement
/// si ce qu'il affiche a changé : chaque élément demande un aller-retour au thread principal.
pub(crate) fn changed(app: &AppHandle) {
    let _ = app.emit("changed", ());
    let a = app.clone();
    let tx = MENU.get_or_init(move || debounce(Duration::from_millis(200), move || refresh_tray(&a)));
    let _ = tx.send(());
}

fn refresh_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("tray") else { return };
    let data = MenuData::read(app);
    let print = serde_json::to_string(&(&data, tray_name(app), store::light_taskbar())).unwrap_or_default();
    if *lock(&SHOWN) == print {
        return;
    }
    let _ = tray.set_icon(Some(tray_icon(app)));
    if let Ok(menu) = tray_menu(app, &data) {
        let _ = tray.set_menu(Some(menu));
        *lock(&SHOWN) = print;
    }
}

pub(crate) fn tray_menu(app: &AppHandle, d: &MenuData) -> tauri::Result<Menu<Wry>> {
    let mut menu = MenuBuilder::new(app);
    if d.profiles.is_empty() {
        menu = menu.item(&MenuItemBuilder::with_id("open", t("Aucun profil enregistré", "No saved profiles")).enabled(false).build(app)?);
    }
    for (id, name, hotkey, active) in &d.profiles {
        let item = || CheckMenuItemBuilder::with_id(format!("profile|{id}"), name).checked(*active);
        let built = match hotkey {
            Some(h) => item().accelerator(h).build(app).or_else(|_| item().build(app))?,
            None => item().build(app)?,
        };
        menu = menu.item(&built);
    }
    if let Some(u) = &d.undo {
        let short: String = u.chars().take(48).collect();
        menu = menu.separator().item(&MenuItemBuilder::with_id("undo", tr!("Annuler : {short}", "Undo: {short}")).build(app)?);
    }
    menu = menu.separator();

    let actives = d.mons.iter().filter(|m| m.active).count();
    for m in &d.mons {
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
    // Aucun verrou ici : `build` attend le thread principal, qui peut attendre un verrou
    // (raccourci, fermeture de fenêtre). D'où `MenuData`, lu avant.
    let off = match &d.off_key {
        Some(h) => off().accelerator(h).build(app).or_else(|_| off().build(app))?,
        None => off().build(app)?,
    };
    let topo = d.topo;
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

pub(crate) fn menu_action(app: &AppHandle, id: &str) {
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
            from_menu(app, &mode_label(&m.name, w, h, hz), true, || display::set_mode(key, w, h, hz))
        }),
        ["hz", key, hz] => mon(key).and_then(|m| {
            let hz = hz.parse().unwrap_or(m.hz);
            from_menu(app, &mode_label(&m.name, m.w, m.h, hz), true, || display::set_mode(key, m.w, m.h, hz))
        }),
        ["topo", kind] => from_menu(app, &tr!("Projection : {}", "Projection: {}", topology_label(kind)), true, || display::set_topology(kind)),
        ["bright", key, pct] => {
            let pct: u32 = pct.parse().unwrap_or(50);
            display::set_brightness(key, pct).map(|()| {
                follow(app, key, pct);
                note(app, Level::Ok, Src::Menu, &tr!("{} : luminosité {pct}{NNBSP}%", "{}: brightness {pct}%", display::name(key)));
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
            journal::write(Level::Info, Src::System.label(), t("MonitorMG fermé", "MonitorMG closed"));
            app.exit(0);
            return;
        }
        _ => return,
    };
    outcome(app, r);
}
