//! Écrans dont Windows a changé l'identifiant de connecteur : pilote graphique réinstallé
//! ou mis à jour, écran rebranché sur un autre port. Profils et préférences DDC/CI
//! désignent chaque écran par sa clé (chemin de l'adaptateur + identifiant du connecteur) :
//! sans ce rattachement, un profil échoue (« Aucun écran de ce profil n'est branché ») et
//! les réglages DDC/CI d'un écran ne s'appliquent plus.
//!
//! Chaque clé mémorisée qui ne désigne plus le bon écran est rattachée à l'écran branché
//! du même modèle, sur le même adaptateur. Le modèle vient de l'EDID (profils et
//! préférences enregistrés à partir de cette version) ; pour les profils plus anciens,
//! du nom de la tuile placée à la position de l'écran.

use crate::display::{self, Seen};
use crate::store::{Profile, Screen, Store};
use std::collections::{BTreeMap, BTreeSet};

/// Ce qu'on sait de l'écran qu'une clé désignait.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ident {
    pub model: Option<String>,
    pub name: Option<String>,
}

impl Ident {
    fn known(&self) -> bool {
        self.model.is_some() || self.name.is_some()
    }

    /// L'écran branché est-il celui-là ? Le modèle prime ; à défaut, le nom de Windows.
    fn is(&self, s: &Seen) -> bool {
        match &self.model {
            Some(m) if !s.model.is_empty() => m.eq_ignore_ascii_case(&s.model),
            _ => self.name.as_deref().is_some_and(|n| n == s.name),
        }
    }
}

/// Chemin de l'adaptateur d'une clé.
fn base(key: &str) -> &str {
    key.rsplit_once('#').map_or(key, |(b, _)| b)
}

/// Identifiant du connecteur d'une clé.
fn number(key: &str) -> u32 {
    key.rsplit_once('#').and_then(|(_, n)| n.parse().ok()).unwrap_or(u32::MAX)
}

/// « HP 324ph (2) » → « HP 324ph » : le numéro des écrans jumeaux suit l'ordre des
/// connecteurs, pas l'écran.
fn bare(name: &str) -> String {
    match name.strip_suffix(')').and_then(|n| n.rsplit_once(" (")) {
        Some((n, d)) if !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()) => n.to_string(),
        _ => name.to_string(),
    }
}

/// Écrans d'un profil et ce qu'on sait de chacun : son modèle (profils récents), ou le
/// nom de sa tuile, retrouvée par sa position (profils plus anciens).
fn idents(p: &Profile) -> Vec<(String, Ident)> {
    let placed = display::placed(&p.snap);
    let mut out: Vec<(String, Ident)> = placed
        .iter()
        .map(|(k, pos)| {
            // Deux écrans à la même position (dupliqués) : la tuile ne dit pas lequel.
            let alone = pos.is_some() && placed.iter().filter(|(_, q)| q == pos).count() == 1;
            let name = pos
                .filter(|_| alone)
                .and_then(|(x, y)| p.tiles.iter().find(|t| t.x == x && t.y == y))
                .map(|t| bare(&t.name));
            (k.clone(), Ident { model: p.snap.models.get(k).cloned(), name })
        })
        .collect();
    // Écrans branchés mais éteints dans le profil : connus par leur modèle seulement.
    for (k, m) in &p.snap.models {
        if !out.iter().any(|(x, _)| x == k) {
            out.push((k.clone(), Ident { model: Some(m.clone()), name: None }));
        }
    }
    out
}

/// Nouvelle clé de chaque écran mémorisé qui n'est plus là où on l'attend.
/// `known` : les clés d'un même ensemble (un profil, ou les préférences des écrans) et
/// ce qu'on sait de l'écran de chacune.
pub fn matches(known: &[(String, Ident)], seen: &[Seen]) -> BTreeMap<String, String> {
    let here = |k: &str| seen.iter().find(|s| s.key == k);
    // En place : branché, et c'est bien lui (ou rien ne permet d'en douter).
    let fine = |(k, id): &(String, Ident)| here(k).is_some_and(|s| !id.known() || id.is(s));
    let taken: BTreeSet<&str> = known.iter().filter(|x| fine(x)).map(|(k, _)| k.as_str()).collect();
    // Candidats de chaque écran perdu : même adaptateur, même écran, clé libre.
    let mut groups: BTreeMap<Vec<&str>, Vec<&str>> = BTreeMap::new();
    for (k, id) in known.iter().filter(|x| !fine(x) && x.1.known()) {
        let mut cands: Vec<&str> = seen
            .iter()
            .filter(|s| base(&s.key) == base(k) && id.is(s) && !taken.contains(s.key.as_str()))
            .map(|s| s.key.as_str())
            .collect();
        cands.sort_by_key(|c| (number(c), c.to_string()));
        if !cands.is_empty() {
            groups.entry(cands).or_default().push(k);
        }
    }
    let mut out = BTreeMap::new();
    let mut used = BTreeSet::new();
    for (cands, mut olds) in groups {
        // Écrans jumeaux : autant d'anciennes clés que de candidats, appariés dans
        // l'ordre des connecteurs. Sinon, impossible de savoir lequel est lequel.
        if cands.len() != olds.len() || cands.iter().any(|c| used.contains(c)) {
            continue;
        }
        olds.sort_by_key(|k| (number(k), k.to_string()));
        for (o, c) in olds.into_iter().zip(cands) {
            used.insert(c);
            out.insert(o.to_string(), c.to_string());
        }
    }
    out
}

