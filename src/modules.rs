// modules — v0.5 · Vantyx.cc — les cinq modules "meta":
//
//   RCS          contrôle du recul DELTA-BASED: m_iShotsFired détecte le tir,
//                le kick mesuré sur dwViewAngles est compensé par SendInput
//                inverse (l'aimpunch cache a disparu du build 14193 — la voie
//                punch services est instable, le différentiel de vue est la
//                mécanique externe éprouvée). S'assemble avec l'aimbot: même
//                canal souris, deux updates indépendantes.
//   BombTimer    m_bBombPlanted (game rules) valide, décompte depuis la
//                plantation (m_flTimerLength = mèche), état defuse via
//                m_flDefuseCountDown — bandeau + barre + pulse < 10 s.
//   SoundEsp     pas des ennemis visualisés: m_vecVelocity > 215 u/s →
//                anneau horodaté à la position (proxy externe standard des
//                footsteps — pas de hook audio nécessaire), rendu monde + radar.
//   Bunnyhop     espace SendInput conditionné à m_fFlags FL_ONGROUND: tenir
//                ESPACE → un tap au moment de toucher le sol, cadencé 30 ms.
//   AccuracyLab  stats de session: fronts de m_iShotsFired = tirs, baisses de
//                HP d'un ennemi pendant que tu tires = hit (cooldown 60 ms),
//                armes via les services d'armes du pawn local.
//
// Tous READ-ONLY (RPM) + SendInput. Zéro écriture dans le process du jeu.
//
// language: Rust, file: modules.rs, target: Windows 11, windows-rs 0.58

use std::collections::HashMap;
use std::time::Instant;

use crate::entity::Snapshot;
use crate::math::{Vec3, wrap180};
use crate::mem::Mem;
use crate::offsets::Offsets;
use crate::{dlog, dwarn};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_KEYUP, MOUSEEVENTF_MOVE, MOUSEINPUT, VIRTUAL_KEY, VK_LBUTTON, VK_SPACE,
};

/// Source m_yaw: 1 count souris = 0.022° (partagé avec l'aimbot)
const M_YAW: f32 = 0.022;

// ── mouvement souris relatif (même canal que l'aimbot) ─────────────────────
pub(crate) fn send_move(dx: i32, dy: i32) {
    unsafe {
        let mut input = INPUT::default();
        input.r#type = INPUT_MOUSE;
        input.Anonymous = INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

// ── tap clavier synthétique (bunnyhop) ──────────────────────────────────────
fn send_tap(vk: VIRTUAL_KEY) {
    unsafe {
        let mut down = INPUT::default();
        down.r#type = INPUT_KEYBOARD;
        down.Anonymous = INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: Default::default(), // keydown
                time: 0,
                dwExtraInfo: 0,
            },
        };
        let mut up = INPUT::default();
        up.r#type = INPUT_KEYBOARD;
        up.Anonymous = INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        };
        SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32);
    }
}

// ═══ RCS — compensation du recul delta-based ═══════════════════════════════
pub struct Rcs {
    /// tir courant de la rafale (m_iShotsFired au dernier tick)
    last_shots: i32,
    /// vue de référence (dernière vue TRAITÉE — pas de nouveau tir dessus)
    base_view: (f32, f32),
    /// dernière vue traitée (gate vue fraîche, comme l'aimbot)
    last_view: Option<(f32, f32)>,
    /// dernier kick mesuré (affiché dans le menu)
    pub last_kick: (f32, f32),
    /// en train de compenser (affiché)
    pub active: bool,
}

impl Rcs {
    pub fn new() -> Self {
        Rcs { last_shots: 0, base_view: (0.0, 0.0), last_view: None, last_kick: (0.0, 0.0), active: false }
    }

    /// appelé à chaque itération (~1 kHz) — `view` = vue LIVE du snapshot.
    /// La gate "vue fraîche" évite de mesurer un kick sur une vue périmée.
    /// v0.8.2 — `aim_active`: l'aimbot est verrouillé → le RCS SE TAIT (voir
    /// l'interlock dans le corps) — il ne doit pas compenser les mouvements
    /// DE l'aimbot comme du recul.
    pub fn update(
        &mut self,
        snap: Option<&Snapshot>,
        enabled: bool,
        strength: f32, // 0..1
        sens: f32,     // sensibilité effective du jeu
        ctx_ok: bool,  // jeu premier plan, pas de menu, vivant
        aim_active: bool, // v0.8.2 — interlock aimbot
    ) {
        self.active = false;
        let Some(s) = snap else { self.last_shots = 0; self.last_view = None; return };
        if !enabled || !ctx_ok || !s.local_alive || !s.local_view_ok {
            self.last_shots = 0;
            self.last_view = None;
            return;
        }
        let view = s.local_view;
        // gate vue fraîche: la vue mémoire suit le FRAMERATE du jeu, pas
        // notre boucle — traiter deux fois la même vue = double compensation
        if let Some((lp, ly)) = self.last_view {
            if (lp - view.0).abs() < 1e-4 && (ly - view.1).abs() < 1e-4 {
                return; // pas encore rafraîchie par le jeu
            }
        }
        self.last_view = Some(view);
        let shots = s.shots_fired.max(0);

        // ═══ v0.8.2 — INTERLOCK AIMBOT (le bug "le spray part sur le bord") ═══
        // Pendant que l'aimbot est verrouillé, il RÉ-AVISE la cible à chaque
        // vue fraîche — il absorbe le recul tout seul. Mesurer un "kick" ici
        // revient à additionner recul + MOUVEMENTS DE L'AIMBOT: chaque tir
        // d'un spray au trigger renvoyait la vue en ARRIÈRE (la compensation
        // annulait la correction d'aim) → oscillation autour de la tête, la
        // balle partait du bord. On reste ANCRÉ (base = vue courante) pour
        // qu'à la reprise du RCS aucune valeur périmée ne parte d'un coup.
        if aim_active {
            self.base_view = view;
            self.last_shots = shots;
            return;
        }

        if shots == 0 {
            // rafale terminée (ou pas commencée): la vue devient la référence
            self.last_shots = 0;
            self.base_view = view;
            return;
        }

        if shots > self.last_shots {
            // ── NOUVEAU TIR: le recul a déplacé la vue entre base et ici ──
            let kick_yaw = wrap180(view.1 - self.base_view.1);
            let kick_pitch = view.0 - self.base_view.0;
            self.last_kick = (kick_pitch, kick_yaw);
            // kick invraisemblable (motion souris simultanée > 25°) → ignorer
            if kick_pitch.abs() < 25.0 && kick_yaw.abs() < 25.0 {
                self.active = true;
                let deg_per_count = M_YAW * sens.max(0.05);
                // v0.7 — FIX SIGNE X (même bug que l'aimbot v0.6.2, passé inaperçu
                // car le kick yaw est petit et centré): dx > 0 = yaw DÉCROÎT → pour
                // annuler un kick yaw POSITIF il faut dx POSITIF. L'ancien code
                // inversait l'axe horizontal de la compensation.
                let dx = (kick_yaw * strength / deg_per_count).clamp(-80.0, 80.0);
                let dy = (-kick_pitch * strength / deg_per_count).clamp(-80.0, 80.0);
                if dx.abs() >= 1.0 || dy.abs() >= 1.0 {
                    send_move(dx.round() as i32, dy.round() as i32);
                    dlog!("[ rcs] tir #{shots}: kick ({kick_pitch:+.2}°, {kick_yaw:+.2}°) → comp ({dx:+.0}, {dy:+.0}) counts");
                }
            }
        }
        // référence pour le prochain tir = vue courante (post-kick/post-comp)
        self.base_view = view;
        self.last_shots = shots;
    }
}

