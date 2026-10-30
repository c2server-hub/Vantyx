// Vantyx.cc LAUNCHER v2.0 — Rust · windows-rs 0.58 · Windows 11
//
// Fenêtre layerée (UpdateLayeredWindow + D2D prémultiplié + DirectWrite),
// même technique éprouvée que l'overlay du cheat. Carte de verre flottante:
//   · liseré accent supérieur dégradé (hairline — signature v2)
//   · emblème VISEUR: anneau + arcs rotatifs + 12 ticks radiaux en rotation
//   · chip version "what's new" + 3 chips des nouveautés v0.9
//   · LED de statut par vérification (CS2 / moteur / offsets / polices)
//   · bouton LANCER double hauteur (label + sous-ligne) + % de progression
//   · DRAG PARTOUT: toute la carte déplace la fenêtre (hors bouton/croix)
//   · bouton LANCER: dégradé, glow pulsé, hover halo, flash au clic,
//     puis barre de progression balayée (~0.9 s) → CreateProcessW sur
//     vantyx.exe → fondu de sortie → fermeture propre
//   · Space Grotesk chargée EN MÉMOIRE (fonts/ à côté de l'exe, comme le
//     cheat) — repli Segoe UI
//
// touches: ESC ferme · glisser la carte PARTOUT (sauf bouton/croix)
//
// build (dans le workspace): cargo build --release -p vantyx-launcher
// language: Rust, file: launcher/src/main.rs, target: Windows 11 x64
#![windows_subsystem = "windows"] // pas de console — launcher pur

use std::time::Instant;

use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::{BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
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
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    IDWriteFactory, IDWriteTextFormat, IDWriteFactory5, IDWriteFontCollection1,
    IDWriteInMemoryFontFileLoader,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, GetDC, ReleaseDC, SelectObject,
    HDC, HGDIOBJ, ScreenToClient, // ScreenToClient vit dans Gdi, PAS WAM (piège 0.58)
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    CreateProcessW, CREATE_NEW_CONSOLE, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::System::Threading::Sleep;
use windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext;
use windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY, VK_ESCAPE, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, CS_HREDRAW, CS_VREDRAW, DefWindowProcW, DispatchMessageW, GetCursorPos,
    GetSystemMetrics, PeekMessageW, PM_REMOVE, RegisterClassExW,
    TranslateMessage, UpdateLayeredWindow, WNDCLASSEXW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_QUIT, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE, SM_CXSCREEN, SM_CYSCREEN,
    ULW_ALPHA,
};

// ── géométrie de la carte ───────────────────────────────────────────────────
const W: f32 = 520.0;
const H: f32 = 640.0;

// ── identité visuelle (Void Purple — cohérence avec le cheat) ──────────────
const BG: [f32; 4] = [0.0314, 0.0314, 0.0510, 0.97];
const SURFACE: [f32; 4] = [0.0667, 0.0667, 0.1059, 1.0];
const PRIMARY: [f32; 4] = [0.5451, 0.3608, 0.9647, 1.0];
const LIGHT: [f32; 4] = [0.7255, 0.6275, 1.0000, 1.0];
const TEXT: [f32; 4] = [0.9608, 0.9529, 1.0000, 1.0];
const DIM: [f32; 4] = [0.55, 0.53, 0.64, 1.0];
const OK_GREEN: [f32; 4] = [0.30, 0.90, 0.50, 1.0];
const WARN_AMBER: [f32; 4] = [1.00, 0.70, 0.25, 1.0];
const BAD_RED: [f32; 4] = [1.00, 0.36, 0.36, 1.0];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
fn cf(c: [f32; 4]) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c[0], g: c[1], b: c[2], a: c[3] }
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}
fn with_a(c: [f32; 4], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], c[3] * a]
}

// ── état du launcher ────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    FadeIn,
    Ready,
    Launching(Instant),
    FadeOut,
}

struct Status {
    cs2_running: bool,
    engine_present: bool,
    offsets_present: bool,
    fonts_present: bool,
}

impl Status {
    fn ready_enough(&self) -> bool {
        self.engine_present
    }
}

