// overlay — fenêtre layerée, alpha PAR PIXEL (UpdateLayeredWindow), boîtes,
// SQUELETTE, RADAR, menu glass Direct2D, texte DirectWrite, TRIGGERBOT.
//
// ARCHITECTURE v0.3.0:
//   DIB section 32bpp top-down (CreateDIBSection) → DC render target Direct2D
//   en mode PREMULTIPLIED → dessin D2D + texte DirectWrite → UpdateLayered-
//   Window(ULW_ALPHA) présente le bitmap prémultiplié.
//
//   POLICE: Space Grotesk chargée EN MÉMOIRE via IDWriteFactory5/
//   InMemoryFontFileLoader + FontSetBuilder → collection custom (aucune
//   installation système requise). Repli automatique sur Segoe UI.
//
//   ESP "JEU SEULEMENT": l'overlay se CACHE quand CS2 n'est plus au premier
//   plan (alt-tab Discord etc.) et l'ESP se coupe quand un menu du jeu est
//   ouvert (curseur visible OU m_bIsBuyMenuOpen) — plus de boîtes flottant
//   au-dessus de Discord.
//
//   TRIGGERBOT: m_iIDEntIndex (entité sous le viseur) → ennemi vivant détecté
//   par le snapshot → machine à états (armé → délai → clic → cooldown) via
//   SendInput. Touche configurable (ALT par défaut) + vérif équipe OBLIGATOIRE.
//
// NOTES windows-rs 0.58 (signatures VÉRIFIÉES contre le tag source 0.58.0 —
// scripts téléchargés dans my-project/scripts/winrs058b):
//   · D2D1CreateFactory: API OFFICIELLE = D2D1CreateFactory::<T>(type,
//     Option<*const OPTIONS>) -> Result<T>. NE JAMAIS régresser vers un FFI
//     brut 3-args (bug v0.1.3/v0.1.4: access violation).
//   · DWriteCreateFactory::<T>(DWRITE_FACTORY_TYPE) -> Result<T> (1 arg!)
//   · IDWriteFactory5::CreateInMemoryFontFileLoader() -> Result<..> (0 arg)
//   · IDWriteInMemoryFontFileLoader::CreateInMemoryFontFileReference(&IDWrite-
//     Factory, *const c_void, u32, None) -> Result<IDWriteFontFile>
//   · IDWriteFactory5::CreateFontSetBuilder() -> Result<IDWriteFontSetBuilder1>
//     (AddFontFile vit sur Builder1, CreateFontSet sur la base via Deref)
//   · IDWriteFactory3::CreateFontCollectionFromFontSet(&IDWriteFontSet)
//   · IDWriteFactory::CreateTextFormat(family, &Collection1 (CanInto), weight,
//     style, stretch, size, locale) — &IDWriteFontCollection1 passe DIRECTEMENT
//     (impl Param<IDWriteFontCollection> pour &U: CanInto<T>)
//   · GetCursorInfo(*mut CURSORINFO) -> Result<()>; CURSOR_SHOWING.0 teste les
//     flags · ShowWindow(hwnd, SHOW_WINDOW_CMD) · IsIconic(hwnd) -> BOOL
//   · SendInput(&[INPUT], i32) — INPUT{r#type, Anonymous: INPUT_0{mi: MOUSE-
//     INPUT{dx,dy,mouseData,dwFlags,time,dwExtraInfo}}}
//   · UpdateLayeredWindow: pptDst/psize EXPLICITES OBLIGATOIRES (sinon DWM
//     n'applique jamais la présentation — vérifié en live)
//   · CreateDIBSection: *mut *mut c_void BRUT (pas Option)
//   · ScreenToClient vit dans Win32::Graphics::Gdi (pas WAM!)

use std::time::Instant;

use crate::debug;
use crate::entity::{self, Snapshot, BONE_CONNECTS, B_HEAD};
use crate::gui;
use crate::math::PLAYER_HEIGHT;
use crate::mem::Mem;
use crate::menu::{self, mix4, Action, Align, Mouse, Options, PALETTES, StatusInfo};
use crate::offsets::Offsets;
use crate::{dlog, dwarn};

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_ELLIPSE, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    ID2D1DCRenderTarget, ID2D1Factory, ID2D1GradientStopCollection, ID2D1LinearGradientBrush,
    ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
    D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory5, IDWriteFontCollection1, IDWriteInMemoryFontFileLoader,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET,
    DEFAULT_PITCH, DeleteObject, DIB_RGB_COLORS, GetDC, GetObjectW, HBITMAP, HDC, HFONT, HGDIOBJ,
    OUT_DEFAULT_PRECIS, ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::Sleep;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, VIRTUAL_KEY, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEINPUT, VK_CONTROL, VK_END, VK_F1, VK_F2, VK_F4, VK_F8, VK_INSERT,
    VK_MENU, VK_SHIFT, VK_XBUTTON2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, CS_HREDRAW, CS_VREDRAW, CURSORINFO, CURSOR_SHOWING, DefWindowProcW,
    DestroyWindow, DispatchMessageW, FindWindowW, GetClientRect, GetCursorInfo,
    GetForegroundWindow, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, GWL_EXSTYLE,
    HWND_TOPMOST, IsIconic, MSG, MoveWindow, PeekMessageW, PM_REMOVE, RegisterClassExW, ShowWindow,
    SetWindowLongPtrW, SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOW,
    SM_CXSCREEN, SM_CYSCREEN, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow,
    WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSEXW, WM_QUIT, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};

pub enum ExitCode {
    UserQuit,
    GameClosed,
    Fatal(String), // initialisation impossible — l'étape fautive est nommée
}

pub struct Config {
    pub panel: bool,           // --panel: panneau diagnostic dès le boot
    pub teammates: bool,       // --teammates: override ini
    pub verbose: bool,         // --verbose: override ini
    pub interval_ms: Option<u64>, // --interval-ms N: override ini
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
fn cf(c: [f32; 4]) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a: c[3] }
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

// ── fenêtre du jeu: HWND + bornes (fenêtré / borderless) ────────────────────
fn game_hwnd() -> Option<HWND> {
    let title = wide(crate::GAME_WINDOW);
    unsafe { FindWindowW(None, PCWSTR(title.as_ptr())).ok() }
}

fn game_window_bounds() -> Option<(i32, i32, i32, i32)> {
    let hwnd = game_hwnd()?;
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(hwnd, &mut rc) }.is_err() {
        return None;
    }
    Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
}

// CS2 est-il au premier plan (et pas minimisé) ? — pilote la visibilité de
// l'overlay: caché dès que tu alt-tab vers Discord, revenu dès que tu reviens.
fn game_foreground() -> bool {
    match game_hwnd() {
        Some(g) => {
            let fg = unsafe { GetForegroundWindow() };
            let iconic = unsafe { IsIconic(g) }.as_bool();
            fg == g && !iconic
        }
        None => true, // fenêtre introuvable (chargement) — ne pas cacher
    }
}

// curseur SYSTÈME visible ? — vrai dans les menus du jeu, le menu d'achat,
// l'ESC, le menu principal. C'est le signal "interface du jeu ouverte".
fn cursor_visible() -> bool {
    unsafe {
        let mut ci: CURSORINFO = std::mem::zeroed();
        ci.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
        if GetCursorInfo(&mut ci).is_ok() {
            ci.flags.0 & CURSOR_SHOWING.0 != 0
        } else {
            false
        }
    }
}

// la classe overlay n'est enregistrée qu'UNE fois par process — au re-attach
// après GameClosed, un second RegisterClassExW échouerait (ERROR_CLASS_ALREADY_
// EXISTS) et run() retournait Fatal pour de mauvaises raisons.
static OVERLAY_CLASS_DONE: std::sync::OnceLock<()> = std::sync::OnceLock::new();