// ═══ Spinbot — anti-aim par écriture directe de dwViewAngles (v0.7) ═══════
// MÉCANIQUE: WriteProcessMemory du QAngle (pitch, yaw, roll) dans CSGOInput,
// à CHAQUE itération de boucle (~1 kHz). Le jeu ré-écrase la valeur à chaque
// frame depuis la souris → notre valeur tient entre l'écrasement et la
// construction du usercmd une partie du temps: le serveur voit un modèle qui
// TOURNE (anti-aim — impossible à viser pour les ennemis), le rendu local
// montre un mélange spin/souris selon la race.
//
// LIMITES HONNÊTES (à savoir avant de râler en jeu):
//   · la caméra locale SUIT partiellement le spin — en externe le rendu lit
//     LA MÊME adresse que le usercmd. La variante "caméra 100% figée côté
//     client" exige un hook interne (CreateMove + rendu découplé): c'est
//     le mur externe/interne, documenté, pas contournable par WPM.
//     Le mode FLUTTER (v0.7.1) est la meilleure approximation externe: la
//     caméra OSCILLE SUR PLACE (un coup spinné / un coup vue souris) au lieu
//     de tourner en continu — le serveur reçoit quand même les angles spinnés
//     une écriture sur deux.
//   · le spin part PAR ÉCHANTILLONS (race contre l'écrasement souris) —
//     c'est le comportement de tous les spinbots externes CS2.
//   · COMBO: spinbot + triggerbot (délai 0) = les balles partent quand la
//     vue croise un ennemi pendant la rotation. L'aimbot verrouillé coupe
//     le spin (priorité à la visée propre).
//
// v0.7.1 — DEUX CORRECTIFS DE COMPORTEMENT:
//   · RÉ-ANCRAGE: à la reprise d'un cède (aimbot relâché / fin de flick),
//     master_yaw repart de la vue RÉELLE courante. Avant, il gardait sa
//     valeur d'avant le verrouillage → la caméra TÉLÉPORTAIT jusqu'à 180°
//     vers un angle périmé dès que l'aimbot lâchait sa cible → "l'aimbot
//     vise des cibles inexistantes" (c'était le spin, pas l'aimbot).
//   · INTERLOCK: le spin cède PENDANT le flick du silent aim (pending),
//     plus à son simple ARMEMENT. Avant, armed==true dès qu'une cible était
//     dans le FOV silent → le spin était COUPÉ dès qu'un ennemi approchait
//     (l'anti-aim mourait au pire moment) et tournait PENDANT le flick —
//     il écrasait l'angle du tir → la balle silencieuse partait dans la
//     direction du spin.
//
// modes: 0 SPIN (rotation continue °/s) · 1 JITTER (angles aléatoires au tick)
//        · 2 BACKWARDS (dos à la direction visée — anti-aim classique)
//        · 3 FLUTTER (v0.7.1 — un coup spin / un coup vue souris: caméra sur
//          place, serveur spinné. Vitesse effective ≈ la moitié du slider.)
//        · 4 GHOST (v0.9 — WRITE-RESTORE frame-sync: le SERVEUR voit l'angle
//          spinné, le RENDU retrouve la vraie vue. C'est le mode "je vois
//          tout normalement": écrit l'angle spinné juste après que le jeu a
//          traité l'input du frame (le usercmd le consomme), puis RESTAURE
//          la vue souris après une fenêtre courte (µs). Caméra quasi fixe,
//          anti-aim ~maximum, aucun angle spinné qui dort en mémoire.)
// pitch: 0 GARDE (ne touche pas au pitch souris) · 1 BAS (-89°) · 2 HAUT (+89°)
pub struct Spinbot {
    /// état maître du yaw (le spin cumule ici, indépendant de la souris)
    master_yaw: f32,
    /// dernier tick jitter (le jitter tire un nouvel angle au tick, pas à la boucle)
    last_jitter: Option<Instant>,
    /// dernier write (dt pour la vitesse °/s)
    last: Option<Instant>,
    /// xorshift — RNG jitter sans dépendance
    rng: u32,
    /// dernier angle écrit (diagnostic)
    pub last_written: (f32, f32),
    /// écritures réussies ce cycle (statut)
    pub writes: u32,
    /// v0.7.1 — FLUTTER: true = on écrit l'angle spinné, false = l'anchor souris
    flutter_spin: bool,
    /// v0.7.1 — FLUTTER: dernière vue écrite par le JEU (retour caméra)
    flutter_anchor: Option<(f32, f32)>,
    /// v0.8.1 — FLUTTER: bascule de phase TEMPORELLE. La boucle v0.8 tourne à
    /// ~0,5 ms (timer HR): un flip par itération = un stroboscope à 1 kHz que
    /// le rendu échantillonne au hasard. On tient chaque phase ~1 tick serveur
    /// (15,6 ms) — le serveur voit du spinné 1 tick sur 2, l'œil voit la vue
    /// souris 1 frame sur 2 (comportement identique à la v0.7 et sa boucle 15 ms).
    flutter_flip: Option<Instant>,
    // ── v0.9 — GHOST: write-restore frame-sync (mode 4) ──
    /// vraie vue (souris) du frame courant — la valeur RESTAURÉE au rendu
    ghost_anchor: Option<(f32, f32)>,
    /// true = l'angle spinné est actuellement en mémoire (fenêtre ouverte)
    ghost_hold: bool,
    /// fin de la fenêtre ghost (restauration à cette échéance)
    ghost_until: Option<Instant>,
    /// dernier frame jeu détecté (mesure de période)
    ghost_frame_at: Option<Instant>,
    /// période de frame estimée (EMA) — cadence de re-spin souris idle
    ghost_period: f32,
}

impl Spinbot {
    pub fn new() -> Self {
        Spinbot {
            master_yaw: 0.0,
            last_jitter: None,
            last: None,
            rng: 0x9E3779B9,
            last_written: (0.0, 0.0),
            writes: 0,
            flutter_spin: true,
            flutter_anchor: None,
            flutter_flip: None,
            ghost_anchor: None,
            ghost_hold: false,
            ghost_until: None,
            ghost_frame_at: None,
            ghost_period: 0.007, // ~144 fps par défaut, EMA dès la 1re détection
        }
    }

    /// v0.9 — écriture QAngle factorisée (tous modes)
    fn w(&mut self, mem: &crate::mem::Mem, addr: u64, pitch: f32, yaw: f32) -> bool {
        let angles = [pitch.clamp(-89.0, 89.0), wrap180(yaw), 0.0];
        if mem.write(addr, &angles) {
            self.last_written = (pitch, wrap180(yaw));
            self.writes += 1;
            true
        } else {
            false
        }
    }

