//! Luminosité logicielle : un voile noir, traversé par les clics, posé sur chaque écran
//! que ni DDC/CI ni WMI ne savent régler. Les voiles sont des fenêtres du thread de
//! surveillance (`watch.rs`), qui a déjà sa boucle de messages. Rien n'est écrit dans
//! l'écran : le voile disparaît avec MonitorMG, et une capture d'écran le contient.

use super::*;
use std::cell::RefCell;
use std::sync::Once;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};

/// Message qui demande au thread de surveillance de mettre les voiles à jour.
pub(crate) const WM_VEILS: u32 = WM_APP + 1;

thread_local! {
    /// Voile affiché sur chaque écran : clé -> fenêtre. Propre au thread de surveillance.
    static SHOWN: RefCell<BTreeMap<String, isize>> = const { RefCell::new(BTreeMap::new()) };
}

/// Demande la mise à jour des voiles (luminosité logicielle ou disposition modifiée).
/// Sans fenêtre de surveillance (raccourci bureau, ligne de commande), il n'y a pas de voile.
pub(crate) fn refresh() {
    let h = WATCHER.load(Ordering::SeqCst);
    if h != 0 {
        let _ = unsafe { PostMessageW(Some(HWND(h as *mut _)), WM_VEILS, WPARAM(0), LPARAM(0)) };
    }
}

unsafe extern "system" fn veil_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(h, msg, wp, lp)
}

/// Crée, déplace ou retire les voiles d'après `display::veils()`. À appeler sur le thread
/// de surveillance : une fenêtre appartient au thread qui la crée.
pub(crate) fn sync() {
    static CLASS: Once = Once::new();
    let class = w!("MonitorMGVeil");
    let inst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    CLASS.call_once(|| unsafe {
        let black = HBRUSH(GetStockObject(BLACK_BRUSH).0);
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(veil_proc), hInstance: inst.into(), lpszClassName: class, hbrBackground: black, ..Default::default() });
    });
    let want = display::veils();
    SHOWN.with_borrow_mut(|shown| {
        shown.retain(|key, h| {
            let keep = want.contains_key(key);
            if !keep {
                let _ = unsafe { DestroyWindow(HWND(*h as *mut _)) };
            }
            keep
        });
        for (key, (x, y, w, h, alpha)) in want {
            let known = shown.get(&key).map(|&h| HWND(h as *mut _));
            // Transparent aux clics, hors de la barre des tâches et d'Alt+Tab, jamais actif.
            let style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            let Some(veil) = known.or_else(|| unsafe { CreateWindowExW(style, class, w!(""), WS_POPUP, x, y, w, h, None, None, Some(inst.into()), None) }.ok()) else {
                continue;
            };
            shown.insert(key, veil.0 as isize);
            unsafe {
                let _ = SetLayeredWindowAttributes(veil, COLORREF(0), alpha, LWA_ALPHA);
                let _ = SetWindowPos(veil, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
        }
    });
}