/// processus lancé ? (ToolHelp, zéro dépendance)
fn process_running(name: &str) -> bool {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        if Process32FirstW(snap, &mut pe).is_ok() {
            loop {
                let end = pe.szExeFile.iter().position(|&c| c == 0).unwrap_or(pe.szExeFile.len());
                let pn = String::from_utf16_lossy(&pe.szExeFile[..end]);
                if pn.eq_ignore_ascii_case(name) {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
        found
    }
}

/// vérifie l'état du déploiement (2 Hz — appelé par la boucle)
fn check_status() -> Status {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let Some(dir) = dir else {
        return Status { cs2_running: false, engine_present: false, offsets_present: false, fonts_present: false };
    };
    Status {
        cs2_running: process_running("cs2.exe"),
        engine_present: dir.join("vantyx.exe").exists(),
        offsets_present: dir.join("offsets.json").exists(),
        fonts_present: dir.join("fonts").join("SpaceGrotesk-Regular.ttf").exists(),
    }
}

/// lance vantyx.exe (CREATE_NEW_CONSOLE → la console debug du cheat vit sa vie)
fn spawn_engine() -> bool {
    let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) else {
        return false;
    };
    let exe = dir.join("vantyx.exe");
    if !exe.exists() {
        return false;
    }
    unsafe {
        let mut cmd = wide(&format!("\"{}\"", exe.display()));
        let cwd = wide(dir.display().to_string().trim_end_matches('\\'));
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        CreateProcessW(
            PCWSTR::null(),
            PWSTR(cmd.as_mut_ptr()),
            None,
            None,
            BOOL::default(), // v0.6.1 — BOOL vit dans Win32::Foundation en 0.58 (pas windows::core)
            CREATE_NEW_CONSOLE,
            None,
            PCWSTR(cwd.as_ptr()),
            &si,
            &mut pi,
        )
        .is_ok()
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

// ── Space Grotesk en mémoire (même plomberie que le cheat) ─────────────────
static FONT_CACHE: std::sync::OnceLock<(Option<IDWriteFontCollection1>, Vec<Vec<u8>>)> =
    std::sync::OnceLock::new();
const FONT_FILES: [&str; 3] = [
    "SpaceGrotesk-Regular.ttf",
    "SpaceGrotesk-SemiBold.ttf",
    "SpaceGrotesk-Bold.ttf",
];

unsafe fn grotesk_collection(dw: &IDWriteFactory) -> &'static (Option<IDWriteFontCollection1>, Vec<Vec<u8>>) {
    FONT_CACHE.get_or_init(|| {
        let fail = || (None, Vec::new());
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
        let Some(dir) = dir else { return fail() };
        let Ok(f5) = dw.cast::<IDWriteFactory5>() else { return fail() };
        let Ok(loader): Result<IDWriteInMemoryFontFileLoader, _> = f5.CreateInMemoryFontFileLoader() else {
            return fail();
        };
        if dw.RegisterFontFileLoader(&loader).is_err() {
            return fail();
        }
        let Ok(builder) = f5.CreateFontSetBuilder() else { return fail() };
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
                data_vec.push(data); // les octets restent vivants
                added += 1;
            }
        }
        if added == 0 {
            return fail();
        }
        let Ok(set) = builder.CreateFontSet() else { return fail() };
        let Ok(coll) = f5.CreateFontCollectionFromFontSet(&set) else { return fail() };
        (Some(coll), data_vec)
    })
}

// ── render target (DIB + D2D + DWrite + brushes) ────────────────────────────
struct Renderer {
    hwnd: HWND,
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    grad: ID2D1LinearGradientBrush, // primary → light
    memdc: HDC,
    size: (i32, i32),
    fmt_logo: IDWriteTextFormat,   // 34 semibold
    fmt_title: IDWriteTextFormat,  // 15 semibold
    fmt_b: IDWriteTextFormat,      // 13 normal
    fmt_s: IDWriteTextFormat,      // 11 normal
    #[allow(dead_code)] // réservé: pied de carte technique / version
    fmt_mono: IDWriteTextFormat,   // 12 Consolas
}

#[allow(clippy::new_ret_no_self)]
fn renderer_new(hwnd: HWND, w: i32, h: i32) -> Result<Renderer, String> {
    unsafe {
        let windc = GetDC(hwnd);
        if windc.0.is_null() {
            return Err("GetDC → NULL".into());
        }
        let memdc = CreateCompatibleDC(windc);
        if memdc.0.is_null() {
            return Err("CreateCompatibleDC → NULL".into());
        }
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(
            windc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            windows::Win32::Foundation::HANDLE::default(),
            0,
        )
        .map_err(|e| format!("CreateDIBSection: {e}"))?;
        SelectObject(memdc, HGDIOBJ(dib.0));
        ReleaseDC(hwnd, windc);

        let factory: ID2D1Factory =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                .map_err(|e| format!("D2D1CreateFactory: {e}"))?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt: ID2D1DCRenderTarget = factory
            .CreateDCRenderTarget(&props)
            .map_err(|e| format!("CreateDCRenderTarget: {e}"))?;
        let brush: ID2D1SolidColorBrush = rt
            .CreateSolidColorBrush(&cf([1.0, 1.0, 1.0, 1.0]), None)
            .map_err(|e| format!("CreateSolidColorBrush: {e}"))?;
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.0, color: cf(PRIMARY) },
            D2D1_GRADIENT_STOP { position: 1.0, color: cf(LIGHT) },
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

        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)
            .map_err(|e| format!("DWriteCreateFactory: {e}"))?;
        let (font_coll, _data) = grotesk_collection(&dw);
        let mkf = |size: f32, weight: DWRITE_FONT_WEIGHT, face: &str| -> Result<IDWriteTextFormat, String> {
            let f = wide(face);
            let loc = wide("en-US");
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
        Ok(Renderer {
            hwnd,
            rt,
            brush,
            grad,
            memdc,
            size: (w, h),
            fmt_logo: mkf(34.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, "Space Grotesk")?,
            fmt_title: mkf(15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, "Space Grotesk")?,
            fmt_b: mkf(13.0, DWRITE_FONT_WEIGHT_NORMAL, "Space Grotesk")?,
            fmt_s: mkf(11.0, DWRITE_FONT_WEIGHT_NORMAL, "Space Grotesk")?,
            fmt_mono: mkf(12.0, DWRITE_FONT_WEIGHT_NORMAL, "Consolas")?,
        })
    }
}