    /// v0.9 — GHOST: restaure la vraie vue AVANT de céder (aimbot / flick
    /// silent / désactivation / mort). Invariant: JAMAIS un angle spinné qui
    /// dort en mémoire quand le spin ne contrôle plus la vue — sinon l'aimbot
    /// hériterait d'un delta calculé depuis un angle faux.
    fn ghost_recover(&mut self, mem: &crate::mem::Mem, offs: &crate::offsets::Offsets) {
        if self.ghost_hold {
            let addr = offs.dw_view_angles_addr(&mem.client);
            if let Some((p, y)) = self.ghost_anchor {
                let _ = self.w(mem, addr, p, y);
            }
            self.ghost_hold = false;
            self.ghost_until = None;
        }
    }

    /// v0.9 — pitch de l'angle SPUN selon le mode pitch (le pitch RESTAURÉ est
    /// toujours celui de l'anchor — la vraie vue)
    fn ghost_pitch(cur_pitch: f32, pitch_mode: usize) -> f32 {
        match pitch_mode {
            1 => -89.0,
            2 => 89.0,
            _ => cur_pitch,
        }
    }

    fn rand_deg(&mut self, lo: f32, hi: f32) -> f32 {
        // xorshift32 → [0,1) → lerp — suffisant pour du jitter d'anti-aim
        let mut x = self.rng | 1;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        let unit = (x >> 8) as f32 / 16777216.0;
        lo + unit * (hi - lo)
    }

    /// appelé à chaque itération (~1 kHz). Retourne true si un write est parti.
    /// aim_active: l'aimbot a priorité sur la vue — le spin cède.
    /// silent_flick: v0.7.1 — cède PENDANT le flick (pending), pas à l'armement:
    ///            l'angle du tir doit rester celui du flick jusqu'à ce que le
    ///            frame jeu l'ait consommé (voir SilentAim ci-dessous).
    /// ghost_hold_us: v0.9 — fenêtre ghost (mode 4) en microsecondes.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        mem: &crate::mem::Mem,
        offs: &crate::offsets::Offsets,
        snap: Option<&Snapshot>,
        enabled: bool,
        mode: usize,       // 0 spin · 1 jitter · 2 backwards · 3 flutter · 4 ghost
        rate_dps: f32,     // °/s (mode spin; flutter ≈ la moitié)
        pitch_mode: usize, // 0 garde · 1 bas · 2 haut
        aim_active: bool,
        silent_flick: bool,
        ctx_ok: bool,
        ghost_hold_us: f32,
    ) -> bool {
        self.writes = 0;
        if !enabled || !ctx_ok || aim_active || silent_flick {
            // v0.9 — GHOST: restaurer la vraie vue AVANT de céder la place
            self.ghost_recover(mem, offs);
            self.last = None; // repart propre à la réactivation (→ ré-ancrage)
            return false;
        }
        let Some(s) = snap else {
            self.ghost_recover(mem, offs);
            self.last = None;
            return false;
        };
        if !s.local_alive || !s.local_view_ok {
            self.ghost_recover(mem, offs);
            self.last = None;
            return false;
        }
        let now = Instant::now();
        let dt = self.last.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.0);
        // v0.7.1 — capturé AVANT la mise à jour: true = on revient d'un cède
        // (aim/flick/désactivé) → il faut ré-ancrer le yaw maître sur la vue
        let resuming = self.last.is_none();
        self.last = Some(now);

        let addr = offs.dw_view_angles_addr(&mem.client);
        // lecture de l'état courant (pitch GARDE + base du BACKWARDS)
        let cur = mem.read::<[f32; 2]>(addr).unwrap_or([s.local_view.0, s.local_view.1]);

        // v0.7.1 — RÉ-ANCRAGE À LA REPRISE: si on revient d'un cède (aim/flick/
        // désactivé), le yaw maître repart de la vue RÉELLE, pas de sa valeur
        // d'avant le verrouillage. C'est ce qui tuait les téléports de caméra
        // "vers des cibles inexistantes" quand l'aimbot relâchait sa cible.
        if resuming {
            self.master_yaw = cur[1];
            self.flutter_anchor = Some((cur[0], cur[1]));
            self.flutter_spin = true;
            self.flutter_flip = None; // v0.8.1 — phase spin pleine durée dès la reprise
            self.ghost_anchor = Some((cur[0], cur[1])); // v0.9 — GHOST repart de la vraie vue
        }

        // ═══ v0.9 — MODE 4 GHOST: write-restore frame-sync ═══════════════════
        // LE SERVEUR VOIT L'ANGLE SPUNNÉ, LE RENDU VOIT LA VRAIE VUE:
        //   1. le jeu écrit la vue souris du frame (détection: la vue lue ≠
        //      notre dernière écriture) → anchor = vraie vue
        //   2. on écrit aussitôt l'angle SPUNNÉ — le usercmd construit juste
        //      après l'input du frame le consomme → le serveur spinné
        //   3. après `ghost_hold_us` (~700 µs) on RESTAURE l'anchor — le rendu
        //      (fin de frame) lit la vraie vue → caméra stable
        //   4. souris idle: la vue ne change pas → détection impossible → on
        //      re-spine sur cadence (période de frame mesurée par EMA)
        // Résidu: un frame de rendu peut échantillonner la fenêtre ghost →
        // micro-scintillement (bien loin du spin continu des modes 0-2).
        if mode == 4 {
            return self.ghost_cycle(mem, addr, cur, rate_dps, pitch_mode, ghost_hold_us, dt, now);
        }
        // v0.7.1 — FLUTTER: vue lue ≠ notre dernière écriture → c'est le JEU
        // qui vient d'écrire (input souris du frame) → nouvel anchor de retour.
        // v0.8.1 — on ne capture QUE si la dernière écriture était un ANCHOR:
        // pendant une phase spin, la vue lue = angle spinné + delta souris —
        // capturer ça dériverait l'anchor VERS le spin (caméra qui saute au
        // lieu de rester sur place). Le mouvement de souris des phases spin
        // est perdu (~15 ms) — acceptable pour un anti-aim.
        if mode == 3 {
            if !self.flutter_spin {
                let (wp, wy) = self.last_written;
                if (cur[0] - wp).abs() > 1e-3 || wrap180(cur[1] - wy).abs() > 1e-3 {
                    self.flutter_anchor = Some((cur[0], cur[1]));
                }
            }
            if self.flutter_anchor.is_none() {
                self.flutter_anchor = Some((cur[0], cur[1]));
            }
        }

        let mut pitch = cur[0];
        let yaw; // chaque mode assigne exactement une fois avant lecture

        match mode {
            0 => {
                // SPIN — rotation continue, le yaw maître avance de rate·dt
                self.master_yaw = wrap180(self.master_yaw + rate_dps * dt.min(0.05));
                yaw = self.master_yaw;
            }
            1 => {
                // JITTER — nouvel angle aléatoire à chaque tick (15.625 ms)
                let fire = self
                    .last_jitter
                    .map(|t| now.duration_since(t).as_secs_f32() >= 0.0156)
                    .unwrap_or(true);
                if fire {
                    self.last_jitter = Some(now);
                    yaw = wrap180(self.rand_deg(-180.0, 180.0));
                    self.master_yaw = yaw;
                    if pitch_mode == 0 {
                        pitch = self.rand_deg(-35.0, 35.0); // le jitter vit aussi en pitch
                    }
                } else {
                    yaw = self.master_yaw; // entre deux ticks: tient l'angle
                }
            }
            3 => {
                // FLUTTER (v0.7.1, adapté timer HR v0.8.1) — un coup spinné, un
                // coup vue souris: le rendu local oscille SUR PLACE (le plus
                // proche externe de "caméra fixe"), le serveur reçoit les angles
                // spinnés un tick sur deux → l'anti-aim tient à ~50 %. La
                // bascule est TEMPORELLE (~1 tick serveur, 15,6 ms): la boucle
                // v0.8 tourne à ~0,5 ms — un flip par itération serait un
                // stroboscope à 1 kHz. Vitesse effective ≈ la moitié du slider
                // (le yaw n'avance que pendant les phases spin).
                let flip = self
                    .flutter_flip
                    .map(|t| now.duration_since(t).as_secs_f32() >= 0.0156)
                    .unwrap_or(true);
                if flip {
                    self.flutter_spin = !self.flutter_spin;
                    self.flutter_flip = Some(now);
                }
                if self.flutter_spin {
                    self.master_yaw = wrap180(self.master_yaw + rate_dps * dt.min(0.05));
                    yaw = self.master_yaw;
                } else {
                    yaw = wrap180(self.flutter_anchor.map(|a| a.1).unwrap_or(cur[1]));
                }
            }
            _ => {
                // 2 BACKWARDS — dos à la direction réelle (la souris reste
                // cohérente). Aussi le repli des valeurs non reconnues
                // (config périmée): ce mode n'a PAS d'état → rien ne déraille.
                yaw = wrap180(cur[1] + 180.0);
            }
        }
        match pitch_mode {
            1 => pitch = -89.0,
            2 => pitch = 89.0,
            _ => {}
        }
        self.w(mem, addr, pitch, yaw)
    }

    /// v0.9 — MODE 4 GHOST: cycle write-restore synchronisé sur les frames du
    /// jeu. Voir le bloc d'appel dans update() pour la mécanique complète.
    /// Retourne true si un write est parti (spin OU restauration).
    #[allow(clippy::too_many_arguments)]
    fn ghost_cycle(
        &mut self,
        mem: &crate::mem::Mem,
        addr: u64,
        cur: [f32; 2],
        rate_dps: f32,
        pitch_mode: usize,
        hold_us: f32,
        dt: f32,
        now: Instant,
    ) -> bool {
        let hold = std::time::Duration::from_micros(hold_us.clamp(150.0, 3000.0) as u64);
        // la vue lue est-elle NOTRE dernière écriture ? sinon c'est le JEU qui
        // vient d'écrire la vue souris d'un NOUVEAU frame
        let ours = (cur[0] - self.last_written.0).abs() < 1e-3
            && wrap180(cur[1] - self.last_written.1).abs() < 1e-3;

        if !ours {
            // ── NOUVEAU FRAME JEU: capture l'anchor (vraie vue) + SPIN ──
            self.ghost_anchor = Some((cur[0], cur[1]));
            // mesure de la période de frame (EMA — cadence souris idle)
            if let Some(t) = self.ghost_frame_at {
                let iv = now.duration_since(t).as_secs_f32();
                if (0.002..0.05).contains(&iv) {
                    self.ghost_period = self.ghost_period * 0.7 + iv * 0.3;
                }
            }
            self.ghost_frame_at = Some(now);
            self.master_yaw = wrap180(self.master_yaw + rate_dps * dt.min(0.05));
            let ok = self.w(mem, addr, Self::ghost_pitch(cur[0], pitch_mode), self.master_yaw);
            if ok {
                self.ghost_hold = true;
                self.ghost_until = Some(now + hold);
            }
            ok
        } else if self.ghost_hold {
            // ── FENÊTRE GHOST OUVERTE: l'angle spunné est en mémoire, le
            // usercmd du frame le consomme. On RESTAURE à l'échéance — AVANT
            // que le rendu (fin de frame) ne lise la vue.
            if let Some(t) = self.ghost_until {
                if now >= t {
                    self.ghost_hold = false;
                    self.ghost_until = None;
                    let (ap, ay) = self.ghost_anchor.unwrap_or((cur[0], cur[1]));
                    return self.w(mem, addr, ap, ay);
                }
            } else {
                self.ghost_hold = false;
            }
            false
        } else {
            // ── RESTAURÉ: notre anchor dort en mémoire. Souris idle = le jeu
            // réécrit la MÊME valeur → indistinguable de notre écriture → on
            // re-spine sur CADENCE (période mesurée, plancher 2 ms) pour que
            // l'anti-aim ne meure pas quand tu ne bouges pas la souris.
            let due = self
                .ghost_frame_at
                .map(|t| now.duration_since(t).as_secs_f32() >= self.ghost_period.max(0.002))
                .unwrap_or(true);
            if due {
                self.ghost_frame_at = Some(now);
                self.master_yaw = wrap180(self.master_yaw + rate_dps * self.ghost_period);
                let ok = self.w(mem, addr, Self::ghost_pitch(cur[0], pitch_mode), self.master_yaw);
                if ok {
                    self.ghost_hold = true;
                    self.ghost_until = Some(now + hold);
                }
                ok
            } else {
                false
            }
        }
    }
}

