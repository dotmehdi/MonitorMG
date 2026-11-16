//! Ligne de commande : les actions des raccourcis, pour un script, le Planificateur de
//! tâches ou une touche de Stream Deck. Une commande est transmise à MonitorMG s'il tourne
//! (journal, réglages liés, luminosité logicielle) ; sinon elle agit seule, puis le
//! processus quitte. `--json` écrit toujours l'état depuis le processus lancé.

use super::*;
use std::io::Write;
use windows::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};

#[derive(Debug, PartialEq)]
pub(crate) enum Cmd {
    /// `--profile <id ou nom>`
    Profile(String),
    /// `--brightness <0 à 100> [--screen <numéro ou nom>]`
    Brightness { pct: u32, screen: Option<String> },
    /// `--input <numéro ou nom d'écran>` : bascule entre ses deux entrées choisies.
    Input(String),
    /// `--off` : éteint les écrans.
    Off,
    /// `--json` : état des écrans et des profils sur la sortie standard.
    Json,
}

/// Valeur qui suit `flag`. `None` : option absente ; `Some(None)` : présente, sans valeur.
fn value<'a>(args: &'a [String], flag: &str) -> Option<Option<&'a String>> {
    args.iter().position(|a| a == flag).map(|i| args.get(i + 1).filter(|v| !v.starts_with("--")))
}

/// Commande demandée par les arguments. `None` : lancement ordinaire.
pub(crate) fn parse(args: &[String]) -> Option<Res<Cmd>> {
    let missing = |flag: &str| tr!("Il manque une valeur après {flag}", "A value is missing after {flag}");
    if let Some(v) = value(args, "--profile") {
        return Some(v.cloned().map(Cmd::Profile).ok_or_else(|| missing("--profile")));
    }
    if let Some(v) = value(args, "--brightness") {
        let pct = v.and_then(|v| v.trim_end_matches('%').parse::<u32>().ok()).filter(|p| *p <= 100);
        let screen = value(args, "--screen").flatten().cloned();
        let bad = || t("--brightness attend une luminosité de 0 à 100", "--brightness expects a brightness from 0 to 100").to_string();
        return Some(pct.map(|pct| Cmd::Brightness { pct, screen }).ok_or_else(bad));
    }
    if let Some(v) = value(args, "--input") {
        return Some(v.cloned().map(Cmd::Input).ok_or_else(|| missing("--input")));
    }
    if args.iter().any(|a| a == "--off") {
        return Some(Ok(Cmd::Off));
    }
    args.iter().any(|a| a == "--json").then_some(Ok(Cmd::Json))
}

/// Écran désigné par son numéro (celui de la fenêtre et de « Identifier ») ou par son nom :
/// celui de Windows ou celui du panneau, sans tenir compte de la casse. `names` : (nom, nom du panneau).
fn pick(names: &[(String, String)], sel: &str) -> Option<usize> {
    let sel = sel.trim();
    if let Ok(n) = sel.parse::<usize>() {
        return (1..=names.len()).contains(&n).then(|| n - 1);
    }
    let s = sel.to_lowercase();
    names.iter().position(|(name, label)| name.to_lowercase() == s || (!label.is_empty() && label.to_lowercase() == s))
}

fn find(mons: &[display::Monitor], screens: &BTreeMap<String, store::Screen>, sel: &str) -> Res<display::Monitor> {
    let names: Vec<(String, String)> = mons.iter().map(|m| (m.name.clone(), screens.get(&m.key).map(|s| s.name.clone()).unwrap_or_default())).collect();
    pick(&names, sel).map(|i| mons[i].clone()).ok_or_else(|| tr!("Écran « {sel} » introuvable", "Display “{sel}” not found"))
}

/// Règle la luminosité de l'écran désigné, ou de tous les écrans réglables, réglages liés
/// compris. Renvoie le texte à consigner.
fn brightness(screens: &BTreeMap<String, store::Screen>, pct: u32, screen: Option<&str>) -> Res<String> {
    let mons = display::monitors()?;
    let targets: Vec<display::Monitor> = match screen {
        Some(sel) => vec![find(&mons, screens, sel)?],
        None => mons.into_iter().filter(|m| m.active && m.brightness.is_some()).collect(),
    };
    if targets.is_empty() {
        return Err(t("Aucun écran ne permet de régler la luminosité", "No display supports brightness control").into());
    }
    for m in &targets {
        display::set_brightness(&m.key, pct)?;
        if let Some(sc) = screens.get(&m.key) {
            follow_with(sc, &m.key, pct);
        }
    }
    Ok(match (screen, targets.as_slice()) {
        (Some(_), [m]) => tr!("{} : luminosité {pct}{NNBSP}%", "{}: brightness {pct}%", m.name),
        _ => tr!("Luminosité {pct}{NNBSP}%", "Brightness {pct}%"),
    })
}

