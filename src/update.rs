//! Mises à jour. La vérification est une option : désactivée par défaut, MonitorMG ne se
//! connecte alors jamais à Internet. Activée, elle demande une fois par jour à GitHub le
//! nom de la dernière version publiée. Selon les réglages (`store::AutoUpdate`), la version
//! trouvée est ensuite téléchargée, vérifiée par son empreinte SHA-256, puis installée :
//! MonitorMG lance l'installateur et se ferme, puis revient une fois l'installation finie.

use super::*;
use std::ffi::c_void;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use windows::Win32::Networking::WinHttp::*;
use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::UI::Shell::ShellExecuteW;

/// Serveurs des versions : GitHub, ou un serveur local pour une version d'essai
/// (MONITORMG_UPDATES à la compilation, voir docs/DEVELOPPEMENT.md).
const API: &str = match option_env!("MONITORMG_UPDATES") {
    Some(u) => u,
    None => "https://api.github.com",
};
const SITE: &str = match option_env!("MONITORMG_UPDATES") {
    Some(u) => u,
    None => "https://github.com",
};
const REPO: &str = "dotmehdi/MonitorMG";
/// Page de la dernière version, ouverte dans le navigateur à la demande.
const RELEASES: PCWSTR = w!("https://github.com/dotmehdi/MonitorMG/releases/latest");
const DAY: Duration = Duration::from_secs(24 * 3600);
/// Une réponse plus longue n'est pas celle qu'on attend.
const MAX_BODY: usize = 512 * 1024;
/// L'installateur pèse 4 Mo ; au-delà de cette taille, ce n'est pas lui.
const MAX_SETUP: usize = 64 * 1024 * 1024;

/// Version publiée plus récente que celle-ci, d'après la dernière vérification.
static NEWER: Mutex<Option<String>> = Mutex::new(None);

/// Où en est la version trouvée.
#[derive(Clone, PartialEq)]
enum Step {
    Idle,
    Downloading,
    /// Téléchargée et vérifiée : version, fichier et empreinte attendue.
    Ready(String, PathBuf, String),
    Installing,
}

static STEP: Mutex<Step> = Mutex::new(Step::Idle);

pub(crate) fn newer() -> Option<String> {
    lock(&NEWER).clone()
}

/// Étape affichée par l'interface : « download », « ready » ou « install ».
pub(crate) fn step() -> Option<&'static str> {
    match *lock(&STEP) {
        Step::Idle => None,
        Step::Downloading => Some("download"),
        Step::Ready(..) => Some("ready"),
        Step::Installing => Some("install"),
    }
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

/// Empreinte annoncée pour `file` dans un SHA256SUMS.txt (« <empreinte>  <fichier> »).
fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, name) = l.trim().split_once(char::is_whitespace)?;
        let ok = name.trim().trim_start_matches('*') == file && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        ok.then(|| hash.to_ascii_lowercase())
    })
}

/// SHA-256 en hexadécimal, par l'API de chiffrement de Windows.
fn sha256(data: &[u8]) -> Res<String> {
    let mut out = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) }.ok().map_err(|e| e.message())?;
    Ok(out.iter().map(|b| format!("{b:02x}")).collect())
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

