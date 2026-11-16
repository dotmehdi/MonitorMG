//! Place des fenêtres : un profil peut mémoriser où se trouvent les fenêtres ouvertes et
//! les y remettre après son application. Une fenêtre est reconnue par son programme, sa
//! classe et l'empreinte de son titre : le titre lui-même (souvent un nom de document)
//! n'est jamais enregistré.

use super::*;
use serde::Deserialize;
use windows::Win32::Foundation::{CloseHandle, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};

/// Au-delà, les fenêtres ne sont plus mémorisées : un profil reste petit.
const MAX: usize = 60;
/// Fenêtres de l'explorateur qui ne sont pas des fenêtres d'application.
const SHELL: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// Fenêtre mémorisée dans un profil.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Saved {
    /// Nom de l'exécutable, en minuscules.
    pub exe: String,
    pub class: String,
    /// Empreinte du titre : distingue deux fenêtres d'un même programme.
    pub title: u64,
    /// Position normale (gauche, haut, droite, bas), telle que Windows la rend et la reprend.
    pub rect: (i32, i32, i32, i32),
    /// 1 : normale, 2 : réduite, 3 : agrandie (valeurs SW_* de Windows).
    pub show: u32,
    /// Réduite, mais agrandie quand on la rouvre.
    #[serde(default)]
    pub max: bool,
}

/// Fenêtre ouverte en ce moment : sa poignée, et ce qu'on en mémorise.
struct Live {
    hwnd: isize,
    saved: Saved,
}

/// Empreinte FNV-1a : stable d'un lancement à l'autre, contrairement au hachage de `std`.
fn fingerprint(title: &str) -> u64 {
    title.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn exe_of(hwnd: HWND) -> Option<String> {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 || pid == unsafe { GetCurrentProcessId() } {
        return None; // nos propres fenêtres ne se mémorisent pas
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let read = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len) };
    let _ = unsafe { CloseHandle(process) };
    read.ok()?;
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    Some(path.rsplit(['\\', '/']).next().unwrap_or_default().to_lowercase())
}

/// Fenêtre d'application : visible, avec un titre, sans propriétaire, ni outil ni masquée
/// (bureau virtuel, application du Store suspendue).
fn describe(hwnd: HWND) -> Option<Live> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return None;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, size_of::<u32>() as u32);
        let mut title = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut title);
        if cloaked != 0 || n <= 0 {
            return None;
        }
        let mut class = [0u16; 128];
        let c = GetClassNameW(hwnd, &mut class);
        let class = String::from_utf16_lossy(&class[..c.max(0) as usize]);
        if SHELL.contains(&class.as_str()) {
            return None;
        }
        let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        GetWindowPlacement(hwnd, &mut wp).ok()?;
        let r = wp.rcNormalPosition;
        Some(Live {
            hwnd: hwnd.0 as isize,
            saved: Saved {
                exe: exe_of(hwnd)?,
                class,
                title: fingerprint(&String::from_utf16_lossy(&title[..n as usize])),
                rect: (r.left, r.top, r.right, r.bottom),
                show: wp.showCmd,
                max: wp.flags.0 & WPF_RESTORETOMAXIMIZED.0 != 0,
            },
        })
    }
}

fn open_windows() -> Vec<Live> {
    unsafe extern "system" fn each(hwnd: HWND, out: LPARAM) -> windows::core::BOOL {
        if let Some(w) = describe(hwnd) {
            (*(out.0 as *mut Vec<Live>)).push(w);
        }
        true.into()
    }
    let mut out: Vec<Live> = Vec::new();
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize)) };
    out
}

/// Les fenêtres d'application ouvertes, pour un profil.
pub fn capture() -> Vec<Saved> {
    open_windows().into_iter().map(|w| w.saved).take(MAX).collect()
}