/// Bascule l'entrée de l'écran désigné entre les deux choisies dans la vue DDC/CI.
fn input(screens: &BTreeMap<String, store::Screen>, sel: &str) -> Res<(Level, String)> {
    let m = find(&display::monitors()?, screens, sel)?;
    let pair = screens
        .get(&m.key)
        .and_then(|s| s.inputs)
        .ok_or_else(|| tr!("{} : choisissez d'abord ses deux entrées dans la vue DDC/CI", "{}: pick its two inputs in the DDC/CI view first", m.name))?;
    Ok(switch_input(&m.key, pair))
}

/// Consigne le résultat d'une commande dans l'activité ; une erreur le reste pour `outcome`.
fn logged(app: &AppHandle, r: Res<(Level, String)>) -> Res<String> {
    match r {
        Ok((Level::Error, text)) | Err(text) => {
            note(app, Level::Error, Src::Command, &text);
            Err(text)
        }
        Ok((level, text)) => {
            note(app, level, Src::Command, &text);
            changed(app);
            Ok(String::new())
        }
    }
}

/// Commande reçue par MonitorMG en marche (second lancement).
pub(crate) fn run(app: &AppHandle, cmd: Cmd) {
    let screens = store(app).screens.clone();
    let r = match cmd {
        Cmd::Profile(id) => {
            let found = store(app).profiles.iter().find(|p| p.id == id || p.name == id).map(|p| p.id.clone());
            match found {
                Some(pid) => apply_profile(app, Src::Desktop, &pid),
                None => {
                    let e = tr!("Profil « {id} » introuvable", "Profile “{id}” not found");
                    note(app, Level::Error, Src::Desktop, &e);
                    Err(e)
                }
            }
        }
        Cmd::Brightness { pct, screen } => logged(app, brightness(&screens, pct, screen.as_deref()).map(|text| (Level::Ok, text))),
        Cmd::Input(sel) => logged(app, input(&screens, &sel)),
        Cmd::Off => {
            screens_off(app, Src::Command);
            Ok(String::new())
        }
        Cmd::Json => Ok(String::new()),
    };
    outcome(app, r);
}

/// Commande lancée quand MonitorMG ne tourne pas : elle agit ici, puis le processus quitte.
/// La luminosité logicielle n'existe pas dans ce cas : son voile est une fenêtre de l'application.
pub(crate) fn alone(cmd: Cmd) {
    if let Cmd::Profile(id) = &cmd {
        return apply_from_desktop(id);
    }
    let s = store::Store::load();
    display::tune(s.screens.iter().map(|(k, sc)| (k.clone(), sc.tune())).collect());
    let src = Src::Command.label();
    let r = match cmd {
        Cmd::Brightness { pct, screen } => {
            display::read_brightness();
            brightness(&s.screens, pct, screen.as_deref()).map(|text| (Level::Ok, text))
        }
        Cmd::Input(sel) => input(&s.screens, &sel),
        Cmd::Off => {
            journal::write(Level::Ok, src, t("Écrans éteints", "Displays turned off"));
            return power_down(s.panel.off, s.panel.power, &s.screens, HWND_BROADCAST);
        }
        Cmd::Profile(_) | Cmd::Json => return,
    };
    match r {
        Ok((Level::Error, text)) | Err(text) => {
            journal::write(Level::Error, src, &text);
            store::alert(&text);
        }
        Ok((level, text)) => {
            journal::write(level, src, &text);
        }
    }
}

/// Lancé d'un terminal sans redirection, un programme graphique n'a pas de sortie
/// standard : on se rattache alors à la console du parent.
fn console() {
    let has = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }.is_ok_and(|h| !h.is_invalid() && !h.0.is_null());
    if !has {
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}

/// Arguments refusés : dit pourquoi, dans le terminal et dans une boîte de dialogue.
pub(crate) fn refuse(e: &str) {
    console();
    let _ = writeln!(std::io::stderr(), "MonitorMG : {e}");
    store::alert(e);
}