/// GET par WinHTTP : le proxy et les certificats sont ceux de Windows, sans dépendance de
/// plus. Les redirections (GitHub sert ses fichiers depuis un autre domaine) sont suivies,
/// jamais de HTTPS vers HTTP.
fn get(url: &str, max: usize) -> Res<Vec<u8>> {
    let err = |e: windows::core::Error| e.message();
    let wide: Vec<u16> = url.encode_utf16().collect();
    let mut c = URL_COMPONENTS {
        dwStructSize: size_of::<URL_COMPONENTS>() as u32,
        dwHostNameLength: u32::MAX,
        dwUrlPathLength: u32::MAX,
        dwExtraInfoLength: u32::MAX,
        ..Default::default()
    };
    unsafe {
        WinHttpCrackUrl(&wide, 0, &mut c).map_err(err)?;
        let part = |p: windows::core::PWSTR, n: u32| -> Vec<u16> { std::slice::from_raw_parts(p.0, n as usize).iter().copied().chain([0]).collect() };
        let host = part(c.lpszHostName, c.dwHostNameLength);
        let mut path = part(c.lpszUrlPath, c.dwUrlPathLength + c.dwExtraInfoLength);
        if path.len() == 1 {
            path.insert(0, u16::from(b'/'));
        }
        let secure = if c.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let session = Handle::open(WinHttpOpen(w!("MonitorMG"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0))?;
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 30_000);
        let connection = Handle::open(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), c.nPort, 0))?;
        let request = Handle::open(WinHttpOpenRequest(connection.0, w!("GET"), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), secure))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(err)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(err)?;
        let (mut status, mut size) = (0u32, size_of::<u32>() as u32);
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(err)?;
        if status != 200 {
            return Err(tr!("Réponse HTTP {status}", "HTTP response {status}"));
        }
        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr() as *mut c_void, chunk.len() as u32, &mut read).map_err(err)?;
            if read == 0 {
                return Ok(body);
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > max {
                return Err(t("Réponse trop volumineuse", "Response too large").into());
            }
        }
    }
}

/// Interroge GitHub. Renvoie la version disponible si elle est plus récente que celle-ci,
/// et la garde pour l'interface.
pub(crate) fn check() -> Res<Option<String>> {
    let failed = |e: String| tr!("Vérification des mises à jour impossible. {e}", "Can't check for updates. {e}");
    let body = get(&format!("{API}/repos/{REPO}/releases/latest"), MAX_BODY).map_err(failed)?;
    let tag = latest_tag(&String::from_utf8_lossy(&body)).ok_or_else(|| failed(t("Réponse inattendue de GitHub", "Unexpected answer from GitHub").into()))?;
    let found = is_newer(&tag, VERSION).then_some(tag);
    let mut step = lock(&STEP);
    // Un téléchargement d'une autre version que celle trouvée ne sert plus.
    if matches!(&*step, Step::Ready(v, ..) if Some(v) != found.as_ref()) {
        *step = Step::Idle;
    }
    drop(step);
    *lock(&NEWER) = found.clone();
    Ok(found)
}

/// Vérifie et, si une version vient d'apparaître, l'annonce dans l'activité. Les échecs
/// (pas de réseau…) restent dans le journal de diagnostic : rien ne dérange l'utilisateur.
pub(crate) fn check_quietly(app: &AppHandle) {
    let before = newer();
    match check() {
        Ok(Some(v)) => {
            if before.as_ref() != Some(&v) {
                note(app, Level::Ok, Src::System, &tr!("MonitorMG {v} est disponible", "MonitorMG {v} is available"));
            }
            changed(app);
            follow(app);
        }
        Ok(None) => changed(app),
        Err(e) => note(app, Level::Info, Src::System, &e),
    }
}

/// Suite automatique d'une version trouvée, selon les réglages : téléchargement, qui
/// enchaîne lui-même sur l'installation automatique.
pub(crate) fn follow(app: &AppHandle) {
    let auto = store(app).auto_update;
    if newer().is_none() || store::portable() {
        return;
    }
    if auto.download && step().is_none() {
        let _ = download(app);
    } else if auto.install && step() == Some("ready") {
        install_when_idle(app);
    }
}

/// Dossier des téléchargements, vidé à chaque démarrage.
fn dir() -> PathBuf {
    std::env::temp_dir().join(format!("{}-maj", store::DATA_NAME))
}

