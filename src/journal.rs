//! Journal d'activité : une ligne JSON par événement, dans le dossier de données.
//! Il nourrit la vue Activité et sert au diagnostic ; le processus lancé par un
//! raccourci bureau (`--profile`) y écrit aussi. Le fichier tourne au-delà de 512 Ko.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::SystemTime;

const MAX: u64 = 512 * 1024;
const TAIL: u64 = 128 * 1024;

/// Gravité d'une entrée. `Info` relève du diagnostic : masquée dans la vue Activité.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Error,
    Info,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    /// Millisecondes depuis 1970.
    pub at: u64,
    pub level: Level,
    /// Fenêtre, Panneau, Menu, Raccourci, Branchement, Bureau, Système, Planification,
    /// Alimentation ou Programme.
    pub source: String,
    pub text: String,
    /// Modification annulable au moment où elle a été faite.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub undo: bool,
}

pub fn dir() -> PathBuf {
    crate::store::data_dir().join("logs")
}

fn file() -> PathBuf {
    dir().join("activite.jsonl")
}

pub fn write(level: Level, source: &str, text: &str) -> Entry {
    write_with(level, source, text, false)
}

pub fn write_with(level: Level, source: &str, text: &str, undo: bool) -> Entry {
    let at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let e = Entry { at, level, source: source.into(), text: text.into(), undo };
    let f = file();
    let _ = fs::create_dir_all(dir());
    if fs::metadata(&f).is_ok_and(|m| m.len() > MAX) {
        let _ = fs::rename(&f, f.with_extension("1.jsonl"));
    }
    if let (Ok(mut out), Ok(line)) = (OpenOptions::new().create(true).append(true).open(&f), serde_json::to_string(&e)) {
        // Une seule écriture par ligne : deux threads (ou le processus d'un raccourci
        // bureau) ne peuvent pas entremêler leurs lignes.
        let _ = out.write_all(format!("{line}
").as_bytes());
    }
    e
}

/// Dernières entrées, de la plus récente à la plus ancienne.
pub fn recent(n: usize) -> Vec<Entry> {
    let Ok(mut f) = fs::File::open(file()) else { return Vec::new() };
    let len = f.metadata().map_or(0, |m| m.len());
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(TAIL)));
    let mut bytes = Vec::new();
    let _ = f.read_to_end(&mut bytes);
    // La première ligne peut être coupée : elle ne se décode pas et disparaît.
    String::from_utf8_lossy(&bytes).lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(n).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn entries_roundtrip_as_json_lines() {
        let e = super::Entry { at: 1, level: super::Level::Ok, source: "Menu".into(), text: "Profil « Bureau » appliqué".into(), undo: true };
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains('\n'));
        let back: super::Entry = serde_json::from_str(&line).unwrap();
        assert_eq!((back.at, back.text), (1, e.text));
    }
}
