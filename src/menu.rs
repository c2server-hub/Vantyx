// menu — menu IN-OVERLAY glassmorphism PREMIUM, rendu Direct2D + DirectWrite.
//
// v0.6 — LA REFONTE:
//   · CATÉGORIES: COMBAT · VISUEL · MISC · THÈME (4 onglets denses)
//   · FOND ANIMÉ: 3 orbes lumineux dérivant derrière le verre (lissajous),
//     balayage de lumière sur la barre de titre, LED qui respire
//   · BOUTONS ANIMÉS: flash au clic (fondu 300 ms), hover halos, glow
//     pulsé sur l'onglet actif, indicateur qui glisse
//   · STUDIO DE COULEURS: thème CUSTOM complet — 6 rôles éditables en RGB
//     en direct (fond, cartes, accent, lumineux, texte, bordures), persisté
//     dans vantyx.ini, appliqué LIVE au menu + radar + watermark + ESP
//   · STREAMPROOF TOGGLE: l'anti screen-share est enfin désactivable au
//     menu (MISC → OVERLAY & CAPTURE) + raccourci F10
//   · BOMB TIMER: source d'horloge (serveur/local) affichée en statut
//
// MODE IMMÉDIAT (comme ImGui): chaque widget se dessine ET se teste à la
// souris dans la même passe — layout fixe, zéro rétention d'état par widget
// (seuls hover/drag/slider/click-flash vivent dans Menu).
//
// SOURIS: pollée (GetCursorPos + GetAsyncKeyState VK_LBUTTON) — aucun
// wndproc nécessaire. La boucle overlay appelle Mouse::update à ~1 kHz;
// `clicked` (front descendant) est CONSOMMÉ au frame menu (~166 Hz).
//
// CONFIG: vantyx.ini à côté de l'exe (std::fs pur, zéro API Win32) —
// chargé au boot, sauvegardé 400 ms après le dernier changement (sliders
// inclus) + à la fermeture du menu / arrêt.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::overlay::Renderer;
use crate::theme::{self, CustomColors, Theme, CUSTOM_IDX, THEME_COUNT};
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

// ── options — tout ce que le menu contrôle, persisté dans vantyx.ini ──────
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
    pub trigger_hitbox: usize, // 0 tout · 1 tête · 2 corps · 3 membres
    /// v0.9.6 — « tête à chaque tir »: quand l'aimbot tient une cible, le
    /// triggerbot ne clique QUE sur le gate physique tête (la balle partant
    /// maintenant atterrit dans la tête — aimcore::shot_lands_in_head)
    pub trigger_head_only: bool,
    // ── aimbot ──
    pub aim: bool,
    pub aim_mode: usize,     // v0.8 — 0 SMOOTH (convergence lissée) · 1 FLICK (snap tête burst)
    pub aim_key: usize,      // 0 alt · 1 ctrl · 2 btn4 · 3 maj · 4 auto
    pub aim_hitbox: usize,   // 0 tête · 1 cou · 2 poitrine · 3 corps
    pub aim_fov: f32,        // champ de visée en degrés
    pub aim_smooth: f32,     // lissage (diviseur du delta)
    pub aim_speed: f32,      // v0.9.1 — vitesse de visée 1-10 (gain × plafond, see aimcore::speed_factor)
    pub aim_sens: f32,       // sensibilité IN-GAME (degrés → counts souris)
    pub aim_fov_circle: bool,
    pub aim_sens_auto: bool, // sensibilité LUE dans le jeu (dwSensitivity)
    pub aim_pred_ms: f32,   // v0.9 — prédiction cibles mobiles (ms, aimcore)
    pub aim_lock: bool,     // v0.9 — verrouillage cible à hystérésis (anti stress groupe)
    pub aim_punch: f32,     // v0.9.3 — compensation recul dans la visée (0-1.5, défaut 1.0)
    pub esp_text_size: usize, // 0 petit · 1 moyen · 2 grand (noms ESP)
    pub game_only: bool, // ESP limité au jeu (caché hors premier plan / menus)
    pub watermark: bool,
    pub verbose: bool,
    pub interval_ms: u64,
    // ── thème ──
    pub theme: usize, // 0..4 presets · 5 = CUSTOM (studio de couleurs)
    pub custom: CustomColors, // v0.6 — couleurs du thème custom (RGB live)
    // ── modules ──
    pub rcs: bool,            // contrôle du recul (master enable)
    pub rcs_punch: bool,      // v0.7 — true: punch feed-forward (balistique intégrale) · false: delta-based
    pub rcs_strength: f32,    // 0..1
    // ── v0.7 — RAGE · v0.8 — MASTER TOGGLE (voie B gated) ──
    pub rage_mode: bool,     // v0.8 — MASTER: ON = handle RW ouvert (spinbot/silent armés)
                             // OFF = lecture seule intégrale, les sous-options dorment
    pub spin: bool,           // spinbot (anti-aim, écriture dwViewAngles)
    pub spin_mode: usize,     // 0 spin · 1 jitter · 2 backwards · 3 flutter · 4 GHOST (v0.9: write-restore, caméra stable)
    pub spin_speed: f32,      // °/s (mode spin)
    pub spin_pitch: usize,    // 0 garde · 1 bas (-89°) · 2 haut (+89°)
    pub spin_ghost_us: f32,   // v0.9 — fenêtre ghost (mode 4) en µs: durée pendant laquelle le serveur voit l'angle spunné avant restauration
    pub thirdperson: bool,    // v0.9 — caméra 3e personne (flag CSGOInput, voie RAGE)
    pub tp_patch: bool,       // v0.9 — 3e pers. niveau 2: scan + patch du gate moteur (opt-in)
    pub silent: bool,         // silent aim (clic réel = tir déplacé)
    pub silent_fov: f32,      // FOV propre du silent aim
    pub silent_hitbox: usize, // hitbox du flick (0 tête · 1 cou · 2 poitrine · 3 corps)
    pub streamproof: bool,    // WDA_EXCLUDEFROMCAPTURE sur l'overlay
    pub bomb_timer: bool,     // bandeau bombe plantée
    pub bomb_esp: bool,       // v0.6.1 — ESP bombe: marqueur monde sur le C4 plantée
    pub spectators: bool,     // liste de ceux qui te regardent
    pub weapon_esp: bool,     // armes au sol (filtrées: sans propriétaire)
    pub sound_esp: bool,      // anneaux de pas
    pub bhop: bool,           // bunnyhop (espace conditionné FL_ONGROUND)
    pub grenade_trails: bool, // trajectoires des grenades
    pub hitbox: bool,         // zones de collision (tête/poitrine/corps/membres)
    pub accuracy: bool,       // labo de précision (stats session)
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
            trigger_delay_ms: 12.0, // v0.9.1 — 35 → 12 ms: délai humain mais vif
            trigger_hitbox: 0,
            trigger_head_only: true, // v0.9.6 — chaque balle part pour la tête
            aim: false,
            aim_mode: 0,
            aim_key: 2, // BTN4 par défaut — ALT est pris par le triggerbot
            aim_hitbox: 0,
            aim_fov: 5.0,
            aim_smooth: 2.0, // v0.9.1 — 3.0 → 2.0: suivi de cible plus réactif
            aim_speed: 7.0, // v0.9.1 — facteur 1.55: le tracking gagne contre un strafer
            aim_sens: 2.5,
            aim_fov_circle: true,
            aim_sens_auto: true,
            aim_pred_ms: 25.0, // v0.9 — 25 ms: strafers touchés en tête sans overshoot
            aim_lock: true,    // v0.9 — verrouillage ON par défaut: le fix "stresse en groupe"
            aim_punch: 1.0,    // v0.9.3 — compense le recul à 100% (0 = off)
            esp_text_size: 1,
            game_only: true,
            watermark: true,
            verbose: false,
            interval_ms: 1,
            theme: 0, // Void Purple — le choix de dj
            custom: CustomColors::default(),
            rcs: true,
            rcs_punch: false,
            rcs_strength: 0.85,
            rage_mode: false, // v0.8 — LEGIT par défaut: lecture seule au boot
            spin: false,
            spin_mode: 4, // v0.9 — GHOST par défaut: caméra stable + serveur spunné
            spin_speed: 1080.0, // 3 tours/s — l'anti-aim classique
            spin_pitch: 0,
            spin_ghost_us: 700.0, // v0.9 — fenêtre ghost: laisse le usercmd consommer l'angle, restaure avant le rendu
            thirdperson: false,
            tp_patch: false,
            silent: false,
            silent_fov: 8.0,
            silent_hitbox: 0,
            streamproof: true,
            bomb_timer: true,
            bomb_esp: true,
            spectators: true,
            weapon_esp: true,
            sound_esp: true,
            bhop: false,
            grenade_trails: true,
            hitbox: false,
            accuracy: false,
        }
    }
}

/// statut live injecté dans les onglets MISC + footer.
#[allow(dead_code)] // champs API: écrits par la boucle overlay, lus à la demande
pub struct StatusInfo {
    pub pid: u32,
    pub fps: f32,
    pub players: usize,
    pub snapshot_ok: bool,
    pub renderer_ok: bool,
    pub config_path: String,
    pub trigger_armed: bool, // viseur sur ennemi + touche tenue
    /// v0.9.6 — gate tête physique: la balle partant maintenant atterrit
    /// dans la tête (aimbot actif + écart ≤ rayon angulaire réel)
    pub trigger_head_ready: bool,
    pub game_fg: bool,       // CS2 au premier plan ?
    pub font: String,        // police en service (Space Grotesk / repli)
    // aimbot
    pub aim_active: bool,
    pub aim_target: String,
    pub aim_delta: f32,
    pub aim_send_rate: f32,  // envois/s (≈ framerate du jeu si actif)
    pub aim_flicks: u32,     // v0.8 — flicks envoyés cette session
    pub view_src: &'static str, // "dwViewAngles" | "m_angEyeAngles"
    pub sens_effective: Option<f32>, // sensibilité LUE dans le jeu
    // v0.9.1 — VISUELS DE VISÉE (monde → projetés par l'overlay):
    /// hitbox BRUTE de la cible visée (tête par défaut) — None si pas de lock
    pub aim_vis: Option<(f32, f32, f32)>,
    /// point PRÉDIT (hitbox + v·lead) — là où la balle partira réellement
    pub aim_vis_pred: Option<(f32, f32, f32)>,
    // v0.8 — état du handle RW (bandeau RAGE: LEGIT vert / RAGE rouge)
    pub rage_on: bool,
    // modules
    pub rcs_active: bool,
    pub rcs_kick: (f32, f32),
    // v0.7 — rage
    pub spin_active: bool,
    pub spin_written: (f32, f32),
    // v0.9 — caméra 3e personne
    pub tp_active: bool,
    pub tp_mode: &'static str,
    /// v0.9 — changements de cible aimbot cette session (diagnostic stress)
    pub aim_switches: u32,
    // v0.9.2 — modèle de mouvement adaptatif (prédiction apprise)
    pub aim_learn: u32,   // échantillons d'apprentissage cumulés
    pub aim_w_lead: f32,  // poids de lead appris (1.0 = neutre)
    pub aim_err_u: f32,   // dernière erreur de prédiction observée (u)
    // v0.9.3 — recul dans la visée
    pub aim_punch_on: bool, // viseur réellement DANS la tête (deadzone courante)
    pub aim_punch_deg: f32, // punch compensé (degrés, module)
    pub silent_armed: bool,
    pub silent_flicks: u32,
    pub silent_target: String,
    pub nr_active: bool,
    pub nr_last: (f32, f32),
    pub nr_punch: Option<(f32, f32)>,
    pub bomb_planted: bool,
    pub bomb_remaining_s: f32,
    pub bomb_clock: &'static str, // v0.6 — "serveur" | "local"
    pub spectator_count: usize,
    pub acc_shots: u32,
    pub acc_hits: u32,
    pub acc_kills: u32,
    pub acc_damage: u32,
    pub acc_weapon: String,
    pub bhop_jumps: u32,
    pub world_items: usize,
    pub grenades_live: usize,
}