/// Signature d'un profil (`clé@x,y,lxh,hz;…`) avec les nouvelles clés, triée comme
/// celle que calcule `display::signature`.
fn resigned(sig: &str, map: &BTreeMap<String, String>) -> String {
    if sig.is_empty() {
        return String::new();
    }
    let mut v: Vec<String> = sig
        .split(';')
        .map(|part| match part.split_once('@') {
            Some((k, rest)) => format!("{}@{rest}", map.get(k).map_or(k, String::as_str)),
            None => part.to_string(),
        })
        .collect();
    v.sort();
    v.join(";")
}

/// Préférences sans réglage de l'utilisateur : seulement des capacités lues sur l'écran.
fn untouched(sc: &Screen) -> bool {
    Screen { caps: Default::default(), model: String::new(), ..sc.clone() } == Screen::default()
}

/// Ce qui a été rattaché : anciennes et nouvelles clés, et si les préférences des
/// écrans ont changé (raccourcis de bascule d'entrée à réenregistrer).
#[derive(Debug, Default, PartialEq)]
pub struct Moved {
    pub keys: BTreeMap<String, String>,
    pub screens: bool,
    /// Modèle noté pour la première fois sur des préférences : à enregistrer.
    pub claimed: bool,
}

impl Moved {
    pub fn any(&self) -> bool {
        !self.keys.is_empty() || self.claimed
    }
}

/// Rattache profils et préférences aux nouvelles clés de leurs écrans.
pub fn run(s: &mut Store, seen: &[Seen]) -> Moved {
    let mut moved = Moved::default();
    // Dernier écran connu de chaque clé, d'après le profil le plus récent qui la contient
    // (l'identifiant d'un profil est sa date d'enregistrement, en hexadécimal).
    let mut latest: BTreeMap<String, (u64, Ident)> = BTreeMap::new();
    let mut maps = Vec::with_capacity(s.profiles.len());
    for p in &s.profiles {
        let ids = idents(p);
        let when = u64::from_str_radix(&p.id, 16).unwrap_or(0);
        for (k, id) in ids.iter().filter(|(_, id)| id.known()) {
            if latest.get(k).is_none_or(|(w, _)| *w <= when) {
                latest.insert(k.clone(), (when, id.clone()));
            }
        }
        maps.push(matches(&ids, seen));
    }
    // Même ancienne clé, même nouvelle clé dans tous les profils qui la rattachent.
    let mut union: BTreeMap<String, Option<String>> = BTreeMap::new();
    for m in &maps {
        for (o, n) in m {
            match union.get(o) {
                Some(Some(x)) if x != n => {
                    union.insert(o.clone(), None);
                }
                Some(_) => {}
                None => {
                    union.insert(o.clone(), Some(n.clone()));
                }
            }
        }
    }
    for (p, map) in s.profiles.iter_mut().zip(&maps) {
        if map.is_empty() {
            continue;
        }
        display::rekey(&mut p.snap, map);
        p.sig = resigned(&p.sig, map);
        // Écrans branchés mais éteints d'un ancien profil, sans modèle connu : ils suivent
        // le rattachement fait dans les autres profils, Windows ayant tout renuméroté.
        p.screens = p
            .screens
            .iter()
            .map(|k| match (map.get(k), union.get(k)) {
                (Some(n), _) => n.clone(),
                (None, Some(Some(n))) if !p.screens.contains(n) => n.clone(),
                _ => k.clone(),
            })
            .collect();
        for (o, n) in map {
            moved.keys.insert(o.clone(), n.clone());
        }
    }

    // Préférences des écrans : le modèle noté sur elles, sinon le profil le plus récent.
    let ident = |k: &String, sc: &Screen| match sc.model.is_empty() {
        false => Ident { model: Some(sc.model.clone()), name: None },
        true => latest.get(k).map(|(_, id)| id.clone()).unwrap_or_default(),
    };
    // Préférences sans réglage (capacités lues, rien de plus) : remplaçables, elles ne
    // retiennent pas leur clé.
    let known: Vec<(String, Ident)> = s.screens.iter().filter(|(_, sc)| !untouched(sc)).map(|(k, sc)| (k.clone(), ident(k, sc))).collect();
    let found = matches(&known, seen);
    // Les réglages déjà faits sous la nouvelle clé restent, sauf s'ils partent eux-mêmes ailleurs.
    let blocked = |n: &String| s.screens.get(n).is_some_and(|sc| !untouched(sc)) && !found.contains_key(n);
    let map: BTreeMap<String, String> = found.iter().filter(|(_, n)| !blocked(n)).map(|(o, n)| (o.clone(), n.clone())).collect();
    if !map.is_empty() {
        let mut next = BTreeMap::new();
        for (k, sc) in std::mem::take(&mut s.screens) {
            match map.get(&k) {
                Some(n) => {
                    next.insert(n.clone(), sc);
                }
                // Préférences vides laissées par l'écran sous sa nouvelle clé : remplacées.
                None if map.values().any(|n| *n == k) => {
                    next.entry(k).or_insert(sc);
                }
                None => {
                    next.insert(k, sc);
                }
            }
        }
        s.screens = next;
        moved.screens = true;
        for (o, n) in map {
            moved.keys.insert(o, n);
        }
    }
    // Modèle noté sur les préférences de chaque écran branché, s'il est bien celui-là :
    // la prochaine fois, elles le suivront sans dépendre des profils.
    for x in seen.iter().filter(|x| !x.model.is_empty()) {
        let Some(sc) = s.screens.get_mut(&x.key) else { continue };
        if sc.model.is_empty() && latest.get(&x.key).is_none_or(|(_, id)| !id.known() || id.is(x) || moved.keys.values().any(|n| *n == x.key)) {
            sc.model = x.model.clone();
            moved.claimed = true;
        }
    }
    moved
}

