// gui — panneau de contrôle Win32 natif: statut temps réel, commandes,
// journal en miroir de la console. ZÉRO dépendance ajoutée: BUTTON / EDIT /
// LISTBOX / STATIC sont des classes User32 standard; leurs messages
// WM_COMMAND sont routés vers panel_proc par le DispatchMessageW de la boucle
// overlay existante (une seule pompe pour overlay + panneau).
//
// Fermer le panneau (croix ou bouton) = PostQuitMessage → WM_QUIT remonte
// dans la boucle overlay → arrêt propre (comme END).
//
// NOTES windows-rs 0.58 (VÉRIFIÉES contre le tag source — scripts/wam.rs,
// scripts/controls.rs):
//   · BS_AUTOCHECKBOX / ES_MULTILINE / ES_READONLY = consts i32 BRUTES →
//     caster WINDOW_STYLE(… as u32) pour les combiner à WS_CHILD
//   · BST_CHECKED / BST_UNCHECKED vivent dans Win32::UI::Controls
//     (newtype DLG_BUTTON_CHECK_STATE) → feature Win32_UI_Controls requise
//   · BM_SETCHECK / BN_CLICKED / LB_* = consts u32 simples (WindowsAndMessaging)
//   · SendMessageW(hwnd, msg: u32, wparam, lparam) -> LRESULT (générique Param)
//   · SetWindowTextW / DestroyWindow → Result<()>; PostQuitMessage → ()
//   · l'ID d'un contrôle enfant se pose dans le paramètre hmenu:
//     HMENU(id as *mut c_void)

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, CS_HREDRAW, CS_VREDRAW, CreateWindowExW,
    DefWindowProcW, DestroyWindow, ES_MULTILINE, ES_READONLY, HMENU, LB_ADDSTRING,
    LB_DELETESTRING, LB_GETCOUNT, PostQuitMessage, RegisterClassExW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_SETFONT, WNDCLASSEXW, WS_CAPTION,
    WS_CHILD, WS_EX_CLIENTEDGE, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_VISIBLE,
    WS_VSCROLL, SendMessageW, SetWindowTextW,
};

// ── événements panneau → boucle overlay ─────────────────────────────────────
#[derive(Clone, Copy, Debug)]
pub enum PanelEvent {
    ToggleVerbose,
    ToggleTeammates,
    Dump,
    Quit,
}

static EVENTS: OnceLock<Mutex<VecDeque<PanelEvent>>> = OnceLock::new();
fn events() -> &'static Mutex<VecDeque<PanelEvent>> {
    EVENTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// À drainer chaque itération de la boucle overlay.
