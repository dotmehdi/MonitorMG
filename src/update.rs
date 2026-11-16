//! Vérification des mises à jour, sur option : désactivée par défaut, MonitorMG ne se
//! connecte alors jamais à Internet. Activée, elle demande une fois par jour à GitHub le
//! nom de la dernière version publiée ; rien n'est téléchargé ni installé.

use super::*;
use std::ffi::c_void;
use windows::Win32::Networking::WinHttp::*;
use windows::Win32::UI::Shell::ShellExecuteW;

/// Page de la dernière version, ouverte dans le navigateur à la demande.
const RELEASES: PCWSTR = w!("https://github.com/dotmehdi/MonitorMG/releases/latest");
const DAY: Duration = Duration::from_secs(24 * 3600);
/// Une réponse plus longue n'est pas celle qu'on attend.
const MAX_BODY: usize = 512 * 1024;

/// Version publiée plus récente que celle-ci, d'après la dernière vérification.
static NEWER: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn newer() -> Option<String> {
    lock(&NEWER).clone()
}

/// (majeure, mineure, corrective) de « v2.3.0 » ou « 2.3 ». Une préversion (« 2.3.0-beta »)
/// n'est pas une version à annoncer.
fn version(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.trim().trim_start_matches(['v', 'V']).split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    parts.next().is_none().then_some((major, minor, patch))
}

fn is_newer(latest: &str, current: &str) -> bool {
    matches!((version(latest), version(current)), (Some(l), Some(c)) if l > c)
}

/// Nom de version (« 2.3.0 ») dans la réponse de l'API des releases de GitHub.
fn latest_tag(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(v.get("tag_name")?.as_str()?.trim_start_matches(['v', 'V']).to_string())
}

/// Poignée WinHTTP, fermée quoi qu'il arrive.
struct Handle(*mut c_void);

impl Handle {
    fn open(h: *mut c_void) -> Res<Self> {
        if h.is_null() {
            Err(windows::core::Error::from_thread().message())
        } else {
            Ok(Self(h))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// GET https://api.github.com/repos/dotmehdi/MonitorMG/releases/latest, par WinHTTP : le
/// proxy et les certificats sont ceux de Windows, sans dépendance de plus.
fn fetch() -> Res<String> {
    let err = |e: windows::core::Error| e.message();
    unsafe {
        let session = Handle::open(WinHttpOpen(w!("MonitorMG"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0))?;
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 10_000);
        let connection = Handle::open(WinHttpConnect(session.0, w!("api.github.com"), 443, 0))?;
        let request = Handle::open(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            w!("/repos/dotmehdi/MonitorMG/releases/latest"),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(err)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(err)?;
        let mut body = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr() as *mut c_void, chunk.len() as u32, &mut read).map_err(err)?;
            if read == 0 || body.len() > MAX_BODY {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// Interroge GitHub. Renvoie la version disponible si elle est plus récente que celle-ci,
/// et la garde pour l'interface.
pub(crate) fn check() -> Res<Option<String>> {
    let failed = |e: String| tr!("Vérification des mises à jour impossible. {e}", "Can't check for updates. {e}");
    let tag = latest_tag(&fetch().map_err(failed)?).ok_or_else(|| failed(t("Réponse inattendue de GitHub", "Unexpected answer from GitHub").into()))?;
    let found = is_newer(&tag, VERSION).then_some(tag);
    *lock(&NEWER) = found.clone();
    Ok(found)
}

/// Vérifie et, si une version vient d'apparaître, l'annonce dans l'activité. Les échecs
/// (pas de réseau…) restent dans le journal de diagnostic : rien ne dérange l'utilisateur.
pub(crate) fn check_quietly(app: &AppHandle) {
    let before = newer();
    match check() {
        Ok(Some(v)) if before.as_ref() != Some(&v) => note(app, Level::Ok, Src::System, &tr!("MonitorMG {v} est disponible", "MonitorMG {v} is available")),
        Ok(_) => {}
        Err(e) => note(app, Level::Info, Src::System, &e),
    }
    changed(app);
}

/// Une vérification par jour, tant que l'option est activée.
pub(crate) fn watch(app: AppHandle) {
    thread::spawn(move || {
        // Le démarrage d'abord : la vérification n'a rien d'urgent.
        thread::sleep(Duration::from_secs(60));
        loop {
            if store(&app).updates {
                check_quietly(&app);
            }
            thread::sleep(DAY);
        }
    });
}

/// Ouvre la page de la dernière version dans le navigateur.
pub(crate) fn open_releases() -> Res<()> {
    let r = unsafe { ShellExecuteW(None, w!("open"), RELEASES, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    // ShellExecute renvoie une valeur supérieure à 32 quand il réussit.
    if r.0 as usize > 32 {
        Ok(())
    } else {
        Err(t("Impossible d'ouvrir le navigateur", "Can't open the browser").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_as_numbers() {
        assert!(is_newer("v2.3.0", "2.2.1"));
        assert!(is_newer("2.10.0", "2.9.9"));
        assert!(is_newer("3", "2.9.9"));
        assert!(!is_newer("2.2.1", "2.2.1"));
        assert!(!is_newer("2.2.0", "2.2.1"));
        // Préversion ou nom inattendu : rien à annoncer.
        assert!(!is_newer("2.3.0-beta", "2.2.1"));
        assert!(!is_newer("nightly", "2.2.1"));
        assert!(!is_newer("2.3.0.1", "2.2.1"));
    }

    #[test]
    fn the_latest_tag_is_read_from_the_github_answer() {
        assert_eq!(latest_tag(r#"{"tag_name":"v2.3.0","name":"MonitorMG 2.3.0"}"#).as_deref(), Some("2.3.0"));
        assert_eq!(latest_tag(r#"{"message":"Not Found"}"#), None);
        assert_eq!(latest_tag("<html>"), None);
    }
}