// ═══ Silent Aim — one-frame flick: clic réel = tir déplacé (v0.7) ═════════
// MÉCANIQUE EXTERNE MAXIMALE:
//   1. détection du clic GAUCHE réel à ~1 kHz (GetAsyncKeyState) — plus vite
//      que le frame jeu ne le traite
//   2. au rising edge: envoi IMMÉDIAT des counts exacts vers la cible → le
//      frame qui traitera le clic verra la vue SUR la cible (le tir part
//      déplacé, la main du joueur n'a pas bougé)
//   3. au frame suivant (vue mémoire changée = move consommé + tir parti):
//      envoi des counts INVERSES exacts → la vue revient au pixel près
//
// v0.7.1 — GATE DE CONSOMMATION EXACTE: l'ancien critère de restauration
//   était "la vue a changé" — il se faisait piéger par N'IMPORTE QUEL
//   mouvement (main du joueur, recul, reste de spin) et restaurait AVANT que
//   le frame jeu n'ait consommé le flick → la balle repartait vers l'angle
//   d'origine. Maintenant on restaure quand la vue est ARRIVÉE à l'angle
//   ATTENDU (flick + counts appliqués, tolérance bruit de main + recul),
//   avec l'horloge 60 ms en filet de sécurité. Et le spinbot cède PENDANT
//   le flick (interlock flicking(), voir Spinbot) — avant il tournait au
//   milieu du flick et écrasait l'angle du tir: la balle silencieuse
//   partait dans la direction du spin.
//
// Ce que le joueur voit: UN frame de snap à 300+ fps (quasi invisible).
// Ce que les spectateurs voient: un flick d'un tick. La PREMIÈRE balle est
// déplacée; en full-auto les suivantes partent normales (first-bullet silent).
// La variante 100% invisible (réécriture de l'angle DANS le usercmd, aucun
// effet rendu) exige un hook CreateMove interne — mur externe/interne.
//
// Interlock: si l'aimbot est verrouillé (déjà aligné) → inactif (le clic
// part naturellement sur la cible). Le spinbot lui cède la priorité PENDANT
// le flick (flicking()), pas à son simple armement.
pub struct SilentAim {
    /// état du bouton gauche au dernier passage (détection rising edge)
    last_lbtn: bool,
    /// counts envoyés au flick, en attente de restauration
    pending: Option<(i32, i32)>,
    /// vue au moment du flick (base de la gate de consommation)
    flick_view: (f32, f32),
    /// v0.7.1 — degrés par count AU MOMENT du flick (gate de consommation)
    flick_dpc: f32,
    /// horloge de sécurité du restore (si la vue ne change jamais: 60 ms et on restaure quand même)
    flick_at: Option<Instant>,
    /// cible en conditions de tir (statut + interlock spinbot)
    pub armed: bool,
    /// nom de la dernière cible silencieuse (statut)
    pub last_target: String,
    /// flicks envoyés cette session (statut)
    pub flicks: u32,
}