#[allow(dead_code)] // doc visible dans le code — re-dump attendu au prochain patch CS2
pub const OFFSETS_NOTE: &str = "dump local 29.10.2026 · build 14193 · pins corrigés v0.6";

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

// ── couleurs du menu — tout dérive du thème LIVE (preset OU custom) ────────
// le rouge danger reste fonctionnel (indépendant du thème)
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
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("vantyx.ini")))
}

/// chemin effectif de LECTURE: vantyx.ini sinon cs2-esp.ini (migration)
fn config_read_path() -> Option<PathBuf> {
    let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) else {
        return None;
    };
    if dir.join("vantyx.ini").exists() {
        return Some(dir.join("vantyx.ini"));
    }
    if dir.join("cs2-esp.ini").exists() {
        return Some(dir.join("cs2-esp.ini"));
    }
    None
}

/// charge un hex ini → couleur custom (alpha du rôle préservé)
fn ini_hex(s: &str, cur: [f32; 4]) -> [f32; 4] {
    match theme::from_hex(s) {
        Some(mut c) => {
            c[3] = cur[3]; // alpha géré par rôle (border reste translucide)
            c
        }
        None => cur,
    }
}

pub fn load_options() -> (Options, Option<(f32, f32)>) {
    let mut o = Options::default();
    let mut pos: Option<(f32, f32)> = None;
    let Some(p) = config_read_path() else { return (o, pos) };
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
            "trigger_hitbox" => { o.trigger_hitbox = v.parse().unwrap_or(0).min(3); got += 1; }
            "trigger_head_only" => { o.trigger_head_only = v == "1"; got += 1; }
            "aim" => { o.aim = v == "1"; got += 1; }
            "aim_mode" => { o.aim_mode = v.parse().unwrap_or(0).min(1); got += 1; }
            "aim_key" => { o.aim_key = v.parse().unwrap_or(2).min(4); got += 1; }
            "aim_hitbox" => { o.aim_hitbox = v.parse().unwrap_or(0).min(3); got += 1; }
            "aim_fov" => { o.aim_fov = v.parse::<f32>().unwrap_or(5.0).clamp(1.0, 30.0); got += 1; }
            "aim_smooth" => { o.aim_smooth = v.parse::<f32>().unwrap_or(2.0).clamp(0.0, 20.0); got += 1; }
            "aim_speed" => { o.aim_speed = v.parse::<f32>().unwrap_or(7.0).clamp(1.0, 10.0); got += 1; }
            "aim_sens" => { o.aim_sens = v.parse::<f32>().unwrap_or(2.5).clamp(0.1, 8.0); got += 1; }
            "aim_fov_circle" => { o.aim_fov_circle = v == "1"; got += 1; }
            "aim_sens_auto" => { o.aim_sens_auto = v == "1"; got += 1; }
            "aim_pred_ms" => { o.aim_pred_ms = v.parse::<f32>().unwrap_or(25.0).clamp(0.0, 150.0); got += 1; }
            "aim_lock" => { o.aim_lock = v == "1"; got += 1; }
            "aim_punch" => { o.aim_punch = v.parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.5); got += 1; }
            "esp_text_size" => { o.esp_text_size = v.parse().unwrap_or(1).min(2); got += 1; }
            "game_only" => { o.game_only = v == "1"; got += 1; }
            "watermark" => { o.watermark = v == "1"; got += 1; }
            "verbose" => { o.verbose = v == "1"; got += 1; }
            "interval_ms" => { o.interval_ms = v.parse().unwrap_or(1).clamp(1, 50); got += 1; }
            "theme" => { o.theme = v.parse().unwrap_or(0).min(THEME_COUNT - 1); got += 1; }
            // v0.6 — couleurs custom (hex "#RRGGBB")
            "custom_bg" => { o.custom.bg = ini_hex(v, o.custom.bg); got += 1; }
            "custom_surface" => { o.custom.surface = ini_hex(v, o.custom.surface); got += 1; }
            "custom_primary" => { o.custom.primary = ini_hex(v, o.custom.primary); got += 1; }
            "custom_light" => { o.custom.light = ini_hex(v, o.custom.light); got += 1; }
            "custom_text" => { o.custom.text = ini_hex(v, o.custom.text); got += 1; }
            "custom_border" => { o.custom.border = ini_hex(v, o.custom.border); got += 1; }
            "rcs" => { o.rcs = v == "1"; got += 1; }
            "rcs_punch" => { o.rcs_punch = v == "1"; got += 1; }
            "rcs_strength" => { o.rcs_strength = v.parse::<f32>().unwrap_or(0.85).clamp(0.0, 1.0); got += 1; }
            "rage_mode" => { o.rage_mode = v == "1"; got += 1; }
            "spin" => { o.spin = v == "1"; got += 1; }
            "spin_mode" => { o.spin_mode = v.parse().unwrap_or(0).min(4); got += 1; } // v0.9 — min(4): GHOST doit survivre au reload
            "spin_speed" => { o.spin_speed = v.parse::<f32>().unwrap_or(1080.0).clamp(90.0, 2160.0); got += 1; }
            "spin_pitch" => { o.spin_pitch = v.parse().unwrap_or(0).min(2); got += 1; }
            "spin_ghost_us" => { o.spin_ghost_us = v.parse::<f32>().unwrap_or(700.0).clamp(150.0, 3000.0); got += 1; }
            "thirdperson" => { o.thirdperson = v == "1"; got += 1; }
            "tp_patch" => { o.tp_patch = v == "1"; got += 1; }
            "silent" => { o.silent = v == "1"; got += 1; }
            "silent_fov" => { o.silent_fov = v.parse::<f32>().unwrap_or(8.0).clamp(1.0, 30.0); got += 1; }
            "silent_hitbox" => { o.silent_hitbox = v.parse().unwrap_or(0).min(3); got += 1; }
            "streamproof" => { o.streamproof = v == "1"; got += 1; }
            "bomb_timer" => { o.bomb_timer = v == "1"; got += 1; }
            "bomb_esp" => { o.bomb_esp = v == "1"; got += 1; }
            "spectators" => { o.spectators = v == "1"; got += 1; }
            "weapon_esp" => { o.weapon_esp = v == "1"; got += 1; }
            "sound_esp" => { o.sound_esp = v == "1"; got += 1; }
            "bhop" => { o.bhop = v == "1"; got += 1; }
            "grenade_trails" => { o.grenade_trails = v == "1"; got += 1; }
            "hitbox" => { o.hitbox = v == "1"; got += 1; }
            "accuracy" => { o.accuracy = v == "1"; got += 1; }
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
        "[visuals]\nesp={}\nbox_mode={}\nbox_fill={}\nhealth={}\nsnaplines={}\ncrosshair={}\nskeleton={}\nnames={}\ndistance={}\nteammates={}\nunlimited_dist={}\nmax_dist_m={:.0}\npalette={}\nesp_text_size={}\nhitbox={}\n\n[radar]\nradar={}\nradar_corner={}\nradar_size={:.0}\nradar_range={:.0}\nradar_mates={}\n\n[trigger]\ntrigger={}\ntrigger_key={}\ntrigger_hitbox={}\ntrigger_head_only={}\ntrigger_delay_ms={:.0}\n\n[aimbot]\naim={}\naim_mode={}\naim_key={}\naim_hitbox={}\naim_fov={:.1}\naim_smooth={:.1}\naim_speed={:.0}\naim_sens={:.2}\naim_sens_auto={}\naim_fov_circle={}\naim_pred_ms={:.0}\naim_lock={}\naim_punch={:.2}\n\n[rage]\nrage_mode={}\nspin={}\nspin_mode={}\nspin_speed={:.0}\nspin_pitch={}\nspin_ghost_us={:.0}\nsilent={}\nsilent_fov={:.1}\nsilent_hitbox={}\nthirdperson={}\ntp_patch={}\n\n[outils]\nrcs={}\nrcs_punch={}\nrcs_strength={:.2}\nbomb_timer={}\nbomb_esp={}\nweapon_esp={}\nsound_esp={}\ngrenade_trails={}\nbhop={}\naccuracy={}\n\n[misc]\ngame_only={}\nwatermark={}\nverbose={}\ninterval_ms={}\ntheme={}\nstreamproof={}\nspectators={}\n\n[theme-custom]\ncustom_bg={}\ncustom_surface={}\ncustom_primary={}\ncustom_light={}\ncustom_text={}\ncustom_border={}\n\n[ui]\nui_x={:.0}\nui_y={:.0}\n",
        b(opts.esp), opts.box_mode, b(opts.box_fill), b(opts.health), b(opts.snaplines),
        b(opts.crosshair), b(opts.skeleton), b(opts.names), b(opts.distance), b(opts.teammates),
        b(opts.unlimited_dist), opts.max_dist_m, opts.palette, opts.esp_text_size, b(opts.hitbox),
        b(opts.radar), opts.radar_corner, opts.radar_size, opts.radar_range, b(opts.radar_mates),
        b(opts.trigger), opts.trigger_key, opts.trigger_hitbox, b(opts.trigger_head_only), opts.trigger_delay_ms,
        b(opts.aim), opts.aim_mode, opts.aim_key, opts.aim_hitbox, opts.aim_fov, opts.aim_smooth, opts.aim_speed, opts.aim_sens,
        b(opts.aim_sens_auto), b(opts.aim_fov_circle), opts.aim_pred_ms, b(opts.aim_lock),
        opts.aim_punch,
        // rage (v0.8: rage_mode = master — handle RW · v0.9: ghost + 3e personne)
        b(opts.rage_mode), b(opts.spin), opts.spin_mode, opts.spin_speed, opts.spin_pitch,
        opts.spin_ghost_us,
        b(opts.silent), opts.silent_fov, opts.silent_hitbox, b(opts.thirdperson), b(opts.tp_patch),
        b(opts.rcs), b(opts.rcs_punch), opts.rcs_strength, b(opts.bomb_timer), b(opts.bomb_esp),
        b(opts.weapon_esp), b(opts.sound_esp), b(opts.grenade_trails), b(opts.bhop), b(opts.accuracy),
        b(opts.game_only), b(opts.watermark), b(opts.verbose), opts.interval_ms,
        opts.theme, b(opts.streamproof), b(opts.spectators),
        theme::to_hex(opts.custom.bg), theme::to_hex(opts.custom.surface),
        theme::to_hex(opts.custom.primary), theme::to_hex(opts.custom.light),
        theme::to_hex(opts.custom.text), theme::to_hex(opts.custom.border),
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
    clicks: HashMap<u64, Instant>, // v0.6 — flash de clic (fondu 300 ms)
    dirty_at: Option<Instant>,
    t: f32, // horloge anims (orbes / LED / balayage)
    sw: f32,
    sh: f32,
    cur_theme: usize,        // resynchronisé depuis opts.theme à chaque frame
    cur_custom: CustomColors, // v0.6 — idem pour les couleurs custom
    color_role: usize,        // v0.6 — rôle en cours d'édition dans le studio
}

// accès thème LIVE depuis Menu (resynchronisé chaque frame)
impl Menu {
    fn th(&self) -> Theme {
        theme::resolve(self.cur_theme, &self.cur_custom)
    }
    fn t_primary(&self) -> [f32; 4] { self.th().primary }
    fn t_text(&self) -> [f32; 4] { self.th().text }
    fn t_dim(&self) -> [f32; 4] { theme::dim(&self.th()) }
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
            clicks: HashMap::new(),
            dirty_at: None,
            t: 0.0,
            sw: sw as f32,
            sh: sh as f32,
            cur_theme: 0,
            cur_custom: CustomColors::default(),
            color_role: 2, // ACCENT par défaut dans le studio
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