/// Noms des écrans rattachés, pour le journal.
pub fn describe(moved: &Moved, seen: &[Seen]) -> String {
    let mut names: Vec<&str> = moved.keys.values().filter_map(|n| seen.iter().find(|s| s.key == *n)).map(|s| s.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = r"\\?\PCI#VEN_1002&DEV_1900#{5b45201d-f2f2-4f3b-85bb-30ff1f953599}";

    fn key(n: u32) -> String {
        format!("{A}#{n}")
    }

    fn seen(n: u32, model: &str, name: &str) -> Seen {
        Seen { key: key(n), model: model.into(), name: name.into() }
    }

    fn named(n: &str) -> Ident {
        Ident { model: None, name: Some(n.into()) }
    }

    fn modeled(m: &str) -> Ident {
        Ident { model: Some(m.into()), name: None }
    }

    fn map(pairs: &[(u32, u32)]) -> BTreeMap<String, String> {
        pairs.iter().map(|&(a, b)| (key(a), key(b))).collect()
    }

    #[test]
    fn renumbered_connectors_are_followed() {
        // Cas réel : BenQ 271 → 273, AORUS 272 → 274 après un changement de pilote.
        let now = [seen(273, "BNQ7950", "BenQ EW3270U"), seen(274, "GBT3214", "AORUS FO32U2")];
        assert_eq!(matches(&[(key(272), named("AORUS FO32U2"))], &now), map(&[(272, 274)]));
        let both = [(key(271), named("BenQ EW3270U")), (key(272), named("AORUS FO32U2"))];
        assert_eq!(matches(&both, &now), map(&[(271, 273), (272, 274)]));
        assert_eq!(matches(&[(key(272), modeled("gbt3214"))], &now), map(&[(272, 274)]));
    }

    #[test]
    fn screens_in_place_or_absent_are_left_alone() {
        let now = [seen(273, "BNQ7950", "BenQ EW3270U"), seen(274, "GBT3214", "AORUS FO32U2")];
        // Déjà à sa place.
        assert!(matches(&[(key(274), named("AORUS FO32U2"))], &now).is_empty());
        // Écran absent : rien à quoi le rattacher.
        assert!(matches(&[(key(271), named("HP 324ph"))], &now).is_empty());
        // Rien de connu sur l'écran : on ne devine pas.
        assert!(matches(&[(key(271), Ident::default())], &now).is_empty());
        // Autre adaptateur : pas le même chemin.
        let other = Seen { key: r"\\?\PCI#VEN_10DE#{x}#300".into(), model: "GBT3214".into(), name: "AORUS FO32U2".into() };
        assert!(matches(&[(key(272), modeled("GBT3214"))], &[other]).is_empty());
    }

    #[test]
    fn the_model_wins_over_the_name() {
        let now = [seen(274, "GBT3214", "AORUS FO32U2")];
        assert!(matches(&[(key(272), Ident { model: Some("BNQ7950".into()), name: Some("AORUS FO32U2".into()) })], &now).is_empty());
        // Modèle illisible sur l'écran branché : le nom décide.
        let blind = [seen(274, "", "AORUS FO32U2")];
        assert_eq!(matches(&[(key(272), Ident { model: Some("GBT3214".into()), name: Some("AORUS FO32U2".into()) })], &blind), map(&[(272, 274)]));
    }

    #[test]
    fn swapped_ports_are_swapped_back() {
        let now = [seen(271, "GBT3214", "AORUS FO32U2"), seen(272, "BNQ7950", "BenQ EW3270U")];
        let profile = [(key(271), modeled("BNQ7950")), (key(272), modeled("GBT3214"))];
        assert_eq!(matches(&profile, &now), map(&[(271, 272), (272, 271)]));
    }

    #[test]
    fn twins_are_paired_in_connector_order_or_not_at_all() {
        let twins = [(key(271), named("HP 324ph")), (key(272), named("HP 324ph"))];
        let now = [seen(274, "HPN3B03", "HP 324ph"), seen(273, "HPN3B03", "HP 324ph")];
        assert_eq!(matches(&twins, &now), map(&[(271, 273), (272, 274)]));
        // Un seul des deux branché : lequel est-ce ? On ne touche à rien.
        assert!(matches(&twins, &now[..1]).is_empty());
        // L'un est resté en place : l'autre prend la clé libre.
        let now = [seen(271, "HPN3B03", "HP 324ph"), seen(274, "HPN3B03", "HP 324ph")];
        assert_eq!(matches(&twins, &now), map(&[(272, 274)]));
    }

    #[test]
    fn twin_numbers_are_dropped_from_names() {
        assert_eq!(bare("HP 324ph (2)"), "HP 324ph");
        assert_eq!(bare("HP 324ph"), "HP 324ph");
        assert_eq!(bare("Écran (gauche)"), "Écran (gauche)");
        assert_eq!(bare("()"), "()");
    }

    #[test]
    fn signatures_follow_and_stay_sorted() {
        let sig = format!("{}@-1920,0,1920x1080,60;{}@0,0,3840x2160,200", key(271), key(272));
        let want = format!("{}@-1920,0,1920x1080,60;{}@0,0,3840x2160,200", key(273), key(274));
        assert_eq!(resigned(&sig, &map(&[(271, 273), (272, 274)])), want);
        let swapped = format!("{}@-1920,0,1920x1080,60;{}@0,0,3840x2160,200", key(272), key(271));
        let mut v: Vec<&str> = swapped.split(';').collect();
        v.sort();
        assert_eq!(resigned(&sig, &map(&[(271, 272), (272, 271)])), v.join(";"));
        assert_eq!(resigned("", &map(&[(271, 273)])), "");
    }

    fn tuned() -> Screen {
        Screen { range: display::Range(10, 60), vcp: Some(0x10), ..Default::default() }
    }

    #[test]
    fn screen_preferences_follow_their_screen() {
        let now = [seen(273, "BNQ7950", "BenQ EW3270U"), seen(274, "GBT3214", "AORUS FO32U2")];
        let mut s = Store::default();
        s.screens.insert(key(272), Screen { model: "GBT3214".into(), ..tuned() });
        // Préférences vides créées entre-temps sous la nouvelle clé (capacités lues) : remplacées.
        let mut caps = BTreeMap::new();
        caps.insert(0x10u8, Vec::new());
        s.screens.insert(key(274), Screen { caps, ..Default::default() });
        // Écran inconnu (jamais vu depuis cette version, dans aucun profil) : laissé.
        s.screens.insert(key(261), tuned());
        let moved = run(&mut s, &now);
        assert_eq!(moved.keys, map(&[(272, 274)]));
        assert!(moved.screens);
        assert_eq!(s.screens.keys().cloned().collect::<Vec<_>>(), [key(261), key(274)]);
        assert_eq!(s.screens[&key(274)].range, display::Range(10, 60));
        // Réglages déjà faits sous la nouvelle clé : on ne les écrase pas.
        let mut s = Store::default();
        s.screens.insert(key(272), Screen { model: "GBT3214".into(), ..tuned() });
        s.screens.insert(key(274), Screen { name: "Jeu".into(), ..Default::default() });
        assert!(run(&mut s, &now).keys.is_empty());
        assert_eq!(s.screens[&key(274)].name, "Jeu");
        assert_eq!(s.screens[&key(274)].model, "GBT3214");
    }

    #[test]
    fn connected_preferences_get_their_model() {
        let now = [seen(274, "GBT3214", "AORUS FO32U2")];
        let mut s = Store::default();
        s.screens.insert(key(274), tuned());
        let moved = run(&mut s, &now);
        assert!(moved.claimed && moved.keys.is_empty());
        assert_eq!(s.screens[&key(274)].model, "GBT3214");
        // Déjà noté : rien à enregistrer.
        assert!(!run(&mut s, &now).any());
    }
}