/// Télécharge l'installateur de la version trouvée et le vérifie ; l'installe ensuite si
/// l'installation automatique est réglée. Sans effet si c'est déjà fait ou en cours.
pub(crate) fn download(app: &AppHandle) -> Res<()> {
    let v = newer().ok_or_else(|| t("Aucune version plus récente n'est connue", "No newer version is known"))?;
    if store::portable() {
        return Err(t("L'édition portable se met à jour depuis la page de la version", "The portable edition is updated from the release page").into());
    }
    {
        let mut step = lock(&STEP);
        if *step != Step::Idle {
            return Ok(());
        }
        *step = Step::Downloading;
    }
    changed(app);
    let r = fetch_setup(&v);
    *lock(&STEP) = match &r {
        Ok((file, sha)) => Step::Ready(v.clone(), file.clone(), sha.clone()),
        Err(_) => Step::Idle,
    };
    changed(app);
    match r {
        Ok(_) => {
            note(app, Level::Ok, Src::System, &tr!("MonitorMG {v} téléchargé et vérifié", "MonitorMG {v} downloaded and verified"));
            if store(app).auto_update.install {
                let app = app.clone();
                thread::spawn(move || install_when_idle(&app));
            }
            Ok(())
        }
        Err(e) => {
            note(app, Level::Error, Src::System, &e);
            Err(e)
        }
    }
}

/// Installateur et SHA256SUMS.txt de la release `v` ; l'installateur n'est gardé que si son
/// empreinte est celle annoncée.
fn fetch_setup(v: &str) -> Res<(PathBuf, String)> {
    let failed = |e: String| tr!("Téléchargement de MonitorMG {v} impossible. {e}", "Can't download MonitorMG {v}. {e}");
    // `v` vient de `version()` : des chiffres et des points, rien d'autre dans l'adresse.
    let file = format!("MonitorMG-{v}-setup.exe");
    let base = format!("{SITE}/{REPO}/releases/download/v{v}");
    let sums = get(&format!("{base}/SHA256SUMS.txt"), MAX_BODY).map_err(failed)?;
    let want = sum_for(&String::from_utf8_lossy(&sums), &file).ok_or_else(|| failed(t("Empreinte absente de SHA256SUMS.txt", "Hash missing from SHA256SUMS.txt").into()))?;
    let body = get(&format!("{base}/{file}"), MAX_SETUP).map_err(failed)?;
    if sha256(&body)? != want {
        return Err(failed(t("Le fichier reçu ne correspond pas à son empreinte", "The downloaded file doesn't match its hash").into()));
    }
    let path = dir().join(&file);
    fs::create_dir_all(dir()).and_then(|_| fs::write(&path, &body)).map_err(|e| failed(e.to_string()))?;
    Ok((path, want))
}

/// Un changement d'affichage attend confirmation, ou l'identification assombrit des écrans :
/// fermer MonitorMG maintenant laisserait l'un ou l'autre en plan.
fn busy(app: &AppHandle) -> bool {
    lock(&state_of(app).pending).is_some() || identifying()
}

/// Installation automatique : attend que MonitorMG soit libre.
/// ponytail: 30 essais, un par minute ; au-delà, un clic ou la prochaine vérification la relance.
fn install_when_idle(app: &AppHandle) {
    for _ in 0..30 {
        if !busy(app) {
            if let Err(e) = install(app) {
                note(app, Level::Error, Src::System, &e);
            }
            return;
        }
        thread::sleep(Duration::from_secs(60));
    }
}

/// Ligne de `cmd /c` : l'installateur Tauri (NSIS), puis MonitorMG relancé, même si
/// l'installation a échoué (l'ancienne version revient alors). Installateur : `/S` silencieux
/// ou `/P` progression seule, `/UPDATE` mise à jour en place (raccourcis gardés) ; `/FRESH`
/// est le nôtre (`windows/hooks.nsh`) et efface les données une fois les fichiers installés.
/// Le `/R` de l'installateur relancerait MonitorMG lui aussi, mais pas après un échec.
fn command_line(setup: &Path, exe: &Path, silent: bool, keep_data: bool, minimized: bool) -> String {
    format!(
        "start \"\" /wait \"{}\" {} /UPDATE{} & start \"\" \"{}\"{}",
        setup.display(),
        if silent { "/S" } else { "/P" },
        if keep_data { "" } else { " /FRESH" },
        exe.display(),
        if minimized { " --minimized" } else { "" },
    )
}