    /// v0.6 — enregistre un clic pour le flash animé (fondu 300 ms).
    fn flash(&mut self, id: u64) {
        self.clicks.insert(id, Instant::now());
    }

    /// intensité du flash de clic (1 → 0 sur 300 ms).
    fn flash_amt(&self, id: u64) -> f32 {
        match self.clicks.get(&id) {
            Some(at) => {
                let e = at.elapsed().as_secs_f32();
                if e < 0.30 {
                    1.0 - e / 0.30
                } else {
                    0.0
                }
            }
            None => 0.0,
        }
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
        self.cur_theme = opts.theme; // thème live — chaque widget dérive de lui
        self.cur_custom = opts.custom;
        let was_visible = self.anim > 0.0;
        let target = if self.open { 1.0 } else { 0.0 };
        self.anim += (target - self.anim) * (dt * 10.0).min(1.0);
        if (target - self.anim).abs() < 0.012 {
            self.anim = target;
        }
        // purge des flashes expirés (une fois par frame, coût nul)
        self.clicks.retain(|_, at| at.elapsed().as_secs_f32() < 0.30);
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

    // ── dessin principal (ombre, verre, ORBES ANIMÉS, titre, sidebar, footer) ──
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
        let th = self.th();

        // ombre douce multi-couches (le "flou" du glassmorphism, simulé)
        r.fill_round(x - 18.0, y - 12.0, W + 36.0, H + 30.0, 30.0, with_a([0.0, 0.0, 0.0, 0.14], a));
        r.fill_round(x - 10.0, y - 6.0, W + 20.0, H + 18.0, 22.0, with_a([0.0, 0.0, 0.0, 0.18], a));
        r.fill_round(x - 4.0, y - 2.0, W + 8.0, H + 8.0, 16.0, with_a([0.0, 0.0, 0.0, 0.22], a));

        // panneau verre principal — translucide RÉEL (le jeu transparaît),
        // teinté par le thème LIVE (preset OU custom)
        r.fill_round(x, y, W, H, 12.0, with_a(theme::glass_bg(&th, 1.0), a));
        r.sheen(x, y, W, 96.0, a); // reflet supérieur
        r.stroke_round(x, y, W, H, 12.0, with_a([th.border[0], th.border[1], th.border[2], th.border[3] * 1.6], a), 1.0);

        // ═══ v0.6 — FOND ANIMÉ: 3 orbes lumineux dérivant derrière le verre ═══
        // lissajous lents (périodes 20-48 s) — ils traversent le panneau en
        // restant dedans, ambiance "salon premium". Chaque orbe = halo 3
        // couches (blob doux simulé), couleurs: accent / lumineux / blanc.
        let t = self.t;
        // (couleur, phase_x, phx, phase_y, phy, rayon) — centres BORNÉS pour
        // que centre+rayon reste DANS le panneau (zéro blob hors verre)
        let orbs: [([f32; 4], f32, f32, f32, f32, f32); 3] = [
            (th.primary, 0.31, 0.0, 0.38, 1.7, 120.0),
            (th.light, 0.62, 2.1, 0.52, 0.8, 150.0),
            ([1.0, 1.0, 1.0, 1.0], 0.50, 4.0, 0.66, 2.9, 90.0),
        ];
        for (col, px, phx, py, phy, rad) in orbs {
            let ocx = x + W * (px + 0.17 * (t * 0.31 + phx).sin());
            let ocy = y + H * (py + 0.20 * (t * 0.23 + phy).cos());
            // 3 couches: halo large très faible → cœur plus dense
            for (k, alpha) in [(1.0, 0.028), (0.62, 0.05), (0.34, 0.085)] {
                r.dot(ocx, ocy, rad * k, with_a([col[0], col[1], col[2], alpha], a));
            }
        }

        // ── barre de titre (draggable) ──
        r.fill_round(x, y, W, TITLE_H, 12.0, with_a(theme::glass_title(&th, 1.0), a));
        r.fill(x, y + 12.0, W, TITLE_H - 12.0, with_a(theme::glass_title(&th, 1.0), a));
        r.line(x, y + TITLE_H, x + W, y + TITLE_H, with_a([th.border[0], th.border[1], th.border[2], th.border[3] * 1.4], a), 1.0);
        // ═══ v0.6 — BALAYAGE DE LUMIÈRE sur la barre de titre (7 s de cycle) ═══
        // dessiné APRÈS le remplissage (visible dessus) et CLIPPÉ au panneau
        {
            let period = 7.0;
            let ph = (t % period) / period; // 0..1
            let sx = x - 140.0 + ph * (W + 280.0);
            let lx = (sx - x).max(0.0);
            let rx = (sx + 140.0 - x).min(W);
            if rx > lx {
                r.grad_band(x + lx, y, rx - lx, TITLE_H, 0.16 * a);
            }
        }
        let lw = r.measure("Vantyx.cc", &r.fmt_h).0;
        r.text_grad("Vantyx.cc", &r.fmt_h, x + 18.0, y, 130.0, TITLE_H, Align::L, true, a);
        r.text("v0.6.1", &r.fmt_s, x + 18.0 + lw + 8.0, y, 70.0, TITLE_H, with_a(self.t_dim(), a), Align::L, true);

        // v0.6 — pastille ÉTAT en haut à droite: LED qui respire + fps
        {
            let pill_w = 118.0;
            let (px, py) = (x + W - pill_w - 44.0, y + 11.0);
            r.fill_round(px, py, pill_w, 24.0, 12.0, with_a(theme::glass_well(&th, 0.6), a));
            let led = 0.55 + 0.35 * (t * 2.4).sin();
            let pr = self.t_primary();
            let online = st.game_fg;
            let (dot_c, label) = if online {
                ([pr[0], pr[1], pr[2], 0.55 + 0.40 * led], "EN LIGNE")
            } else {
                ([1.0, 0.62, 0.20, 0.5], "EN PAUSE")
            };
            r.ring(px + 16.0, py + 12.0, 5.5, with_a([dot_c[0], dot_c[1], dot_c[2], dot_c[2].min(0.35)], a), 1.4);
            r.dot(px + 16.0, py + 12.0, 2.6, with_a(dot_c, a));
            r.text(label, &r.fmt_s, px + 28.0, py, pill_w - 34.0, 24.0, with_a(self.t_dim(), a), Align::L, true);
        }

        // bouton fermer (ferme le menu — INSERT le rouvre)
        let (cbx, cby) = (x + W - 36.0, y + 8.0);
        let ch = self.hover(99, m, cbx, cby, 28.0, 30.0, dt);
        if m.clicked && self.open && a > 0.85 && hit_r(m, cbx, cby, 28.0, 30.0) {
            self.open = false;
            self.flash(99);
            self.mark_dirty();
        }
        let cflash = self.flash_amt(99);
        if ch > 0.01 {
            r.fill_round(cbx, cby, 28.0, 30.0, 8.0, with_a([1.0, 1.0, 1.0, 0.10 * ch], a));
        }
        if cflash > 0.0 {
            r.fill_round(cbx, cby, 28.0, 30.0, 8.0, with_a([0.95, 0.30, 0.35, 0.35 * cflash], a));
        }
        r.line(cbx + 9.0, cby + 9.0, cbx + 19.0, cby + 21.0, with_a([0.95, 0.96, 0.98, 0.85], a), 1.6);
        r.line(cbx + 19.0, cby + 9.0, cbx + 9.0, cby + 21.0, with_a([0.95, 0.96, 0.98, 0.85], a), 1.6);

        // drag par la barre de titre (hors bouton fermer / pastille)
        if self.open && a > 0.85 {
            if m.held && m.clicked && hit_r(m, x, y, W - 170.0, TITLE_H) {
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
        r.fill(x, y + TITLE_H, SIDE_W, H - TITLE_H - FOOT_H, with_a([th.surface[0], th.surface[1], th.surface[2], 0.50], a));
        r.line(x + SIDE_W, y + TITLE_H, x + SIDE_W, y + H - FOOT_H, with_a([th.border[0], th.border[1], th.border[2], th.border[3] * 1.2], a), 1.0);

        // indicateur d'onglet animé (glisse + RESPIRE vers l'onglet actif)
        let target_y = y + 62.0 + self.tab as f32 * 52.0;
        self.tab_y += (target_y - self.tab_y) * (dt * 14.0).min(1.0);
        let glow = 0.75 + 0.25 * (t * 2.2).sin();
        r.grad_fill_round(x + 8.0, self.tab_y + 5.0, 3.0, 34.0, 1.5, a * glow);

        // v0.8 — 6 CATÉGORIES: COMBAT · RAGE · VISUEL · MOUVEMENT · MISC · THÈME
        const TABS: [&str; 6] = ["COMBAT", "RAGE", "VISUEL", "MOUVEMENT", "MISC", "THÈME"];
        for (i, label) in TABS.iter().enumerate() {
            let iy = y + 62.0 + i as f32 * 52.0;
            let hov = self.hover(200 + i as u64, m, x + 8.0, iy, SIDE_W - 16.0, 44.0, dt);
            if i == self.tab {
                // glow pulsé sur l'onglet actif (respire avec l'horloge)
                let ga = 0.10 + 0.04 * (t * 2.2).sin();
                r.fill_round(x + 8.0, iy, SIDE_W - 16.0, 44.0, 8.0, with_a([th.primary[0], th.primary[1], th.primary[2], ga], a));
            } else if hov > 0.01 {
                r.fill_round(x + 8.0, iy, SIDE_W - 16.0, 44.0, 8.0, with_a([1.0, 1.0, 1.0, 0.04 * hov], a));
            }
            if m.clicked && self.open && a > 0.85 && hit_r(m, x + 8.0, iy, SIDE_W - 16.0, 44.0) {
                self.tab = i;
                self.flash(200 + i as u64);
            }
            let c = if i == self.tab { with_a(self.t_text(), a) } else { with_a(self.t_dim(), a) };
            self.tab_icon(r, i, x + 26.0, iy + 22.0, c);
            r.text(label, &r.fmt_b, x + 48.0, iy, SIDE_W - 60.0, 44.0, c, Align::L, true);
        }

        // bas de sidebar: build + LED qui respire (couleur primaire du thème)
        r.text("Vantyx.cc v0.9.6", &r.fmt_s, x + 14.0, y + H - FOOT_H - 56.0, SIDE_W - 24.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("build 14193 · v0.9.6 · tête à chaque tir", &r.fmt_s, x + 14.0, y + H - FOOT_H - 40.0, SIDE_W - 24.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        let led = 0.55 + 0.25 * (t * 2.2).sin();
        let led_y = y + H - FOOT_H - 14.0;
        let pr = self.t_primary();
        r.ring(x + 21.0, led_y, 7.5, with_a([pr[0], pr[1], pr[2], 0.14 + 0.10 * led], a), 2.0);
        r.dot(x + 21.0, led_y, 3.5, with_a([pr[0], pr[1], pr[2], 0.55 + 0.40 * led], a));
        r.text("session active", &r.fmt_s, x + 36.0, led_y - 8.0, 108.0, 16.0, with_a(self.t_dim(), a), Align::L, true);

        // ── contenu de l'onglet actif (v0.8: 6 onglets) ──
        match self.tab {
            0 => self.tab_combat(r, m, opts, st, a, dt, x, y),
            1 => self.tab_rage(r, m, opts, st, a, dt, x, y),
            2 => self.tab_visuel(r, m, opts, a, dt, x, y),
            3 => self.tab_mouvement(r, m, opts, st, a, dt, x, y),
            4 => self.tab_misc(r, m, opts, st, a, dt, x, y, actions),
            _ => self.tab_theme(r, m, opts, a, dt, x, y),
        }

        // ── footer ──
        r.line(x, y + H - FOOT_H, x + W, y + H - FOOT_H, with_a([th.border[0], th.border[1], th.border[2], th.border[3] * 1.2], a), 1.0);
        r.text(
            "INSERT menu · END quitter · F10 anti screen-share · F1 debug · F4 dump",
            &r.fmt_s, x + 16.0, y + H - FOOT_H, 620.0, FOOT_H, with_a(self.t_dim(), a), Align::L, true,
        );
        r.text(format!("{:.0} fps", st.fps).as_str(), &r.fmt_mono, x + W - 100.0, y + H - FOOT_H, 84.0, FOOT_H, with_a(self.t_dim(), a), Align::R, true);
    }

    /// micro-icônes vectorielles de la sidebar (dessinées — zéro risque police).
    fn tab_icon(&self, r: &Renderer, i: usize, cx: f32, cy: f32, c: [f32; 4]) {
        match i {
            0 => {
                // COMBAT: réticule plein (cercle + 4 traits + point)
                r.ring(cx, cy, 6.5, c, 1.3);
                r.line(cx - 9.0, cy, cx - 3.0, cy, c, 1.3);
                r.line(cx + 3.0, cy, cx + 9.0, cy, c, 1.3);
                r.line(cx, cy - 9.0, cx, cy - 3.0, c, 1.3);
                r.line(cx, cy + 3.0, cx, cy + 9.0, c, 1.3);
                r.dot(cx, cy, 1.6, c);
            }
            1 => {
                // RAGE: spirale (toupie — 3 arcs concentriques + trait de rotation)
                r.ring(cx, cy, 7.5, c, 1.3);
                r.ring(cx, cy, 4.0, c, 1.1);
                r.dot(cx, cy, 1.5, c);
                r.line(cx, cy - 7.5, cx + 4.5, cy - 6.0, c, 1.3);
            }
            2 => {
                // VISUEL: œil (amande + pupille)
                r.line(cx - 8.0, cy, cx - 3.0, cy - 5.0, c, 1.3);
                r.line(cx - 3.0, cy - 5.0, cx + 3.0, cy - 5.0, c, 1.3);
                r.line(cx + 3.0, cy - 5.0, cx + 8.0, cy, c, 1.3);
                r.line(cx - 8.0, cy, cx - 3.0, cy + 5.0, c, 1.3);
                r.line(cx - 3.0, cy + 5.0, cx + 3.0, cy + 5.0, c, 1.3);
                r.line(cx + 3.0, cy + 5.0, cx + 8.0, cy, c, 1.3);
                r.dot(cx, cy, 2.6, c);
                r.ring(cx, cy, 4.6, c, 1.1);
            }
            3 => {
                // MOUVEMENT: double chevron (vitesse — deux flèches empilées)
                r.line(cx - 7.0, cy - 4.0, cx - 1.0, cy + 1.0, c, 1.3);
                r.line(cx - 1.0, cy + 1.0, cx - 7.0, cy + 6.0, c, 1.3);
                r.line(cx - 1.0, cy - 4.0, cx + 5.0, cy + 1.0, c, 1.3);
                r.line(cx + 5.0, cy + 1.0, cx - 1.0, cy + 6.0, c, 1.3);
            }
            4 => {
                // MISC: deux sliders avec molettes
                r.line(cx - 8.0, cy - 4.0, cx + 8.0, cy - 4.0, c, 1.3);
                r.dot(cx - 3.0, cy - 4.0, 2.4, c);
                r.line(cx - 8.0, cy + 4.0, cx + 8.0, cy + 4.0, c, 1.3);
                r.dot(cx + 3.0, cy + 4.0, 2.4, c);
            }
            _ => {
                // THÈME: pastille double (cercle plein + croissant)
                r.dot(cx - 2.0, cy, 6.0, c);
                r.ring(cx + 3.5, cy, 4.5, c, 1.3);
            }
        }
    }

    /// carte glass avec titre + petit trait accent (thème courant).
    fn group(&self, r: &Renderer, gx: f32, gy: f32, gw: f32, gh: f32, title: &str, a: f32) {
        let th = self.th();
        r.fill_round(gx, gy, gw, gh, 10.0, with_a(theme::glass_card(&th, 1.0), a));
        r.stroke_round(gx, gy, gw, gh, 10.0, with_a([th.border[0], th.border[1], th.border[2], th.border[3] * 1.8], a), 1.0);
        r.text(title, &r.fmt_s, gx + 12.0, gy + 8.0, gw - 24.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.grad_fill_round(gx + 12.0, gy + 26.0, 26.0, 2.0, 1.0, a);
    }

    /// checkbox glass (case arrondie, check blanc, hover animé + flash au clic).
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
            self.flash(id);
            self.mark_dirty();
        }
        let well = theme::glass_well(&self.th(), hov);
        r.fill_round(x, y + 5.0, 16.0, 16.0, 4.0, with_a(well, a));
        if *v {
            r.grad_fill_round(x, y + 5.0, 16.0, 16.0, 4.0, a);
        }
        r.stroke_round(x, y + 5.0, 16.0, 16.0, 4.0, with_a([1.0, 1.0, 1.0, 0.14 + 0.24 * hov], a), 1.0);
        if *v {
            r.line(x + 3.8, y + 12.6, x + 6.8, y + 15.6, with_a([1.0, 1.0, 1.0, 0.95], a), 1.8);
            r.line(x + 6.8, y + 15.6, x + 12.4, y + 9.4, with_a([1.0, 1.0, 1.0, 0.95], a), 1.8);
        }
        // v0.6 — flash de clic (fondu blanc 300 ms)
        let fl = self.flash_amt(id);
        if fl > 0.0 {
            r.fill_round(x, y, w.min(26.0 + 170.0 * fl), 26.0, 6.0, with_a([1.0, 1.0, 1.0, 0.22 * fl], a));
        }
        r.text(label, &r.fmt_b, x + 26.0, y, w - 30.0, 26.0, with_a(self.t_text(), a), Align::L, true);
    }

    /// slider (label + valeur mono, piste pill, remplissage dégradé, molette).
    /// `dec` = valeur affichée à 1 décimale sinon arrondie.
    #[allow(clippy::too_many_arguments)]
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
        dec: bool,
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
                self.flash(id);
            }
            *v = min + (max - min) * ((m.x - x) / w).clamp(0.0, 1.0);
            self.mark_dirty();
        }
        let t = ((*v - min) / (max - min)).clamp(0.0, 1.0);
        r.text(label, &r.fmt_b, x, y, w - 80.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        let vs = if dec { format!("{:.1} {unit}", v) } else { format!("{:.0} {unit}", v) };
        r.text(&vs, &r.fmt_mono, x + w - 80.0, y, 80.0, 16.0, with_a(self.t_text(), a), Align::R, false);
        // piste + remplissage accent
        r.fill_round(x, y + 22.0, w, 5.0, 2.5, with_a([1.0, 1.0, 1.0, 0.10], a));
        let fw = (w * t).max(6.0);
        r.grad_fill_round(x, y + 22.0, fw, 5.0, 2.5, a);
        // molette + halo (grossit au hover/drag) — couleur primaire du thème
        let kx = x + w * t;
        let ky = y + 24.5;
        let glow = hov.max(if active { 1.0 } else { 0.0 });
        let pr = self.t_primary();
        r.dot(kx, ky, 10.0 + 2.0 * glow, with_a([pr[0], pr[1], pr[2], 0.16], a));
        r.dot(kx, ky, 5.5, with_a([0.97, 0.98, 1.0, 1.0], a));
    }

    /// contrôle segmenté (mini-onglets dans une pill) — thème courant.
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
        let well = theme::glass_well(&self.th(), 0.0);
        r.fill_round(x, y, w, h, h * 0.5, with_a(well, a));
        let cw = w / options.len() as f32;
        for (i, o) in options.iter().enumerate() {
            let cxi = x + i as f32 * cw;
            let hov = self.hover(id + i as u64, m, cxi, y, cw, h, dt);
            if m.clicked && self.open && a > 0.85 && hit_r(m, cxi, y, cw, h) {
                *idx = i;
                self.flash(id + i as u64);
                self.mark_dirty();
            }
            if i == *idx {
                r.grad_fill_round(cxi + 2.0, y + 2.0, cw - 4.0, h - 4.0, (h - 4.0) * 0.5, a);
                r.text(*o, &r.fmt_b, cxi, y, cw, h, with_a([1.0, 1.0, 1.0, 1.0], a), Align::C, true);
            } else {
                if hov > 0.01 {
                    r.fill_round(cxi + 2.0, y + 2.0, cw - 4.0, h - 4.0, (h - 4.0) * 0.5, with_a([1.0, 1.0, 1.0, 0.06 * hov], a));
                }
                r.text(*o, &r.fmt_b, cxi, y, cw, h, with_a(self.t_dim(), a), Align::C, true);
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
                self.flash(id + i as u64);
                self.mark_dirty();
            }
            let p = &PALETTES[i];
            r.dot(cx - 4.5, cy, 7.0, with_a(p.t, a));
            r.dot(cx + 4.5, cy, 7.0, with_a(p.ct, a));
            if i == *cur {
                let pr = self.t_primary();
                r.ring(cx, cy, 12.5, with_a([pr[0], pr[1], pr[2], 0.9], a), 1.4);
            } else if hov > 0.01 {
                r.ring(cx, cy, 12.0, with_a([1.0, 1.0, 1.0, 0.25 * hov], a), 1.2);
            }
        }
        r.text(PALETTES[*cur].name, &r.fmt_b, x + 194.0, y + 4.0, 80.0, 24.0, with_a(self.t_text(), a), Align::L, true);
    }