fn create_overlay_window() -> Result<HWND, String> {
    unsafe {
        let hmodule = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let hinst: HINSTANCE = hmodule.into();

        OVERLAY_CLASS_DONE.get_or_init(|| {
            let class = wide("cs2_esp_wnd");
            let mut wc: WNDCLASSEXW = std::mem::zeroed();
            wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
            wc.style = CS_HREDRAW | CS_VREDRAW;
            wc.lpfnWndProc = Some(wnd_proc);
            wc.hInstance = hinst;
            wc.lpszClassName = PCWSTR(class.as_ptr());
            RegisterClassExW(&wc);
        });

        let class = wide("cs2_esp_wnd");

        let title = wide("cs2-esp overlay");
        let (x, y, w, h) = game_window_bounds().unwrap_or_else(|| {
            (0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
        });
        dlog!("[*] overlay bounds ({x},{y}) {w}×{h}");

        // click-through (retiré quand le menu est ouvert) + topmost + pas
        // d'activation (le jeu garde le focus clavier, les clics arrivent
        // quand même par hit-test).
        // PAS de SetLayeredWindowAttributes: la voie est UpdateLayeredWindow
        // (alpha par pixel) — SLWA et ULW sont exclusifs.
        let ex: WINDOW_EX_STYLE =
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        let st: WINDOW_STYLE = WS_POPUP | WS_VISIBLE;
        let hwnd = CreateWindowExW(
            ex,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            st,
            x, y, w, h,
            None,
            None,
            hinst,
            None,
        )
        .map_err(|e| format!("CreateWindowExW (overlay): {e}"))?;
        Ok(hwnd)
    }
}

/// RAII: détruit la fenêtre overlay quand run() retourne (re-attach propre).
struct WinGuard(HWND);
impl Drop for WinGuard {
    fn drop(&mut self) {
        unsafe { let _ = DestroyWindow(self.0); }
    }
}

/// menu ouvert → l'overlay capte la souris; fermé → click-through vers le jeu.
fn set_click_through(hwnd: HWND, click_through: bool) {
    unsafe {
        let cur = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let want = if click_through {
            cur | WS_EX_TRANSPARENT.0
        } else {
            cur & !WS_EX_TRANSPARENT.0
        };
        if want != cur {
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want as isize);
        }
    }
}

/// front descendant d'une touche (bit 0x8000 + mémoire de l'état précédent).
fn pressed(vk: VIRTUAL_KEY, prev: &mut bool) -> bool {
    let down = unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 };
    let edge = down && !*prev;
    *prev = down;
    edge
}