/// Lance l'installateur téléchargé et ferme MonitorMG, relancé ensuite, en arrière-plan si
/// sa fenêtre n'était pas ouverte.
pub(crate) fn install(app: &AppHandle) -> Res<()> {
    let Step::Ready(v, file, sha) = lock(&STEP).clone() else {
        return Err(t("La nouvelle version n'est pas encore téléchargée", "The new version isn't downloaded yet").into());
    };
    if busy(app) {
        return Err(t(
            "Installation reportée : un changement d'affichage attend confirmation ou une identification est en cours",
            "Installation postponed: a display change is awaiting confirmation or an identification is running",
        )
        .into());
    }
    // Le fichier a pu changer depuis le téléchargement : il est vérifié à nouveau.
    if fs::read(&file).ok().map(|b| sha256(&b)).and_then(Result::ok).as_ref() != Some(&sha) {
        *lock(&STEP) = Step::Idle;
        changed(app);
        return Err(t("Le fichier téléchargé a changé : téléchargez-le à nouveau", "The downloaded file has changed: download it again").into());
    }
    let auto = store(app).auto_update;
    let shown = app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false));
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cmd = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join(r"System32\cmd.exe");
    std::process::Command::new(cmd)
        .arg("/c")
        .raw_arg(command_line(&file, &exe, auto.silent, auto.keep_data, !shown))
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map_err(|e| e.to_string())?;
    *lock(&STEP) = Step::Installing;
    journal::write(Level::Info, Src::System.label(), &tr!("Installation de MonitorMG {v}, puis redémarrage", "Installing MonitorMG {v}, then restarting"));
    app.exit(0);
    Ok(())
}

/// Une vérification par jour, tant que l'option est activée.
pub(crate) fn watch(app: AppHandle) {
    thread::spawn(move || {
        // Téléchargements d'avant : installés ou abandonnés, ils ne servent plus.
        let _ = fs::remove_dir_all(dir());
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

    #[test]
    fn the_installer_hash_is_read_from_sha256sums() {
        let h = "dc2252bc0b1e34ff96d9711436d2ccbd944113a7f3f9ee8ed4bb6135d477f2a7";
        let sums = format!("e33c2e2341fdd622b93136b0099f849e59c942b3c6849b68f63c0ba5bdeb01c4  MonitorMG-2.2.4-portable.zip\r\n{}  MonitorMG-2.2.4-setup.exe\n", h.to_uppercase());
        assert_eq!(sum_for(&sums, "MonitorMG-2.2.4-setup.exe").as_deref(), Some(h));
        assert_eq!(sum_for(&format!("{h} *MonitorMG-2.2.4-setup.exe"), "MonitorMG-2.2.4-setup.exe").as_deref(), Some(h));
        assert_eq!(sum_for(&sums, "MonitorMG-2.2.5-setup.exe"), None);
        assert_eq!(sum_for("abc  MonitorMG-2.2.4-setup.exe", "MonitorMG-2.2.4-setup.exe"), None);
    }

    #[test]
    fn sha256_matches_the_reference() {
        assert_eq!(sha256(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn the_command_line_follows_the_settings() {
        let setup = Path::new(r"C:\Temp\MonitorMG-maj\MonitorMG-2.3.0-setup.exe");
        let exe = Path::new(r"C:\Users\a b\MonitorMG\MonitorMG.exe");
        assert_eq!(
            command_line(setup, exe, true, true, false),
            r#"start "" /wait "C:\Temp\MonitorMG-maj\MonitorMG-2.3.0-setup.exe" /S /UPDATE & start "" "C:\Users\a b\MonitorMG\MonitorMG.exe""#
        );
        assert_eq!(
            command_line(setup, exe, false, false, true),
            r#"start "" /wait "C:\Temp\MonitorMG-maj\MonitorMG-2.3.0-setup.exe" /P /UPDATE /FRESH & start "" "C:\Users\a b\MonitorMG\MonitorMG.exe" --minimized"#
        );
    }
}