    /// bouton glass ANIMÉ — flash au clic (fondu 300 ms), halo au hover.
    /// retourne true si cliqué ce frame.
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
        if pressed {
            self.flash(id);
        }
        let th = self.th();
        let base: [f32; 4] = if danger { [0.16, 0.05, 0.06, 0.55] } else { theme::glass_card(&th, 0.9) };
        // v0.6 — le bouton S'ÉCLAIRE progressivement au hover (lift lumineux)
        r.fill_round(
            x, y, w, h, 8.0,
            with_a([base[0] + 0.06 * hov, base[1] + 0.05 * hov, base[2] + 0.06 * hov, base[3] + 0.30 * hov], a),
        );
        let bc = if danger { C_DANGER } else { [1.0, 1.0, 1.0, 1.0] };
        r.stroke_round(x, y, w, h, 8.0, with_a([bc[0], bc[1], bc[2], 0.16 + 0.30 * hov], a), 1.0);
        // flash de clic — onde blanche qui s'estompe
        let fl = self.flash_amt(id);
        if fl > 0.0 {
            r.fill_round(x, y, w, h, 8.0, with_a([1.0, 1.0, 1.0, 0.30 * fl], a));
        }
        let tc = if danger { mix4(bc, [1.0, 0.62, 0.62, 1.0], 0.45) } else { self.t_text() };
        r.text(label, &r.fmt_b, x, y, w, h, with_a(tc, a), Align::C, true);
        pressed
    }

    /// ligne label ↔ valeur — ancrage par position (valeurs mesurées, jamais
    /// de wrap: les textes longs s'étendent vers la gauche, sans clipping).
    fn info_row(&self, r: &Renderer, x: f32, y: f32, w: f32, label: &str, value: &str, a: f32) {
        r.text(label, &r.fmt_b, x, y, w * 0.5, 20.0, with_a(self.t_dim(), a), Align::L, true);
        let (vw, _) = r.measure(value, &r.fmt_mono);
        let vx = (x + w - vw - 2.0).max(x + w * 0.5);
        r.text(value, &r.fmt_mono, vx, y, vw + 4.0, 20.0, with_a(self.t_text(), a), Align::L, true);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Onglet COMBAT (v0.8) — aimbot SMOOTH/FLICK · triggerbot · RCS · statut
    // ═══════════════════════════════════════════════════════════════════════
    #[allow(clippy::too_many_arguments)]
    fn tab_combat(
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
        let zones = ["TOUT", "TÊTE", "CORPS", "MEMBRES"];
        let keys = ["ALT", "CTRL", "BTN4", "MAJ"];

        // carte L1 - Aimbot v0.9.6 (vitesse + prédiction + recul + gate tête)
        // v0.9.6 — rangées resserrées (pas de 28 px) pour laisser la place à
        // la carte TRIGGERBOT v3 enrichie en dessous.
        let (gx, gy) = (x + CX0, y + CY0);
        self.group(r, gx, gy, CW, 296.0, "AIMBOT v0.9.6", a);
        self.checkbox(r, m, 360, gx + 14.0, gy + 32.0, CW - 28.0, "aimbot activé", &mut opts.aim, a, dt);
        let modes = ["SMOOTH", "FLICK"];
        self.segmented(r, m, 367, gx + 14.0, gy + 60.0, CW - 28.0, &modes, &mut opts.aim_mode, a, dt);
        let akeys = ["ALT", "CTRL", "BTN4", "MAJ", "AUTO"];
        self.segmented(r, m, 361, gx + 14.0, gy + 88.0, CW - 28.0, &akeys, &mut opts.aim_key, a, dt);
        let hbs = ["TÊTE", "COU", "POITRINE", "CORPS"];
        self.segmented(r, m, 362, gx + 14.0, gy + 116.0, CW - 28.0, &hbs, &mut opts.aim_hitbox, a, dt);
        let mut f = opts.aim_fov;
        self.slider(r, m, 363, gx + 14.0, gy + 144.0, CW - 28.0, "champ de visée (FOV)", 1.0, 30.0, &mut f, "°", a, dt, false);
        opts.aim_fov = f;
        // v0.9.1 — VITESSE DE VISÉE: gain × plafond — le fix "elle bouge et gagne"
        let mut sp = opts.aim_speed;
        self.slider(r, m, 370, gx + 14.0, gy + 172.0, CW - 28.0, "vitesse de visée", 1.0, 10.0, &mut sp, "", a, dt, false);
        opts.aim_speed = sp;
        // v0.9 — prédiction cibles mobiles: vise où l'ennemi SERA
        let mut pm = opts.aim_pred_ms;
        self.slider(r, m, 368, gx + 14.0, gy + 200.0, CW - 28.0, "prédiction (cibles mobiles)", 0.0, 150.0, &mut pm, "ms", a, dt, false);
        opts.aim_pred_ms = pm;
        // v0.9.3 — COMPENSATION DU RECUL: la balle part vers vue + punch;
        // à 1.0 la vue vise tête − punch → le projectile arrive dans la tête
        // même quand le viseur "monte" pendant la rafale.
        let mut pg = opts.aim_punch;
        self.slider(r, m, 372, gx + 14.0, gy + 228.0, CW - 28.0, "compensation recul (aim)", 0.0, 1.5, &mut pg, "×", a, dt, false);
        opts.aim_punch = pg;
        // v0.9 — verrouillage: le fix "stresse en groupe"
        self.checkbox(r, m, 369, gx + 14.0, gy + 254.0, CW - 28.0, "verrouillage cible (anti-stress groupe)", &mut opts.aim_lock, a, dt);
        if opts.aim_mode == 1 {
            r.text("FLICK: snap burst + suivi vif · rafale re-centrée balle par balle", &r.fmt_s, gx + 14.0, gy + 278.0, CW - 28.0, 14.0, with_a(self.t_primary(), a), Align::L, false);
        } else {
            self.checkbox(r, m, 366, gx + 14.0, gy + 276.0, CW - 28.0, "visuel: cercle FOV + marqueur tête + ligne", &mut opts.aim_fov_circle, a, dt);
        }

        // carte L2 — Triggerbot v3 (v0.9.6: gate physique tête + clic commis)
        let (gx, gy) = (x + CX0, y + CY0 + 300.0);
        self.group(r, gx, gy, CW, 166.0, "TRIGGERBOT v3", a);
        self.checkbox(r, m, 350, gx + 14.0, gy + 32.0, CW - 28.0, "triggerbot activé", &mut opts.trigger, a, dt);
        // v0.9.6 — LE gate « à chaque tir je touche la tête »: couplé à
        // l'aimbot, le clic ne part QUE si la balle atterrit dans la tête.
        self.checkbox(r, m, 354, gx + 14.0, gy + 58.0, CW - 28.0, "tête à chaque tir (gate physique)", &mut opts.trigger_head_only, a, dt);
        self.segmented(r, m, 353, gx + 14.0, gy + 84.0, CW - 28.0, &zones, &mut opts.trigger_hitbox, a, dt);
        self.segmented(r, m, 351, gx + 14.0, gy + 112.0, CW - 28.0, &keys, &mut opts.trigger_key, a, dt);
        let mut d = opts.trigger_delay_ms;
        self.slider(r, m, 352, gx + 14.0, gy + 140.0, CW - 28.0, "délai moyen (gaussien)", 0.0, 200.0, &mut d, "ms", a, dt, false);
        opts.trigger_delay_ms = d;
        r.text("clic 18 ms COMMISS · re-tir 38 ms · gate tête: x0.4 délai", &r.fmt_s, gx + 14.0, gy + 166.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);

        // carte R1 — Contrôle du recul (RCS)
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 200.0, "CONTRÔLE DU RECUL (RCS)", a);
        self.checkbox(r, m, 500, gx + 14.0, gy + 34.0, CW - 28.0, "compenser le recul (delta-based)", &mut opts.rcs, a, dt);
        let mut rs = opts.rcs_strength;
        self.slider(r, m, 501, gx + 14.0, gy + 64.0, CW - 28.0, "force de compensation", 0.0, 1.0, &mut rs, "%", a, dt, false);
        opts.rcs_strength = rs;
        self.checkbox(r, m, 502, gx + 14.0, gy + 94.0, CW - 28.0, "punch feed-forward (balistique intégrale)", &mut opts.rcs_punch, a, dt);
        let kick = if st.rcs_active {
            format!("({:+.2}°, {:+.2}°)", st.rcs_kick.0, st.rcs_kick.1)
        } else {
            "—".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 124.0, CW - 28.0, "dernier kick mesuré", &kick, a);
        self.info_row(r, gx + 14.0, gy + 148.0, CW - 28.0, "état", if st.rcs_active { "COMPENSE" } else { "repos" }, a);
        self.info_row(r, gx + 14.0, gy + 172.0, CW - 28.0, "mode", if opts.rcs_punch { "PUNCH FF (onglet RAGE)" } else { "delta-based (vue)" }, a);

        // carte R2 — Statut combat live (v0.9: verrou + prédiction)
        let (gx, gy) = (x + CX1, y + CY0 + 212.0);
        self.group(r, gx, gy, CW, 252.0, "STATUT COMBAT", a);
        let aim_state = if !opts.aim {
            "désactivé".to_string()
        } else if st.aim_active {
            format!("VERROUILLÉ — {} ({})", if opts.aim_mode == 1 { "FLICK" } else { "SMOOTH" }, st.aim_target.as_str())
        } else {
            "repos — pas de cible".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 34.0, CW - 28.0, "aimbot", &aim_state, a);
        let delta_s = if st.aim_active {
            let state = if st.aim_punch_on { "SUR LA TÊTE" } else { "recalage…" };
            format!("{:.2}° · {} · punch {:.2}°", st.aim_delta, state, st.aim_punch_deg)
        } else {
            "—".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 58.0, CW - 28.0, "angle restant", &delta_s, a);
        // v0.9 — diagnostics du nouveau moteur de visée
        let lock_s = if !opts.aim_lock {
            "libre (verrouillage off)".to_string()
        } else if st.aim_active {
            format!("VERROU — {}", st.aim_target.as_str())
        } else {
            "repos".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 82.0, CW - 28.0, "verrouillage", &lock_s, a);
        let pred_s = if st.aim_learn > 0 {
            format!("adaptatif · w={:.2} · err {:.1} u ({} éch.)", st.aim_w_lead, st.aim_err_u, st.aim_learn)
        } else if opts.aim_pred_ms < 0.5 {
            "adaptatif (amorçage)".to_string()
        } else {
            format!("{:.0} ms + modèle adaptatif", opts.aim_pred_ms)
        };
        self.info_row(r, gx + 14.0, gy + 106.0, CW - 28.0, "prédiction", &pred_s, a);
        self.info_row(r, gx + 14.0, gy + 130.0, CW - 28.0, "changements de cible", &st.aim_switches.to_string(), a);
        let rate = if st.aim_send_rate > 0.0 { format!("{:.0}/s", st.aim_send_rate) } else { "—".to_string() };
        self.info_row(r, gx + 14.0, gy + 154.0, CW - 28.0, "envois souris", &rate, a);
        self.info_row(r, gx + 14.0, gy + 178.0, CW - 28.0, "source de vue", st.view_src, a);
        let sens = st.sens_effective.map(|v| format!("{v:.2} (lue)")).unwrap_or_else(|| "repli manuel".into());
        self.info_row(r, gx + 14.0, gy + 202.0, CW - 28.0, "sensibilité", &sens, a);
        let trig_state = if st.trigger_head_ready {
            "PRÊT TÊTE — la balle part dans la tête"
        } else if st.trigger_armed {
            "ARMÉ — ennemi sous le viseur"
        } else if opts.trigger {
            "repos — viseur libre"
        } else {
            "désactivé"
        };
        self.info_row(r, gx + 14.0, gy + 226.0, CW - 28.0, "trigger", trig_state, a);
    }
    // ═══════════════════════════════════════════════════════════════════════
    // Onglet RAGE (v0.8) — BANDEAU état + master toggle · spinbot · silent
    // aim · no-recoil punch · vérité no-spread. Le bandeau reflète l'état
    // RÉEL du handle d'écriture (st.rage_on): LEGIT vert = lecture seule,
    // RAGE rouge = WPM armé. Les sous-modules dorment quand le master est OFF.
    // ═══════════════════════════════════════════════════════════════════════
    #[allow(clippy::too_many_arguments)]
    fn tab_rage(
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
        // ── BANDEAU D'ÉTAT (cliquable = master toggle) ──
        let bw = CW * 2.0 + PAD;
        let (bx, by) = (x + CX0, y + CY0);
        let t = self.t;
        let rage = opts.rage_mode && st.rage_on;
        let hov = self.hover(700, m, bx, by, bw, 36.0, dt);
        if self.open && a > 0.85 && m.clicked && hit_r(m, bx, by, bw, 36.0) {
            opts.rage_mode = !opts.rage_mode;
            self.flash(700);
            self.mark_dirty();
        }
        // fond du bandeau: vert calme (LEGIT) / rouge pulsé (RAGE)
        let (base, led_c, label, sub) = if rage {
            let pulse = 0.5 + 0.5 * (t * 3.0).sin();
            ([0.34, 0.06, 0.08, 0.85], [1.0, 0.25 + 0.35 * pulse, 0.25, 1.0], "MODE RAGE", "écriture mémoire ACTIVE — handle RW ouvert")
        } else {
            ([0.05, 0.16, 0.09, 0.85], [0.30, 0.95, 0.55, 1.0], "MODE LEGIT", "lecture seule — handle RW fermé, spinbot/silent endormis")
        };
        r.fill_round(bx, by, bw, 36.0, 9.0, with_a(base, a));
        if hov > 0.01 {
            r.fill_round(bx, by, bw, 36.0, 9.0, with_a([1.0, 1.0, 1.0, 0.06 * hov], a));
        }
        let fl = self.flash_amt(700);
        if fl > 0.0 {
            r.fill_round(bx, by, bw, 36.0, 9.0, with_a([1.0, 1.0, 1.0, 0.20 * fl], a));
        }
        // LED + textes
        let led = 0.55 + 0.45 * (t * if rage { 3.0 } else { 2.0 }).sin();
        r.ring(bx + 20.0, by + 18.0, 7.0, with_a([led_c[0], led_c[1], led_c[2], 0.30 + 0.30 * led], a), 1.6);
        r.dot(bx + 20.0, by + 18.0, 3.4, with_a(led_c, a));
        r.text(label, &r.fmt_b, bx + 34.0, by, 130.0, 36.0, with_a([1.0, 1.0, 1.0, 0.95], a), Align::L, true);
        r.text(sub, &r.fmt_s, bx + 150.0, by, bw - 250.0, 36.0, with_a([1.0, 1.0, 1.0, 0.65], a), Align::L, true);
        // mini interrupteur à droite
        {
            let (sw_x, sw_y) = (bx + bw - 62.0, by + 11.0);
            r.fill_round(sw_x, sw_y, 44.0, 14.0, 7.0, with_a([0.0, 0.0, 0.0, 0.35], a));
            if rage {
                r.grad_fill_round(sw_x, sw_y, 44.0, 14.0, 7.0, a);
            } else {
                r.fill_round(sw_x, sw_y, 44.0, 14.0, 7.0, with_a([0.30, 0.55, 0.40, 0.5], a));
            }
            let kx = if rage { sw_x + 36.0 } else { sw_x + 8.0 };
            r.dot(kx, sw_y + 7.0, 7.0, with_a([1.0, 1.0, 1.0, 0.95], a));
        }

        // les sous-modules dorment en LEGIT — voile d'inactivité sur le texte
        let dim = if rage { 1.0 } else { 0.45 };

        // carte L1 — Spinbot (v0.9: + GHOST write-restore)
        let (gx, gy) = (x + CX0, y + CY0 + 44.0);
        self.group(r, gx, gy, CW, 240.0, "SPINBOT (ANTI-AIM)", a);
        self.checkbox(r, m, 710, gx + 14.0, gy + 34.0, CW - 28.0, "spinbot activé (écriture mémoire)", &mut opts.spin, a, dt);
        let smodes = ["SPIN", "JIT", "BACK", "FLUT", "GHOST"];
        self.segmented(r, m, 711, gx + 14.0, gy + 64.0, CW - 28.0, &smodes, &mut opts.spin_mode, a, dt);
        let mut spd = opts.spin_speed;
        self.slider(r, m, 712, gx + 14.0, gy + 94.0, CW - 28.0, "vitesse de rotation", 90.0, 2160.0, &mut spd, "°/s", a, dt, false);
        opts.spin_speed = spd;
        let spitches = ["GARDE", "BAS", "HAUT"];
        self.segmented(r, m, 713, gx + 14.0, gy + 124.0, CW - 28.0, &spitches, &mut opts.spin_pitch, a, dt);
        // v0.9 — GHOST: fenêtre write-restore (le serveur voit l'angle spunné,
        // le rendu retrouve la vraie vue)
        if opts.spin_mode == 4 {
            let mut gu = opts.spin_ghost_us;
            self.slider(r, m, 714, gx + 14.0, gy + 154.0, CW - 28.0, "fenêtre ghost (µs)", 150.0, 3000.0, &mut gu, "µs", a, dt, false);
            opts.spin_ghost_us = gu;
        }
        let spin_state = if !rage {
            "ENDORMI — master RAGE off".to_string()
        } else if st.spin_active {
            if opts.spin_mode == 4 {
                format!("GHOST ({:+.0}°, {:+.0}°) — écrit+restaure", st.spin_written.0, st.spin_written.1)
            } else {
                format!("ÉCRIT ({:+.0}°, {:+.0}°)", st.spin_written.0, st.spin_written.1)
            }
        } else if opts.spin {
            "cède (aimbot / flick silent)".to_string()
        } else {
            "désactivé".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 184.0, CW - 28.0, "état", &spin_state, a);
        if opts.spin_mode == 4 {
            r.text("GHOST: serveur spunné · TA caméra reste jouable", &r.fmt_s, gx + 14.0, gy + 210.0, CW - 28.0, 14.0, with_a(self.t_primary(), a * dim), Align::L, false);
            r.text("(FLUT = caméra sur place à ~50 % · GHOST = bien mieux)", &r.fmt_s, gx + 14.0, gy + 224.0, CW - 28.0, 14.0, with_a(self.t_dim(), a * dim), Align::L, false);
        } else {
            r.text("FLUT = caméra sur place · anti-aim ~50 %", &r.fmt_s, gx + 14.0, gy + 210.0, CW - 28.0, 14.0, with_a(self.t_dim(), a * dim), Align::L, false);
        }

        // carte L2 — Silent Aim (v0.9: compactée, flicks dans l'état)
        let (gx, gy) = (x + CX0, y + CY0 + 296.0);
        self.group(r, gx, gy, CW, 164.0, "SILENT AIM", a);
        self.checkbox(r, m, 720, gx + 14.0, gy + 34.0, CW - 28.0, "silent aim activé (clic = tir déplacé)", &mut opts.silent, a, dt);
        let mut sf = opts.silent_fov;
        self.slider(r, m, 721, gx + 14.0, gy + 64.0, CW - 28.0, "champ de capture (FOV)", 1.0, 30.0, &mut sf, "°", a, dt, false);
        opts.silent_fov = sf;
        let shbs = ["TÊTE", "COU", "POITRINE", "CORPS"];
        self.segmented(r, m, 722, gx + 14.0, gy + 94.0, CW - 28.0, &shbs, &mut opts.silent_hitbox, a, dt);
        let silent_state = if !rage {
            "ENDORMI — master RAGE off".to_string()
        } else if st.silent_armed {
            format!("ARMÉ — {} · {} flicks", st.silent_target, st.silent_flicks)
        } else if opts.silent {
            "repos — pas de cible".to_string()
        } else {
            "désactivé".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 126.0, CW - 28.0, "état", &silent_state, a);
        r.text("clic = 1re balle déplacée · restore exact", &r.fmt_s, gx + 14.0, gy + 148.0, CW - 28.0, 14.0, with_a(self.t_dim(), a * dim), Align::L, false);

        // carte R1 — No recoil / no spread
        let (gx, gy) = (x + CX1, y + CY0 + 44.0);
        self.group(r, gx, gy, CW, 198.0, "NO RECOIL / NO SPREAD", a);
        self.checkbox(r, m, 730, gx + 14.0, gy + 34.0, CW - 28.0, "compenser le recul", &mut opts.rcs, a, dt);
        self.checkbox(r, m, 731, gx + 14.0, gy + 64.0, CW - 28.0, "mode punch FF (recul balistique)", &mut opts.rcs_punch, a, dt);
        let mut rs = opts.rcs_strength;
        self.slider(r, m, 732, gx + 14.0, gy + 94.0, CW - 28.0, "force de compensation", 0.0, 1.0, &mut rs, "%", a, dt, false);
        opts.rcs_strength = rs;
        let punch = st.nr_punch.map(|(p, y)| format!("({:+.2}°, {:+.2}°)", p, y)).unwrap_or_else(|| "—".into());
        self.info_row(r, gx + 14.0, gy + 124.0, CW - 28.0, "punch live (AimPunch)", &punch, a);
        let nr_state = if st.nr_active {
            format!("COMPENSE — dernière ({:+.2}°, {:+.2}°)", st.nr_last.0, st.nr_last.1)
        } else {
            "repos".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 148.0, CW - 28.0, "état", &nr_state, a);
        r.text("si ça AMPLIFIE le recul: flip PUNCH_SIGN (modules.rs)", &r.fmt_s, gx + 14.0, gy + 178.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);

        // carte R2 — CAMÉRA 3E PERSONNE (v0.9) + vérité no-spread en pied
        let (gx, gy) = (x + CX1, y + CY0 + 254.0);
        self.group(r, gx, gy, CW, 206.0, "CAMÉRA 3E PERSONNE (v0.9)", a);
        self.checkbox(r, m, 740, gx + 14.0, gy + 34.0, CW - 28.0, "forcer la 3e personne (flag CSGOInput)", &mut opts.thirdperson, a, dt);
        self.checkbox(r, m, 741, gx + 14.0, gy + 62.0, CW - 28.0, "patch moteur (scan signature — opt-in)", &mut opts.tp_patch, a, dt);
        let tp_state = if !rage {
            "ENDORMI — master RAGE off".to_string()
        } else if st.tp_active {
            format!("ACTIVE — {}", st.tp_mode)
        } else if opts.thirdperson {
            "inactive (offset à re-dériver ?)".to_string()
        } else {
            "désactivé".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 92.0, CW - 28.0, "état", &tp_state, a);
        r.text("tu vois ton perso bouger — combo parfait avec", &r.fmt_s, gx + 14.0, gy + 118.0, CW - 28.0, 14.0, with_a(self.t_primary(), a * dim), Align::L, false);
        r.text("GHOST: caméra stable, modèle spunné pour les ennemis", &r.fmt_s, gx + 14.0, gy + 132.0, CW - 28.0, 14.0, with_a(self.t_dim(), a * dim), Align::L, false);
        r.text("no-spread: RNG seed SERVEUR — l'externe n'y peut", &r.fmt_s, gx + 14.0, gy + 156.0, CW - 28.0, 14.0, with_a(self.t_dim(), a * dim), Align::L, false);
        r.text("rien · tête + 1re balle reste la contre-mesure", &r.fmt_s, gx + 14.0, gy + 170.0, CW - 28.0, 14.0, with_a(self.t_dim(), a * dim), Align::L, false);
        r.text("offset m_bCameraInThirdPerson PAR BUILD — offsets.json", &r.fmt_s, gx + 14.0, gy + 190.0, CW - 28.0, 14.0, with_a(self.t_dim(), a * dim), Align::L, false);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Onglet MOUVEMENT (v0.8) — bunnyhop · labo de précision · stats monde
    // ═══════════════════════════════════════════════════════════════════════
    #[allow(clippy::too_many_arguments)]
    fn tab_mouvement(
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
        // carte L1 — Mouvement
        let (gx, gy) = (x + CX0, y + CY0);
        self.group(r, gx, gy, CW, 222.0, "MOUVEMENT", a);
        self.checkbox(r, m, 507, gx + 14.0, gy + 34.0, CW - 28.0, "bunnyhop (tenir ESPACE)", &mut opts.bhop, a, dt);
        self.info_row(r, gx + 14.0, gy + 64.0, CW - 28.0, "sauts cette session", &st.bhop_jumps.to_string(), a);
        r.text("un tap ESPACE calé sur FL_ONGROUND au moment", &r.fmt_s, gx + 14.0, gy + 92.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("de toucher le sol — cadencé 30 ms, la roue", &r.fmt_s, gx + 14.0, gy + 106.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("n'est pas écrasée en boucle (indétectable", &r.fmt_s, gx + 14.0, gy + 120.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("par les checks m_fFlags naïfs)", &r.fmt_s, gx + 14.0, gy + 134.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);

        // carte L2 — Labo de précision
        let (gx, gy) = (x + CX0, y + CY0 + 234.0);
        self.group(r, gx, gy, CW, 222.0, "LABO DE PRÉCISION", a);
        self.checkbox(r, m, 508, gx + 14.0, gy + 34.0, CW - 28.0, "accuracy lab (stats de session)", &mut opts.accuracy, a, dt);
        let acc = format!("{}/{} · {:.0}%", st.acc_hits, st.acc_shots, if st.acc_shots > 0 { st.acc_hits as f32 / st.acc_shots as f32 * 100.0 } else { 0.0 });
        self.info_row(r, gx + 14.0, gy + 64.0, CW - 28.0, "précision (H/T)", &acc, a);
        self.info_row(r, gx + 14.0, gy + 88.0, CW - 28.0, "élims / dégâts", &format!("{} · {} dmg", st.acc_kills, st.acc_damage), a);
        self.info_row(r, gx + 14.0, gy + 112.0, CW - 28.0, "arme courante", &st.acc_weapon, a);
        r.text("tirs = fronts de m_iShotsFired", &r.fmt_s, gx + 14.0, gy + 140.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("hits = baisse de HP d'un ennemi pendant", &r.fmt_s, gx + 14.0, gy + 154.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("que tu tires (cooldown 60 ms)", &r.fmt_s, gx + 14.0, gy + 168.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);

        // carte R1 — Statut monde
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 222.0, "STATUT MONDE", a);
        self.info_row(r, gx + 14.0, gy + 34.0, CW - 28.0, "armes au sol", &st.world_items.to_string(), a);
        self.info_row(r, gx + 14.0, gy + 58.0, CW - 28.0, "grenades en vol", &st.grenades_live.to_string(), a);
        self.info_row(r, gx + 14.0, gy + 82.0, CW - 28.0, "joueurs tracés", &st.players.to_string(), a);
        self.info_row(r, gx + 14.0, gy + 106.0, CW - 28.0, "spectateurs", &st.spectator_count.to_string(), a);
        r.text("les armes fantômes sont purgées au reset du", &r.fmt_s, gx + 14.0, gy + 134.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("round (m_totalRoundsPlayed) + validation de", &r.fmt_s, gx + 14.0, gy + 148.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("position + TTL 2 scans sur les nodes mortes", &r.fmt_s, gx + 14.0, gy + 162.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);

        // carte R2 — Perf boucle (v0.8)
        let (gx, gy) = (x + CX1, y + CY0 + 234.0);
        self.group(r, gx, gy, CW, 222.0, "PERFORMANCE (v0.8)", a);
        self.info_row(r, gx + 14.0, gy + 34.0, CW - 28.0, "fps overlay", &format!("{:.0}", st.fps), a);
        r.text("la boucle lit chaque pawn EN BULK (1 RPM au", &r.fmt_s, gx + 14.0, gy + 62.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("lieu de ~15) — ~10× moins de syscalls, et", &r.fmt_s, gx + 14.0, gy + 76.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("l'ESP interpole les positions entre deux", &r.fmt_s, gx + 14.0, gy + 90.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("ticks serveur (origin + velocity·âge):", &r.fmt_s, gx + 14.0, gy + 104.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("les boîtes glissent au lieu de sauter.", &r.fmt_s, gx + 14.0, gy + 118.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("timer haute résolution: la cadence 1 ms", &r.fmt_s, gx + 14.0, gy + 146.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("est RÉELLE (l'ancien Sleep tombait à ~15 ms", &r.fmt_s, gx + 14.0, gy + 160.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("de granularité sans timeBeginPeriod)", &r.fmt_s, gx + 14.0, gy + 174.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Onglet VISUEL — boîtes/squelette · étiquettes · radar · monde & effets
    // ═══════════════════════════════════════════════════════════════════════
    fn tab_visuel(
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
        self.group(r, gx, gy, CW, 222.0, "BOÎTES & SQUELETTE", a);
        let modes = ["coins", "complet", "off"];
        self.segmented(r, m, 400, gx + 14.0, gy + 34.0, CW - 28.0, &modes, &mut opts.box_mode, a, dt);
        self.checkbox(r, m, 301, gx + 14.0, gy + 64.0, CW - 28.0, "remplissage translucide", &mut opts.box_fill, a, dt);
        self.checkbox(r, m, 302, gx + 14.0, gy + 90.0, CW - 28.0, "barre de vie", &mut opts.health, a, dt);
        self.checkbox(r, m, 303, gx + 14.0, gy + 116.0, CW - 28.0, "snaplines (vers ennemis)", &mut opts.snaplines, a, dt);
        self.checkbox(r, m, 305, gx + 14.0, gy + 142.0, CW - 28.0, "squelette (os reliés + articulations)", &mut opts.skeleton, a, dt);
        self.checkbox(r, m, 304, gx + 14.0, gy + 168.0, CW - 28.0, "crosshair maison", &mut opts.crosshair, a, dt);

        // carte L2 — Étiquettes & filtres
        let (gx, gy) = (x + CX0, y + CY0 + 234.0);
        self.group(r, gx, gy, CW, 222.0, "ÉTIQUETTES & FILTRES", a);
        self.checkbox(r, m, 320, gx + 14.0, gy + 34.0, CW - 28.0, "noms des joueurs", &mut opts.names, a, dt);
        self.checkbox(r, m, 321, gx + 14.0, gy + 60.0, CW - 28.0, "distance (m)", &mut opts.distance, a, dt);
        r.text("taille des noms", &r.fmt_s, gx + 14.0, gy + 88.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        let sizes = ["PETIT", "MOYEN", "GRAND"];
        self.segmented(r, m, 325, gx + 14.0, gy + 102.0, CW - 28.0, &sizes, &mut opts.esp_text_size, a, dt);
        self.checkbox(r, m, 322, gx + 14.0, gy + 134.0, CW - 28.0, "coéquipiers (F2)", &mut opts.teammates, a, dt);
        self.checkbox(r, m, 324, gx + 14.0, gy + 160.0, CW - 28.0, "distance ILLIMITÉE", &mut opts.unlimited_dist, a, dt);
        let mut v = opts.max_dist_m;
        self.slider(r, m, 323, gx + 14.0, gy + 186.0, CW - 28.0, "distance max", 20.0, 2000.0, &mut v, "m", a, dt, false);
        opts.max_dist_m = v;

        // carte R1 — Radar (réglages intégrés)
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 222.0, "RADAR / MINIMAP", a);
        self.checkbox(r, m, 340, gx + 14.0, gy + 34.0, CW - 28.0, "radar activé (rotation = ta vue)", &mut opts.radar, a, dt);
        let corners = ["haut G", "haut D", "bas G", "bas D"];
        self.segmented(r, m, 341, gx + 14.0, gy + 64.0, CW - 28.0, &corners, &mut opts.radar_corner, a, dt);
        self.checkbox(r, m, 342, gx + 14.0, gy + 94.0, CW - 28.0, "coéquipiers sur le radar", &mut opts.radar_mates, a, dt);
        let mut sz = opts.radar_size;
        self.slider(r, m, 343, gx + 14.0, gy + 120.0, CW - 28.0, "taille", 120.0, 400.0, &mut sz, "px", a, dt, false);
        opts.radar_size = sz;
        let mut rg = opts.radar_range;
        self.slider(r, m, 344, gx + 14.0, gy + 158.0, CW - 28.0, "portée", 500.0, 6000.0, &mut rg, "u", a, dt, false);
        opts.radar_range = rg;
        r.text("point creux au bord = hors portée (direction)", &r.fmt_s, gx + 14.0, gy + 196.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);

        // carte R2 — Monde & effets (armes au sol, grenades, pas, hitbox)
        let (gx, gy) = (x + CX1, y + CY0 + 234.0);
        self.group(r, gx, gy, CW, 222.0, "MONDE & EFFETS", a);
        self.checkbox(r, m, 504, gx + 14.0, gy + 34.0, CW - 28.0, "armes au sol (filtre anti-fantômes)", &mut opts.weapon_esp, a, dt);
        self.checkbox(r, m, 505, gx + 14.0, gy + 60.0, CW - 28.0, "trajectoires des grenades", &mut opts.grenade_trails, a, dt);
        self.checkbox(r, m, 506, gx + 14.0, gy + 86.0, CW - 28.0, "sound ESP (anneaux de pas)", &mut opts.sound_esp, a, dt);
        self.checkbox(r, m, 306, gx + 14.0, gy + 112.0, CW - 28.0, "zones de collision (visuel)", &mut opts.hitbox, a, dt);
        // légende hitbox compacte (code couleur fonctionnel)
        let zones: [(&str, [f32; 4]); 4] = [
            ("tête ×4", [1.00, 0.28, 0.32, 1.0]),
            ("poitrine", [1.00, 0.60, 0.20, 1.0]),
            ("corps", [1.00, 0.90, 0.30, 1.0]),
            ("membres", [0.35, 0.65, 1.00, 1.0]),
        ];
        for (i, (lbl, c)) in zones.iter().enumerate() {
            let lx = gx + 18.0 + (i as f32 % 2.0) * 138.0;
            let ly = gy + 146.0 + (i as f32 / 2.0).floor() * 24.0;
            r.stroke_round(lx, ly + 5.0, 14.0, 12.0, 3.0, with_a(*c, a), 1.6);
            r.text(lbl, &r.fmt_b, lx + 20.0, ly, 110.0, 20.0, with_a(self.t_dim(), a), Align::L, true);
        }
        r.text("les armes possédées ne s'affichent plus (owner)", &r.fmt_s, gx + 14.0, gy + 196.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Onglet MISC (v0.8) — overlay & capture · bombe & spectateurs ·
    // console & diagnostic · session (mouvement/labo → onglet MOUVEMENT)
    // ═══════════════════════════════════════════════════════════════════════
    #[allow(clippy::too_many_arguments)]
    fn tab_misc(
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
        // carte L1 — Overlay & capture (avec le TOGGLE STREAMPROOF)
        let (gx, gy) = (x + CX0, y + CY0);
        self.group(r, gx, gy, CW, 222.0, "OVERLAY & CAPTURE", a);
        self.checkbox(r, m, 330, gx + 14.0, gy + 34.0, CW - 28.0, "ESP limité au jeu (cache Discord)", &mut opts.game_only, a, dt);
        self.checkbox(r, m, 336, gx + 14.0, gy + 60.0, CW - 28.0, "watermark (coin haut-gauche)", &mut opts.watermark, a, dt);
        self.checkbox(r, m, 331, gx + 14.0, gy + 86.0, CW - 28.0, "debug verbeux (F1)", &mut opts.verbose, a, dt);
        let mut iv = opts.interval_ms as f32;
        self.slider(r, m, 332, gx + 14.0, gy + 112.0, CW - 28.0, "cadence de boucle", 1.0, 50.0, &mut iv, "ms", a, dt, false);
        opts.interval_ms = iv.round().max(1.0) as u64;
        // v0.6 — LE toggle anti screen-share (enfin désactivable au menu)
        self.checkbox(r, m, 337, gx + 14.0, gy + 158.0, CW - 28.0, "ANTI SCREEN-SHARE (streamproof)", &mut opts.streamproof, a, dt);
        r.text("ON: invisible pour OBS/Discord/ShadowPlay · F10", &r.fmt_s, gx + 14.0, gy + 186.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);

        // carte L2 — Bombe & spectateurs
        let (gx, gy) = (x + CX0, y + CY0 + 234.0);
        self.group(r, gx, gy, CW, 222.0, "BOMBE & SPECTATEURS", a);
        self.checkbox(r, m, 502, gx + 14.0, gy + 34.0, CW - 28.0, "bomb timer (bandeau + barre)", &mut opts.bomb_timer, a, dt);
        self.checkbox(r, m, 509, gx + 14.0, gy + 60.0, CW - 28.0, "ESP bombe (marqueur monde C4)", &mut opts.bomb_esp, a, dt);
        let bomb = if st.bomb_planted {
            format!("{:.1} s · horloge {}", st.bomb_remaining_s, st.bomb_clock)
        } else {
            "non posée".to_string()
        };
        self.info_row(r, gx + 14.0, gy + 88.0, CW - 28.0, "bombe", &bomb, a);
        self.checkbox(r, m, 503, gx + 14.0, gy + 114.0, CW - 28.0, "liste des spectateurs (coin)", &mut opts.spectators, a, dt);
        self.info_row(r, gx + 14.0, gy + 140.0, CW - 28.0, "spectateurs", &st.spectator_count.to_string(), a);
        let bomb_src = if st.bomb_planted { format!("{:.1} s · {}", st.bomb_remaining_s, st.bomb_clock) } else { "non posée".to_string() };
        self.info_row(r, gx + 14.0, gy + 164.0, CW - 28.0, "décompte", &bomb_src, a);
        r.text("marqueur pulsant sur la bombe · defuse en direct", &r.fmt_s, gx + 14.0, gy + 192.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);

        // carte R1 — Console & diagnostic (v0.8: en R1)
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 222.0, "CONSOLE & DIAGNOSTIC", a);
        if self.button(r, m, 333, gx + 14.0, gy + 34.0, CW - 28.0, 30.0, "dump console (F4)", false, a, dt) {
            actions.push(Action::Dump);
        }
        if self.button(r, m, 334, gx + 14.0, gy + 70.0, CW - 28.0, 30.0, "panneau de diagnostic", false, a, dt) {
            actions.push(Action::ShowPanel);
        }
        self.info_row(r, gx + 14.0, gy + 108.0, CW - 28.0, "snapshot", if st.snapshot_ok { "OK — entités résolues" } else { "AUCUNE — stale ?" }, a);
        self.info_row(r, gx + 14.0, gy + 132.0, CW - 28.0, "rendu", if st.renderer_ok { "OK — D2D + ULW" } else { "ÉCHEC" }, a);
        self.info_row(r, gx + 14.0, gy + 156.0, CW - 28.0, "police", &st.font, a);
        self.info_row(r, gx + 14.0, gy + 180.0, CW - 28.0, "cadence boucle", &format!("{} ms", opts.interval_ms), a);

        // carte R2 — Session & arrêt
        let (gx, gy) = (x + CX1, y + CY0 + 234.0);
        self.group(r, gx, gy, CW, 222.0, "SESSION", a);
        self.info_row(r, gx + 14.0, gy + 34.0, CW - 28.0, "pid", &st.pid.to_string(), a);
        self.info_row(r, gx + 14.0, gy + 58.0, CW - 28.0, "joueurs vivants", &st.players.to_string(), a);
        self.info_row(r, gx + 14.0, gy + 82.0, CW - 28.0, "fps overlay", &format!("{:.0}", st.fps), a);
        self.info_row(r, gx + 14.0, gy + 106.0, CW - 28.0, "config", &st.config_path, a);
        self.info_row(r, gx + 14.0, gy + 130.0, CW - 28.0, "mode mémoire", if st.rage_on { "RAGE — handle RW ouvert" } else { "LEGIT — lecture seule" }, a);
        if self.button(r, m, 335, gx + 14.0, gy + 164.0, CW - 28.0, 30.0, "QUITTER (END)", true, a, dt) {
            actions.push(Action::Quit);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Onglet THÈME — presets + STUDIO DE COULEURS RGB live + aperçu
    // ═══════════════════════════════════════════════════════════════════════
    fn tab_theme(
        &mut self,
        r: &Renderer,
        m: &Mouse,
        opts: &mut Options,
        a: f32,
        dt: f32,
        x: f32,
        y: f32,
    ) {
        // carte L1 — Identité (5 presets + CUSTOM)
        let (gx, gy) = (x + CX0, y + CY0);
        self.group(r, gx, gy, CW, 222.0, "IDENTITÉ", a);
        for i in 0..THEME_COUNT {
            let cx = gx + 36.0 + i as f32 * 44.0;
            let cy = gy + 62.0;
            let hov = self.hover(600 + i as u64, m, cx - 14.0, cy - 14.0, 28.0, 28.0, dt);
            if m.clicked && self.open && a > 0.85 && hit_r(m, cx - 14.0, cy - 14.0, 28.0, 28.0) {
                opts.theme = i;
                self.flash(600 + i as u64);
                self.mark_dirty();
            }
            let (c1, c2) = if i == CUSTOM_IDX {
                (opts.custom.primary, opts.custom.light)
            } else {
                (theme::THEMES[i].primary, theme::THEMES[i].light)
            };
            // pastille: moitié primary, moitié light (l'identité)
            r.dot(cx - 4.0, cy, 8.0, with_a(c1, a));
            r.dot(cx + 4.0, cy, 8.0, with_a(c2, a));
            if i == opts.theme {
                let pr = self.t_primary();
                r.ring(cx, cy, 14.5, with_a([pr[0], pr[1], pr[2], 0.95], a), 1.6);
            } else if hov > 0.01 {
                r.ring(cx, cy, 13.5, with_a([1.0, 1.0, 1.0, 0.25 * hov], a), 1.2);
            }
        }
        let cur = self.th();
        r.text(cur.name, &r.fmt_b, gx + 14.0, gy + 92.0, CW - 28.0, 22.0, with_a(self.t_text(), a), Align::L, true);
        r.text(cur.vibe, &r.fmt_s, gx + 14.0, gy + 116.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("le thème habille le menu, le radar, le watermark,", &r.fmt_s, gx + 14.0, gy + 142.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("l'ESP monde et les accents — en direct.", &r.fmt_s, gx + 14.0, gy + 158.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        if opts.theme == CUSTOM_IDX {
            r.text("CUSTOM actif — sculpte tes couleurs en bas à gauche", &r.fmt_s, gx + 14.0, gy + 190.0, CW - 28.0, 16.0, with_a(self.t_primary(), a), Align::L, false);
        } else {
            r.text("passe sur la pastille CUSTOM (6e) pour sculpter", &r.fmt_s, gx + 14.0, gy + 190.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        }

        // carte L2 — STUDIO DE COULEURS (RGB live, 6 rôles)
        let (gx, gy) = (x + CX0, y + CY0 + 234.0);
        self.group(r, gx, gy, CW, 222.0, "STUDIO DE COULEURS", a);
        r.text("rôle à éditer (passe en CUSTOM automatiquement)", &r.fmt_s, gx + 14.0, gy + 28.0, CW - 28.0, 14.0, with_a(self.t_dim(), a), Align::L, false);
        // v0.6.1 — FIX E0499: color_role copié AVANT le borrow de self
        // (self.segmented prend &mut self; &mut self.color_role en même temps
        // = double borrow mutable → E0499, le crate ne compilait pas)
        let mut role = self.color_role;
        self.segmented(r, m, 620, gx + 14.0, gy + 44.0, CW - 28.0, &CustomColors::ROLES, &mut role, a, dt);
        self.color_role = role;
        // RGB — édition live: chaque mouvement bascule en CUSTOM
        let mut col = opts.custom.role(self.color_role);
        let labels = ["rouge", "vert", "bleu"];
        let mut rgb = [col[0], col[1], col[2]];
        for i in 0..3 {
            let mut v = rgb[i];
            self.slider(r, m, 630 + i as u64, gx + 14.0, gy + 76.0 + i as f32 * 30.0, CW - 78.0, labels[i], 0.0, 1.0, &mut v, "", a, dt, true);
            rgb[i] = v;
        }
        col = [rgb[0], rgb[1], rgb[2], col[3]];
        // si les couleurs BOUGENT → on sculpte: bascule en CUSTOM + application
        if col != opts.custom.role(self.color_role) {
            opts.custom.set_role(self.color_role, col);
            if opts.theme != CUSTOM_IDX {
                opts.theme = CUSTOM_IDX; // le studio sculpte → custom immédiat
                self.cur_theme = CUSTOM_IDX;
            }
            self.mark_dirty();
        }
        // pastille d'aperçu du rôle courant + hex
        let sw_x = gx + CW - 58.0;
        r.fill_round(sw_x, gy + 190.0, 44.0, 20.0, 5.0, with_a(col, a));
        r.stroke_round(sw_x, gy + 190.0, 44.0, 20.0, 5.0, with_a([1.0, 1.0, 1.0, 0.25], a), 1.0);
        r.text(theme::to_hex(col).as_str(), &r.fmt_mono, gx + 14.0, gy + 190.0, 200.0, 20.0, with_a(self.t_text(), a), Align::L, true);

        // carte R1 — Palette équipes (T / CT / toi)
        let (gx, gy) = (x + CX1, y + CY0);
        self.group(r, gx, gy, CW, 222.0, "PALETTE ÉQUIPES", a);
        self.swatches(r, m, 310, gx + 14.0, gy + 36.0, &mut opts.palette, a, dt);
        let p = &PALETTES[opts.palette];
        let legend: [(&str, [f32; 4]); 3] = [("terroristes", p.t), ("anti-terroristes", p.ct), ("toi", p.me)];
        for (i, (lbl, c)) in legend.iter().enumerate() {
            let ly = gy + 88.0 + i as f32 * 28.0;
            r.dot(gx + 24.0, ly + 12.0, 5.0, with_a(*c, a));
            r.text(lbl, &r.fmt_b, gx + 38.0, ly, 200.0, 24.0, with_a(self.t_dim(), a), Align::L, true);
        }
        r.text("s'applique aux boîtes, squelette, radar", &r.fmt_s, gx + 14.0, gy + 178.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("et snaplines — indépendante du thème", &r.fmt_s, gx + 14.0, gy + 194.0, CW - 28.0, 16.0, with_a(self.t_dim(), a), Align::L, false);

        // carte R2 — Aperçu live du thème (mockup animé)
        let (gx, gy) = (x + CX1, y + CY0 + 234.0);
        self.group(r, gx, gy, CW, 222.0, "APERÇU LIVE", a);
        let t = self.t;
        let (mx, my, mw, mh) = (gx + 22.0, gy + 34.0, 150.0, 128.0);
        r.fill_round(mx, my, mw, mh, 8.0, with_a(theme::glass_bg(&cur, 1.0), a));
        r.stroke_round(mx, my, mw, mh, 8.0, with_a([cur.border[0], cur.border[1], cur.border[2], cur.border[3] * 1.6], a), 1.0);
        r.fill_round(mx + 10.0, my + 10.0, mw - 20.0, 32.0, 5.0, with_a(theme::glass_card(&cur, 1.0), a));
        r.fill_round(mx + 16.0, my + 18.0, 44.0, 5.0, 2.5, with_a(cur.primary, a));
        r.fill_round(mx + 16.0, my + 29.0, 64.0, 5.0, 2.5, with_a(cur.surface, a));
        // point qui pulse au rythme du thème
        let pulse = 0.5 + 0.5 * (t * 2.2).sin();
        r.ring(mx + 24.0, my + 66.0, 8.0 + 2.0 * pulse, with_a([cur.light[0], cur.light[1], cur.light[2], 0.4 + 0.3 * pulse], a), 1.4);
        r.dot(mx + 24.0, my + 66.0, 5.0, with_a(cur.primary, a));
        r.dot(mx + 58.0, my + 66.0, 5.0, with_a(cur.light, a));
        // slider factice rempli par l'accent
        r.fill_round(mx + 16.0, my + 94.0, 118.0, 7.0, 3.5, with_a([1.0, 1.0, 1.0, 0.10], a));
        let fill_w = 60.0 + 30.0 * (t * 0.8).sin();
        r.grad_fill_round(mx + 16.0, my + 94.0, fill_w.max(6.0), 7.0, 3.5, a);
        r.text("Vantyx.cc", &r.fmt_b, mx + 92.0, my + 58.0, 54.0, 16.0, with_a(cur.text, a), Align::C, true);
        // notes
        r.text("6 rôles éditables · live", &r.fmt_s, gx + 182.0, gy + 40.0, CW - 196.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("accent → dégradés, LED,", &r.fmt_s, gx + 182.0, gy + 58.0, CW - 196.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("indicateurs, remplissages", &r.fmt_s, gx + 182.0, gy + 76.0, CW - 196.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("sauvegardé dans vantyx.ini", &r.fmt_s, gx + 182.0, gy + 100.0, CW - 196.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
        r.text("(theme=5 + custom_*=hex)", &r.fmt_s, gx + 182.0, gy + 118.0, CW - 196.0, 16.0, with_a(self.t_dim(), a), Align::L, false);
    }
}