impl Renderer {
    fn set_color(&self, c: [f32; 4]) {
        unsafe { self.brush.SetColor(&cf(c)); }
    }
    #[allow(dead_code)] // primitive complète gardée pour les états futurs
    fn fill(&self, x: f32, y: f32, w: f32, h: f32, c: [f32; 4]) {
        self.set_color(c);
        unsafe { self.rt.FillRectangle(&rect(x, y, w, h), &self.brush); }
    }
    fn fill_round(&self, x: f32, y: f32, w: f32, h: f32, rad: f32, c: [f32; 4]) {
        self.set_color(c);
        let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: rad, radiusY: rad };
        unsafe { self.rt.FillRoundedRectangle(&rr, &self.brush); }
    }
    fn stroke_round(&self, x: f32, y: f32, w: f32, h: f32, rad: f32, c: [f32; 4], wd: f32) {
        self.set_color(c);
        let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: rad, radiusY: rad };
        unsafe { self.rt.DrawRoundedRectangle(&rr, &self.brush, wd, None); }
    }
    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 4], w: f32) {
        self.set_color(c);
        unsafe {
            self.rt.DrawLine(D2D_POINT_2F { x: x0, y: y0 }, D2D_POINT_2F { x: x1, y: y1 }, &self.brush, w, None);
        }
    }
    fn dot(&self, cx: f32, cy: f32, radius: f32, c: [f32; 4]) {
        self.set_color(c);
        let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillEllipse(&e, &self.brush); }
    }
    fn ring(&self, cx: f32, cy: f32, radius: f32, c: [f32; 4], w: f32) {
        self.set_color(c);
        let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: radius, radiusY: radius };
        unsafe { self.rt.DrawEllipse(&e, &self.brush, w, None); }
    }
    /// arc rotatif (segment d'anneau — l'emblème "spin" décoratif).
    /// phase 0..1 → départ d'angle, sweep en radians.
    fn arc(&self, cx: f32, cy: f32, radius: f32, phase: f32, sweep: f32, c: [f32; 4], w: f32) {
        const STEPS: usize = 18;
        let a0 = phase * std::f32::consts::TAU;
        for i in 0..STEPS {
            let t0 = a0 + sweep * (i as f32 / STEPS as f32);
            let t1 = a0 + sweep * ((i + 1) as f32 / STEPS as f32);
            let (x0, y0) = (cx + radius * t0.cos(), cy + radius * t0.sin());
            let (x1, y1) = (cx + radius * t1.cos(), cy + radius * t1.sin());
            self.line(x0, y0, x1, y1, c, w);
        }
    }
    /// remplissage dégradé horizontal (bouton LANCER / barres).
    fn grad_fill_round(&self, x: f32, y: f32, w: f32, h: f32, rad: f32, opacity: f32) {
        unsafe {
            self.grad.SetOpacity(opacity);
            self.grad.SetStartPoint(D2D_POINT_2F { x, y: 0.0 });
            self.grad.SetEndPoint(D2D_POINT_2F { x: x + w.max(1.0), y: 0.0 });
            let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, w, h), radiusX: rad, radiusY: rad };
            self.rt.FillRoundedRectangle(&rr, &self.grad);
        }
    }
    fn grad_text(&self, s: &str, fmt: &IDWriteTextFormat, x: f32, y: f32, w: f32, h: f32, align: u8, opacity: f32) {
        if s.is_empty() {
            return;
        }
        unsafe {
            let _ = fmt.SetTextAlignment(match align {
                1 => DWRITE_TEXT_ALIGNMENT_CENTER,
                2 => DWRITE_TEXT_ALIGNMENT_TRAILING,
                _ => DWRITE_TEXT_ALIGNMENT_LEADING,
            });
            let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let w16: Vec<u16> = s.encode_utf16().collect();
            self.grad.SetOpacity(opacity);
            self.grad.SetStartPoint(D2D_POINT_2F { x, y: 0.0 });
            self.grad.SetEndPoint(D2D_POINT_2F { x: x + w.max(1.0), y: 0.0 });
            self.rt.DrawText(&w16, fmt, &rect(x, y, w, h), &self.grad, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
    }
    fn text(&self, s: &str, fmt: &IDWriteTextFormat, x: f32, y: f32, w: f32, h: f32, c: [f32; 4], align: u8) {
        if s.is_empty() {
            return;
        }
        unsafe {
            let _ = fmt.SetTextAlignment(match align {
                1 => DWRITE_TEXT_ALIGNMENT_CENTER,
                2 => DWRITE_TEXT_ALIGNMENT_TRAILING,
                _ => DWRITE_TEXT_ALIGNMENT_LEADING,
            });
            let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let w16: Vec<u16> = s.encode_utf16().collect();
            self.set_color(c);
            self.rt.DrawText(&w16, fmt, &rect(x, y, w, h), &self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
    }
    fn present(&self) {
        unsafe {
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                HDC::default(),
                Some(&POINT { x: 0, y: 0 }),
                Some(&SIZE { cx: self.size.0, cy: self.size.1 }),
                self.memdc,
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }
}

// ── la frame animée ─────────────────────────────────────────────────────────
struct Mouse {
    x: f32,
    y: f32,
    held: bool,
    clicked: bool,
    prev: bool,
}

fn vk_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    r: &Renderer,
    st: &Status,
    ph: Phase,
    fade: f32,
    t: f32,
    dt: f32,
    m: &mut Mouse,
    drag: &mut Option<(f32, f32)>,
    launch_flash: &mut f32,
) -> Option<QuitOrLaunch> {
    let mut action = None;
    // la carte occupe TOUTE la zone client — le drag déplace la FENÊTRE
    let (wx, wy) = (0.0f32, 0.0f32);
    unsafe {
        if r
            .rt
            .BindDC(r.memdc, &RECT { left: 0, top: 0, right: W as i32, bottom: H as i32 })
            .is_err()
        {
            return None;
        }
        r.rt.BeginDraw();
        // fond ENTIÈREMENT transparent — la carte de verre flotte sur le bureau
        r.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));

        // ombre portée multi-couches (la carte "flotte")
        r.fill_round(wx - 18.0, wy - 12.0, W + 36.0, H + 28.0, 32.0, with_a([0.0, 0.0, 0.0, 0.34], fade));
        r.fill_round(wx - 9.0, wy - 6.0, W + 18.0, H + 14.0, 24.0, with_a([0.0, 0.0, 0.0, 0.38], fade));

        // panneau verre principal + double liseré (v2)
        r.fill_round(wx, wy, W, H, 20.0, with_a(BG, fade));
        r.stroke_round(wx, wy, W, H, 20.0, with_a([PRIMARY[0], PRIMARY[1], PRIMARY[2], 0.38], fade), 1.3);
        r.stroke_round(wx + 2.0, wy + 2.0, W - 4.0, H - 4.0, 18.0, with_a([1.0, 1.0, 1.0, 0.05], fade), 1.0);

        // liseré accent supérieur — hairline dégradée (la signature v2)
        r.grad.SetOpacity(0.9 * fade);
        r.grad.SetStartPoint(D2D_POINT_2F { x: wx + 20.0, y: wy + 3.0 });
        r.grad.SetEndPoint(D2D_POINT_2F { x: wx + W - 20.0, y: wy + 3.0 });
        r.rt.FillRectangle(&rect(wx + 20.0, wy + 1.5, W - 40.0, 2.6), &r.grad);

        // ── ORBES ANIMÉS (2 lissajous lents, 3 couches chacun) ──
        let orb = |cx: f32, cy: f32, rad: f32, col: [f32; 4]| {
            for (k, alpha) in [(1.0, 0.045), (0.6, 0.08), (0.32, 0.12)] {
                r.dot(cx, cy, rad * k, with_a([col[0], col[1], col[2], alpha], fade));
            }
        };
        // centres bornés pour que centre+rayon reste DANS la carte (zéro bleed)
        let o1x = wx + 170.0 + 85.0 * (t * 0.21).sin();
        let o1y = wy + 220.0 + 60.0 * (t * 0.17 + 1.0).cos();
        orb(o1x, o1y, 155.0, PRIMARY);
        let o2x = wx + 320.0 + 55.0 * (t * 0.15 + 2.0).sin();
        let o2y = wy + 440.0 + 60.0 * (t * 0.19).cos();
        orb(o2x, o2y, 145.0, LIGHT);

        // ── balayage de lumière diagonal (7 s de cycle) sur toute la carte ──
        {
            let period = 7.0;
            let p = (t % period) / period;
            let sx = wx - 160.0 + p * (W + 320.0);
            // CLIPPÉ aux bornes de la carte (la bande ne saigne jamais au-dehors)
            let lx = (sx - wx).max(0.0);
            let rx = (sx + 160.0 - wx).min(W);
            if rx > lx {
                r.grad.SetOpacity(0.10 * fade);
                r.grad.SetStartPoint(D2D_POINT_2F { x: wx + lx, y: wy });
                r.grad.SetEndPoint(D2D_POINT_2F { x: wx + rx, y: wy + H });
                r.rt.FillRectangle(&rect(wx + lx, wy, rx - lx, H), &r.grad);
            }
        }

        // ── en-tête v2: wordmark + chip version + pastille état + croix ──
        let title_y = wy + 22.0;
        r.grad_text("VANTYX", &r.fmt_logo, wx + 24.0, title_y, 210.0, 46.0, 0, fade);
        r.text("launcher", &r.fmt_s, wx + 178.0, title_y + 24.0, 90.0, 20.0, with_a(DIM, fade), 0);
        // chip version (sous le wordmark) — la ligne "what's new"
        {
            let (chx, chy) = (wx + 25.0, wy + 66.0);
            let chw = 250.0;
            r.fill_round(chx, chy, chw, 20.0, 10.0, with_a(SURFACE, 0.85 * fade));
            r.dot(chx + 11.0, chy + 10.0, 2.6, with_a([PRIMARY[0], PRIMARY[1], PRIMARY[2], 0.9], fade));
            r.text("v0.9 · GHOST SPIN · AIM v0.9 · 3ᵉ PERS.", &r.fmt_s, chx + 20.0, chy, chw - 26.0, 20.0, with_a(LIGHT, 0.9 * fade), 0);
        }
        // pastille CS2 live
        let (pill_c, pill_t) = if st.cs2_running {
            (OK_GREEN, "CS2 DÉTECTÉ")
        } else {
            (WARN_AMBER, "CS2 AU REPOS")
        };
        let pw = 122.0;
        let (px, py) = (wx + W - pw - 58.0, title_y + 6.0);
        r.fill_round(px, py, pw, 24.0, 12.0, with_a(SURFACE, 0.8 * fade));
        let led = 0.55 + 0.35 * (t * 2.6).sin();
        r.ring(px + 15.0, py + 12.0, 5.5, with_a([pill_c[0], pill_c[1], pill_c[2], 0.4], fade), 1.4);
        r.dot(px + 15.0, py + 12.0, 2.6, with_a([pill_c[0], pill_c[1], pill_c[2], 0.5 + 0.4 * led], fade));
        r.text(pill_t, &r.fmt_s, px + 27.0, py, pw - 32.0, 24.0, with_a(TEXT, 0.85 * fade), 0);
        r.line(wx + 24.0, wy + 96.0, wx + W - 24.0, wy + 96.0, with_a([1.0, 1.0, 1.0, 0.07], fade), 1.0);

        // ── EMBLÈME v2: viseur — anneau + arcs rotatifs + 12 ticks radiaux ──
        let (ecx, ecy) = (wx + W * 0.5, wy + 168.0);
        let breathe = 0.5 + 0.5 * (t * 1.6).sin();
        // halo
        r.dot(ecx, ecy, 50.0 + 7.0 * breathe, with_a([PRIMARY[0], PRIMARY[1], PRIMARY[2], 0.05 + 0.04 * breathe], fade));
        // ticks radiaux en rotation lente (cadran de viseur)
        for i in 0..12 {
            let a = (i as f32 / 12.0) * std::f32::consts::TAU + t * 0.15;
            let (c, s) = (a.cos(), a.sin());
            r.line(ecx + c * 52.0, ecy + s * 52.0, ecx + c * 58.0, ecy + s * 58.0, with_a([1.0, 1.0, 1.0, 0.16], fade), 1.2);
        }
        // anneau extérieur discret
        r.ring(ecx, ecy, 40.0, with_a([1.0, 1.0, 1.0, 0.10], fade), 1.2);
        // arc rotatif dégradé (2 arcs opposés, rotation 0.4 tr/s)
        let phase = (t * 0.4) % 1.0;
        r.arc(ecx, ecy, 40.0, phase, 1.9, with_a(PRIMARY, 0.85 * fade), 2.8);
        r.arc(ecx, ecy, 40.0, phase + 0.5, 1.9, with_a(LIGHT, 0.45 * fade), 2.0);
        // cœur: double pastille primary/light (l'identité Vantyx)
        r.dot(ecx - 5.0, ecy, 9.0, with_a(PRIMARY, fade));
        r.dot(ecx + 5.0, ecy, 9.0, with_a(LIGHT, fade));
        r.ring(ecx, ecy, 19.0, with_a([PRIMARY[0], PRIMARY[1], PRIMARY[2], 0.35], fade), 1.0);

        // ── CHIPS DES TROIS NOUVEAUTÉS v0.9 (sous l'emblème) ──
        {
            let feats: [(&str, f32); 3] = [("GHOST SPIN", 0.55), ("CAM 3ᵉ PERS.", 0.35), ("AIM PRÉDICTION", 0.75)];
            let cw3 = 150.0;
            let gap = 11.0;
            let total = cw3 * 3.0 + gap * 2.0;
            let fx0 = wx + (W - total) / 2.0;
            for (i, (label, _p)) in feats.iter().enumerate() {
                let cx = fx0 + i as f32 * (cw3 + gap);
                let cy = wy + 236.0;
                r.fill_round(cx, cy, cw3, 26.0, 13.0, with_a(SURFACE, 0.75 * fade));
                r.stroke_round(cx, cy, cw3, 26.0, 13.0, with_a([PRIMARY[0], PRIMARY[1], PRIMARY[2], 0.22], fade), 1.0);
                let bl = 0.6 + 0.4 * (t * 2.4 + i as f32 * 1.7).sin();
                r.dot(cx + 13.0, cy + 13.0, 3.0, with_a([LIGHT[0], LIGHT[1], LIGHT[2], bl], fade));
                r.text(label, &r.fmt_s, cx + 22.0, cy, cw3 - 28.0, 26.0, with_a(TEXT, 0.9 * fade), 1);
            }
        }

        // ── VÉRIFICATIONS (LED + libellé + détail) ──
        let checks: [(bool, &str, &str); 4] = [
            (st.cs2_running, "Counter-Strike 2", if st.cs2_running { "processus détecté" } else { "lance le jeu pour l'ESP live" }),
            (st.engine_present, "Moteur Vantyx.cc", if st.engine_present { "vantyx.exe prêt" } else { "vantyx.exe INTROUVABLE" }),
            (st.offsets_present, "Offsets", if st.offsets_present { "offsets.json · build 14193" } else { "offsets.json absent (pins internes)" }),
            (st.fonts_present, "Polices", if st.fonts_present { "Space Grotesk en mémoire" } else { "repli Segoe UI" }),
        ];
        let rows_y = wy + 278.0;
        for (i, (ok, label, detail)) in checks.iter().enumerate() {
            let ly = rows_y + i as f32 * 38.0;
            let c = if *ok { OK_GREEN } else if i == 1 { BAD_RED } else { WARN_AMBER };
            let led = 0.5 + 0.5 * (t * 2.2 + i as f32 * 1.3).sin();
            r.fill_round(wx + 24.0, ly, W - 48.0, 31.0, 8.0, with_a(SURFACE, 0.55 * fade));
            if *ok {
                r.ring(wx + 41.0, ly + 15.5, 6.0, with_a([c[0], c[1], c[2], 0.30 + 0.20 * led], fade), 1.6);
            }
            r.dot(wx + 41.0, ly + 15.5, 3.2, with_a([c[0], c[1], c[2], 0.65 + 0.30 * led], fade));
            r.text(label, &r.fmt_b, wx + 56.0, ly + 0.5, 190.0, 30.0, with_a(TEXT, 0.95 * fade), 0);
            r.text(detail, &r.fmt_s, wx + 240.0, ly + 0.5, W - 270.0, 30.0, with_a(DIM, 0.9 * fade), 2);
        }

        // ── BOUTON LANCER v2 (double hauteur, label + sous-ligne) ──
        let btn_y = wy + H - 148.0;
        let (bx, bw, bh) = (wx + 24.0, W - 48.0, 62.0);
        let hovering = m.x >= bx && m.x < bx + bw && m.y >= btn_y && m.y < btn_y + bh;
        match ph {
            Phase::Launching(t0) => {
                // barre de progression balayée (~0.9 s) puis spawn
                let p = (t0.elapsed().as_secs_f32() / 0.9).clamp(0.0, 1.0);
                r.fill_round(bx, btn_y, bw, bh, 14.0, with_a(SURFACE, 0.9 * fade));
                r.fill_round(bx, btn_y, bw, bh, 14.0, with_a([PRIMARY[0], PRIMARY[1], PRIMARY[2], 0.10], fade));
                r.grad_fill_round(bx, btn_y, (bw * p).max(10.0), bh, 14.0, 0.85 * fade);
                r.stroke_round(bx, btn_y, bw, bh, 14.0, with_a([PRIMARY[0], PRIMARY[1], PRIMARY[2], 0.5], fade), 1.2);
                let label = if p < 0.35 {
                    "INJECTION DES MODULES"
                } else if p < 0.7 {
                    "ACCROCHE DU OVERLAY"
                } else {
                    "LANCÉ — BON JEUX"
                };
                r.text(label, &r.fmt_title, bx, btn_y + 6.0, bw, 30.0, with_a(TEXT, fade), 1);
                r.text(format!("{:.0} %", p * 100.0).as_str(), &r.fmt_s, bx, btn_y + 34.0, bw, 18.0, with_a([1.0, 1.0, 1.0, 0.75], fade), 1);
                action = Some(QuitOrLaunch::Progress(p));
            }
            _ => {
                // état normal: bouton LANCER animé
                let ready = st.ready_enough();
                let glow = 0.5 + 0.5 * (t * 2.0).sin();
                let hov = if hovering { 1.0 } else { 0.0 };
                // halo extérieur pulsé (attire l'œil)
                r.fill_round(bx - 3.0, btn_y - 3.0, bw + 6.0, bh + 6.0, 16.0, with_a([PRIMARY[0], PRIMARY[1], PRIMARY[2], 0.10 + 0.08 * glow + 0.10 * hov], fade));
                if ready {
                    r.grad_fill_round(bx, btn_y, bw, bh, 14.0, (0.85 + 0.10 * glow + 0.05 * hov) * fade);
                    r.text("LANCER VANTYX", &r.fmt_title, bx, btn_y + 8.0, bw, 28.0, with_a([1.0, 1.0, 1.0, 1.0], fade), 1);
                    r.text("ESP · AIM v0.9 · RAGE (GHOST + 3ᵉ PERS.)", &r.fmt_s, bx, btn_y + 36.0, bw, 18.0, with_a([1.0, 1.0, 1.0, 0.72], fade), 1);
                } else {
                    r.fill_round(bx, btn_y, bw, bh, 14.0, with_a([0.30, 0.08, 0.10, 0.85], fade));
                    r.stroke_round(bx, btn_y, bw, bh, 14.0, with_a([BAD_RED[0], BAD_RED[1], BAD_RED[2], 0.4], fade), 1.2);
                    r.text("MOTEUR MANQUANT", &r.fmt_title, bx, btn_y + 8.0, bw, 28.0, with_a([1.0, 0.62, 0.62, 1.0], fade), 1);
                    r.text("pose vantyx.exe à côté du launcher", &r.fmt_s, bx, btn_y + 36.0, bw, 18.0, with_a([1.0, 0.62, 0.62, 0.7], fade), 1);
                }
                // flash de clic (onde qui s'estompe)
                if *launch_flash > 0.0 {
                    r.fill_round(bx, btn_y, bw, bh, 14.0, with_a([1.0, 1.0, 1.0, 0.35 * *launch_flash], fade));
                }
                if m.clicked && hovering && ready {
                    *launch_flash = 1.0;
                    action = Some(QuitOrLaunch::Launch);
                }
            }
        }
        // décroissance du flash de clic (~300 ms à 120 fps)
        if *launch_flash > 0.0 {
            *launch_flash = (*launch_flash - dt * 3.4).max(0.0);
        }

        // ── pied de carte v2: séparateur + trois lignes hiérarchisées ──
        r.line(wx + 24.0, wy + H - 74.0, wx + W - 24.0, wy + H - 74.0, with_a([1.0, 1.0, 1.0, 0.06], fade), 1.0);
        r.text(
            "vantyx.exe v0.9.1 · build 14193 · offsets.json live",
            &r.fmt_s, wx + 24.0, wy + H - 66.0, W - 48.0, 18.0, with_a(TEXT, 0.72 * fade), 1,
        );
        r.text(
            "INSERT menu en jeu · END quitter · F10 anti screen-share",
            &r.fmt_s, wx + 24.0, wy + H - 46.0, W - 48.0, 18.0, with_a(DIM, 0.8 * fade), 1,
        );
        r.text(
            "usage local · RPM + SendInput · mode RAGE: écriture dwViewAngles",
            &r.fmt_s, wx + 24.0, wy + H - 28.0, W - 48.0, 18.0, with_a(DIM, 0.55 * fade), 1,
        );

        // bouton fermer (croix) haut-droite — pastille ronde avec halo hover
        let (cbx, cby) = (wx + W - 38.0, wy + 14.0);
        let c_hover = m.x >= cbx && m.x < cbx + 28.0 && m.y >= cby && m.y < cby + 28.0;
        if c_hover {
            r.fill_round(cbx, cby, 28.0, 28.0, 14.0, with_a([1.0, 1.0, 1.0, 0.10], fade));
        }
        r.ring(cbx + 14.0, cby + 14.0, 11.0, with_a([1.0, 1.0, 1.0, 0.14], fade), 1.0);
        if m.clicked && c_hover {
            action = Some(QuitOrLaunch::Quit);
        }
        r.line(cbx + 9.5, cby + 9.5, cbx + 18.5, cby + 18.5, with_a(TEXT, (if c_hover { 0.9 } else { 0.55 }) * fade), 1.6);
        r.line(cbx + 18.5, cby + 9.5, cbx + 9.5, cby + 18.5, with_a(TEXT, (if c_hover { 0.9 } else { 0.55 }) * fade), 1.6);

        // ── v2: DRAG PARTOUT — toute la carte est une poignée, SAUF le bouton
        // LANCER et la croix (les deux zones cliquables). C'est le correctif
        // "rends le launcher movable": avant, seule la bande d'en-tête (84 px)
        // déplaçait la fenêtre. ──
        let over_btn = m.x >= bx && m.x < bx + bw && m.y >= btn_y && m.y < btn_y + bh;
        let over_close = c_hover;
        let in_card = m.x >= wx && m.x < wx + W && m.y >= wy && m.y < wy + H;
        if m.held && m.clicked && in_card && !over_btn && !over_close {
            *drag = Some((m.x - wx, m.y - wy));
        }
        if let Some((ox, oy)) = *drag {
            if m.held {
                action = Some(QuitOrLaunch::Drag(ox, oy));
            } else {
                *drag = None;
            }
        }

        if r.rt.EndDraw(None, None).is_err() {
            return None;
        }
    }
    r.present();
    m.clicked = false;
    action
}

enum QuitOrLaunch {
    Quit,
    Launch,
    Progress(f32),
    Drag(f32, f32),
}

// ═══ main ═══════════════════════════════════════════════════════════════════
fn main() {
    unsafe { let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); }

    // v0.6.1 — GetModuleHandleW/zeroed sont unsafe: blocs explicites
    let hmodule = match unsafe { GetModuleHandleW(None) } {
        Ok(h) => h,
        Err(_) => std::process::exit(1),
    };
    let hinst: HINSTANCE = hmodule.into();
    let class = wide("vantyx_launcher");
    let wc: WNDCLASSEXW = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinst,
        lpszClassName: PCWSTR(class.as_ptr()),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe { RegisterClassExW(&wc) };

    // centre de l'écran primaire
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let (ww, wh) = (W as i32, H as i32);
    let (x, y) = ((sw - ww) / 2, (sh - wh) / 2);

    let title = wide("Vantyx.cc Launcher");
    let ex: WINDOW_EX_STYLE = WS_EX_LAYERED | WS_EX_TOOLWINDOW;
    let st: WINDOW_STYLE = WS_POPUP | WS_VISIBLE;
    let hwnd = match unsafe {
        CreateWindowExW(
            ex,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            st,
            x, y, ww, wh,
            None,
            None,
            hinst,
            None,
        )
    } {
        Ok(h) => h,
        Err(_) => std::process::exit(2),
    };

    let renderer = match renderer_new(hwnd, ww, wh) {
        Ok(r) => r,
        Err(_) => std::process::exit(3),
    };

    let mut phase = Phase::FadeIn;
    let mut fade = 0.0f32;
    let mut t = 0.0f32;
    let mut mouse = Mouse { x: -1.0, y: -1.0, held: false, clicked: false, prev: false };
    let mut drag: Option<(f32, f32)> = None;
    let mut launch_flash = 0.0f32;
    let mut status = check_status();
    let mut last_status = Instant::now();
    let mut last_frame = Instant::now();
    let mut spawned = false;
    let mut k_esc = false;

    'run: loop {
        // pompe à messages
        let mut msg: windows::Win32::UI::WindowsAndMessaging::MSG = unsafe { std::mem::zeroed() };
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'run;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // ESC → fondu de sortie (jamais de kill sec — c'est premium)
        let esc = vk_down(VK_ESCAPE);
        if esc && !k_esc && phase != Phase::FadeOut {
            phase = Phase::FadeOut;
            fade = fade.max(0.01);
        }
        k_esc = esc;

        // horloge
        let now = Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f32().min(0.1);
        last_frame = now;
        t += dt;

        // statut rafraîchi à 2 Hz
        if now.duration_since(last_status).as_millis() >= 500 {
            status = check_status();
            last_status = now;
        }

        // souris (pollée comme le menu du cheat)
        unsafe {
            let mut pt: POINT = std::mem::zeroed();
            if GetCursorPos(&mut pt).is_ok() {
                let _ = ScreenToClient(hwnd, &mut pt);
                mouse.x = pt.x as f32;
                mouse.y = pt.y as f32;
            }
            let down = vk_down(VK_LBUTTON);
            if down && !mouse.prev {
                mouse.clicked = true;
            }
            mouse.prev = down;
            mouse.held = down;
        }

        // machine à états
        match phase {
            Phase::FadeIn => {
                fade += dt * 3.2;
                if fade >= 1.0 {
                    fade = 1.0;
                    phase = Phase::Ready;
                }
            }
            Phase::Ready => {}
            Phase::Launching(t0) => {
                if t0.elapsed().as_secs_f32() >= 0.9 && !spawned {
                    spawned = true;
                    if !spawn_engine() {
                        // moteur introuvable → retour à l'état prêt (message par LED)
                        phase = Phase::Ready;
                        spawned = false;
                    } else {
                        phase = Phase::FadeOut;
                        fade = fade.max(0.01);
                    }
                }
            }
            Phase::FadeOut => {
                fade -= dt * 2.4;
                if fade <= 0.0 {
                    break 'run;
                }
            }
        }

        // frame
        let act = draw_frame(
            &renderer,
            &status,
            phase,
            fade.clamp(0.0, 1.0),
            t,
            dt,
            &mut mouse,
            &mut drag,
            &mut launch_flash,
        );
        match act {
            Some(QuitOrLaunch::Quit) => {
                if phase != Phase::FadeOut {
                    phase = Phase::FadeOut;
                    fade = fade.max(0.01);
                }
            }
            Some(QuitOrLaunch::Launch) if phase == Phase::Ready => {
                phase = Phase::Launching(Instant::now());
            }
            Some(QuitOrLaunch::Progress(p)) => {
                if p >= 1.0 && !spawned {
                    // le spawn est géré par la machine à états au tour suivant
                }
            }
            Some(QuitOrLaunch::Drag(ox, oy)) => {
                // repositionne la FENÊTRE entière (le drag déplace la carte)
                unsafe {
                    let mut pt: POINT = std::mem::zeroed();
                    if GetCursorPos(&mut pt).is_ok() {
                        let _res = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                            hwnd,
                            None,
                            pt.x - ox as i32,
                            pt.y - oy as i32,
                            0,
                            0,
                            windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                                | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                        );
                    }
                }
            }
            _ => {}
        }
        unsafe { Sleep(8) } // ~120 fps — fluide, léger
    }
}