pub fn drain_events() -> Vec<PanelEvent> {
    events().lock().map(|mut q| q.drain(..).collect()).unwrap_or_default()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const ID_CHK_VERBOSE: usize = 2001;
const ID_CHK_TM: usize = 2002;
const ID_BTN_DUMP: usize = 2003;
const ID_BTN_QUIT: usize = 2004;

const LOG_MAX: i64 = 400; // lignes gardées dans la listbox

// la classe n'est enregistrée qu'UNE fois par process — au re-attach après
// GameClosed, un second RegisterClassExW échouerait (ERROR_CLASS_ALREADY_EXISTS)
// et le panneau de diagnostic disparaissait pour de mauvaises raisons.
static CLASS_DONE: OnceLock<()> = OnceLock::new();

unsafe extern "system" fn panel_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wp.0 & 0xFFFF) as usize;
            let code = (wp.0 >> 16) & 0xFFFF; // usize — cast BN_CLICKED en usize
            if code == BN_CLICKED as usize {
                let ev = match id {
                    ID_CHK_VERBOSE => Some(PanelEvent::ToggleVerbose),
                    ID_CHK_TM => Some(PanelEvent::ToggleTeammates),
                    ID_BTN_DUMP => Some(PanelEvent::Dump),
                    ID_BTN_QUIT => Some(PanelEvent::Quit),
                    _ => None,
                };
                if let Some(e) = ev {
                    if let Ok(mut q) = events().lock() {
                        q.push_back(e);
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0); // remonte jusqu'à la boucle overlay via PeekMessageW
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub struct Panel {
    hwnd: HWND,
    status: HWND, // EDIT read-only multi-lignes — verdict temps réel
    log: HWND,    // LISTBOX — miroir console
    chk_verbose: HWND,
    chk_tm: HWND,
    #[allow(dead_code)] // gardée vivante: les contrôles référencent cette police
    font: HFONT,
}

impl Panel {
    pub fn new() -> Result<Panel, String> {
        unsafe {
            let hmodule = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
            let hinst: HINSTANCE = hmodule.into();

            CLASS_DONE.get_or_init(|| {
                let class = wide("vantyx_panel_wnd");
                let mut wc: WNDCLASSEXW = std::mem::zeroed();
                wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
                wc.style = CS_HREDRAW | CS_VREDRAW;
                wc.lpfnWndProc = Some(panel_proc);
                wc.hInstance = hinst;
                wc.lpszClassName = PCWSTR(class.as_ptr());
                RegisterClassExW(&wc);
            });

            let class = wide("vantyx_panel_wnd");
            let title = wide("Vantyx.cc — panneau de contrôle");
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE,
                48, 48, 470, 640,
                None, None, hinst, None,
            )
            .map_err(|e| format!("CreateWindowExW (panneau): {e}"))?;

            let font = crate::overlay::make_font(-14, 400, "Segoe UI");

            // contrôle enfant générique: la classe User32 + l'ID → WM_COMMAND
            let mk_child = |class: &str, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE, text: &str,
                            x: i32, y: i32, w: i32, h: i32, id: usize|
                -> Result<HWND, String> {
                let c = wide(class);
                let t = wide(text);
                // (le corps de closure est lexicalement DANS le unsafe externe)
                let child = CreateWindowExW(
                        ex,
                        PCWSTR(c.as_ptr()),
                        PCWSTR(t.as_ptr()),
                        WS_CHILD | WS_VISIBLE | style,
                        x, y, w, h,
                        hwnd,
                        HMENU(id as *mut core::ffi::c_void),
                        hinst,
                        None,
                )
                .map_err(|e| format!("contrôle {class}: {e}"))?;
                SendMessageW(child, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
                Ok(child)
            };

            // statut — 4 lignes max, maj 2×/s par overlay::run
            let status = mk_child(
                "EDIT",
                WINDOW_STYLE((ES_MULTILINE | ES_READONLY) as u32) | WS_VSCROLL,
                WS_EX_CLIENTEDGE,
                "",
                12, 12, 430, 100, 0,
            )?;
            let chk_verbose = mk_child(
                "BUTTON",
                WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                WINDOW_EX_STYLE(0),
                "debug verbeux (F1)",
                16, 124, 205, 24, ID_CHK_VERBOSE,
            )?;
            let chk_tm = mk_child(
                "BUTTON",
                WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
                WINDOW_EX_STYLE(0),
                "coéquipiers (F2)",
                236, 124, 205, 24, ID_CHK_TM,
            )?;
            let _btn_dump = mk_child(
                "BUTTON",
                WINDOW_STYLE(0),
                WINDOW_EX_STYLE(0),
                "dump console (F4)",
                16, 156, 207, 34, ID_BTN_DUMP,
            )?;
            let _btn_quit = mk_child(
                "BUTTON",
                WINDOW_STYLE(0),
                WINDOW_EX_STYLE(0),
                "quitter (END)",
                236, 156, 207, 34, ID_BTN_QUIT,
            )?;
            let _lbl = mk_child(
                "STATIC",
                WINDOW_STYLE(0),
                WINDOW_EX_STYLE(0),
                "journal (console en miroir):",
                16, 200, 320, 20, 0,
            )?;
            let log = mk_child(
                "LISTBOX",
                WS_VSCROLL,
                WS_EX_CLIENTEDGE,
                "",
                12, 224, 430, 372, 0,
            )?;

            Ok(Panel { hwnd, status, log, chk_verbose, chk_tm, font })
        }
    }

    /// statut multi-lignes (EDIT): verdict snapshot, fps, rendu, offsets.
    pub fn set_status(&self, s: &str) {
        let w = wide(s);
        unsafe {
            let _ = SetWindowTextW(self.status, PCWSTR(w.as_ptr()));
        }
    }

    /// ajoute les lignes drainées du journal, plafonne la listbox.
    pub fn push_log(&self, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        unsafe {
            for s in lines {
                let w = wide(s);
                SendMessageW(self.log, LB_ADDSTRING, WPARAM(0), LPARAM(w.as_ptr() as isize));
            }
            let n = SendMessageW(self.log, LB_GETCOUNT, WPARAM(0), LPARAM(0)).0 as i64;
            let mut excess = n - LOG_MAX;
            while excess > 0 {
                SendMessageW(self.log, LB_DELETESTRING, WPARAM(0), LPARAM(0));
                excess -= 1;
            }
        }
    }

    /// resynchronise les cases (F1/F2 clavier ↔ panneau — la vérité = les atomics).
    pub fn sync_checks(&self, verbose: bool, teammates: bool) {
        unsafe {
            SendMessageW(
                self.chk_verbose,
                BM_SETCHECK,
                WPARAM(if verbose { BST_CHECKED.0 as usize } else { BST_UNCHECKED.0 as usize }),
                LPARAM(0),
            );
            SendMessageW(
                self.chk_tm,
                BM_SETCHECK,
                WPARAM(if teammates { BST_CHECKED.0 as usize } else { BST_UNCHECKED.0 as usize }),
                LPARAM(0),
            );
        }
    }
}

// run() peut retourner (jeu fermé) puis être rappelé — sans ça, la fenêtre
// orpheline du panneau précédent resterait vivante et dupliquée.
impl Drop for Panel {
    fn drop(&mut self) {
        unsafe { let _ = DestroyWindow(self.hwnd); }
    }
}
