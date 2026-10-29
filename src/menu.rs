// menu — menu IN-OVERLAY glassmorphism, rendu Direct2D + texte DirectWrite.
//
// DESIGN (inspiré des références GitHub actuelles — IMXNOOBX/cs2-external-esp
// v18 oct.2026, DragonBurn, AimStar, FoxSense): panneau verre sombre
// translucide (alpha par pixel via UpdateLayeredWindow — on voit VRAIMENT le
// jeu à travers), reflet supérieur (sheen), ombre douce multi-couches,
// bordure claire 1px, accent dégradé mint→azur, sidebar à onglets avec
// indicateur animé, cartes glass, checkboxes/sliders/segmented/swatches,
// aperçu en direct des réglages ESP, LED "en ligne" qui respire.
//
// MODE IMMÉDIAT (comme ImGui): chaque widget se dessine ET se teste à la
// souris dans la même passe — layout fixe, zéro rétention d'état par widget
// (seuls hover/drag/slider vivent dans Menu).
//
// SOURIS: pollée (GetCursorPos + GetAsyncKeyState VK_LBUTTON) — aucun
// wndproc nécessaire. La boucle overlay appelle Mouse::update à ~1 kHz;
// `clicked` (front descendant) est CONSOMMÉ au frame menu (~166 Hz).
// Quand le menu est ouvert, overlay.rs retire WS_EX_TRANSPARENT → les clics
// tombent dans l'overlay (et ne tirnent plus dans le jeu).
//
// CONFIG: cs2-esp.ini à côté de l'exe (std::fs pur, zéro API Win32) —
// chargé au boot, sauvegardé 400 ms après le dernier changement (sliders
// inclus) + à la fermeture du menu / arrêt.
//
// NOTES windows-rs 0.58 (VÉRIFIÉES contre le tag source — DirectWrite/mod.rs,
// Gdi/mod.rs): GetCursorPos(*mut POINT) -> Result<()>; ScreenToClient vit
// dans Win32::Graphics::Gdi (pas WindowsAndMessaging!) et retourne BOOL nu.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::overlay::Renderer;
use crate::{dlog, dwarn};

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ScreenToClient; // piège windows-rs: vit dans Gdi, PAS WAM
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos; // (lui est bien dans WAM)

/// alignement horizontal pour Renderer::text / text_grad.
#[derive(Clone, Copy)]
pub enum Align {
    L,
    C,
    R,
}

/// commandes ponctuelles remontées à la boucle overlay.
#[derive(Clone, Copy, Debug)]
pub enum Action {
    Dump,
    ShowPanel,
    Quit,
}

// ── souris: pollée, aucun message requis ────────────────────────────────────
pub struct Mouse {
    pub x: f32,
    pub y: f32,
    pub held: bool,
    pub clicked: bool, // front descendant depuis la dernière consommation
    prev: bool,
}

impl Default for Mouse {
    fn default() -> Self {
        Mouse { x: -1.0, y: -1.0, held: false, clicked: false, prev: false }
    }
}

impl Mouse {
    /// à chaque itération de boucle (~1 kHz); `clicked` est consommé au frame.
    pub fn update(&mut self, hwnd: HWND) {
        unsafe {
            let mut pt: POINT = std::mem::zeroed();
            if GetCursorPos(&mut pt).is_ok() {
                let _ = ScreenToClient(hwnd, &mut pt);
                self.x = pt.x as f32;
                self.y = pt.y as f32;
            }
            let down = (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0;
            if down && !self.prev {
                self.clicked = true;
            }
            self.prev = down;
            self.held = down;
        }
    }
}

// ── options — tout ce que le menu contrôle, persisté dans cs2-esp.ini ──────
#[derive(Clone)]
pub struct Options {
    pub esp: bool,
    pub box_mode: usize, // 0 coins · 1 complet · 2 off
    pub box_fill: bool,
    pub health: bool,
    pub snaplines: bool,
    pub crosshair: bool,
    pub skeleton: bool, // squelette ESP (17 segments d'os)
    pub names: bool,
    pub distance: bool,
    pub teammates: bool,
    pub unlimited_dist: bool, // PAS de limite de distance ESP
    pub max_dist_m: f32,
    pub palette: usize,
    pub radar: bool,
    pub radar_corner: usize, // 0 h-g · 1 h-d · 2 b-g · 3 b-d
    pub radar_size: f32,
    pub radar_range: f32,
    pub radar_mates: bool,
    pub trigger: bool,
    pub trigger_key: usize, // 0 alt · 1 ctrl · 2 bouton4 · 3 maj
    pub trigger_delay_ms: f32,
    pub game_only: bool, // ESP limité au jeu (caché hors premier plan / menus)
    pub watermark: bool,
    pub verbose: bool,
    pub interval_ms: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            esp: true,
            box_mode: 0,
            box_fill: true,
            health: true,
            snaplines: false,
            crosshair: true,
            skeleton: true,
            names: true,
            distance: true,
            teammates: false,
            unlimited_dist: true,
            max_dist_m: 300.0,
            palette: 0,
            radar: true,
            radar_corner: 1,
            radar_size: 200.0,
            radar_range: 2500.0,
            radar_mates: true,
            trigger: true,
            trigger_key: 0,
            trigger_delay_ms: 35.0,
            game_only: true,
            watermark: true,
            verbose: false,
            interval_ms: 1,
        }
    }
}

/// statut live injecté dans les onglets Divers/Info + footer.
pub struct StatusInfo {
    pub pid: u32,
    pub fps: f32,
    pub players: usize,
    pub snapshot_ok: bool,
    pub renderer_ok: bool,
    pub config_path: String,
    pub trigger_armed: bool, // viseur sur ennemi + touche tenue
    pub game_fg: bool,       // CS2 au premier plan ?
    pub font: String,        // police en service (Space Grotesk / repli)
}

pub const OFFSETS_NOTE: &str = "dump local 29.10.2026 · build 14193 — override: offsets.json";

// ── palettes ESP (T / CT / toi) ─────────────────────────────────────────────
pub struct EspPalette {
    pub name: &'static str,
    pub t: [f32; 4],
    pub ct: [f32; 4],
    pub me: [f32; 4],
}

pub const PALETTES: [EspPalette; 4] = [
    EspPalette { name: "défaut", t: [1.00, 0.62, 0.20, 1.0], ct: [0.30, 0.68, 1.00, 1.0], me: [0.32, 1.00, 0.50, 1.0] },
    EspPalette { name: "néon", t: [1.00, 0.22, 0.80, 1.0], ct: [0.10, 0.94, 1.00, 1.0], me: [0.70, 1.00, 0.20, 1.0] },
    EspPalette { name: "pastel", t: [0.99, 0.55, 0.62, 1.0], ct: [0.55, 0.75, 1.00, 1.0], me: [0.60, 0.95, 0.80, 1.0] },
    EspPalette { name: "mono", t: [0.95, 0.90, 0.82, 1.0], ct: [0.75, 0.80, 0.85, 1.0], me: [1.00, 1.00, 1.00, 1.0] },
];

// ── couleurs du menu (accent mint → azur, verre sombre) ─────────────────────
pub const ACCENT_A: [f32; 4] = [0.42, 0.90, 0.85, 1.0];
pub const ACCENT_B: [f32; 4] = [0.31, 0.62, 1.00, 1.0];
pub const C_TEXT: [f32; 4] = [0.93, 0.95, 0.97, 1.0];
pub const C_DIM: [f32; 4] = [0.60, 0.65, 0.72, 1.0];
pub const C_DANGER: [f32; 4] = [1.00, 0.36, 0.36, 1.0];