impl SilentAim {
    pub fn new() -> Self {
        SilentAim { last_lbtn: false, pending: None, flick_view: (0.0, 0.0), flick_dpc: 0.022, flick_at: None, armed: false, last_target: String::new(), flicks: 0 }
    }

    /// v0.7.1 — un flick est EN COURS (counts partis, restauration en attente).
    /// C'est CE signal que le spinbot doit respecter (pas `armed`): pendant la
    /// fenêtre de ~2-5 ms, l'angle du tir doit rester celui du flick.
    pub fn flicking(&self) -> bool {
        self.pending.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        snap: Option<&Snapshot>,
        enabled: bool,
        fov_deg: f32,
        hitbox: usize,
        sens_manual: f32,
        sens_auto: Option<f32>,
        aim_active: bool,
        ctx_ok: bool,
    ) {
        self.armed = false;
        let lbtn = unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 };
        let rising = lbtn && !self.last_lbtn;
        self.last_lbtn = lbtn;

        // ── phase RESTORE: priorité absolue, même si le module vient d'être coupé ──
        if let Some((rx, ry)) = self.pending {
            // v0.7.1 — GATE DE CONSOMMATION: on restaure quand la vue est
            // ARRIVÉE à l'angle attendu (counts APPLIQUÉS par le jeu), pas sur
            // n'importe quel changement. Δattendu: Δyaw = -rx·dpc (dx>0 = yaw
            // décroît), Δpitch = +ry·dpc. Tolérance = bruit de main + recul de
            // première balle. Dwell ≥ 2 ms: laisser au moins un frame jeu
            // consommer le move avant de restaurer. Filet de sécurité: 60 ms.
            let exp_dy = -(rx as f32) * self.flick_dpc;
            let exp_dp = (ry as f32) * self.flick_dpc;
            let consumed = snap.map(|s| {
                let moved_y = wrap180(s.local_view.1 - self.flick_view.1);
                let moved_p = s.local_view.0 - self.flick_view.0;
                (moved_y - exp_dy).abs() <= 2.0 + exp_dy.abs() * 0.25
                    && (moved_p - exp_dp).abs() <= 3.0 + exp_dp.abs() * 0.25
            }).unwrap_or(false);
            let dwell_ok = self
                .flick_at
                .map(|t| t.elapsed().as_millis() >= 2)
                .unwrap_or(true);
            let timeout = self
                .flick_at
                .map(|t| t.elapsed().as_millis() >= 60)
                .unwrap_or(true);
            if (consumed && dwell_ok) || timeout {
                send_move(-rx, -ry); // retour EXACT: la main revient au pixel
                self.pending = None;
                self.flick_at = None;
            }
            return; // pendant un flick: rien d'autre ne touche la souris
        }

        if !enabled || !ctx_ok || aim_active {
            return;
        }
        let Some(s) = snap else { return };
        if !s.local_alive || !s.local_view_ok || !s.matrix_ok {
            return;
        }

        // sélection de cible: mêmes règles que l'aimbot (la plus proche angulairement)
        // v0.7.1 — + filtre DORMANT: position périmée = fantôme (voir aimbot.rs)
        // v0.8.2 — cam_pos corrigé (algèbre dés-échelonnage) + repli accordéon
        let ducking = s.local_flags & 2 != 0; // FL_DUCKING (bit 1 de m_fFlags)
        let cam = crate::aimbot::cam_pos(&s.matrix, &s.local_origin, ducking);
        let sens = sens_auto.filter(|v| (0.01..=10.0).contains(v)).unwrap_or(sens_manual).max(0.05);
        let mut best: Option<(f32, &crate::entity::Player, f32, f32)> = None;
        for p in &s.players {
            if !p.alive || p.dormant || (p.team != 2 && p.team != 3) || p.team == s.local_team || p.pawn == s.local_pawn {
                continue;
            }
            let pos = crate::aimbot::hitbox_pos(&p.bones, hitbox, &p.origin, p.ducking);
            let (tp, ty) = crate::aimbot::angles_to(cam, pos);
            let dyaw = wrap180(ty - s.local_view.1);
            let dpitch = tp - s.local_view.0;
            let ang = dyaw.hypot(dpitch);
            if ang > fov_deg {
                continue;
            }
            if best.map(|(a, _, _, _)| ang < a).unwrap_or(true) {
                best = Some((ang, p, dyaw, dpitch));
            }
        }
        let Some((_, p, dyaw, dpitch)) = best else { return };
        self.armed = true;

        if rising {
            // ── LE FLICK: counts EXACTS (pas de lissage, pas de carry) vers la
            // cible, partis AVANT le prochain frame jeu. Le clic réel sera
            // traité au même frame: le tir part sur la cible.
            let dpc = M_YAW * sens;
            let dx = (-dyaw / dpc).clamp(-150.0, 150.0).round() as i32;
            let dy = (dpitch / dpc).clamp(-150.0, 150.0).round() as i32;
            if dx != 0 || dy != 0 {
                send_move(dx, dy);
                self.pending = Some((dx, dy));
                self.flick_view = s.local_view;
                self.flick_dpc = dpc; // v0.7.1 — gate de consommation exacte
                self.flick_at = Some(Instant::now());
                self.last_target = p.name.clone();
                self.flicks += 1;
                dlog!("[snt ] flick → {} ({:+.1}°, {:+.1}°) — restore à la consommation", p.name, dyaw, dpitch);
            }
        }
    }
}

// ═══ NoRecoil FF — compensation feed-forward du punch balistique (v0.7) ═══
// Principe: la deviation balistique du recul vit dans AimPunchServices
// (pred + unpred, lu dans le snapshot). On compense CHAQUE variation du punch
// par le mouvement souris INVERSE → vue + punch = constant → les balles vont
// droit SANS attendre que la vue dévie (le RCS delta-based corrige APRÈS
// coup, sur la part trackée dans la vue seulement — le punch FF voit tout).
//
// PUNCH_SIGN: convention balistique = view + punch. Si un test live montre
// que la compensation AMPLIFIE le recul au lieu de le réduire, la convention
// du build est view − punch → flip PUNCH_SIGN à +1.0 (un seul endroit).
//
// No-spread (la question qu'on va me poser): la dispersion est tirée d'un
// RNG SEED côté SERVEUR — aucun externe ne peut la lire ni la compenser.
// La voie réelle = prédiction de seed + réécriture de l'angle du tir dans le
// usercmd (hook CreateMove interne). Ici on livre le no-recoil intégral,
// le spread reste au serveur — viser la tête + première balle reste la
// meilleure contre-mesure externe (inaccuracy monte avec shots_fired).
const PUNCH_SIGN: f32 = -1.0; // Δview = PUNCH_SIGN · Δpunch