/// touche du triggerbot enfoncée ? (0=ALT 1=CTRL 2=bouton4 3=MAJ)
fn trigger_key_down(sel: usize) -> bool {
    let vk = match sel {
        0 => VK_MENU,
        1 => VK_CONTROL,
        2 => VK_XBUTTON2,
        _ => VK_SHIFT,
    };
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

/// clic souris SYNTHÉTIQUE (SendInput) — le jeu le reçoit comme un vrai clic
/// gauche. Utilisé par le triggerbot uniquement.
fn send_click(down: bool) {
    unsafe {
        let mut input = INPUT::default();
        input.r#type = INPUT_MOUSE;
        input.Anonymous = INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: if down { MOUSEEVENTF_LEFTDOWN } else { MOUSEEVENTF_LEFTUP },
                time: 0,
                dwExtraInfo: 0,
            },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

// ═══ machine à états du triggerbot ═════════════════════════════════════════
// Idle → (conditions réunies) → Armed(délai réaction) → Firing(clic tenu
// ~45 ms) → Cooldown(~120 ms) → Idle. Les conditions qui lâchent en plein
// tir relâchent le clic immédiatement.
#[derive(Debug)]
enum TrigState {
    Idle,
    Armed(Instant),
    Firing(Instant),
    Cooldown(Instant),
}

struct TriggerBot {
    state: TrigState,
    pub armed_visual: bool, // affiché: viseur sur ennemi + touche tenue
}

impl TriggerBot {
    fn new() -> Self {
        TriggerBot { state: TrigState::Idle, armed_visual: false }
    }

    /// conditions = ennemi vivant sous le viseur + touche tenue + contexte OK
    fn update(&mut self, conditions: bool, delay_ms: u32) {
        const HOLD_MS: u128 = 45;
        const COOLDOWN_MS: u128 = 120;
        self.armed_visual = conditions;
        loop {
            match self.state {
                TrigState::Idle => {
                    if conditions {
                        self.state = TrigState::Armed(Instant::now());
                        continue;
                    }
                    break;
                }
                TrigState::Armed(t0) => {
                    if !conditions {
                        self.state = TrigState::Idle;
                        continue;
                    }
                    if t0.elapsed().as_millis() >= delay_ms as u128 {
                        send_click(true);
                        self.state = TrigState::Firing(Instant::now());
                        dlog!("[trig] ENNEMI sous le viseur → clic (délai {delay_ms} ms)");
                    }
                    break;
                }
                TrigState::Firing(t0) => {
                    if !conditions {
                        send_click(false); // relâche immédiatement
                        self.state = TrigState::Idle;
                        continue;
                    }
                    if t0.elapsed().as_millis() >= HOLD_MS {
                        send_click(false);
                        self.state = TrigState::Cooldown(Instant::now());
                    }
                    break;
                }
                TrigState::Cooldown(t0) => {
                    if t0.elapsed().as_millis() >= COOLDOWN_MS {
                        self.state = TrigState::Idle;
                        continue;
                    }
                    break;
                }
            }
        }
    }
}

// ═══ police Space Grotesk en mémoire (cache process, une seule fois) ═══════
// IDWriteFactory5 → InMemoryFontFileLoader → FontSetBuilder → collection.
// Les octets TTF restent vivants dans le cache (le loader y référence).
static FONT_CACHE: std::sync::OnceLock<(Option<IDWriteFontCollection1>, Vec<Vec<u8>>, String)> =
    std::sync::OnceLock::new();

const FONT_FILES: [&str; 4] = [
    "SpaceGrotesk-Regular.ttf",
    "SpaceGrotesk-Medium.ttf",
    "SpaceGrotesk-SemiBold.ttf",
    "SpaceGrotesk-Bold.ttf",
];

unsafe fn space_grotesk_collection(dw: &IDWriteFactory) -> &'static (Option<IDWriteFontCollection1>, Vec<Vec<u8>>, String) {
    FONT_CACHE.get_or_init(|| {
        let fail = |why: &str| {
            dwarn!("[font] {why} — repli sur la police système (Segoe UI)");
            (None, Vec::new(), why.to_string())
        };
        // répertoire: fonts/ à côté de l'exe, sinon l'exe lui-même
        let mut dir: Option<std::path::PathBuf> = None;
        if let Ok(exe) = std::env::current_exe() {
            if let Some(d) = exe.parent() {
                if d.join("fonts").join(FONT_FILES[0]).exists() {
                    dir = Some(d.join("fonts"));
                } else if d.join(FONT_FILES[0]).exists() {
                    dir = Some(d.to_path_buf());
                }
            }
        }
        let Some(dir) = dir else {
            return fail("fonts/ introuvable à côté de l'exe");
        };
        let Ok(f5) = dw.cast::<IDWriteFactory5>() else {
            return fail("IDWriteFactory5 indisponible (Windows < 10 1704?)");
        };
        let Ok(loader): Result<IDWriteInMemoryFontFileLoader, _> = f5.CreateInMemoryFontFileLoader() else {
            return fail("CreateInMemoryFontFileLoader a échoué");
        };
        if dw.RegisterFontFileLoader(&loader).is_err() {
            return fail("RegisterFontFileLoader a échoué");
        }
        let Ok(builder) = f5.CreateFontSetBuilder() else {
            return fail("CreateFontSetBuilder a échoué");
        };
        let mut data_vec: Vec<Vec<u8>> = Vec::new();
        let mut added = 0usize;
        for name in FONT_FILES {
            let Ok(data) = std::fs::read(dir.join(name)) else { continue };
            let Ok(file) = loader.CreateInMemoryFontFileReference(
                dw,
                data.as_ptr().cast(),
                data.len() as u32,
                None,
            ) else {
                continue;
            };
            if builder.AddFontFile(&file).is_ok() {
                // les octets DOIVENT rester vivants tant que la collection vit
                data_vec.push(data);
                added += 1;
            }
        }
        if added == 0 {
            return fail("aucun TTF Space Grotesk lisible");
        }
        let Ok(set) = builder.CreateFontSet() else {
            return fail("CreateFontSet a échoué");
        };
        let Ok(coll) = f5.CreateFontCollectionFromFontSet(&set) else {
            return fail("CreateFontCollectionFromFontSet a échoué");
        };
        let src = format!("Space Grotesk ×{added} (en mémoire)");
        dlog!("[font] {src} — {:#x} octets chargés", data_vec.iter().map(|v| v.len()).sum::<usize>());
        (Some(coll), data_vec, src)
    })
}

pub(crate) fn make_font(height: i32, weight: i32, face: &str) -> HFONT {
    let f = wide(face);
    unsafe {
        CreateFontW(
            height, 0, 0, 0, weight, 0, 0, 0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            PCWSTR(f.as_ptr()),
        )
    }
}

// DIB 32bpp top-down — le buffer qu'UpdateLayeredWindow présente.
unsafe fn create_dib(windc: HDC, w: i32, h: i32) -> Result<(HBITMAP, *mut core::ffi::c_void), String> {
    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // négatif = top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let dib = CreateDIBSection(windc, &bmi, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0)
        .map_err(|e| format!("CreateDIBSection: {e}"))?;
    Ok((dib, bits))
}

pub(crate) struct Renderer {
    hwnd: HWND,
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    grad: ID2D1LinearGradientBrush, // accent mint→azur, repositionné par usage
    sheen: ID2D1LinearGradientBrush, // reflet blanc supérieur du verre
    memdc: HDC,
    dib: HBITMAP,
    dw: IDWriteFactory,
    // formats texte (DPI 96 forcé → taille = pixels) — Space Grotesk si dispo
    pub(crate) fmt_h: IDWriteTextFormat,    // 16 semibold — logo/titres
    pub(crate) fmt_b: IDWriteTextFormat,    // 13 normal — libellés
    pub(crate) fmt_s: IDWriteTextFormat,    // 11 normal — petits textes
    pub(crate) fmt_name: IDWriteTextFormat, // 13 semibold — noms joueurs
    pub(crate) fmt_mono: IDWriteTextFormat, // 12 Consolas — valeurs/watermark
    pub(crate) font_src: String,            // quelle police est en service
    size: (i32, i32),
}

impl Renderer {
    // PLUS AUCUN expect/panic: chaque étape se nomme dans l'erreur — un
    // échec de rendu laisse le panneau + console vivants pour diagnostiquer.
    fn try_new(hwnd: HWND) -> Result<Renderer, String> {
        unsafe {
            let mut rc: RECT = std::mem::zeroed();
            let _ = GetClientRect(hwnd, &mut rc);
            let (w, h) = (rc.right.max(2), rc.bottom.max(2));

            let windc = GetDC(hwnd);
            if windc.0.is_null() {
                return Err("GetDC → NULL".into());
            }
            let memdc = CreateCompatibleDC(windc);
            if memdc.0.is_null() {
                return Err("CreateCompatibleDC → NULL".into());
            }
            let (dib, _bits) = create_dib(windc, w, h)?;
            SelectObject(memdc, HGDIOBJ(dib.0));
            ReleaseDC(hwnd, windc);

            // D2D — factory par l'API OFFICIELLE windows-rs 0.58 (le FFI brut
            // 3-args de v0.1.3 crashait: voir note anti-régression en tête).
            let factory: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                    .map_err(|e| format!("D2D1CreateFactory: {e}"))?;

            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED, // alpha par pixel
                },
                dpiX: 96.0,
                dpiY: 96.0, // 1 DIP = 1 px — layout du menu pixel-exact
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt: ID2D1DCRenderTarget = factory
                .CreateDCRenderTarget(&props)
                .map_err(|e| format!("CreateDCRenderTarget: {e}"))?;
            let brush: ID2D1SolidColorBrush = rt
                .CreateSolidColorBrush(&cf([1.0, 1.0, 1.0, 1.0]), None)
                .map_err(|e| format!("CreateSolidColorBrush: {e}"))?;

            // dégradé accent (mint → azur)
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: cf(menu::ACCENT_A) },
                D2D1_GRADIENT_STOP { position: 1.0, color: cf(menu::ACCENT_B) },
            ];
            let coll: ID2D1GradientStopCollection = rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                .map_err(|e| format!("CreateGradientStopCollection: {e}"))?;
            let gp = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: 0.0, y: 0.0 },
                endPoint: D2D_POINT_2F { x: 100.0, y: 0.0 },
            };
            let grad: ID2D1LinearGradientBrush = rt
                .CreateLinearGradientBrush(&gp, None, &coll)
                .map_err(|e| format!("CreateLinearGradientBrush: {e}"))?;

            // reflet "sheen" (fondu blanc vertical — la signature glass)
            let sheen_stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: cf([1.0, 1.0, 1.0, 0.16]) },
                D2D1_GRADIENT_STOP { position: 1.0, color: cf([1.0, 1.0, 1.0, 0.0]) },
            ];
            let sheen_coll: ID2D1GradientStopCollection = rt
                .CreateGradientStopCollection(&sheen_stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                .map_err(|e| format!("CreateGradientStopCollection(sheen): {e}"))?;
            let sheen: ID2D1LinearGradientBrush = rt
                .CreateLinearGradientBrush(&gp, None, &sheen_coll)
                .map_err(|e| format!("CreateLinearGradientBrush(sheen): {e}"))?;

            // DirectWrite — AA correct + alpha prémultiplié (le GDI écraserait
            // l'octet alpha → incompatible UpdateLayeredWindow)
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)
                .map_err(|e| format!("DWriteCreateFactory: {e}"))?;

            // ── Space Grotesk en mémoire (repli Segoe UI) ──
            let (font_coll, _font_data, font_src) = space_grotesk_collection(&dw);
            let mkf = |dw: &IDWriteFactory, size: f32, weight: DWRITE_FONT_WEIGHT, face: &str| -> Result<IDWriteTextFormat, String> {
                let f = wide(face);
                let loc = wide("en-US");
                // 1) collection custom (Space Grotesk en mémoire)
                if let Some(coll) = font_coll.as_ref() {
                    if let Ok(fmt) = dw.CreateTextFormat(
                        PCWSTR(f.as_ptr()),
                        coll,
                        weight,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        PCWSTR(loc.as_ptr()),
                    ) {
                        return Ok(fmt);
                    }
                }
                // 2) police système installée portant ce nom
                if let Ok(fmt) = dw.CreateTextFormat(
                    PCWSTR(f.as_ptr()),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    PCWSTR(loc.as_ptr()),
                ) {
                    return Ok(fmt);
                }
                // 3) repli final
                let seg = wide("Segoe UI");
                dw.CreateTextFormat(
                    PCWSTR(seg.as_ptr()),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    PCWSTR(loc.as_ptr()),
                )
                .map_err(|e| format!("CreateTextFormat({face}): {e}"))
            };
            let fmt_h = mkf(&dw, 16.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, "Space Grotesk")?;
            let fmt_b = mkf(&dw, 13.0, DWRITE_FONT_WEIGHT_NORMAL, "Space Grotesk")?;
            let fmt_s = mkf(&dw, 11.0, DWRITE_FONT_WEIGHT_NORMAL, "Space Grotesk")?;
            let fmt_name = mkf(&dw, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, "Space Grotesk")?;
            let fmt_mono = mkf(&dw, 12.0, DWRITE_FONT_WEIGHT_NORMAL, "Consolas")?;

            dlog!("[*] Direct2D prêt — {w}×{h}, alpha prémultiplié + ULW (verre réel) · police: {font_src}");

            Ok(Renderer {
                hwnd, rt, brush, grad, sheen, memdc, dib, dw,
                fmt_h, fmt_b, fmt_s, fmt_name, fmt_mono,
                font_src: font_src.clone(),
                size: (w, h),
            })
        }
    }

    // ═══ primitives de dessin (menu + ESP) — couleurs [r,g,b,a] ═══════════
    fn set_color(&self, c: [f32; 4]) {
        unsafe { self.brush.SetColor(&cf(c)); }
    }

    pub(crate) fn fill(&self, x: f32, y: f32, w: f32, h: f32, c: [f32; 4]) {
        self.set_color(c);
        unsafe { self.rt.FillRectangle(&rect(x, y, w, h), &self.brush); }
    }

    pub(crate) fn stroke(&self, x: f32, y: f32, w: f32, h: f32, c: [f32; 4], wd: f32) {
        self.set_color(c);
        unsafe { self.rt.DrawRectangle(&rect(x, y, w, h), &self.brush, wd, None); }
    }

    pub(crate) fn fill_round(&self, x: f32, y: f32, w: f32, h: f32, rad: f32, c: [f32; 4]) {
        self.set_color(c);
        let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: rad, radiusY: rad };
        unsafe { self.rt.FillRoundedRectangle(&rr, &self.brush); }
    }

    pub(crate) fn stroke_round(&self, x: f32, y: f32, w: f32, h: f32, rad: f32, c: [f32; 4], wd: f32) {
        self.set_color(c);
        let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: rad, radiusY: rad };
        unsafe { self.rt.DrawRoundedRectangle(&rr, &self.brush, wd, None); }
    }

    pub(crate) fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 4], w: f32) {
        self.set_color(c);
        unsafe {
            self.rt.DrawLine(
                D2D_POINT_2F { x: x0, y: y0 },
                D2D_POINT_2F { x: x1, y: y1 },
                &self.brush,
                w,
                None,
            );
        }
    }

    pub(crate) fn dot(&self, cx: f32, cy: f32, radius: f32, c: [f32; 4]) {
        self.set_color(c);
        let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillEllipse(&e, &self.brush); }
    }

    pub(crate) fn ring(&self, cx: f32, cy: f32, radius: f32, c: [f32; 4], w: f32) {
        self.set_color(c);
        let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: radius, radiusY: radius };
        unsafe { self.rt.DrawEllipse(&e, &self.brush, w, None); }
    }

    /// remplissage dégradé accent (horizontal), opacité globale animable.
    pub(crate) fn grad_fill_round(&self, x: f32, y: f32, w: f32, h: f32, rad: f32, opacity: f32) {
        unsafe {
            self.grad.SetOpacity(opacity);
            self.grad.SetStartPoint(D2D_POINT_2F { x, y: 0.0 });
            self.grad.SetEndPoint(D2D_POINT_2F { x: x + w.max(1.0), y: 0.0 });
            let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: rad, radiusY: rad };
            self.rt.FillRoundedRectangle(&rr, &self.grad);
        }
    }

    /// reflet supérieur du verre (fondu blanc vertical).
    pub(crate) fn sheen(&self, x: f32, y: f32, w: f32, h: f32, opacity: f32) {
        unsafe {
            self.sheen.SetOpacity(opacity);
            self.sheen.SetStartPoint(D2D_POINT_2F { x: 0.0, y });
            self.sheen.SetEndPoint(D2D_POINT_2F { x: 0.0, y: y + h });
            let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: 12.0, radiusY: 12.0 };
            self.rt.FillRoundedRectangle(&rr, &self.sheen);
        }
    }

    /// texte DirectWrite (couleur unie). vcenter + align gérés par le format.
    pub(crate) fn text(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        c: [f32; 4],
        align: Align,
        vcenter: bool,
    ) {
        if s.is_empty() {
            return;
        }
        unsafe {
            let _ = fmt.SetTextAlignment(match align {
                Align::L => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::C => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::R => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = fmt.SetParagraphAlignment(if vcenter {
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER
            } else {
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR
            });
            let w16: Vec<u16> = s.encode_utf16().collect();
            self.set_color(c);
            self.rt.DrawText(
                &w16,
                fmt,
                &rect(x, y, w, h),
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// texte dans le dégradé accent (logo du menu).
    pub(crate) fn text_grad(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        align: Align,
        vcenter: bool,
        opacity: f32,
    ) {
        if s.is_empty() {
            return;
        }
        unsafe {
            let _ = fmt.SetTextAlignment(match align {
                Align::L => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::C => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::R => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = fmt.SetParagraphAlignment(if vcenter {
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER
            } else {
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR
            });
            let w16: Vec<u16> = s.encode_utf16().collect();
            self.grad.SetOpacity(opacity);
            self.grad.SetStartPoint(D2D_POINT_2F { x, y: 0.0 });
            self.grad.SetEndPoint(D2D_POINT_2F { x: x + w.max(1.0), y: 0.0 });
            self.rt.DrawText(
                &w16,
                fmt,
                &rect(x, y, w, h),
                &self.grad,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// mesure précise (DWrite layout) — largeur/hauteur du texte en px.
    pub(crate) fn measure(&self, s: &str, fmt: &IDWriteTextFormat) -> (f32, f32) {
        if s.is_empty() {
            return (0.0, 0.0);
        }
        unsafe {
            let w16: Vec<u16> = s.encode_utf16().collect();
            let Ok(layout) = self.dw.CreateTextLayout(&w16, fmt, 4000.0, 100.0) else {
                return (60.0, 16.0);
            };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            (m.widthIncludingTrailingWhitespace, m.height)
        }
    }
    /// F8 — dump du bitmap EXACT que présente UpdateLayeredWindow (debug menu).
    pub fn dump_bitmap(&self) {
        unsafe {
            let mut bm = BITMAP::default();
            if GetObjectW(HGDIOBJ(self.dib.0), std::mem::size_of::<BITMAP>() as i32,
                Some((&mut bm as *mut BITMAP).cast())) == 0
            {
                dwarn!("dump F8: GetObjectW a échoué");
                return;
            }
            let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
            let pitch = if bm.bmWidthBytes > 0 { bm.bmWidthBytes as usize } else { 0 };
            let bits: *const u8 = bm.bmBits.cast();
            if h <= 0 || w <= 0 || pitch == 0 || bits.is_null() {
                dwarn!("dump F8:BITMAP incohérent ({w}×{h}, pitch {pitch}, bits nul?)");
                return;
            }
            let data = std::slice::from_raw_parts(bits, pitch * h as usize);
            let mut buf = Vec::with_capacity(54 + data.len());
            let img_size = (pitch * h as usize) as u32;
            let row = (w as usize) * 4;
            buf.extend_from_slice(b"BM");
            buf.extend_from_slice(&(54u32 + img_size).to_le_bytes());
            buf.extend_from_slice(&[0; 4]);
            buf.extend_from_slice(&54u32.to_le_bytes());
            buf.extend_from_slice(&40u32.to_le_bytes());
            buf.extend_from_slice(&(w as i32).to_le_bytes());
            buf.extend_from_slice(&(h as i32).to_le_bytes());
            buf.extend_from_slice(&1u16.to_le_bytes());
            buf.extend_from_slice(&32u16.to_le_bytes());
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&img_size.to_le_bytes());
            buf.extend_from_slice(&[0; 16]);
            for r in (0..h as usize).rev() {
                buf.extend_from_slice(&data[r * pitch..r * pitch + row]);
            }
            match std::fs::write("menu_frame.bmp", &buf) {
                Ok(_) => debug::emit("[*] F8 → menu_frame.bmp écrit".into()),
                Err(e) => dwarn!("dump F8: écriture: {e}"),
            }
        }
    }

    fn resize(&mut self, w: i32, h: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        unsafe {
            let windc = GetDC(self.hwnd);
            let nb = match create_dib(windc, w, h) {
                Ok((b, _)) => b,
                Err(e) => {
                    dwarn!("resize: {e}");
                    ReleaseDC(self.hwnd, windc);
                    return;
                }
            };
            ReleaseDC(self.hwnd, windc);
            let old = SelectObject(self.memdc, HGDIOBJ(nb.0));
            let _ = DeleteObject(old);
            self.dib = nb;
            self.size = (w, h);
            dlog!("[ dbg] resize render target → {w}×{h}");
        }
    }

    // ═══ frame: ESP + squelette + radar + watermark + menu → ULW ══════════
    // `suppress` = un menu du jeu est ouvert (achat/ESC/menus) → on n'affiche
    // NI boîtes NI squelette NI radar NI viseur maison — le jeu reste net.
    #[allow(clippy::too_many_arguments)]
    fn frame(
        &self,
        snap: Option<&Snapshot>,
        opts: &mut Options,
        menu: &mut menu::Menu,
        mouse: &mut Mouse,
        st: &StatusInfo,
        fps: f32,
        sw: i32,
        sh: i32,
        dt: f32,
        suppress: bool,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        unsafe {
            if self.rt.BindDC(self.memdc, &RECT { left: 0, top: 0, right: sw, bottom: sh }).is_err() {
                if crate::debug::once_per_sec() {
                    dwarn!("BindDC échoué ({sw}×{sh}) — avant: échec silencieux, frame sautée sans trace");
                }
                return actions;
            }
            self.rt.BeginDraw();
            // fond ENTIÈREMENT transparent (alpha 0 partout)
            self.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

            let (swf, shf) = (sw as f32, sh as f32);

            // ── viseur maison (coupé pendant les menus du jeu) ──
            if opts.crosshair && !suppress {
                let (cx, cy) = (swf * 0.5, shf * 0.5);
                let c = [1.0, 1.0, 1.0, 0.9];
                self.line(cx - 7.0, cy, cx - 3.0, cy, c, 1.5);
                self.line(cx + 3.0, cy, cx + 7.0, cy, c, 1.5);
                self.line(cx, cy - 7.0, cx, cy - 3.0, c, 1.5);
                self.line(cx, cy + 3.0, cx, cy + 7.0, c, 1.5);
            }

            // ── indicateur triggerbot: anneau autour du centre quand ARMÉ ──
            if opts.trigger && st.trigger_armed && !suppress {
                let (cx, cy) = (swf * 0.5, shf * 0.5);
                let a = menu::ACCENT_A;
                self.ring(cx, cy, 11.0, [a[0], a[1], a[2], 0.55], 1.6);
                self.ring(cx, cy, 15.0, [a[0], a[1], a[2], 0.22], 1.0);
            }

            // ── boîtes + squelette (asservi aux options du menu) ──
            if let Some(snap) = snap {
                if opts.esp && snap.matrix_ok && !suppress {
                    let pal = &PALETTES[opts.palette];
                    for p in &snap.players {
                        if p.team != 2 && p.team != 3 {
                            continue; // spectateur / non-assigné
                        }
                        if p.team == snap.local_team && !opts.teammates && p.pawn != snap.local_pawn {
                            continue; // coéquipiers filtrés (F2/menu)
                        }
                        let dist_m = p.origin.dist(&snap.local_origin) * 0.019;
                        if !opts.unlimited_dist && dist_m > opts.max_dist_m {
                            continue; // filtre distance (désactivable = illimité)
                        }
                        let feet = snap.matrix.w2s(p.origin, swf, shf);
                        let head = snap.matrix.w2s(p.origin.z_up(PLAYER_HEIGHT), swf, shf);
                        let (Some((fx, fy)), Some((hx, hy))) = (feet, head) else { continue };
                        let bh = fy - hy;
                        if !(24.0..=800.0).contains(&bh) {
                            continue;
                        }
                        let bw = (bh * 0.45).clamp(16.0, 240.0);
                        let bcx = (fx + hx) * 0.5;
                        let is_local = p.pawn == snap.local_pawn;
                        let border = if is_local { pal.me } else if p.team == 2 { pal.t } else { pal.ct };

                        // snapline: du bas d'écran vers le pied du joueur
                        if opts.snaplines && !is_local {
                            self.line(swf * 0.5, shf, bcx, fy - 1.0, [border[0], border[1], border[2], 0.5], 1.2);
                        }

                        // boîte: coins / complet / off + remplissage translucide
                        let (bx, by) = (bcx - bw * 0.5, hy);
                        if opts.box_fill && opts.box_mode != 2 {
                            self.fill(bx, by, bw, bh, [border[0] * 0.22, border[1] * 0.22, border[2] * 0.22, 0.30]);
                        }
                        match opts.box_mode {
                            0 => {
                                let l = (bh * 0.28).clamp(7.0, 16.0);
                                let ac = mix4(border, [1.0, 1.0, 1.0, 1.0], 0.35);
                                self.line(bx, by, bx + l, by, ac, 2.6);
                                self.line(bx, by, bx, by + l, ac, 2.6);
                                self.line(bx + bw - l, by, bx + bw, by, ac, 2.6);
                                self.line(bx + bw, by, bx + bw, by + l, ac, 2.6);
                                self.line(bx, by + bh - l, bx, by + bh, ac, 2.6);
                                self.line(bx, by + bh, bx + l, by + bh, ac, 2.6);
                                self.line(bx + bw - l, by + bh - l, bx + bw, by + bh, ac, 2.6);
                                self.line(bx + bw, by + bh - l, bx + bw, by + bh, ac, 2.6);
                            }
                            1 => self.stroke(bx, by, bw, bh, border, 1.75),
                            _ => {}
                        }

                        // ── SQUELETTE: 17 segments os par os (cache 0x20) ──
                        // couleur = teinte d'équipe éclaircie, tête en point.
                        if opts.skeleton && !is_local {
                            if let Some(bones) = &p.bones {
                                let sk = mix4(border, [1.0, 1.0, 1.0, 1.0], 0.28);
                                let skc = [sk[0], sk[1], sk[2], 0.95];
                                for (a, b) in BONE_CONNECTS {
                                    if let (Some(ba), Some(bb)) = (
                                        bones.get(a).and_then(|p| snap.matrix.w2s(*p, swf, shf)),
                                        bones.get(b).and_then(|p| snap.matrix.w2s(*p, swf, shf)),
                                    ) {
                                        self.line(ba.0, ba.1, bb.0, bb.1, skc, 1.7);
                                    }
                                }
                                if let Some(h) = bones.get(B_HEAD).and_then(|p| snap.matrix.w2s(*p, swf, shf)) {
                                    self.dot(h.0, h.1, 2.3, skc);
                                }
                            }
                        }

                        // barre de vie verticale à gauche — dégradé rouge→vert
                        if opts.health {
                            let hp = (p.health as f32 / 100.0).clamp(0.0, 1.0);
                            let bar_w = 4.0;
                            let bar_x = bx - bar_w - 5.0;
                            self.stroke(bar_x - 1.0, by - 1.0, bar_w + 2.0, bh + 2.0, [0.85, 0.85, 0.85, 0.6], 1.0);
                            let hc = mix4([1.0, 0.15, 0.15, 1.0], [0.15, 1.0, 0.35, 1.0], hp);
                            self.fill(bar_x, by + bh * (1.0 - hp), bar_w, bh * hp, hc);
                        }

                        // pastille nom (mesurée DWrite — centrage exact)
                        if opts.names {
                            let name = if is_local { format!("{} (toi)", p.name) } else { p.name.clone() };
                            let (nw, nh) = self.measure(&name, &self.fmt_name);
                            let pw = nw + 14.0;
                            let ph = nh + 7.0;
                            let px = bcx - pw * 0.5;
                            let py = by - ph - 5.0;
                            self.fill_round(px, py, pw, ph, 6.0, [0.04, 0.05, 0.07, 0.62]);
                            self.text(&name, &self.fmt_name, px, py, pw, ph, [0.94, 0.96, 0.98, 0.97], Align::C, true);
                        }

                        // pastille distance
                        if opts.distance {
                            let ds = if opts.unlimited_dist {
                                format!("{dist_m:.0} m")
                            } else {
                                format!("{dist_m:.0} m / {max:.0}", max = opts.max_dist_m)
                            };
                            let (dw, dh) = self.measure(&ds, &self.fmt_s);
                            let pw = dw + 12.0;
                            let ph = dh + 6.0;
                            let px = bcx - pw * 0.5;
                            let py = fy + 4.0;
                            self.fill_round(px, py, pw, ph, 6.0, [0.04, 0.05, 0.07, 0.55]);
                            self.text(&ds, &self.fmt_s, px, py, pw, ph, [0.78, 0.82, 0.87, 0.95], Align::C, true);
                        }
                    }
                }

                // ── RADAR / MINIMAP (rotation caméra via la matrice de vue) ──
                if opts.radar && snap.matrix_ok && snap.local_alive && !suppress {
                    self.draw_radar(snap, opts, sw, sh);
                }
            }

            // ── watermark (pill glass) ──
            if opts.watermark {
                let wm = format!(
                    "cs2-esp v0.3.0 │ {:>3.0} fps │ {} joueurs │ INSERT menu",
                    fps,
                    snap.map(|s| s.players.len()).unwrap_or(0)
                );
                let (tw, th) = self.measure(&wm, &self.fmt_mono);
                let pw = tw + 20.0;
                let ph = th + 10.0;
                self.fill_round(10.0, 10.0, pw, ph, ph * 0.5, [0.05, 0.06, 0.085, 0.55]);
                self.stroke_round(10.0, 10.0, pw, ph, ph * 0.5, [1.0, 1.0, 1.0, 0.12], 1.0);
                self.text(&wm, &self.fmt_mono, 10.0, 10.0, pw, ph, [0.72, 0.86, 0.83, 0.92], Align::C, true);
            }

            // ── menu glass (dessin + interaction immédiate) ──
            actions = menu.frame(self, mouse, opts, st, dt);
            if let Err(e) = self.rt.EndDraw(None, None) {
                if crate::debug::once_per_sec() {
                    dwarn!("EndDraw: {e} — cible D2D perdue ?");
                }
                return actions;
            }
        }

        // présentation atomique — le bitmap prémultiplié devient la fenêtre
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            // pptDst/psize EXPLICITES OBLIGATOIRES (sinon DWM n'applique jamais)
            if let Err(e) = UpdateLayeredWindow(
                self.hwnd,
                HDC::default(),
                Some(&POINT { x: 0, y: 0 }),
                Some(&SIZE { cx: self.size.0, cy: self.size.1 }),
                self.memdc,
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            ) {
                if crate::debug::once_per_sec() {
                    dwarn!("UpdateLayeredWindow: {e} — frame non présentée");
                }
            }
        }
        actions
    }

    /// RADAR: panneau verre, points par équipe, flèche locale au centre,
    /// rotation = ta vue (base prise sur la ligne 0 de la matrice caméra:
    /// (m0,m1) = droite 2D, (-m1,m0) = devant 2D — même maths que le w2s).
    fn draw_radar(&self, snap: &Snapshot, opts: &Options, sw: i32, sh: i32) {
        let pal = &PALETTES[opts.palette];
        let size = opts.radar_size.clamp(120.0, 400.0);
        let margin = 12.0f32;
        let (px, py) = match opts.radar_corner {
            0 => (margin, 48.0),                                  // haut-gauche (sous le watermark)
            1 => (sw as f32 - size - margin, margin),             // haut-droite
            2 => (margin, sh as f32 - size - margin),             // bas-gauche
            _ => (sw as f32 - size - margin, sh as f32 - size - margin), // bas-droite
        };
        let (cx, cy, half) = (px + size * 0.5, py + size * 0.5, size * 0.5);

        // fond verre + bordure
        self.fill_round(px, py, size, size, 12.0, [0.03, 0.04, 0.06, 0.55]);
        self.stroke_round(px, py, size, size, 12.0, [1.0, 1.0, 1.0, 0.10], 1.0);
        // anneaux de distance + croix
        self.ring(cx, cy, half * 0.33, [1.0, 1.0, 1.0, 0.05], 1.0);
        self.ring(cx, cy, half * 0.66, [1.0, 1.0, 1.0, 0.05], 1.0);
        self.line(px + 6.0, cy, px + size - 6.0, cy, [1.0, 1.0, 1.0, 0.05], 1.0);
        self.line(cx, py + 6.0, cx, py + size - 6.0, [1.0, 1.0, 1.0, 0.05], 1.0);

        // base de rotation (caméra): devant = HAUT du radar
        let (m0, m1) = (snap.matrix.m[0], snap.matrix.m[1]);
        let len = (m0 * m0 + m1 * m1).sqrt();
        let (rx, ry) = if len > 1e-4 { (m0 / len, m1 / len) } else { (1.0, 0.0) };
        let (fx, fy) = (-ry, rx);
        let range = opts.radar_range.max(100.0);
        let reach = half - 9.0;

        for p in &snap.players {
            if (p.team != 2 && p.team != 3) || p.pawn == snap.local_pawn {
                continue;
            }
            let mate = p.team == snap.local_team;
            if mate && !opts.radar_mates {
                continue;
            }
            let (dx, dy) = (p.origin.x - snap.local_origin.x, p.origin.y - snap.local_origin.y);
            let mut nx = dx / range;
            let mut ny = dy / range;
            let dnorm = (nx * nx + ny * ny).sqrt();
            let clamped = dnorm > 1.0;
            if clamped {
                let s = 1.0 / dnorm.max(1e-6);
                nx *= s;
                ny *= s;
            }
            let rad_x = nx * rx + ny * ry;
            let rad_y = nx * fx + ny * fy;
            let sx = cx + rad_x * reach;
            let sy = cy - rad_y * reach;
            let col: [f32; 4] = if p.team == 2 { pal.t } else { pal.ct };
            if clamped {
                // hors portée: pastille creuse au bord — direction conservée
                self.ring(sx, sy, 3.2, [col[0], col[1], col[2], 0.9], 1.5);
            } else {
                self.dot(sx, sy, 4.0, col);
                self.ring(sx, sy, 4.0, [0.0, 0.0, 0.0, 0.45], 1.0);
            }
        }

        // TOI: flèche au centre + cône de vision
        let lc = pal.me;
        let la: [f32; 4] = [lc[0], lc[1], lc[2], 0.30];
        self.line(cx, cy, cx - half * 0.45, cy - half * 0.78, la, 1.0);
        self.line(cx, cy, cx + half * 0.45, cy - half * 0.78, la, 1.0);
        let lf: [f32; 4] = [lc[0], lc[1], lc[2], 1.0];
        self.line(cx, cy - 7.5, cx - 5.5, cy + 6.0, lf, 2.0);
        self.line(cx, cy - 7.5, cx + 5.5, cy + 6.0, lf, 2.0);
        self.line(cx - 5.5, cy + 6.0, cx + 5.5, cy + 6.0, lf, 1.4);

        // étiquettes
        self.text("RADAR", &self.fmt_s, px + 10.0, py + 6.0, 70.0, 14.0, [0.60, 0.65, 0.72, 0.8], Align::L, false);
        let rs = format!("{:.0} u", range);
        self.text(&rs, &self.fmt_s, px + size - 80.0, py + 6.0, 70.0, 14.0, [0.60, 0.65, 0.72, 0.8], Align::R, false);
    }
}

fn dump_now(mem: &Mem, offs: &Offsets) {
    match entity::snapshot_full(mem, offs, true, true) {
        Some(s) => entity::dump_verbose(&s, mem, offs),
        None => debug::emit("[!] dump: pas de snapshot (pas en partie / offsets stale)".into()),
    }
}

// ESP_RED_TEST=1: un unique rect rouge OPAQUE plein écran — debug compositing.
fn r_red_test(renderer: &Option<Renderer>, w: i32, h: i32) -> Option<Vec<Action>> {
    let r = renderer.as_ref()?;
    unsafe {
        if r.rt.BindDC(r.memdc, &RECT { left: 0, top: 0, right: w, bottom: h }).is_err() {
            debug::emit("[RED] BindDC: ÉCHEC".into());
            return None;
        }
        r.rt.BeginDraw();
        r.rt.Clear(Some(&D2D1_COLOR_F { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }));
        if r.rt.EndDraw(None, None).is_err() {
            debug::emit("[RED] EndDraw: ÉCHEC".into());
            return None;
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let res = UpdateLayeredWindow(
            r.hwnd,
            HDC::default(),
            Some(&POINT { x: 0, y: 0 }),
            Some(&SIZE { cx: w, cy: h }),
            r.memdc,
            Some(&POINT { x: 0, y: 0 }),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        if res.is_err() {
            debug::emit(format!("[RED] ULW explicite: ÉCHEC {res:?}"));
        } else if crate::debug::once_per_sec() {
            dlog!("[RED] ULW explicite: OK");
        }
    }
    Some(Vec::new())
}

pub fn run(mem: &Mem, offs: &Offsets, cfg: Config) -> ExitCode {
    unsafe { let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }
    let red_test = std::env::var("ESP_RED_TEST").is_ok();

    // config: ini d'abord, CLI en override (le menu est la source de vérité
    // ensuite — tout est live-éditable et re-persistée)
    let (mut opts, saved_pos) = menu::load_options();
    if cfg.teammates {
        opts.teammates = true;
    }
    if cfg.verbose {
        opts.verbose = true;
    }
    if let Some(iv) = cfg.interval_ms {
        opts.interval_ms = iv;
    }
    debug::set_verbose(opts.verbose);
    let config_disp = menu::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "n/a (exe introuvable)".into());

    let hwnd = match create_overlay_window() {
        Ok(h) => h,
        Err(e) => return ExitCode::Fatal(e),
    };
    let _win_guard = WinGuard(hwnd);
    let mut renderer = match Renderer::try_new(hwnd) {
        Ok(r) => Some(r),
        Err(e) => {
            dwarn!("rendu overlay: {e} — ESP inactif, diagnostic continue");
            None
        }
    };

    // panneau diagnostic: au boot si --panel OU si le rendu a échoué
    let mut panel = None;
    if cfg.panel || renderer.is_none() {
        match gui::Panel::new() {
            Ok(p) => {
                dlog!("[*] panneau GUI créé — statut, commandes, journal en miroir");
                panel = Some(p);
            }
            Err(e) => dwarn!("panneau GUI: {e} — poursuite en console seule"),
        }
    }

    let mut bounds = game_window_bounds();
    let mut menu = menu::Menu::new(saved_pos, bounds.map(|b| b.2).unwrap_or(1920), bounds.map(|b| b.3).unwrap_or(1080));
    let mut mouse = Mouse::default();
    let mut fail_streak: u32 = 0;
    let mut last = Instant::now();
    let mut last_draw = Instant::now();
    let mut fps = 0.0f32;
    let mut frame_i: u64 = 0;
    let (mut k_end, mut k_f1, mut k_f2, mut k_f4, mut k_f8, mut k_ins) = (false, false, false, false, false, false);
    let mut last_menu_open = false; // resync du click-through (INSERT OU croix)

    // ── v0.3: visibilité jeu-seulement + triggerbot ──
    let mut overlay_visible = true;
    let mut suppress_logged = false;
    let mut trigger = TriggerBot::new();

    debug::emit("[*] overlay actif — INSERT: menu glass · END: quitter · F1: debug · F2: coéquipiers · F4: dump".into());
    debug::emit("[*] ESP limité au jeu: caché si CS2 n'est plus au premier plan (Discord & co), coupé dans les menus/achat".into());
    debug::emit("[*] triggerbot: maintiens la touche (ALT par défaut) — tir auto sur ennemi sous le viseur".into());
    debug::emit("[*] rien ne s'affiche ? → jeu en plein écran SANS BORDURE + overlay visible (2e écran / alt-tab)".into());
    debug::emit(format!("[*] config: {config_disp} — réglages persistés automatiquement"));
    debug::emit(String::new());

    loop {
        // pompe à messages
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    menu.save_now(&opts);
                    return ExitCode::UserQuit;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // options → debug atomique (source de vérité unique: le menu)
        debug::set_verbose(opts.verbose);

        // touches globales (front descendant — plus de re-trigger si tenu)
        if pressed(VK_END, &mut k_end) {
            debug::emit("[*] END → arrêt".into());
            menu.save_now(&opts);
            return ExitCode::UserQuit;
        }
        if pressed(VK_F1, &mut k_f1) {
            opts.verbose = !opts.verbose;
            debug::emit(format!("[ dbg] mode verbeux = {}", opts.verbose));
        }
        if pressed(VK_F2, &mut k_f2) {
            opts.teammates = !opts.teammates;
            debug::emit(format!("[ dbg] coéquipiers = {}", opts.teammates));
        }
        if pressed(VK_F4, &mut k_f4) {
            dump_now(mem, offs);
        }
        if pressed(VK_F8, &mut k_f8) {
            if let Some(r) = renderer.as_ref() {
                r.dump_bitmap();
            } else {
                dwarn!("F8: renderer absent — rien à dumper");
            }
        }
        if pressed(VK_INSERT, &mut k_ins) {
            menu.toggle();
            debug::emit(format!("[ dbg] menu = {}", menu.open));
        }

        // la croix du menu ferme SANS INSERT → resynchronise le click-through
        if menu.open != last_menu_open {
            last_menu_open = menu.open;
            set_click_through(hwnd, !menu.open);
        }

        // commandes du panneau GUI (cases + boutons)
        for ev in gui::drain_events() {
            match ev {
                gui::PanelEvent::ToggleVerbose => {
                    opts.verbose = !opts.verbose;
                    debug::emit(format!("[ dbg] mode verbeux = {}", opts.verbose));
                }
                gui::PanelEvent::ToggleTeammates => {
                    opts.teammates = !opts.teammates;
                    debug::emit(format!("[ dbg] coéquipiers = {}", opts.teammates));
                }
                gui::PanelEvent::Dump => dump_now(mem, offs),
                gui::PanelEvent::Quit => {
                    debug::emit("[*] panneau → arrêt".into());
                    menu.save_now(&opts);
                    return ExitCode::UserQuit;
                }
            }
        }

        // suit la fenêtre du jeu (fenêtré, déplacement, redimension)
        if let Some((x, y, w, h)) = game_window_bounds() {
            if bounds != Some((x, y, w, h)) {
                unsafe { let _ = MoveWindow(hwnd, x, y, w, h, false); }
                bounds = Some((x, y, w, h));
                menu.set_bounds(w, h);
                dlog!("[ dbg] overlay repositionné ({x},{y}) {w}×{h}");
            }
        }

        // liveness: lecture de l'en-tête PE de client.dll ("MZ")
        let alive = mem.read::<u16>(mem.client.base) == Some(0x5A4D);
        if alive {
            fail_streak = 0;
        } else {
            fail_streak += 1;
            if fail_streak > 50 {
                menu.save_now(&opts);
                return ExitCode::GameClosed;
            }
        }

        // ── CS2 au premier plan ? sinon: overlay CACHÉ + lecture/trigger en pause ──
        let game_fg = if opts.game_only { game_foreground() } else { true };
        if game_fg != overlay_visible {
            unsafe { let _ = ShowWindow(hwnd, if game_fg { SW_SHOW } else { SW_HIDE }); }
            overlay_visible = game_fg;
            if game_fg {
                dlog!("[ ov ] CS2 reprend le premier plan → overlay visible");
            } else {
                dlog!("[ ov ] CS2 perd le premier plan (Discord ? bureau ?) → overlay caché");
            }
        }

        // taille client courante → resize du DIB si besoin
        let mut rc: RECT = unsafe { std::mem::zeroed() };
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        let (w, h) = (rc.right.max(2), rc.bottom.max(2));
        if overlay_visible {
            if let Some(r) = renderer.as_mut() {
                if r.size != (w, h) {
                    r.resize(w, h);
                }
            }
        }

        // snapshot des données — uniquement quand le jeu est au premier plan
        // (économie CPU + pas de fantômes au-dessus de Discord) ; les bones
        // ne sont lues que si le squelette est activé.
        let snap = if game_fg {
            entity::snapshot_full(mem, offs, debug::verbose() && frame_i % 150 == 0, opts.skeleton)
        } else {
            None
        };

        // ── menus du jeu ouverts ? curseur visible OU menu d'achat (mémoire)
        // → ESP/squelette/radar/trigger COUPÉS (notre menu glass reste actif).
        let buy_open = snap.as_ref().map(|s| s.buy_menu).unwrap_or(false);
        let cursor_shown = cursor_visible() && !menu.open;
        let suppress = (buy_open || cursor_shown) && !menu.open;
        if suppress && !suppress_logged {
            dlog!("[ ov ] menu du jeu ouvert (achat/ESC/menus) → ESP coupé");
            suppress_logged = true;
        } else if !suppress && suppress_logged {
            dlog!("[ ov ] retour en jeu → ESP réactivé");
            suppress_logged = false;
        }

        // ── TRIGGERBOT: ennemi sous le viseur + touche + contexte de jeu sain ──
        let trig_conditions = game_fg
            && !suppress
            && !menu.open
            && opts.trigger
            && snap.as_ref().map(|s| s.local_alive && s.crosshair_enemy).unwrap_or(false)
            && trigger_key_down(opts.trigger_key);
        trigger.update(trig_conditions, opts.trigger_delay_ms.max(0.0) as u32);

        // panneau: journal en miroir + statut resynchronisé (2×/s)
        if let Some(p) = panel.as_ref() {
            p.push_log(&debug::drain_log());
            if frame_i % 30 == 0 {
                p.sync_checks(opts.verbose, opts.teammates);
                let verdict = match snap.as_ref() {
                    None if !game_fg => "EN PAUSE — CS2 pas au premier plan".to_string(),
                    None => "AUCUNE — pas en partie OU offsets STALE → re-dump".to_string(),
                    Some(s) if !s.matrix_ok => format!("partielle — {} joueurs, MATRICE invalide", s.players.len()),
                    Some(s) => format!("OK — {} joueurs tracés", s.players.len()),
                };
                let rendu = if renderer.is_some() { "OK — Direct2D + ULW" } else { "ÉCHEC (voir journal)" };
                p.set_status(&format!(
                    "cs2-esp v0.3.0 (menu glass: INSERT)\r\npid {} │ {:.0} fps │ snapshot: {}\r\nrendu overlay: {} │ coéquipiers: {}\r\noffsets: dump local 29.10.2026 — build 14193 (cs2-dumper)",
                    mem.pid, fps, verdict, rendu, if opts.teammates { "oui" } else { "non" }
                ));
            }
        }

        // souris: pollée à chaque boucle; clics ignorés si menu fermé
        mouse.update(hwnd);
        if menu.hidden() {
            mouse.clicked = false;
        }

        // rendu throttlé (~166 fps max: le polling mémoire reste à la cadence
        // configurée, l'upload ULW 1080p est le seul coût plafonné)
        let now = Instant::now();
        if now.duration_since(last_draw).as_millis() >= 6 && overlay_visible {
            let dt = now.duration_since(last_draw).as_secs_f32().min(0.1);
            last_draw = now;
            let st = StatusInfo {
                pid: mem.pid,
                fps,
                players: snap.as_ref().map(|s| s.players.len()).unwrap_or(0),
                snapshot_ok: snap.is_some(),
                renderer_ok: renderer.is_some(),
                config_path: config_disp.clone(),
                trigger_armed: trigger.armed_visual,
                game_fg,
                font: renderer.as_ref().map(|r| r.font_src.clone()).unwrap_or_else(|| "—".into()),
            };
            // une panique de frame ne tue pas l'overlay
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if red_test {
                    r_red_test(&renderer, w, h)
                } else {
                    renderer.as_ref().map(|r| r.frame(snap.as_ref(), &mut opts, &mut menu, &mut mouse, &st, fps, w, h, dt, suppress))
                }
            }));
            let actions = match result {
                Ok(Some(a)) => a,
                Ok(None) => Vec::new(),
                Err(_) => {
                    dwarn!("panique pendant le rendu — frame sautée");
                    Vec::new()
                }
            };
            for act in actions {
                match act {
                    Action::Dump => dump_now(mem, offs),
                    Action::ShowPanel => {
                        if panel.is_none() {
                            match gui::Panel::new() {
                                Ok(p) => {
                                    panel = Some(p);
                                    dlog!("[*] panneau diagnostic ouvert (onglet DIVERS)");
                                }
                                Err(e) => dwarn!("panneau GUI: {e}"),
                            }
                        }
                    }
                    Action::Quit => {
                        debug::emit("[*] menu → arrêt".into());
                        menu.save_now(&opts);
                        return ExitCode::UserQuit;
                    }
                }
            }
        }

        // stats fps (EMA) + sauvegarde différée de la config
        let dt = now.duration_since(last).as_secs_f32();
        last = now;
        if dt > 0.0 {
            fps = fps * 0.9 + (1.0 / dt) * 0.1;
        }
        menu.maybe_save(&opts);

        // topmost périodique — reste au-dessus quoi qu'il arrive (si visible)
        frame_i = debug::next_frame();
        if frame_i % 120 == 0 && overlay_visible {
            unsafe {
                let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }

        unsafe { Sleep(opts.interval_ms.clamp(1, 50) as u32); }
    }
}