// géométrie (px — DPI 96 forcé dans le render target → 1 DIP = 1 px)
const W: f32 = 780.0;
const H: f32 = 560.0;
const TITLE_H: f32 = 46.0;
const FOOT_H: f32 = 34.0;
const SIDE_W: f32 = 150.0;
const PAD: f32 = 16.0;
const CW: f32 = 291.0; // largeur de carte
const CX0: f32 = SIDE_W + PAD;
const CX1: f32 = CX0 + CW + PAD;
const CY0: f32 = TITLE_H + PAD;

pub(crate) fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, 1.0]
}
fn with_a(c: [f32; 4], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], c[3] * a]
}
fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}
fn hit_r(m: &Mouse, x: f32, y: f32, w: f32, h: f32) -> bool {
    m.x >= x && m.x < x + w && m.y >= y && m.y < y + h
}

// ═══ config ini (std::fs pur — zéro API profil Win32) ═══════════════════════
pub(crate) fn config_path() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("cs2-esp.ini")))
}

pub fn load_options() -> (Options, Option<(f32, f32)>) {
    let mut o = Options::default();
    let mut pos: Option<(f32, f32)> = None;
    let Some(p) = config_path() else { return (o, pos) };
    let Ok(content) = std::fs::read_to_string(&p) else { return (o, pos) };
    let mut got = 0u32;
    let (mut ux, mut uy) = (None, None);
    for line in content.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "esp" => { o.esp = v == "1"; got += 1; }
            "box_mode" => { o.box_mode = v.parse().unwrap_or(0).min(2); got += 1; }
            "box_fill" => { o.box_fill = v == "1"; got += 1; }
            "health" => { o.health = v == "1"; got += 1; }
            "snaplines" => { o.snaplines = v == "1"; got += 1; }
            "crosshair" => { o.crosshair = v == "1"; got += 1; }
            "skeleton" => { o.skeleton = v == "1"; got += 1; }
            "names" => { o.names = v == "1"; got += 1; }
            "distance" => { o.distance = v == "1"; got += 1; }
            "teammates" => { o.teammates = v == "1"; got += 1; }
            "unlimited_dist" => { o.unlimited_dist = v == "1"; got += 1; }
            "max_dist_m" => { o.max_dist_m = v.parse::<f32>().unwrap_or(300.0).clamp(20.0, 2000.0); got += 1; }
            "palette" => { o.palette = v.parse().unwrap_or(0).min(3); got += 1; }
            "radar" => { o.radar = v == "1"; got += 1; }
            "radar_corner" => { o.radar_corner = v.parse().unwrap_or(1).min(3); got += 1; }
            "radar_size" => { o.radar_size = v.parse::<f32>().unwrap_or(200.0).clamp(120.0, 400.0); got += 1; }
            "radar_range" => { o.radar_range = v.parse::<f32>().unwrap_or(2500.0).clamp(300.0, 8000.0); got += 1; }
            "radar_mates" => { o.radar_mates = v == "1"; got += 1; }
            "trigger" => { o.trigger = v == "1"; got += 1; }
            "trigger_key" => { o.trigger_key = v.parse().unwrap_or(0).min(3); got += 1; }
            "trigger_delay_ms" => { o.trigger_delay_ms = v.parse::<f32>().unwrap_or(35.0).clamp(0.0, 250.0); got += 1; }
            "game_only" => { o.game_only = v == "1"; got += 1; }
            "watermark" => { o.watermark = v == "1"; got += 1; }
            "verbose" => { o.verbose = v == "1"; got += 1; }
            "interval_ms" => { o.interval_ms = v.parse().unwrap_or(1).clamp(1, 50); got += 1; }
            "ui_x" => { ux = v.parse().ok(); }
            "ui_y" => { uy = v.parse().ok(); }
            _ => {}
        }
    }
    if let (Some(x), Some(y)) = (ux, uy) {
        pos = Some((x, y));
    }
    if got > 0 {
        dlog!("[*] config chargée ({got} clés) — {}", p.display());
    }
    (o, pos)
}

pub fn save_options(opts: &Options, pos: (f32, f32)) {
    let Some(p) = config_path() else { return };
    let b = |v: bool| if v { "1" } else { "0" };
    let s = format!(
        "[visuals]\nesp={}\nbox_mode={}\nbox_fill={}\nhealth={}\nsnaplines={}\ncrosshair={}\nskeleton={}\nnames={}\ndistance={}\nteammates={}\nunlimited_dist={}\nmax_dist_m={:.0}\npalette={}\n\n[radar]\nradar={}\nradar_corner={}\nradar_size={:.0}\nradar_range={:.0}\nradar_mates={}\n\n[trigger]\ntrigger={}\ntrigger_key={}\ntrigger_delay_ms={:.0}\n\n[misc]\ngame_only={}\nwatermark={}\nverbose={}\ninterval_ms={}\n\n[ui]\nui_x={:.0}\nui_y={:.0}\n",
        b(opts.esp), opts.box_mode, b(opts.box_fill), b(opts.health), b(opts.snaplines),
        b(opts.crosshair), b(opts.skeleton), b(opts.names), b(opts.distance), b(opts.teammates),
        b(opts.unlimited_dist), opts.max_dist_m, opts.palette,
        b(opts.radar), opts.radar_corner, opts.radar_size, opts.radar_range, b(opts.radar_mates),
        b(opts.trigger), opts.trigger_key, opts.trigger_delay_ms,
        b(opts.game_only), b(opts.watermark), b(opts.verbose), opts.interval_ms,
        pos.0, pos.1
    );
    match std::fs::write(&p, s) {
        Ok(()) => dlog!("[*] config sauvegardée — {}", p.display()),
        Err(e) => dwarn!("config non écrite ({p:?}): {e}"),
    }
}

// ═══ le menu ════════════════════════════════════════════════════════════════
pub struct Menu {
    pub open: bool,
    x: f32,
    y: f32,
    tab: usize,
    anim: f32, // 0..1 — animation d'ouverture (fade + slide-up)
    tab_y: f32, // indicateur d'onglet animé (offset px)
    drag: Option<(f32, f32)>,
    slider: Option<u64>, // id du slider en cours de glissement
    hover: HashMap<u64, f32>,
    dirty_at: Option<Instant>,
    t: f32, // horloge anims (respiration LED)
    sw: f32,
    sh: f32,
}

impl Menu {
    pub fn new(saved: Option<(f32, f32)>, sw: i32, sh: i32) -> Menu {
        let (x, y) = saved.unwrap_or_else(|| ((sw as f32 - W) * 0.5, (sh as f32 - H) * 0.45));
        let mut m = Menu {
            open: false,
            x,
            y,
            tab: 0,
            anim: 0.0,
            tab_y: 60.0,
            drag: None,
            slider: None,
            hover: HashMap::new(),
            dirty_at: None,
            t: 0.0,
            sw: sw as f32,
            sh: sh as f32,
        };
        m.clamp_pos();
        m
    }

    /// bounds overlay (suit la fenêtre du jeu) — re-clippe le menu dedans.
    pub fn set_bounds(&mut self, sw: i32, sh: i32) {
        self.sw = sw as f32;
        self.sh = sh as f32;
        self.clamp_pos();
    }

    fn clamp_pos(&mut self) {
        self.x = self.x.clamp(8.0, (self.sw - W - 8.0).max(8.0));
        self.y = self.y.clamp(8.0, (self.sh - H - 8.0).max(8.0));
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.mark_dirty(); // persiste la position à la fermeture
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty_at = Some(Instant::now());
    }