pub struct NoRecoil {
    /// dernier punch traité (gate valeur: compense seulement les CHANGEMENTS)
    last_punch: Option<(f32, f32)>,
    /// dernier kick compensé (statut menu)
    pub last_comp: (f32, f32),
    /// compensations envoyées cette session (statut)
    pub comps: u32,
    /// actif à la dernière itération (statut)
    pub active: bool,
}

impl NoRecoil {
    pub fn new() -> Self {
        NoRecoil { last_punch: None, last_comp: (0.0, 0.0), comps: 0, active: false }
    }

    pub fn update(&mut self, snap: Option<&Snapshot>, enabled: bool, strength: f32, sens: f32, ctx_ok: bool) {
        self.active = false;
        let Some(s) = snap else { self.last_punch = None; return };
        if !enabled || !ctx_ok || !s.local_alive {
            self.last_punch = None; // hors combat: la référence re-zéro (le punch
            // est relatif à la rafale, pas absolu)
            return;
        }
        let Some(punch) = s.local_punch else { return };
        let Some((pp, py)) = self.last_punch else {
            self.last_punch = Some(punch); // premier sample: référence, pas de tir
            return;
        };
        let d_py = wrap180(punch.1 - py);
        let d_pp = punch.0 - pp;
        self.last_punch = Some(punch);
        if d_py.abs() < 1e-4 && d_pp.abs() < 1e-4 {
            return; // punch stable — rien à compenser
        }
        // Δview voulu = PUNCH_SIGN · Δpunch, converti en counts (convention
        // v0.6.2: dx = -Δyaw/dpc, dy = Δpitch/dpc)
        let dpc = M_YAW * sens.max(0.05);
        let dv_yaw = PUNCH_SIGN * d_py;
        let dv_pitch = PUNCH_SIGN * d_pp;
        let dx = (-dv_yaw * strength / dpc).round().clamp(-60.0, 60.0) as i32;
        let dy = (dv_pitch * strength / dpc).round().clamp(-60.0, 60.0) as i32;
        if dx != 0 || dy != 0 {
            send_move(dx, dy);
            self.last_comp = (d_pp, d_py);
            self.comps += 1;
            self.active = true;
        }
    }
}

// ═══ BombTimer — bombe plantée + décompte + defuse ═════════════════════════
// v0.6.1 — LE FIX DU LAG "dès que la bombe est plantée tout rame":
//   L'ancienne sonde curtime endormait la boucle 60 ms À CHAQUE itération
//   (1 kHz) tant qu'elle n'avait pas trouvé le champ → la boucle overlay
//   tombait à ~16 fps, l'aimbot/triggerbot mouraient de faim, l'ESP gelait.
//   Trois correctifs:
//   1. THROTTLE 30 Hz: le décompte affiche 0,1 s de précision — 30 lectures/s
//      suffisent. 970 itérations sur 1000 retournent immédiatement.
//   2. SONDE NON-BLOQUANTE: phase 1 échantillonne les candidats, phase 2
//      (au tick suivant, ≥50 ms plus tard) compare — zéro sleep dans la boucle.
//   3. BACKOFF 5 s: une sonde ratée ne se relance pas en rafale — un log,
//      un timestamp, on re-tente 5 s plus tard (ou au prochain plant).
//   Le décompte serveur (m_flC4Blow − curtime) reste la voie prioritaire;
//   le repli Instant local reste exact si la sonde échoue.
pub struct BombTimer {
    pub planted: bool,
    pub remaining_s: f32,
    pub timer_length: f32,
    pub defusing: bool,
    pub defuse_remaining_s: f32,
    pub defused: bool,
    pub pos: Vec3,
    planted_at: Option<Instant>,
    logged_plant: bool,
    /// v0.6 — timestamp d'explosion SERVEUR (m_flC4Blow)
    c4_blow: Option<f32>,
    /// v0.6 — offset du champ curtime DANS GlobalVars (sondé + validé)
    curtime_off: Option<u64>,
    /// v0.6 — dernier curtime lu (validation de monotonie)
    last_curtime: Option<f32>,
    /// v0.6 — source du décompte (affichée en debug: "serveur" | "local")
    pub clock_src: &'static str,
    /// v0.6.1 — throttle 30 Hz (dernier tick traité)
    last_tick: Option<Instant>,
    /// v0.6.1 — sonde non-bloquante: candidats échantillonnés en phase 1
    probe_phase: Option<(Vec<(u64, f32)>, Instant)>,
    /// v0.6.1 — backoff: dernière sonde ratée (re-essai ≥5 s)
    probe_failed_at: Option<Instant>,
}

impl BombTimer {
    pub fn new() -> Self {
        BombTimer {
            planted: false,
            remaining_s: 0.0,
            timer_length: 40.0,
            defusing: false,
            defuse_remaining_s: 0.0,
            defused: false,
            pos: Vec3::default(),
            planted_at: None,
            logged_plant: false,
            c4_blow: None,
            curtime_off: None,
            last_curtime: None,
            clock_src: "local",
            last_tick: None,
            probe_phase: None,
            probe_failed_at: None,
        }
    }

    /// v0.6.1 — SONDE curtime NON-BLOQUANTE, 2 phases espacées ≥50 ms:
    ///   phase 1: échantillonne GlobalVars+0x08..0x80 (f32 plausibles)
    ///   phase 2: relit et garde le f32 qui AVANCE en temps réel
    /// Validé par m_flC4Blow (curtime < blow) quand la bombe est posée.
    /// Zéro sleep — la boucle overlay reste à sa cadence pleine.
    fn probe_curtime(&mut self, mem: &Mem, offs: &Offsets) -> Option<u64> {
        let gv = offs.global_vars_addr(&mem.client);
        let read_at = |o: u64| -> Option<f32> {
            let v = mem.read::<f32>(gv + o)?;
            (v.is_finite() && (0.0..100_000.0).contains(&v)).then_some(v)
        };
        // backoff: une sonde ratée attend 5 s avant de re-tenter
        if let Some(t) = self.probe_failed_at {
            if t.elapsed().as_secs() < 5 {
                return None;
            }
            self.probe_failed_at = None;
        }
        // phase 1: échantillonnage
        let Some(phase) = self.probe_phase.take() else {
            let mut cands: Vec<(u64, f32)> = Vec::new();
            for off in (0x08..0x80).step_by(4) {
                if let Some(v) = read_at(off) {
                    cands.push((off, v));
                }
            }
            if cands.is_empty() {
                self.probe_failed_at = Some(Instant::now());
                return None;
            }
            self.probe_phase = Some((cands, Instant::now()));
            return None; // phase 2 au prochain tick (≥33 ms plus tard)
        };
        // phase 2: pas encore 50 ms → remet en attente sans rien faire
        let (cands, taken) = phase;
        if taken.elapsed().as_millis() < 50 {
            self.probe_phase = Some((cands, taken));
            return None;
        }
        let mut best: Option<(u64, f32, f32)> = None; // (offset, Δt, v1)
        for (off, v0) in &cands {
            let Some(v1) = read_at(*off) else { continue };
            let dt = v1 - v0;
            // fenêtre élargie: 2 phases espacées de ~50-100 ms → Δ attendu 0.03..0.25
            if !(0.02..=0.30).contains(&dt) {
                continue;
            }
            // cohérence avec c4_blow si connu (curtime doit être < blow)
            if let Some(blow) = self.c4_blow {
                if v1 >= blow {
                    continue;
                }
            }
            if best.map(|(_, bdt, _)| dt < bdt).unwrap_or(true) {
                best = Some((*off, dt, v1));
            }
        }
        match best {
            Some((off, dt, v1)) => {
                self.last_curtime = Some(v1);
                dlog!("[bomb] curtime sondé @ GlobalVars+{off:#x} (Δ={dt:.3} s — sonde non-bloquante)");
                Some(off)
            }
            None => {
                self.probe_failed_at = Some(Instant::now());
                dlog!("[bomb] sonde curtime: aucun candidat — décompte LOCAL (re-essai dans 5 s)");
                None
            }
        }
    }