/// `--json` : l'état des écrans et des profils, pour un script.
pub(crate) fn print_state() {
    console();
    let s = store::Store::load();
    display::tune(s.screens.iter().map(|(k, sc)| (k.clone(), sc.tune())).collect());
    display::read_brightness();
    let state = state_json(&s, &display::monitors().unwrap_or_default(), &display::signature().unwrap_or_default());
    let _ = writeln!(std::io::stdout(), "{}", serde_json::to_string_pretty(&state).unwrap_or_default());
}

fn state_json(s: &store::Store, mons: &[display::Monitor], sig: &str) -> serde_json::Value {
    let monitors: Vec<serde_json::Value> = mons
        .iter()
        .enumerate()
        .map(|(i, m)| {
            serde_json::json!({
                "number": i + 1,
                "name": m.name,
                "label": s.screens.get(&m.key).map(|x| x.name.as_str()).filter(|n| !n.is_empty()),
                "connector": m.connector,
                "internal": m.internal,
                "active": m.active,
                "primary": m.primary,
                "x": m.x,
                "y": m.y,
                "width": m.w,
                "height": m.h,
                "refresh": m.hz,
                "rotation": m.rotation,
                "scale": m.scale.as_ref().map(|x| x.current),
                "hdr": m.hdr,
                "brightness": m.brightness,
            })
        })
        .collect();
    let profiles: Vec<serde_json::Value> =
        s.profiles.iter().map(|p| serde_json::json!({ "id": p.id, "name": p.name, "active": p.sig == sig, "hotkey": p.hotkey })).collect();
    serde_json::json!({ "version": VERSION, "monitors": monitors, "profiles": profiles })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        std::iter::once("MonitorMG.exe").chain(line.split_whitespace()).map(String::from).collect()
    }

    #[test]
    fn arguments_become_commands() {
        assert_eq!(parse(&args("")), None);
        assert_eq!(parse(&args("--minimized")), None);
        assert_eq!(parse(&args("--profile Bureau")), Some(Ok(Cmd::Profile("Bureau".into()))));
        assert_eq!(parse(&args("--brightness 40")), Some(Ok(Cmd::Brightness { pct: 40, screen: None })));
        assert_eq!(parse(&args("--brightness 40% --screen 2")), Some(Ok(Cmd::Brightness { pct: 40, screen: Some("2".into()) })));
        assert_eq!(parse(&args("--screen 2 --brightness 0")), Some(Ok(Cmd::Brightness { pct: 0, screen: Some("2".into()) })));
        assert_eq!(parse(&args("--input 1")), Some(Ok(Cmd::Input("1".into()))));
        assert_eq!(parse(&args("--off")), Some(Ok(Cmd::Off)));
        assert_eq!(parse(&args("--json")), Some(Ok(Cmd::Json)));
        // Valeur absente ou hors bornes : refusée, jamais devinée.
        for bad in ["--brightness", "--brightness 140", "--brightness fort", "--profile", "--profile --off", "--input"] {
            assert!(matches!(parse(&args(bad)), Some(Err(_))), "{bad}");
        }
    }

    #[test]
    fn screens_are_picked_by_number_or_name() {
        let names = [("DELL U2723QE".to_string(), String::new()), ("LG 27UL850".to_string(), "Vertical".to_string())];
        assert_eq!(pick(&names, "1"), Some(0));
        assert_eq!(pick(&names, " 2 "), Some(1));
        assert_eq!(pick(&names, "3"), None);
        assert_eq!(pick(&names, "0"), None);
        assert_eq!(pick(&names, "dell u2723qe"), Some(0));
        assert_eq!(pick(&names, "vertical"), Some(1));
        assert_eq!(pick(&names, ""), None);
    }

    #[test]
    fn state_is_described_for_scripts() {
        let s: store::Store = serde_json::from_str(r#"{"profiles":[{"id":"1","name":"Bureau","sig":"abc","tiles":[],"snap":{"paths":[],"modes":[],"adapters":{}}}]}"#).unwrap();
        let v = state_json(&s, &[], "abc");
        assert_eq!(v["version"], VERSION);
        assert_eq!(v["profiles"][0], serde_json::json!({ "id": "1", "name": "Bureau", "active": true, "hotkey": null }));
        assert_eq!(v["monitors"], serde_json::json!([]));
    }
}