    /// sauvegarde différée — 400 ms de calme après le dernier changement.
    pub fn maybe_save(&mut self, opts: &Options) {
        if let Some(at) = self.dirty_at {
            if at.elapsed().as_millis() >= 400 {
                save_options(opts, (self.x, self.y));
                self.dirty_at = None;
            }
        }
    }

    pub fn save_now(&mut self, opts: &Options) {
        save_options(opts, (self.x, self.y));
        self.dirty_at = None;
    }

    /// hover 0..1 (approche exponentielle, ~12/s — transitions douces).
    fn hover(&mut self, id: u64, m: &Mouse, x: f32, y: f32, w: f32, h: f32, dt: f32) -> f32 {
        let target = if self.open && hit_r(m, x, y, w, h) { 1.0 } else { 0.0 };
        let e = self.hover.entry(id).or_insert(0.0);
        *e += (target - *e) * (dt * 12.0).min(1.0);
        if (target - *e).abs() < 0.004 {
            *e = target;
        }
        *e
    }

    /// menu complètement fermé et immobile → l'overlay peut jeter les clics.
    pub fn hidden(&self) -> bool {
        !self.open && self.anim <= 0.0
    }

    /// point d'entrée unique: anime, dessine, interagit, consomme le clic.
    pub fn frame(
        &mut self,
        r: &Renderer,
        m: &mut Mouse,
        opts: &mut Options,
        st: &StatusInfo,
        dt: f32,
    ) -> Vec<Action> {
        self.t += dt;
        let was_visible = self.anim > 0.0;
        let target = if self.open { 1.0 } else { 0.0 };
        self.anim += (target - self.anim) * (dt * 10.0).min(1.0);
        if (target - self.anim).abs() < 0.012 {
            self.anim = target;
        }
        // anim retombe à 0 → un DERNIER frame à alpha 0 efface le fantôme,
        // puis on s'endort complètement.
        if self.anim <= 0.0 && !was_visible {
            m.clicked = false;
            return Vec::new();
        }
        if self.anim <= 0.0 {
            self.anim = 0.0;
        }
        let a = ease_out_cubic(self.anim);
        let mut actions = Vec::new();
        self.draw(r, m, opts, st, a, dt, &mut actions);
        m.clicked = false;
        actions
    }
    // ── dessin principal (ombre, verre, titre, sidebar, contenu, footer) ────
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        opts: &mut Options,
        st: &StatusInfo,
        a: f32,
        dt: f32,
        actions: &mut Vec<Action>,
    ) {
        let x = self.x;
        let y = self.y - 10.0 * (1.0 - a); // slide-up + fade à l'ouverture

        // ombre douce multi-couches (le "flou" du glassmorphism, simulé)
        r.fill_round(x - 18.0, y - 12.0, W + 36.0, H + 30.0, 30.0, with_a([0.0, 0.0, 0.0, 0.14], a));
        r.fill_round(x - 10.0, y - 6.0, W + 20.0, H + 18.0, 22.0, with_a([0.0, 0.0, 0.0, 0.18], a));
        r.fill_round(x - 4.0, y - 2.0, W + 8.0, H + 8.0, 16.0, with_a([0.0, 0.0, 0.0, 0.22], a));

        // panneau verre principal — translucide RÉEL (le jeu transparaît)
        r.fill_round(x, y, W, H, 12.0, with_a([0.055, 0.065, 0.092, 0.86], a));
        r.sheen(x, y, W, 96.0, a); // reflet supérieur
        r.stroke_round(x, y, W, H, 12.0, with_a([1.0, 1.0, 1.0, 0.16], a), 1.0);

        // ── barre de titre (draggable) ──
        r.fill_round(x, y, W, TITLE_H, 12.0, with_a([0.028, 0.034, 0.050, 0.72], a));
        r.fill(x, y + 12.0, W, TITLE_H - 12.0, with_a([0.028, 0.034, 0.050, 0.72], a));
        r.line(x, y + TITLE_H, x + W, y + TITLE_H, with_a([1.0, 1.0, 1.0, 0.07], a), 1.0);
        let lw = r.measure("cs2-esp", &r.fmt_h).0;
        r.text_grad("cs2-esp", &r.fmt_h, x + 18.0, y, 120.0, TITLE_H, Align::L, true, a);
        r.text("v0.3.0", &r.fmt_s, x + 18.0 + lw + 8.0, y, 70.0, TITLE_H, with_a(C_DIM, a), Align::L, true);

        // bouton fermer (ferme le menu — INSERT le rouvre)
        let (cbx, cby) = (x + W - 40.0, y + 8.0);
        let ch = self.hover(99, m, cbx, cby, 32.0, 30.0, dt);
        if m.clicked && self.open && a > 0.85 && hit_r(m, cbx, cby, 32.0, 30.0) {
            self.open = false;
            self.mark_dirty();
        }
        if ch > 0.01 {
            r.fill_round(cbx, cby, 32.0, 30.0, 8.0, with_a([1.0, 1.0, 1.0, 0.10 * ch], a));
        }
        r.line(cbx + 10.0, cby + 9.0, cbx + 22.0, cby + 21.0, with_a([0.95, 0.96, 0.98, 0.85], a), 1.6);
        r.line(cbx + 22.0, cby + 9.0, cbx + 10.0, cby + 21.0, with_a([0.95, 0.96, 0.98, 0.85], a), 1.6);

        // drag par la barre de titre (hors bouton fermer)
        if self.open && a > 0.85 {
            if m.held && m.clicked && hit_r(m, x, y, W - 44.0, TITLE_H) {
                self.drag = Some((m.x - self.x, m.y - self.y));
            }
            if let Some((ox, oy)) = self.drag {
                if m.held {
                    self.x = m.x - ox;
                    self.y = m.y - oy;
                    self.clamp_pos();
                } else {
                    self.drag = None;
                }
            }
        }

        // ── sidebar ──
        r.fill(x, y + TITLE_H, SIDE_W, H - TITLE_H - FOOT_H, with_a([0.02, 0.026, 0.04, 0.50], a));
        r.line(x + SIDE_W, y + TITLE_H, x + SIDE_W, y + H - FOOT_H, with_a([1.0, 1.0, 1.0, 0.06], a), 1.0);

        // indicateur d'onglet animé (glisse vers l'onglet actif)
        let target_y = y + 60.0 + self.tab as f32 * 46.0;
        self.tab_y += (target_y - self.tab_y) * (dt * 14.0).min(1.0);
        r.grad_fill_round(x + 8.0, self.tab_y + 5.0, 3.0, 30.0, 1.5, a);

        const TABS: [&str; 5] = ["ESP", "RADAR", "TRIGGER", "DIVERS", "INFO"];
        for (i, label) in TABS.iter().enumerate() {
            let iy = y + 60.0 + i as f32 * 46.0;
            let hov = self.hover(200 + i as u64, m, x + 8.0, iy, SIDE_W - 16.0, 40.0, dt);
            if i == self.tab {
                r.fill_round(x + 8.0, iy, SIDE_W - 16.0, 40.0, 8.0, with_a([1.0, 1.0, 1.0, 0.06], a));
            } else if hov > 0.01 {
                r.fill_round(x + 8.0, iy, SIDE_W - 16.0, 40.0, 8.0, with_a([1.0, 1.0, 1.0, 0.04 * hov], a));
            }
            if m.clicked && self.open && a > 0.85 && hit_r(m, x + 8.0, iy, SIDE_W - 16.0, 40.0) {
                self.tab = i;
            }
            let c = if i == self.tab { with_a(C_TEXT, a) } else { with_a(C_DIM, a) };
            self.tab_icon(r, i, x + 26.0, iy + 20.0, c);
            r.text(label, &r.fmt_b, x + 48.0, iy, SIDE_W - 60.0, 40.0, c, Align::L, true);
        }

        // bas de sidebar: build + LED qui respire
        r.text("build v0.3", &r.fmt_s, x + 14.0, y + H - FOOT_H - 52.0, SIDE_W - 24.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        let led = 0.55 + 0.25 * (self.t * 2.2).sin();
        let led_y = y + H - FOOT_H - 18.0;
        r.ring(x + 21.0, led_y, 7.5, with_a([ACCENT_A[0], ACCENT_A[1], ACCENT_A[2], 0.14 + 0.10 * led], a), 2.0);
        r.dot(x + 21.0, led_y, 3.5, with_a([ACCENT_A[0], ACCENT_A[1], ACCENT_A[2], 0.55 + 0.40 * led], a));
        r.text("en ligne", &r.fmt_s, x + 36.0, led_y - 8.0, 100.0, 16.0, with_a(C_DIM, a), Align::L, true);

        // ── contenu de l'onglet actif ──
        match self.tab {
            0 => self.tab_esp(r, m, opts, a, dt, x, y),
            1 => self.tab_radar(r, m, opts, a, dt, x, y),
            2 => self.tab_trigger(r, m, opts, st, a, dt, x, y),
            3 => self.tab_divers(r, m, opts, st, a, dt, x, y, actions),
            _ => self.tab_info(r, st, a, x, y),
        }

        // ── footer ──
        r.line(x, y + H - FOOT_H, x + W, y + H - FOOT_H, with_a([1.0, 1.0, 1.0, 0.06], a), 1.0);
        r.text(
            "INSERT menu · END quitter · F1 debug · F2 coéquipiers · F4 dump · glisser la barre de titre",
            &r.fmt_s, x + 16.0, y + H - FOOT_H, 640.0, FOOT_H, with_a([0.50, 0.55, 0.62, 1.0], a), Align::L, true,
        );
        r.text(format!("{:.0} fps", st.fps).as_str(), &r.fmt_mono, x + W - 100.0, y + H - FOOT_H, 84.0, FOOT_H, with_a(C_DIM, a), Align::R, true);
    }

    /// micro-icônes vectorielles de la sidebar (dessinées — zéro risque police).
    fn tab_icon(&self, r: &Renderer, i: usize, cx: f32, cy: f32, c: [f32; 4]) {
        match i {
            0 => {
                // ESP: boîte + barre de vie
                r.stroke_round(cx - 8.0, cy - 6.0, 16.0, 12.0, 3.0, c, 1.3);
                r.line(cx - 4.5, cy - 3.0, cx - 4.5, cy + 3.0, c, 1.3);
            }
            1 => {
                // RADAR: cercle + points
                r.ring(cx, cy, 7.5, c, 1.3);
                r.dot(cx - 2.5, cy - 2.5, 1.4, c);
                r.dot(cx + 3.0, cy + 1.5, 1.4, c);
                r.line(cx, cy, cx + 5.0, cy - 6.0, c, 1.3);
            }
            2 => {
                // TRIGGER: viseur
                r.ring(cx, cy, 6.0, c, 1.3);
                r.line(cx - 9.0, cy, cx - 3.5, cy, c, 1.3);
                r.line(cx + 3.5, cy, cx + 9.0, cy, c, 1.3);
                r.dot(cx, cy, 1.2, c);
            }
            3 => {
                // DIVERS: deux sliders avec molettes
                r.line(cx - 8.0, cy - 4.0, cx + 8.0, cy - 4.0, c, 1.3);
                r.dot(cx - 3.0, cy - 4.0, 2.4, c);
                r.line(cx - 8.0, cy + 4.0, cx + 8.0, cy + 4.0, c, 1.3);
                r.dot(cx + 3.0, cy + 4.0, 2.4, c);
            }
            _ => {
                // INFO: anneau + point
                r.ring(cx, cy, 6.5, c, 1.3);
                r.dot(cx, cy - 2.8, 1.4, c);
            }
        }
    }

    /// carte glass avec titre + petit trait accent.
    fn group(&self, r: &Renderer, gx: f32, gy: f32, gw: f32, gh: f32, title: &str, a: f32) {
        r.fill_round(gx, gy, gw, gh, 10.0, with_a([0.07, 0.082, 0.115, 0.55], a));
        r.stroke_round(gx, gy, gw, gh, 10.0, with_a([1.0, 1.0, 1.0, 0.10], a), 1.0);
        r.text(title, &r.fmt_s, gx + 12.0, gy + 8.0, gw - 24.0, 14.0, with_a([0.66, 0.71, 0.78, 1.0], a), Align::L, false);
        r.grad_fill_round(gx + 12.0, gy + 26.0, 26.0, 2.0, 1.0, a);
    }

    /// checkbox glass (case arrondie, check blanc, hover animé).
    fn checkbox(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        id: u64,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        v: &mut bool,
        a: f32,
        dt: f32,
    ) {
        let hov = self.hover(id, m, x, y, w, 26.0, dt);
        if self.open && a > 0.85 && m.clicked && hit_r(m, x, y, w, 26.0) {
            *v = !*v;
            self.mark_dirty();
        }
        r.fill_round(x, y + 5.0, 16.0, 16.0, 4.0, with_a([0.10, 0.12, 0.16, 0.55 + 0.20 * hov], a));
        if *v {
            r.grad_fill_round(x, y + 5.0, 16.0, 16.0, 4.0, a);
        }
        r.stroke_round(x, y + 5.0, 16.0, 16.0, 4.0, with_a([1.0, 1.0, 1.0, 0.14 + 0.24 * hov], a), 1.0);
        if *v {
            r.line(x + 3.8, y + 12.6, x + 6.8, y + 15.6, with_a([1.0, 1.0, 1.0, 0.95], a), 1.8);
            r.line(x + 6.8, y + 15.6, x + 12.4, y + 9.4, with_a([1.0, 1.0, 1.0, 0.95], a), 1.8);
        }
        r.text(label, &r.fmt_b, x + 26.0, y, w - 30.0, 26.0, with_a(C_TEXT, a), Align::L, true);
    }

    /// slider (label + valeur mono, piste pill, remplissage dégradé, molette).
    fn slider(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        id: u64,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        min: f32,
        max: f32,
        v: &mut f32,
        unit: &str,
        a: f32,
        dt: f32,
    ) {
        let hov = self.hover(id, m, x, y - 2.0, w, 38.0, dt);
        let active = self.slider == Some(id);
        if active && !m.held {
            self.slider = None;
        }
        if self.open
            && a > 0.85
            && (active || (m.clicked && m.held && hit_r(m, x, y - 2.0, w, 38.0)))
        {
            if !active {
                self.slider = Some(id);
            }
            *v = min + (max - min) * ((m.x - x) / w).clamp(0.0, 1.0);
            self.mark_dirty();
        }
        let t = ((*v - min) / (max - min)).clamp(0.0, 1.0);
        r.text(label, &r.fmt_b, x, y, w - 80.0, 16.0, with_a(C_DIM, a), Align::L, false);
        let vs = format!("{:.0} {unit}", v);
        r.text(&vs, &r.fmt_mono, x + w - 80.0, y, 80.0, 16.0, with_a(C_TEXT, a), Align::R, false);
        // piste + remplissage accent
        r.fill_round(x, y + 22.0, w, 5.0, 2.5, with_a([1.0, 1.0, 1.0, 0.10], a));
        let fw = (w * t).max(6.0);
        r.grad_fill_round(x, y + 22.0, fw, 5.0, 2.5, a);
        // molette + halo (grossit au hover/drag)
        let kx = x + w * t;
        let ky = y + 24.5;
        let glow = hov.max(if active { 1.0 } else { 0.0 });
        r.dot(kx, ky, 10.0 + 2.0 * glow, with_a([ACCENT_A[0], ACCENT_A[1], ACCENT_A[2], 0.16], a));
        r.dot(kx, ky, 5.5, with_a([0.97, 0.98, 1.0, 1.0], a));
    }

    /// contrôle segmenté (3 mini-onglets dans une pill).
    fn segmented(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        id: u64,
        x: f32,
        y: f32,
        w: f32,
        options: &[&str],
        idx: &mut usize,
        a: f32,
        dt: f32,
    ) {
        let h = 26.0;
        r.fill_round(x, y, w, h, h * 0.5, with_a([0.08, 0.095, 0.13, 0.65], a));
        let cw = w / options.len() as f32;
        for (i, o) in options.iter().enumerate() {
            let cxi = x + i as f32 * cw;
            let hov = self.hover(id + i as u64, m, cxi, y, cw, h, dt);
            if m.clicked && self.open && a > 0.85 && hit_r(m, cxi, y, cw, h) {
                *idx = i;
                self.mark_dirty();
            }
            if i == *idx {
                r.grad_fill_round(cxi + 2.0, y + 2.0, cw - 4.0, h - 4.0, (h - 4.0) * 0.5, a);
                r.text(*o, &r.fmt_b, cxi, y, cw, h, with_a([1.0, 1.0, 1.0, 1.0], a), Align::C, true);
            } else {
                if hov > 0.01 {
                    r.fill_round(cxi + 2.0, y + 2.0, cw - 4.0, h - 4.0, (h - 4.0) * 0.5, with_a([1.0, 1.0, 1.0, 0.06 * hov], a));
                }
                r.text(*o, &r.fmt_b, cxi, y, cw, h, with_a(C_DIM, a), Align::C, true);
            }
        }
    }

    /// pastilles de palette (double pastille T/CT, anneau accent si active).
    fn swatches(&mut self, r: &Renderer, m: &Mouse, id: u64, x: f32, y: f32, cur: &mut usize, a: f32, dt: f32) {
        for i in 0..PALETTES.len() {
            let cx = x + 20.0 + i as f32 * 40.0;
            let cy = y + 16.0;
            let hov = self.hover(id + i as u64, m, cx - 13.0, cy - 13.0, 26.0, 26.0, dt);
            if m.clicked && self.open && a > 0.85 && hit_r(m, cx - 13.0, cy - 13.0, 26.0, 26.0) {
                *cur = i;
                self.mark_dirty();
            }
            let p = &PALETTES[i];
            r.dot(cx - 4.5, cy, 7.0, with_a(p.t, a));
            r.dot(cx + 4.5, cy, 7.0, with_a(p.ct, a));
            if i == *cur {
                r.ring(cx, cy, 12.5, with_a([ACCENT_A[0], ACCENT_A[1], ACCENT_A[2], 0.9], a), 1.4);
            } else if hov > 0.01 {
                r.ring(cx, cy, 12.0, with_a([1.0, 1.0, 1.0, 0.25 * hov], a), 1.2);
            }
        }
        r.text(PALETTES[*cur].name, &r.fmt_b, x + 194.0, y + 4.0, 80.0, 24.0, with_a(C_TEXT, a), Align::L, true);
    }

    /// bouton glass — retourne true si cliqué ce frame.
    fn button(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        id: u64,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: &str,
        danger: bool,
        a: f32,
        dt: f32,
    ) -> bool {
        let hov = self.hover(id, m, x, y, w, h, dt);
        let pressed = self.open && a > 0.85 && m.clicked && hit_r(m, x, y, w, h);
        let base: [f32; 4] = if danger { [0.16, 0.05, 0.06, 0.55] } else { [0.10, 0.115, 0.155, 0.50] };
        r.fill_round(
            x, y, w, h, 8.0,
            with_a([base[0] + 0.06 * hov, base[1] + 0.05 * hov, base[2] + 0.06 * hov, base[3] + 0.30 * hov], a),
        );
        let bc = if danger { C_DANGER } else { [1.0, 1.0, 1.0, 1.0] };
        r.stroke_round(x, y, w, h, 8.0, with_a([bc[0], bc[1], bc[2], 0.16 + 0.30 * hov], a), 1.0);
        let tc = if danger { mix4(bc, [1.0, 0.62, 0.62, 1.0], 0.45) } else { C_TEXT };
        r.text(label, &r.fmt_b, x, y, w, h, with_a(tc, a), Align::C, true);
        pressed
    }

    /// ligne label ↔ valeur — ancrage par position (valeurs mesurées, jamais
    /// de wrap: les textes longs s'étendent vers la gauche, sans clipping).
    fn info_row(&self, r: &Renderer, x: f32, y: f32, w: f32, label: &str, value: &str, a: f32) {
        r.text(label, &r.fmt_b, x, y, w * 0.5, 20.0, with_a(C_DIM, a), Align::L, true);
        let (vw, _) = r.measure(value, &r.fmt_mono);
        let vx = (x + w - vw - 2.0).max(x + w * 0.5);
        r.text(value, &r.fmt_mono, vx, y, vw + 4.0, 20.0, with_a(C_TEXT, a), Align::L, true);
    }
    // ── onglet ESP: boîtes, squelette, aperçu live, palette, filtres ──────
    fn tab_esp(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        opts: &mut Options,
        a: f32,
        dt: f32,
        x: f32,
        y: f32,
    ) {
        // carte L1 — Boîtes & squelette
        let (gx, gy) = (x + CX0, y + CY0);
        self.group(r, gx, gy, CW, 238.0, "BOÎTES & SQUELETTE", a);
        let modes = ["coins", "complet", "off"];
        self.segmented(r, m, 400, gx + 14.0, gy + 40.0, CW - 28.0, &modes, &mut opts.box_mode, a, dt);
        self.checkbox(r, m, 301, gx + 14.0, gy + 78.0, CW - 28.0, "remplissage translucide", &mut opts.box_fill, a, dt);
        self.checkbox(r, m, 302, gx + 14.0, gy + 106.0, CW - 28.0, "barre de vie", &mut opts.health, a, dt);
        self.checkbox(r, m, 303, gx + 14.0, gy + 134.0, CW - 28.0, "snaplines (vers ennemis)", &mut opts.snaplines, a, dt);
        self.checkbox(r, m, 304, gx + 14.0, gy + 162.0, CW - 28.0, "crosshair", &mut opts.crosshair, a, dt);
        self.checkbox(r, m, 305, gx + 14.0, gy + 190.0, CW - 28.0, "squelette (os reliés)", &mut opts.skeleton, a, dt);

        // carte L2 — Étiquettes & filtres (+ distance max)
        let (gx, gy) = (x + CX0, y + CY0 + 250.0);
        self.group(r, gx, gy, CW, 202.0, "ÉTIQUETTES & FILTRES", a);
        self.checkbox(r, m, 320, gx + 14.0, gy + 38.0, CW - 28.0, "noms des joueurs", &mut opts.names, a, dt);
        self.checkbox(r, m, 321, gx + 14.0, gy + 66.0, CW - 28.0, "distance (m)", &mut opts.distance, a, dt);
        self.checkbox(r, m, 322, gx + 14.0, gy + 94.0, CW - 28.0, "coéquipiers (F2)", &mut opts.teammates, a, dt);
        self.checkbox(r, m, 324, gx + 14.0, gy + 122.0, CW - 28.0, "distance ILLIMITÉE", &mut opts.unlimited_dist, a, dt);
        let mut v = opts.max_dist_m;
        self.slider(r, m, 323, gx + 14.0, gy + 156.0, CW - 28.0, "distance max", 20.0, 2000.0, &mut v, "m", a, dt);
        opts.max_dist_m = v;
        if opts.unlimited_dist {
            r.text("illimité actif — la limite ci-dessus est ignorée", &r.fmt_s, gx + 14.0, gy + 186.0, CW - 28.0, 14.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        }

        // carte R1 — Palette
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 210.0, "PALETTE", a);
        self.swatches(r, m, 310, gx + 14.0, gy + 38.0, &mut opts.palette, a, dt);
        let p = &PALETTES[opts.palette];
        let legend: [(&str, [f32; 4]); 3] = [("terroristes", p.t), ("anti-terroristes", p.ct), ("toi", p.me)];
        for (i, (lbl, c)) in legend.iter().enumerate() {
            let ly = gy + 84.0 + i as f32 * 28.0;
            r.dot(gx + 24.0, ly + 12.0, 5.0, with_a(*c, a));
            r.text(lbl, &r.fmt_b, gx + 38.0, ly, 200.0, 24.0, with_a(C_DIM, a), Align::L, true);
        }
        r.text("s'applique aux boîtes, squelette", &r.fmt_s, gx + 150.0, gy + 96.0, CW - 164.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        r.text("radar et snaplines", &r.fmt_s, gx + 150.0, gy + 112.0, CW - 164.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);

        // carte R2 — Aperçu en direct (mini-scène avec les réglages courants)
        let (gx, gy) = (x + CX1, y + CY0 + 222.0);
        self.group(r, gx, gy, CW, 202.0, "APERÇU EN DIRECT", a);
        self.preview(r, gx, gy, opts, a);
    }

    /// mini-scène: deux boîtes T/CT rendues avec les réglages COURANTS.
    fn preview(&self, r: &Renderer, gx: f32, gy: f32, opts: &Options, a: f32) {
        if !opts.esp {
            r.text("ESP désactivé", &r.fmt_b, gx + 12.0, gy + 36.0, CW - 24.0, 120.0, with_a(C_DIM, a), Align::C, true);
            return;
        }
        let nothing = opts.box_mode == 2 && !opts.names && !opts.distance && !opts.health && !opts.snaplines && !opts.skeleton;
        if nothing {
            r.text("aucun rendu ESP actif", &r.fmt_b, gx + 12.0, gy + 36.0, CW - 24.0, 120.0, with_a(C_DIM, a), Align::C, true);
            return;
        }
        let pal = &PALETTES[opts.palette];
        let items: [(&str, &str, [f32; 4], f32); 2] = [
            ("ennemi T", "18 m", pal.t, 0.7),
            ("ennemi CT", "42 m", pal.ct, 1.0),
        ];
        for (i, (name, dist, col, hp)) in items.iter().enumerate() {
            let bx = gx + 96.0 + i as f32 * 116.0;
            let by = gy + 66.0;
            let (bw, bh) = (46.0, 58.0);
            if opts.box_fill && opts.box_mode != 2 {
                r.fill_round(bx, by, bw, bh, 3.0, with_a([col[0] * 0.25, col[1] * 0.25, col[2] * 0.25, 0.38], a));
            }
            match opts.box_mode {
                0 => {
                    let l = 12.0;
                    let ac = mix4(*col, [1.0, 1.0, 1.0, 1.0], 0.35);
                    r.line(bx, by, bx + l, by, with_a(ac, a), 2.2);
                    r.line(bx, by, bx, by + l, with_a(ac, a), 2.2);
                    r.line(bx + bw - l, by, bx + bw, by, with_a(ac, a), 2.2);
                    r.line(bx + bw, by, bx + bw, by + l, with_a(ac, a), 2.2);
                    r.line(bx, by + bh - l, bx, by + bh, with_a(ac, a), 2.2);
                    r.line(bx, by + bh, bx + l, by + bh, with_a(ac, a), 2.2);
                    r.line(bx + bw - l, by + bh, bx + bw, by + bh, with_a(ac, a), 2.2);
                    r.line(bx + bw, by + bh - l, bx + bw, by + bh, with_a(ac, a), 2.2);
                }
                1 => r.stroke_round(bx, by, bw, bh, 3.0, with_a(*col, a), 1.6),
                _ => {}
            }
            // aperçu squelette: bonhomme dans la boîte (7 segments)
            if opts.skeleton {
                let sk = mix4(*col, [1.0, 1.0, 1.0, 1.0], 0.28);
                let skc = with_a(sk, 0.95);
                let (cx, hby) = (bx + bw * 0.5, by + 8.0);
                r.dot(cx, hby + 3.0, 2.2, skc);
                r.line(cx, hby + 6.0, cx, by + bh * 0.55, skc, 1.5);
                r.line(cx - 7.0, hby + 14.0, cx + 7.0, hby + 14.0, skc, 1.5);
                r.line(cx - 7.0, hby + 14.0, cx - 10.0, hby + 26.0, skc, 1.5);
                r.line(cx + 7.0, hby + 14.0, cx + 10.0, hby + 26.0, skc, 1.5);
                r.line(cx, by + bh * 0.55, cx - 8.0, by + bh - 4.0, skc, 1.5);
                r.line(cx, by + bh * 0.55, cx + 8.0, by + bh - 4.0, skc, 1.5);
            }
            if opts.health {
                r.fill_round(bx - 8.0, by, 4.0, bh, 1.5, with_a([1.0, 1.0, 1.0, 0.14], a));
                r.fill_round(bx - 8.0, by + bh * (1.0 - hp), 4.0, bh * hp, 1.5, with_a(mix4([1.0, 0.15, 0.15, 1.0], [0.15, 1.0, 0.35, 1.0], *hp), a));
            }
            if opts.names {
                let (nw, _) = r.measure(name, &r.fmt_s);
                let pw = nw + 12.0;
                r.fill_round(bx + bw * 0.5 - pw * 0.5, by - 24.0, pw, 18.0, 9.0, with_a([0.05, 0.06, 0.085, 0.75], a));
                r.text(name, &r.fmt_s, bx + bw * 0.5 - pw * 0.5, by - 24.0, pw, 18.0, with_a(C_TEXT, a), Align::C, true);
            }
            if opts.distance {
                let (dw, _) = r.measure(dist, &r.fmt_s);
                let pw = dw + 12.0;
                r.fill_round(bx + bw * 0.5 - pw * 0.5, by + bh + 5.0, pw, 18.0, 9.0, with_a([0.05, 0.06, 0.085, 0.75], a));
                r.text(dist, &r.fmt_s, bx + bw * 0.5 - pw * 0.5, by + bh + 5.0, pw, 18.0, with_a(C_DIM, a), Align::C, true);
            }
            if opts.snaplines {
                r.line(gx + CW * 0.5, gy + 158.0, bx + bw * 0.5, by + bh, with_a([col[0], col[1], col[2], 0.5], a), 1.2);
            }
        }
    }

    // ── onglet RADAR: minimap rotative ──────────────────────────────────────
    fn tab_radar(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        opts: &mut Options,
        a: f32,
        dt: f32,
        x: f32,
        y: f32,
    ) {
        // carte L1 — Radar
        let (gx, gy) = (x + CX0, y + CY0);
        self.group(r, gx, gy, CW, 210.0, "RADAR", a);
        self.checkbox(r, m, 340, gx + 14.0, gy + 40.0, CW - 28.0, "radar / minimap activé", &mut opts.radar, a, dt);
        let corners = ["haut G", "haut D", "bas G", "bas D"];
        self.segmented(r, m, 341, gx + 14.0, gy + 72.0, CW - 28.0, &corners, &mut opts.radar_corner, a, dt);
        self.checkbox(r, m, 342, gx + 14.0, gy + 112.0, CW - 28.0, "coéquipiers sur le radar", &mut opts.radar_mates, a, dt);
        r.text("la rotation suit ta vue", &r.fmt_s, gx + 14.0, gy + 146.0, CW - 28.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        r.text("point creux = hors portée (direction)", &r.fmt_s, gx + 14.0, gy + 162.0, CW - 28.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);

        // carte L2 — Réglages
        let (gx, gy) = (x + CX0, y + CY0 + 222.0);
        self.group(r, gx, gy, CW, 170.0, "RÉGLAGES", a);
        let mut sz = opts.radar_size;
        self.slider(r, m, 343, gx + 14.0, gy + 40.0, CW - 28.0, "taille", 120.0, 400.0, &mut sz, "px", a, dt);
        opts.radar_size = sz;
        let mut rg = opts.radar_range;
        self.slider(r, m, 344, gx + 14.0, gy + 92.0, CW - 28.0, "portée", 500.0, 6000.0, &mut rg, "u", a, dt);
        opts.radar_range = rg;

        // carte R1 — Légende
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 210.0, "LÉGENDE", a);
        let pal = &PALETTES[opts.palette];
        let rows: [(&str, bool, [f32; 4]); 4] = [
            ("terroriste (point plein)", true, pal.t),
            ("anti-terroriste (point plein)", true, pal.ct),
            ("hors portée (point creux)", false, [0.75, 0.78, 0.82, 1.0]),
            ("toi (flèche + cône)", true, pal.me),
        ];
        for (i, (lbl, full, c)) in rows.iter().enumerate() {
            let ly = gy + 44.0 + i as f32 * 32.0;
            if *full {
                r.dot(gx + 26.0, ly + 10.0, 5.0, with_a(*c, a));
                r.ring(gx + 26.0, ly + 10.0, 5.0, with_a([0.0, 0.0, 0.0, 0.45], a), 1.0);
            } else {
                r.ring(gx + 26.0, ly + 10.0, 4.0, with_a(*c, a), 1.6);
            }
            r.text(lbl, &r.fmt_b, gx + 40.0, ly, CW - 54.0, 24.0, with_a(C_DIM, a), Align::L, true);
        }

        // carte R2 — Aide
        let (gx, gy) = (x + CX1, y + CY0 + 222.0);
        self.group(r, gx, gy, CW, 170.0, "AIDE", a);
        let lines = [
            "· le radar tourne avec ta caméra:",
            "   ce qui est DEVANT toi est en HAUT",
            "· 2500 u ≈ la moitié d'une carte moyenne",
            "· coupé dans les menus du jeu (achat,",
            "   ESC) exactement comme l'ESP",
        ];
        for (i, s) in lines.iter().enumerate() {
            r.text(s, &r.fmt_s, gx + 14.0, gy + 40.0 + i as f32 * 22.0, CW - 28.0, 18.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        }
    }

    // ── onglet TRIGGER: tir automatique sur ennemi sous le viseur ───────
    fn tab_trigger(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        opts: &mut Options,
        st: &StatusInfo,
        a: f32,
        dt: f32,
        x: f32,
        y: f32,
    ) {
        // carte L1 — Triggerbot
        let (gx, gy) = (x + CX0, y + CY0);
        self.group(r, gx, gy, CW, 210.0, "TRIGGERBOT", a);
        self.checkbox(r, m, 350, gx + 14.0, gy + 40.0, CW - 28.0, "triggerbot activé", &mut opts.trigger, a, dt);
        let keys = ["ALT", "CTRL", "BTN4", "MAJ"];
        self.segmented(r, m, 351, gx + 14.0, gy + 72.0, CW - 28.0, &keys, &mut opts.trigger_key, a, dt);
        let mut d = opts.trigger_delay_ms;
        self.slider(r, m, 352, gx + 14.0, gy + 110.0, CW - 28.0, "délai de réaction", 0.0, 200.0, &mut d, "ms", a, dt);
        opts.trigger_delay_ms = d;
        r.text("vérification d'équipe TOUJOURS active", &r.fmt_s, gx + 14.0, gy + 152.0, CW - 28.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        r.text("(jamais de tir sur un coéquipier)", &r.fmt_s, gx + 14.0, gy + 168.0, CW - 28.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);

        // carte L2 — Statut live
        let (gx, gy) = (x + CX0, y + CY0 + 222.0);
        self.group(r, gx, gy, CW, 170.0, "STATUT", a);
        let trig_state = if st.trigger_armed {
            ("ARMÉ — ennemi sous le viseur", C_DANGER)
        } else if opts.trigger {
            ("repos — viseur libre", C_DIM)
        } else {
            ("désactivé", C_DIM)
        };
        self.info_row(r, gx + 14.0, gy + 40.0, CW - 28.0, "état", trig_state.0, a);
        self.info_row(r, gx + 14.0, gy + 66.0, CW - 28.0, "jeu au premier plan", if st.game_fg { "oui" } else { "NON (en pause)" }, a);
        self.info_row(r, gx + 14.0, gy + 92.0, CW - 28.0, "touche active", keys[opts.trigger_key], a);
        self.info_row(r, gx + 14.0, gy + 118.0, CW - 28.0, "délai", &format!("{:.0} ms", opts.trigger_delay_ms), a);

        // carte R1 — Fonctionnement
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 210.0, "FONCTIONNEMENT", a);
        let lines = [
            "1. maintiens la touche choisie (ALT)",
            "2. place ton viseur sur un ennemi",
            "3. le tir part tout seul après le délai",
            "",
            "· l'entité sous le viseur est lue via",
            "   m_iIDEntIndex (mémoire, lecture seule)",
            "· le clic est envoyé par SendInput —",
            "   comme un vrai clic de souris",
        ];
        for (i, s) in lines.iter().enumerate() {
            if s.is_empty() {
                continue;
            }
            r.text(s, &r.fmt_s, gx + 14.0, gy + 40.0 + i as f32 * 20.0, CW - 28.0, 18.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        }

        // carte R2 — Sécurités
        let (gx, gy) = (x + CX1, y + CY0 + 222.0);
        self.group(r, gx, gy, CW, 170.0, "SÉCURITÉS", a);
        let lines = [
            "· ne tire JAMAIS hors du jeu",
            "   (CS2 doit être au premier plan)",
            "· coupé dans les menus / achat / ESC",
            "· coupé quand CE menu est ouvert",
            "· coupé si tu es mort / spectateur",
        ];
        for (i, s) in lines.iter().enumerate() {
            r.text(s, &r.fmt_s, gx + 14.0, gy + 40.0 + i as f32 * 22.0, CW - 28.0, 18.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        }
    }

    // ── onglet DIVERS: overlay, actions, statut, session ────────────────────
    fn tab_divers(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        opts: &mut Options,
        st: &StatusInfo,
        a: f32,
        dt: f32,
        x: f32,
        y: f32,
        actions: &mut Vec<Action>,
    ) {
        // carte L1 — Overlay
        let (gx, gy) = (x + CX0, y + CY0);
        self.group(r, gx, gy, CW, 210.0, "OVERLAY", a);
        self.checkbox(r, m, 330, gx + 14.0, gy + 40.0, CW - 28.0, "ESP limité au jeu (cache Discord)", &mut opts.game_only, a, dt);
        self.checkbox(r, m, 336, gx + 14.0, gy + 68.0, CW - 28.0, "watermark (coin haut-gauche)", &mut opts.watermark, a, dt);
        self.checkbox(r, m, 331, gx + 14.0, gy + 96.0, CW - 28.0, "debug verbeux (F1)", &mut opts.verbose, a, dt);
        let mut iv = opts.interval_ms as f32;
        self.slider(r, m, 332, gx + 14.0, gy + 132.0, CW - 28.0, "cadence de boucle", 1.0, 50.0, &mut iv, "ms", a, dt);
        opts.interval_ms = iv.round().max(1.0) as u64;

        // carte L2 — Console & diagnostic
        let (gx, gy) = (x + CX0, y + CY0 + 222.0);
        self.group(r, gx, gy, CW, 170.0, "CONSOLE & DIAGNOSTIC", a);
        if self.button(r, m, 333, gx + 14.0, gy + 40.0, CW - 28.0, 32.0, "dump console (F4)", false, a, dt) {
            actions.push(Action::Dump);
        }
        if self.button(r, m, 334, gx + 14.0, gy + 82.0, CW - 28.0, 32.0, "panneau de diagnostic", false, a, dt) {
            actions.push(Action::ShowPanel);
        }
        r.text("le panneau affiche statut + journal complet en miroir de la console", &r.fmt_s, gx + 14.0, gy + 126.0, CW - 28.0, 34.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);

        // carte R1 — Statut
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 210.0, "STATUT", a);
        self.info_row(r, gx + 14.0, gy + 40.0, CW - 28.0, "pid", &st.pid.to_string(), a);
        self.info_row(r, gx + 14.0, gy + 64.0, CW - 28.0, "fps overlay", &format!("{:.0}", st.fps), a);
        self.info_row(r, gx + 14.0, gy + 88.0, CW - 28.0, "joueurs tracés", &st.players.to_string(), a);
        self.info_row(
            r, gx + 14.0, gy + 112.0, CW - 28.0, "snapshot",
            if st.snapshot_ok { "OK" } else { "AUCUNE — stale ?" }, a,
        );
        self.info_row(
            r, gx + 14.0, gy + 136.0, CW - 28.0, "rendu overlay",
            if st.renderer_ok { "OK" } else { "ÉCHEC — voir journal" }, a,
        );
        self.info_row(r, gx + 14.0, gy + 160.0, CW - 28.0, "offsets", "dump local 29.10.2026 · build 14193", a);

        // carte R2 — Session
        let (gx, gy) = (x + CX1, y + CY0 + 222.0);
        self.group(r, gx, gy, CW, 170.0, "SESSION", a);
        if self.button(r, m, 335, gx + 14.0, gy + 40.0, CW - 28.0, 34.0, "quitter (END)", true, a, dt) {
            actions.push(Action::Quit);
        }
        r.text("réglages sauvegardés automatiquement:", &r.fmt_s, gx + 14.0, gy + 88.0, CW - 28.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
        r.text(&st.config_path, &r.fmt_s, gx + 14.0, gy + 104.0, CW - 28.0, 16.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false);
    }

    // ── onglet INFO: statut complet + raccourcis + note ─────────────────────
    fn tab_info(&mut self, r: &Renderer, st: &StatusInfo, a: f32, x: f32, y: f32) {
        let (gx, gy) = (x + CX0, y + CY0);
        let gw = CW * 2.0 + PAD;
        self.group(r, gx, gy, gw, 392.0, "INFORMATIONS", a);

        self.info_row(r, gx + 14.0, gy + 40.0, gw - 28.0, "version", "v0.3.0 — squelette · radar · triggerbot · Space Grotesk", a);
        self.info_row(r, gx + 14.0, gy + 64.0, gw - 28.0, "police", &st.font, a);
        self.info_row(r, gx + 14.0, gy + 88.0, gw - 28.0, "pid", &st.pid.to_string(), a);
        self.info_row(r, gx + 14.0, gy + 112.0, gw - 28.0, "fps overlay", &format!("{:.0}", st.fps), a);
        self.info_row(r, gx + 14.0, gy + 136.0, gw - 28.0, "joueurs tracés", &st.players.to_string(), a);
        self.info_row(
            r, gx + 14.0, gy + 160.0, gw - 28.0, "snapshot",
            if st.snapshot_ok { "OK — entités résolues" } else { "AUCUNE — pas en partie OU offsets STALE" }, a,
        );
        self.info_row(r, gx + 14.0, gy + 184.0, gw - 28.0, "offsets", OFFSETS_NOTE, a);
        self.info_row(
            r, gx + 14.0, gy + 208.0, gw - 28.0, "rendu overlay",
            if st.renderer_ok { "OK — Direct2D + UpdateLayeredWindow (alpha par pixel)" } else { "ÉCHEC — voir journal / panneau" }, a,
        );

        // raccourcis — grille 2 colonnes
        r.line(gx + 14.0, gy + 238.0, gx + gw - 14.0, gy + 238.0, with_a([1.0, 1.0, 1.0, 0.07], a), 1.0);
        r.text("RACCOURCIS", &r.fmt_s, gx + 14.0, gy + 246.0, 200.0, 14.0, with_a([0.66, 0.71, 0.78, 1.0], a), Align::L, false);
        r.grad_fill_round(gx + 14.0, gy + 264.0, 26.0, 2.0, 1.0, a);
        let colw = (gw - 28.0) * 0.5;
        let keys: [(&str, &str); 6] = [
            ("INSERT", "ouvrir / fermer le menu"),
            ("END", "quitter proprement"),
            ("F1", "debug verbeux"),
            ("F2", "coéquipiers"),
            ("F4", "dump console"),
            ("ALT (tenu)", "triggerbot (si activé)"),
        ];
        for (i, (k, d)) in keys.iter().enumerate() {
            let (kx, ky) = (gx + 14.0 + (i as f32 % 2.0) * colw, gy + 272.0 + (i as f32 / 2.0).floor() * 24.0);
            let (kw, _) = r.measure(k, &r.fmt_mono);
            r.text(k, &r.fmt_mono, kx, ky, 90.0, 20.0, with_a([ACCENT_A[0], ACCENT_A[1], ACCENT_A[2], 0.9], a), Align::L, true);
            r.text(d, &r.fmt_b, kx + kw + 10.0, ky, colw - kw - 20.0, 20.0, with_a(C_DIM, a), Align::L, true);
        }

        r.text(
            "usage éducatif · lecture mémoire seule (ReadProcessMemory) · aucune écriture dans le jeu · pose un offsets.json à jour du dumper à côté de l'exe après chaque patch CS2",
            &r.fmt_s, gx + 14.0, gy + 356.0, gw - 28.0, 30.0, with_a([0.45, 0.50, 0.58, 1.0], a), Align::L, false,
        );
    }
}