    /// curtime serveur (offset sondé) — None tant que la sonde n'a pas locké.
    /// v0.6.1: la sonde est non-bloquante — None ici est NORMAL pendant
    /// ses 2 phases (~2 ticks); le décompte local tient la barre en attendant.
    fn curtime(&mut self, mem: &Mem, offs: &Offsets) -> Option<f32> {
        let gv = offs.global_vars_addr(&mem.client);
        if self.curtime_off.is_none() {
            self.curtime_off = self.probe_curtime(mem, offs);
            if self.curtime_off.is_none() {
                return None; // phase 1/2 en cours OU backoff — pas de lecture
            }
        }
        let off = self.curtime_off?;
        let v = mem.read::<f32>(gv + off).filter(|v| v.is_finite() && *v >= 0.0)?;
        // monotonie: une valeur qui recule ou saute = mauvais champ → re-sonde
        if let Some(prev) = self.last_curtime {
            if v < prev || (v - prev) > 5.0 {
                dwarn!("[bomb] curtime incohérent ({prev:.1} → {v:.1}) — re-sonde");
                self.curtime_off = None;
                self.last_curtime = None;
                self.probe_phase = None;
                return None;
            }
        }
        self.last_curtime = Some(v);
        Some(v)
    }

    /// appelé à chaque itération de la boucle overlay (~1 kHz) — `dt_s` non
    /// utilisé (décompte serveur/Instant). v0.6.1: THROTTLE 30 Hz interne —
    /// 970 appels sur 1000 retournent immédiatement, le décompte garde
    /// 0,1 s de précision affichée, la boucle ne s'endort PLUS jamais.
    pub fn update(&mut self, mem: &Mem, offs: &Offsets, dt_s: f32) {
        if let Some(t) = self.last_tick {
            if t.elapsed().as_millis() < 33 {
                let _ = dt_s;
                return; // tick suivant dans ≥33 ms — état affiché inchangé
            }
        }
        self.last_tick = Some(Instant::now());

        let rules_planted = mem
            .read::<u64>(offs.game_rules_slot(&mem.client))
            .filter(|p| (0x10000..0x0000_7FFF_FFFF_FFFF).contains(p))
            .and_then(|rules| mem.read::<u8>(rules + offs.m_b_bomb_planted_rules))
            .map(|b| b != 0)
            .unwrap_or(false);

        if !rules_planted {
            if self.planted {
                dlog!("[bomb] plus de bombe plantée (fin de manche / explosion)");
            }
            self.planted = false;
            self.planted_at = None;
            self.defusing = false;
            self.defused = false;
            self.logged_plant = false;
            self.c4_blow = None;
            self.last_curtime = None;
            self.probe_phase = None; // v0.6.1 — sonde remise à zéro
            self.clock_src = "local";
            return;
        }

        // bombe plantée → l'entité C_PlantedC4 par dwPlantedC4 (pin corrigé)
        let c4 = mem
            .read::<u64>(offs.planted_c4_slot(&mem.client))
            .filter(|p| (0x10000..0x0000_7FFF_FFFF_FFFF).contains(p));

        if !self.planted {
            self.planted = true;
            self.planted_at = Some(Instant::now());
            // mèche: clampée 5..90 (40 s standard, 80 si le mode la rallonge)
            self.timer_length = c4
                .and_then(|c| mem.read::<f32>(c + offs.m_fl_timer_length))
                .filter(|v| v.is_finite() && (5.0..90.0).contains(v))
                .unwrap_or(40.0);
            // v0.6 — timestamp d'explosion serveur
            self.c4_blow = c4
                .and_then(|c| mem.read::<f32>(c + offs.m_fl_c4_blow))
                .filter(|v| v.is_finite() && (0.0..3600.0).contains(v));
            self.curtime_off = None; // re-sonde pour ce round
            self.last_curtime = None;
            self.probe_phase = None;
            self.probe_failed_at = None;
            if !self.logged_plant {
                dlog!(
                    "[bomb] PLANTÉE — mèche {:.0} s · c4_blow={:?} (horloge serveur)",
                    self.timer_length, self.c4_blow
                );
                self.logged_plant = true;
            }
        }

        // ── v0.6 — DÉCOMPTE: horloge serveur si sonde OK, sinon Instant ──
        let elapsed = self.planted_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
        let blow = self.c4_blow;
        let now = self.curtime(mem, offs);
        match (blow, now) {
            (Some(blow), Some(now)) => {
                self.remaining_s = (blow - now).clamp(0.0, 95.0);
                self.clock_src = "serveur";
            }
            _ => {
                self.remaining_s = (self.timer_length - elapsed).max(0.0);
                self.clock_src = "local";
            }
        }

        self.defused = c4
            .and_then(|c| mem.read::<u8>(c + offs.m_b_bomb_defused))
            .map(|b| b != 0)
            .unwrap_or(false);

        let defuse = c4
            .and_then(|c| mem.read::<f32>(c + offs.m_fl_defuse_countdown))
            .filter(|v| v.is_finite() && (0.0..20.0).contains(v))
            .unwrap_or(0.0);
        self.defusing = defuse > 0.0 && !self.defused;
        self.defuse_remaining_s = defuse;

        // position (distance au bandeau) — node du C4
        if let Some(c) = c4 {
            if let Some(node) = mem.read::<u64>(c + offs.m_p_game_scene_node) {
                if (0x10000..0x0000_7FFF_FFFF_FFFF).contains(&node) {
                    if let Some(p) = mem.read::<Vec3>(node + offs.m_vec_abs_origin) {
                        if p.x.is_finite() {
                            self.pos = p;
                        }
                    }
                }
            }
        }
        let _ = dt_s; // décompte serveur/Instant (plus précis que l'accumulation dt)
    }

    /// distance bombe ↔ joueur local (m)
    pub fn dist_m(&self, local: &Vec3) -> f32 {
        self.pos.dist(local) * 0.019
    }
}