/// Associe chaque fenêtre mémorisée à une fenêtre ouverte : (indice mémorisé, indice ouvert).
/// D'abord même programme, même classe et même titre ; puis, pour celles qui restent, même
/// programme et même classe, dans l'ordre. Une fenêtre ouverte ne sert qu'une fois.
fn pair(saved: &[Saved], open: &[Saved]) -> Vec<(usize, usize)> {
    let mut used = vec![false; open.len()];
    let mut out = Vec::new();
    let same_app = |s: &Saved, o: &Saved| s.exe == o.exe && s.class == o.class;
    for exact in [true, false] {
        for (i, s) in saved.iter().enumerate() {
            if out.iter().any(|&(si, _)| si == i) {
                continue;
            }
            let found = open.iter().enumerate().position(|(j, o)| !used[j] && same_app(s, o) && (!exact || s.title == o.title));
            if let Some(j) = found {
                used[j] = true;
                out.push((i, j));
            }
        }
    }
    out
}

/// Remet les fenêtres ouvertes à leur place mémorisée. Renvoie le nombre de fenêtres
/// déplacées. Celles d'un programme lancé en administrateur refusent : elles restent où
/// elles sont.
pub fn restore(saved: &[Saved]) -> usize {
    let open = open_windows();
    let now: Vec<Saved> = open.iter().map(|w| w.saved.clone()).collect();
    let front = unsafe { GetForegroundWindow() };
    let mut moved = 0;
    for (i, j) in pair(saved, &now) {
        let (s, cur) = (&saved[i], &now[j]);
        if (s.rect, s.show, s.max) == (cur.rect, cur.show, cur.max) {
            continue;
        }
        let hwnd = HWND(open[j].hwnd as *mut _);
        let place = |show: SHOW_WINDOW_CMD| {
            // On part de la place actuelle : les positions réduite et agrandie restent celles de Windows.
            let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            if unsafe { GetWindowPlacement(hwnd, &mut wp) }.is_err() {
                return false;
            }
            wp.flags = if s.max && show == SW_SHOWMINNOACTIVE { WPF_RESTORETOMAXIMIZED } else { WINDOWPLACEMENT_FLAGS(0) };
            wp.showCmd = show.0 as u32;
            wp.rcNormalPosition = RECT { left: s.rect.0, top: s.rect.1, right: s.rect.2, bottom: s.rect.3 };
            unsafe { SetWindowPlacement(hwnd, &wp) }.is_ok()
        };
        let done = match s.show {
            2 => place(SW_SHOWMINNOACTIVE),
            // Agrandie : d'abord à sa place normale, pour qu'elle s'agrandisse sur le bon écran.
            3 => place(SW_SHOWNOACTIVATE) && place(SW_SHOWMAXIMIZED),
            _ => place(SW_SHOWNOACTIVATE),
        };
        moved += usize::from(done);
    }
    // Agrandir une fenêtre l'active : la fenêtre de devant reprend sa place.
    if moved > 0 && !front.is_invalid() {
        let _ = unsafe { SetForegroundWindow(front) };
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(exe: &str, title: &str) -> Saved {
        Saved { exe: exe.into(), class: "Classe".into(), title: fingerprint(title), rect: (0, 0, 800, 600), show: 1, max: false }
    }

    #[test]
    fn windows_are_matched_by_title_then_by_program() {
        let saved = [win("code.exe", "projet A"), win("code.exe", "projet B"), win("notes.exe", "liste")];
        // Ouvertes dans un autre ordre ; le titre de « liste » a changé ; « projet A » est fermé.
        let open = [win("code.exe", "projet B"), win("notes.exe", "liste (modifié)"), win("autre.exe", "x")];
        assert_eq!(pair(&saved, &open), [(1, 0), (2, 1)]);
        // Deux fenêtres du même programme sans titre reconnu : dans l'ordre, chacune une fois.
        let open = [win("code.exe", "sans titre 1"), win("code.exe", "sans titre 2")];
        assert_eq!(pair(&saved, &open), [(0, 0), (1, 1)]);
        assert!(pair(&saved, &[]).is_empty());
    }

    #[test]
    fn title_fingerprints_are_stable() {
        assert_eq!(fingerprint(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fingerprint("a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fingerprint("Rapport.docx"), fingerprint("Rapport2.docx"));
    }
}