// ═══ SoundEsp — footsteps en anneaux (proxy vélocité) ══════════════════════
pub struct SoundEvent {
    pub pos: Vec3,
    pub at: Instant,
    pub team: i32, // 2 T / 3 CT — colore l'anneau
}

pub struct SoundEsp {
    pub events: Vec<SoundEvent>,
    last_per_pawn: HashMap<u64, Instant>, // anti-spam: 1 pas / 350 ms / joueur
    pub enabled_events: usize,            // compteur pour le statut
}

const STEP_THRESHOLD: f32 = 215.0; // u/s — course ~250, marche ~130
const EVENT_TTL_MS: u128 = 1200;
const PER_PAWN_COOLDOWN_MS: u128 = 350;

impl SoundEsp {
    pub fn new() -> Self {
        SoundEsp { events: Vec::new(), last_per_pawn: HashMap::new(), enabled_events: 0 }
    }

    /// appelé à chaque itération — détecte les pas ennemis.
    /// `active` = option sound esp cochée (sinon juste purge).
    pub fn update(&mut self, snap: Option<&Snapshot>, active: bool) {
        // purge des événements expirés
        let now = Instant::now();
        self.events.retain(|e| now.duration_since(e.at).as_millis() < EVENT_TTL_MS);
        if !active {
            self.enabled_events = self.events.len();
            return;
        }
        let Some(s) = snap else { return };
        if !s.local_alive {
            return;
        }
        for p in &s.players {
            if !p.alive || p.pawn == s.local_pawn || (p.team != 2 && p.team != 3) {
                continue;
            }
            if p.team == s.local_team {
                continue; // ennemis uniquement
            }
            let speed = (p.velocity.x * p.velocity.x
                + p.velocity.y * p.velocity.y
                + p.velocity.z * p.velocity.z)
                .sqrt();
            if speed < STEP_THRESHOLD {
                continue;
            }
            let fresh = match self.last_per_pawn.get(&p.pawn) {
                Some(at) => now.duration_since(*at).as_millis() >= PER_PAWN_COOLDOWN_MS,
                None => true,
            };
            if fresh {
                self.last_per_pawn.insert(p.pawn, now);
                self.events.push(SoundEvent { pos: p.origin.z_up(36.0), at: now, team: p.team });
            }
        }
        self.enabled_events = self.events.len();
    }

    /// âge normalisé 0..1 d'un événement (pour le rayon/l'alpha du rendu)
    pub fn age01(&self, e: &SoundEvent) -> f32 {
        (e.at.elapsed().as_millis() as f32 / EVENT_TTL_MS as f32).min(1.0)
    }
}

// ═══ Bunnyhop — tap espace sur FL_ONGROUND ═════════════════════════════════
pub struct Bunnyhop {
    last_tap: Option<Instant>,
    pub jumps: u32, // compteur session (statut)
}

impl Bunnyhop {
    pub fn new() -> Self {
        Bunnyhop { last_tap: None, jumps: 0 }
    }

    /// `space_down` = ESPACE physiquement tenu par le joueur.
    /// On tape au moment où le bit SOL est set — le jeu voit un vrai appui.
    pub fn update(&mut self, snap: Option<&Snapshot>, enabled: bool, space_down: bool, ctx_ok: bool) {
        let Some(s) = snap else { return };
        if !enabled || !ctx_ok || !s.local_alive || !space_down {
            return;
        }
        const FL_ONGROUND: i32 = 1;
        if s.local_flags & FL_ONGROUND == 0 {
            return; // en l'air — attendre l'atterrissage
        }
        let cooldown_ok = self
            .last_tap
            .map(|t| t.elapsed().as_millis() >= 30)
            .unwrap_or(true);
        if cooldown_ok {
            send_tap(VK_SPACE);
            self.last_tap = Some(Instant::now());
            self.jumps += 1;
        }
    }
}

// ═══ AccuracyLab — stats de précision de session ═══════════════════════════
pub struct AccuracyLab {
    pub shots: u32,
    pub hits: u32,
    pub kills: u32,
    pub damage: u32,
    pub weapon: String,
    last_shots: i32,
    last_hp: HashMap<u64, i32>, // pawn ennemi → dernier HP
    last_hit_at: Option<Instant>,
    weapon_refreshed_at: Option<Instant>,
}

const HIT_COOLDOWN_MS: u128 = 60; // ≥ 900 rpm sans double-comptage

impl AccuracyLab {
    pub fn new() -> Self {
        AccuracyLab {
            shots: 0,
            hits: 0,
            kills: 0,
            damage: 0,
            weapon: "—".into(),
            last_shots: 0,
            last_hp: HashMap::new(),
            last_hit_at: None,
            weapon_refreshed_at: None,
        }
    }

    /// appelé à chaque itération — `weapon` est résolu par la boucle overlay
    /// (world::local_weapon_name, ~2 Hz) et injecté ici.
    pub fn update(&mut self, snap: Option<&Snapshot>, active: bool, weapon: Option<&str>) {
        let Some(s) = snap else { return };
        if let Some(w) = weapon {
            self.weapon = w.to_string();
            self.weapon_refreshed_at = Some(Instant::now());
        } else if self.weapon_refreshed_at.map(|t| t.elapsed().as_secs() > 5).unwrap_or(true) {
            self.weapon = "—".into();
        }
        if !active {
            self.last_shots = s.shots_fired.max(0);
            self.track_hp(s);
            return;
        }

        // ── tirs: fronts montants de m_iShotsFired ──
        let shots = s.shots_fired.max(0);
        if shots > self.last_shots {
            self.shots += (shots - self.last_shots) as u32;
        }
        self.last_shots = if shots == 0 { 0 } else { shots };

        // ── hits: un ennemi VIVANT perd du HP pendant que TU tires ──
        if shots > 0 {
            let now = Instant::now();
            let cooldown_ok = self
                .last_hit_at
                .map(|t| t.elapsed().as_millis() >= HIT_COOLDOWN_MS)
                .unwrap_or(true);
            if cooldown_ok {
                for p in &s.players {
                    if !p.alive || p.team == s.local_team || (p.team != 2 && p.team != 3) {
                        continue;
                    }
                    if let Some(prev) = self.last_hp.get(&p.pawn) {
                        if p.health < *prev && *prev > 0 && p.health >= 0 {
                            let dmg = (*prev - p.health) as u32;
                            self.hits += 1;
                            self.damage += dmg;
                            self.last_hit_at = Some(now);
                            if p.health == 0 {
                                self.kills += 1;
                                dlog!("[ lab] élimination confirmée ({} dmg)", dmg);
                            }
                            break; // un hit par tick de cooldown
                        }
                    }
                }
            }
        }
        self.track_hp(s);
    }

    fn track_hp(&mut self, s: &Snapshot) {
        self.last_hp.retain(|pawn, _| s.players.iter().any(|p| &p.pawn == pawn));
        for p in &s.players {
            if p.alive && (p.team == 2 || p.team == 3) && p.team != s.local_team {
                self.last_hp.insert(p.pawn, p.health);
            }
        }
    }

    #[allow(dead_code)] // API du labo — le menu calcule depuis StatusInfo
    pub fn accuracy(&self) -> f32 {
        if self.shots == 0 {
            0.0
        } else {
            self.hits as f32 / self.shots as f32 * 100.0
        }
    }
}
