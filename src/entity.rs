// entity — marche de la liste d'entités CS2, chaque étape tracée en debug.
//
// Algorithme — VÉRIFIÉ EN LIVE build 14193 (29.10.2026):
//   system = *(client.dll + dwEntityList)      ← PREMIER DÉRÉFÉRENCEMENT OBLIGATOIRE
//   pour chaque index:
//     chunk = *(system + 8*(index >> 9) + 0x10)
//     obj   = *(chunk + STRIDE * (index & 0x1FF))     [STRIDE = 0x70, configurable]
//   l'objet EST l'entité (contrôleur ou pawn), pas une identité intermédiaire.
//   handle = *(ctrl + m_hPlayerPawn) → u32, index pawn = handle & 0x7FFF
//
// BONES (v0.3 — format CORRIGÉ): le node d'un pawn EST un CSkeletonInstance;
//   modelState = node + 0x140 (EMBARQUÉ) ; cache d'os = *(modelState + 0x80).
//   Chaque os = transform de 0x20 octets: position (x,y,z) dans les 3 PREMIERS
//   f32, quaternion ensuite. Vérifié contre burakacar6/cs2-entitylist-python
//   et IMXNOOBX/cs2-external-esp (bone_data { Vec3 pos; pad[0x14] }).
//   L'ancienne hypothèse "matrices 3×4 / stride 0x30 / translation f[3,7,11]"
//   était FAUSSE — c'est le bug squelette de la v0.2-flash.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::debug;
use crate::math::{Vec3, ViewMatrix, PLAYER_HEIGHT};
use crate::mem::Mem;
use crate::offsets::Offsets;
use crate::{dlog, dmem, dwarn};

// stride par défaut conservé pour l'outillage debug externe — la valeur
// vivante vient de offs.list_entry_stride (offsets.json la surcharge)
#[allow(dead_code)]
pub const LIST_ENTRY_STRIDE: u64 = 0x70;

// ── squelette: indices d'os du modèle player CS2 ─────────────────────────────
// (table communautaire éprouvée — IMXNOOBX/cs2-external-esp, Bones.hpp)
pub const B_HEAD: usize = 7;
pub const B_PELVIS: usize = 1;
/// connexions (parent, enfant) — 17 segments, dessinés par w2s
pub const BONE_CONNECTS: [(usize, usize); 17] = [
    // colonne
    (1, 3),   // bassin → spine_1
    (3, 4),   // spine_1 → spine_2
    (4, 23),  // spine_2 → poitrine
    (23, 6),  // poitrine → cou
    (6, 7),   // cou → tête
    // bras gauche
    (6, 9),   // cou → épaule G
    (9, 10),  // épaule G → coude G
    (10, 11), // coude G → main G
    // bras droit
    (6, 13),  // cou → épaule D
    (13, 14), // épaule D → coude D
    (14, 15), // coude D → main D
    // jambe gauche
    (1, 17),  // bassin → hanche G
    (17, 18), // hanche G → genou G
    (18, 19), // genou G → pied G
    // jambe droite
    (1, 20),  // bassin → hanche D
    (20, 21), // hanche D → genou D
    (21, 22), // genou D → pied D
];

// nombre d'os lus par joueur (0..30 couvre tout le humanoïde: tête incluse)
pub const N_BONES: usize = 30;
// entrée du cache = transform 0x20: Vec3 pos (12 o) + quaternion (16 o) + pad
// (v0.4: stride vivant = offs.bone_stride — surchargeable par offsets.json)
pub const BONE_STRIDE: u64 = 0x20;

/// os effectivement dessinés (points d'articulation) — v0.4: petits points
/// aux articulations pour que le squelette se LISE même petit/loin
pub const JOINT_BONES: [usize; 18] = [1, 3, 4, 23, 6, 7, 9, 10, 11, 13, 14, 15, 17, 18, 19, 20, 21, 22];

// champs conservés pour l'outillage debug externe (dump console / F4)
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Player {
    pub slot: usize,
    pub controller: u64,
    pub pawn: u64,
    pub name: String,
    pub health: i32,
    pub team: i32,    // 2 = T, 3 = CT
    pub origin: Vec3, // m_vecAbsOrigin — aux pieds
    pub alive: bool,
    pub dormant: bool,        // v0.7.1 — m_bDormant: hors PVS → origin/os PÉRIMÉS (fantômes pour l'aim)
    pub ducking: bool,        // v0.9.6 — m_fFlags FL_DUCKING: repli de visée posture-dépendant (aimbot)
    pub velocity: Vec3,      // v0.5 — m_vecVelocity (sound ESP: pas > 215 u/s)
    pub observer_mode: i32,  // v0.5 — >0 = spectate (m_iObserverMode)
    pub observer_target: u32, // v0.5 — handle de la cible observée (m_hObserverTarget)
    pub bones: Option<Vec<Vec3>>, // N_BONES positions monde (None = pas lu / stale)
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    /// v0.8 — HORODATAGE de fin de lecture (interpolation de rendu).
    /// L'ESP extrapole origin + velocity·âge pour afficher des mouvements
    /// fluides entre deux ticks serveur (64 tick vs ~166 fps d'affichage).
    /// La logique de tir (trigger/aim) reste sur les positions BRUTES.
    pub taken_at: Instant,
    pub local_pawn: u64,
    pub local_team: i32,
    pub local_origin: Vec3,
    pub local_alive: bool,
    pub local_eye: (f32, f32), // (pitch, yaw) degrés — debug/sonde (m_angEyeAngles)
    pub local_eye_ok: bool,
    pub local_view: (f32, f32),  // v0.5 — vue LIVE dwViewAngles (fallback: eye)
    pub local_view_ok: bool,
    pub local_view_src: &'static str, // "dwViewAngles" | "m_angEyeAngles"
    pub local_sens: Option<f32>,  // v0.5 — sensibilité RÉELLE lue dans le jeu
    pub local_punch: Option<(f32, f32)>, // v0.7 — recul balistique (pitch, yaw) pred+unpred AimPunch
    pub local_flags: i32,         // v0.5 — m_fFlags local (bunnyhop: bit 0 = sol)
    pub shots_fired: i32,         // v0.5 — m_iShotsFired local (RCS + labo)
    pub buy_menu: bool,        // m_bIsBuyMenuOpen — coupe l'ESP pendant l'achat
    pub crosshair_id: i32,     // m_iIDEntIndex brut (0 = rien sous le viseur)
    pub crosshair_enemy: bool, // ennemi vivant sous le viseur — triggerbot
    pub matrix: ViewMatrix,
    pub matrix_ok: bool,
    pub players: Vec<Player>,
    pub spectators: Vec<String>, // v0.5 — noms de ceux qui TE regardent
}

// résolution d'un index dans la liste d'entités → pointeur d'objet
// `system` = *(client.dll + dwEntityList) — le déréférencement est fait par l'appelant
// v0.5: pub(crate) — le walk du MONDE (armes/grenades) réutilise la même mécanique
pub(crate) fn list_get(mem: &Mem, system: u64, index: u64, stride: u64) -> Option<u64> {
    let chunk = mem.read::<u64>(system + 8 * (index >> 9) + 0x10)?;
    if !looks_like_ptr(chunk) {
        return None;
    }
    let obj = mem.read::<u64>(chunk + stride * (index & 0x1FF))?;
    Some(obj)
}

pub(crate) fn looks_like_ptr(v: u64) -> bool {
    (0x10000..0x0000_7FFF_FFFF_FFFF).contains(&v)
}

// ═══ v0.8 — BULK READS: un RPM par pawn au lieu de ~15 ═══════════════════
// Chaque RPM = syscall + copie (~1-2 µs). Le walk lisait champ par champ:
// ~15 appels/pawn × 10 joueurs × 1 kHz = ~150k syscalls/s — LE goulot de la
// boucle. Ici on lit LE PAWN ENTIÈREMENT en UN appel (0x3700 octets, la
// taille réelle de l'objet — m_angEyeAngles vit à 0x3600) et on sert les
// champs depuis le buffer local. Les lectures À TRAVERS pointeurs (node,
// bones, services) restent individuelles — leurs adresses sont inconnues.
// Échec du bulk (page non mappée en fin d'objet?) → repli champ par champ.
pub(crate) const PAWN_BULK: usize = 0x3700;

pub(crate) struct Bulk {
    buf: Vec<u8>,
}

impl Bulk {
    pub fn read_block(mem: &Mem, base: u64, size: usize) -> Option<Bulk> {
        let mut buf = vec![0u8; size];
        if mem.read_arr(base, &mut buf) {
            Some(Bulk { buf })
        } else {
            None
        }
    }
    fn slice(&self, off: u64, len: usize) -> Option<&[u8]> {
        let off = off as usize;
        off.checked_add(len).and_then(|end| self.buf.get(off..end))
    }
    pub fn u8(&self, off: u64) -> Option<u8> {
        self.slice(off, 1).map(|s| s[0])
    }
    pub fn u32(&self, off: u64) -> Option<u32> {
        self.slice(off, 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn u64(&self, off: u64) -> Option<u64> {
        self.slice(off, 8).map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
    }
    pub fn f32(&self, off: u64) -> Option<f32> {
        self.slice(off, 4).map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn vec3(&self, off: u64) -> Option<Vec3> {
        Some(Vec3 { x: self.f32(off)?, y: self.f32(off + 4)?, z: self.f32(off + 8)? })
    }
}

/// lecteur de champ d'un pawn: sert depuis le BULK si dispo, sinon RPM
/// direct à l'ancienne (repli). Même signatures que mem.read → le code du
/// walk ne change que par la source.
pub(crate) struct PawnReader<'a> {
    mem: &'a Mem,
    pub pawn: u64,
    bulk: Option<&'a Bulk>,
}

impl<'a> PawnReader<'a> {
    pub fn new(mem: &'a Mem, pawn: u64, bulk: Option<&'a Bulk>) -> Self {
        PawnReader { mem, pawn, bulk }
    }
    /// lecture typée à l'offset du pawn — bulk d'abord, RPM en repli.
    /// Sizes 1/4/8 servies depuis le buffer; le reste part en RPM.
    pub fn read<T: Copy>(&self, off: u64) -> Option<T> {
        let size = std::mem::size_of::<T>();
        if let Some(b) = self.bulk {
            let raw: Option<u64> = match size {
                1 => b.u8(off).map(|v| v as u64),
                4 => b.u32(off).map(|v| v as u64),
                8 => b.u64(off),
                _ => None,
            };
            if let Some(v) = raw {
                // SAFETY: T fait exactement `size` octets — la valeur brute
                // zéro-étendue retranscrite octet par octet en little-endian.
                let mut out = [0u8; 8];
                out[..size].copy_from_slice(&v.to_le_bytes()[..size]);
                return Some(unsafe { std::ptr::read_unaligned(out.as_ptr() as *const T) });
            }
        }
        self.mem.read::<T>(self.pawn + off)
    }
    /// Vec3 à l'offset du pawn (bulk ou repli).
    pub fn vec3(&self, off: u64) -> Option<Vec3> {
        if let Some(b) = self.bulk {
            if let Some(v) = b.vec3(off) {
                return Some(v);
            }
        }
        self.mem.read::<Vec3>(self.pawn + off)
    }
}

// ── v0.5/v0.6: vue LIVE pour l'aimbot/RCS ──────────────────────────────────
// m_angEyeAngles (pawn) est rafraîchi au TICK SERVEUR et traîne — la boucle
// tournait à 1 kHz dessus et renvoyait ~15× le même delta calculé sur une vue
// périmée: overshoot en éventail. La vue fraîche vit côté input.
//
// v0.6 — CONVENTION a2x CORRIGÉE (le bug "aimbot ne marche pas"):
//   Dans les dumps a2x/cs2-dumper, dwViewAngles est l'adresse DIRECTE (dans
//   client.dll) du champ QAngle — on lit les 2 f32 À CETTE ADRESSE, SANS
//   déréférencer dwCSGOInput. L'ancien code déréférençait le slot dwCSGOInput
//   puis lisait à ptr + (dwViewAngles − dwCSGOInput): avec le pin FAUX
//   (0x257F940 au lieu de 0x257F980) le delta valait 0x6C8 au lieu de 0x688
//   → lecture de garbage → sanity refusée → repli silencieux sur la vue
//   périmée du pawn → l'aimbot partait de données fausses.
//
//   Ordre de tentatives:
//     1. LECTURE DIRECTE client + dwViewAngles (convention a2x, la bonne)
//     2. CSGOInput déréférencé + delta (certaines builds internes)
//     3. m_angEyeAngles du pawn (fallback — traîne d'un tick)
fn read_view_angles(mem: &Mem, offs: &Offsets) -> Option<((f32, f32), &'static str)> {
    let sane = |p: f32, y: f32| p.is_finite() && y.is_finite() && p.abs() <= 90.0 && y.abs() <= 180.0;
    // 1) CONVENTION a2x: le champ QAngle vit DIRECTEMENT à client+dwViewAngles
    if let Some(a) = mem.read::<[f32; 2]>(offs.dw_view_angles_addr(&mem.client)) {
        if sane(a[0], a[1]) {
            return Some(((a[0], a[1]), "dwViewAngles"));
        }
    }
    // 2) CSGOInput → champ QAngle (convention alternative)
    if let Some(input) = mem.read::<u64>(offs.dw_csgo_input_addr(&mem.client)).filter(|p| looks_like_ptr(*p)) {
        let field = offs.dw_view_angles.saturating_sub(offs.dw_csgo_input);
        if let Some(a) = mem.read::<[f32; 2]>(input + field) {
            if sane(a[0], a[1]) {
                return Some(((a[0], a[1]), "dwViewAngles*"));
            }
        }
    }
    None
}

/// sensibilité RÉELLE du jeu (dwSensitivity → structure → champ +0x58).
/// double tentative: ptr(client+dwSensitivity) puis champ, sinon direct.
fn read_sensitivity(mem: &Mem, offs: &Offsets) -> Option<f32> {
    let slot = offs.dw_sensitivity_addr(&mem.client);
    let try_val = |addr: u64| -> Option<f32> {
        let v = mem.read::<f32>(addr)?;
        (0.01..=10.0).contains(&v).then_some(v)
    };
    if let Some(p) = mem.read::<u64>(slot).filter(|p| looks_like_ptr(*p)) {
        if let Some(v) = try_val(p + offs.dw_sensitivity_field) {
            return Some(v);
        }
    }
    try_val(slot + offs.dw_sensitivity_field)
}

/// v0.7 — recul balistique: AimPunchServices → pred + unpred (pitch, yaw).
/// Le punch est rafraîchi par le jeu AU TICK (64 Hz) — le no-recoil FF
/// compense CHAQUE changement mesuré (gate valeur, comme la gate vue).
fn read_aim_punch(mem: &Mem, offs: &Offsets, pawn: u64) -> Option<(f32, f32)> {
    let svc = mem.read::<u64>(pawn + offs.m_p_aim_punch_services)?;
    if !looks_like_ptr(svc) {
        return None;
    }
    let pred = mem.read::<[f32; 2]>(svc + offs.punch_pred_angle)?;
    let unpred = mem.read::<[f32; 2]>(svc + offs.punch_unpred_angle)?;
    let (p, y) = (pred[0] + unpred[0], pred[1] + unpred[1]);
    let sane = p.is_finite() && y.is_finite() && p.abs() <= 30.0 && y.abs() <= 30.0;
    sane.then_some((p, y))
}

// le champ nom a changé de format selon les époques: char* ou inline.
// les deux tentatives sont tracées — le debug te dit laquelle marche.
// ⚠ le nom est relu À CHAQUE FRAME (l'ESP l'affiche) → le journal ne tire
// QUE quand la valeur change pour un slot (sinon ~1000 lignes/s de spam).
static LAST_NAMES: OnceLock<Mutex<HashMap<usize, String>>> = OnceLock::new();

fn log_name_change(slot: usize, s: &str, format: &str) {
    let map = LAST_NAMES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut m) = map.lock() {
        if m.get(&slot).map(|old| old.as_str()) != Some(s) {
            m.insert(slot, s.to_string());
            dlog!("[ent ] nom slot {slot}: \"{s}\" ({format})");
        }
    }
}

fn read_name(mem: &Mem, ctrl: u64, off: u64, slot: usize) -> String {
    if let Some(ptr) = mem.read::<u64>(ctrl + off) {
        if looks_like_ptr(ptr) {
            if let Some(s) = mem.read_ascii(ptr, 32) {
                if !s.is_empty() && s.chars().all(|c| !c.is_control()) {
                    log_name_change(slot, &s, "char*");
                    return s;
                }
            }
        }
    }
    if let Some(s) = mem.read_ascii(ctrl + off, 32) {
        if !s.is_empty() && s.chars().all(|c| !c.is_control()) {
            log_name_change(slot, &s, "inline");
            return s;
        }
    }
    format!("slot_{slot}")
}

pub fn snapshot(mem: &Mem, offs: &Offsets, verbose_entities: bool) -> Option<Snapshot> {
    snapshot_f(mem, offs, verbose_entities, false, 0)
}

pub fn snapshot_full(
    mem: &Mem,
    offs: &Offsets,
    verbose_entities: bool,
    want_bones: bool,
    trig_hitbox: usize, // v0.4: 0 = m_iIDEntIndex (tout) · 1 tête · 2 corps · 3 membres
) -> Option<Snapshot> {
    snapshot_f(mem, offs, verbose_entities, want_bones, trig_hitbox)
}

fn snapshot_f(mem: &Mem, offs: &Offsets, verbose_entities: bool, want_bones: bool, trig_hitbox: usize) -> Option<Snapshot> {
    let stride = offs.list_entry_stride;
    // dwEntityList/dwGameEntitySystem = SLOT du pointeur CGameEntitySystem — il
    // faut déréférencer une fois (list_get travaille sur le pointeur système).
    let el = match mem.read::<u64>(offs.entity_list(&mem.client)) {
        Some(v) if looks_like_ptr(v) => v,
        _ => {
            dwarn!("dwEntityList @ {:#x} ne déréférence pas vers un ptr valide — offset stale?", offs.entity_list(&mem.client));
            return None;
        }
    };
    let vm_addr = offs.view_matrix(&mem.client);
    let lp_slot = offs.local_pawn_slot(&mem.client);

    // matrice — lue séparément: une matrice morte ne doit PAS masquer le reste
    // (menu = matrice à zéro, c'est normal; offset stale = matrice pourrie).
    // w2s est simplement désactivé pour la frame, le walk d'entités continue.
    let matrix = ViewMatrix { m: mem.read::<[f32; 12]>(vm_addr).unwrap_or([0.0; 12]) };
    let matrix_ok = matrix.sane();
    if !matrix_ok && crate::debug::once_per_sec() {
        dwarn!("dwViewMatrix @ {:#x} invalide (norme axe-Z={:.3}) — w2s coupé; menu OU offset stale → re-dump (voir offsets.rs)", vm_addr, matrix.m[10]);
    }

    // pawn local — le slot client.dll + dwLocalPlayerPawn contient le pointeur
    let local_pawn = mem.read::<u64>(lp_slot)?;
    if !looks_like_ptr(local_pawn) {
        if crate::debug::once_per_sec() {
            dwarn!("dwLocalPlayerPawn @ {:#x} = {:#x} invalide — pas spawné ou offset stale", lp_slot, local_pawn);
        }
        return None;
    }
    dmem!(lp_slot, "local_pawn (slot client+dwLocalPlayerPawn)", local_pawn);

    // v0.8 — PAWN LOCAL EN BULK: un seul RPM couvre tous les champs jusqu'à
    // 0x3700 (team/node/health/life/buy/eye/flags/shots/ident). Repli champ
    // par champ si le bloc échoue (page non mappée).
    let local_bulk = Bulk::read_block(mem, local_pawn, PAWN_BULK);
    let lpr = PawnReader::new(mem, local_pawn, local_bulk.as_ref());

    let local_team = lpr.read::<u8>(offs.m_i_team_num).map(|t| t as i32).unwrap_or(-1);
    let local_node = lpr.read::<u64>(offs.m_p_game_scene_node).unwrap_or(0);
    let local_origin = if looks_like_ptr(local_node) {
        mem.read::<Vec3>(local_node + offs.m_vec_abs_origin).unwrap_or_default()
    } else {
        Vec3::default()
    };
    dmem!(local_pawn + offs.m_i_team_num, "local m_iTeamNum", local_team);
    dmem!(local_node + offs.m_vec_abs_origin, "local m_vecAbsOrigin", format!("({:.1},{:.1},{:.1})", local_origin.x, local_origin.y, local_origin.z));

    // état local: vivant ? menu d'achat ? (degrés: pitch ∈ [-89..89], yaw ∈ [-180..180])
    let local_health = lpr.read::<i32>(offs.m_i_health).unwrap_or(-1);
    let local_life = lpr.read::<u8>(offs.m_life_state).unwrap_or(1);
    let local_alive = local_health > 0 && local_life == 0;
    let buy_menu = lpr.read::<u8>(offs.m_buy_menu_open).unwrap_or(0) != 0;
    let local_eye = lpr
        .read::<[f32; 2]>(offs.m_ang_eye_angles)
        .map(|a| (a[0], a[1]))
        .unwrap_or((0.0, 0.0));
    let local_eye_ok = local_eye.0.is_finite() && local_eye.1.is_finite()
        && local_eye.0.abs() <= 90.0 && local_eye.1.abs() <= 180.0;

    // ── v0.5: vue LIVE (CSGOInput), sensibilité réelle, flags, tirs ──
    let (local_view, local_view_src) = match read_view_angles(mem, offs) {
        Some((v, src)) => (v, src),
        None => (local_eye, "m_angEyeAngles"),
    };
    let local_view_ok = local_view.0.is_finite() && local_view.1.is_finite()
        && local_view.0.abs() <= 90.0 && local_view.1.abs() <= 180.0;
    let local_sens = read_sensitivity(mem, offs);
    // ── v0.7 — recul balistique (AimPunchServices) pour le no-recoil FF ──
    // deviation ≈ predictableBaseAngle + unpredictableBaseAngle (QAngle).
    // Les deux QAngle vivent DANS le services: pred @ +0x50, unpred @ +0xA4.
    // Sanity: |angle| ≤ 30° (un punch de 45° n'existe pas) sinon None.
    let local_punch = read_aim_punch(mem, offs, local_pawn);
    let local_flags = lpr.read::<i32>(offs.m_f_flags).unwrap_or(0);
    let shots_fired = lpr.read::<i32>(offs.m_i_shots_fired).unwrap_or(0);
    dmem!(offs.csgo_input_slot(&mem.client), "CSGOInput slot", format!("vue src={local_view_src} sens={:?}", local_sens));

    // ── triggerbot: entité sous le viseur ──
    // m_iIDEntIndex = index d'entité pointé par le crosshair. On le résout,
    // on lit team/vie; si l'objet est un contrôleur (lecture invalide) on
    // retente via son handle m_hPlayerPawn. Ennemi = équipe adverse vivante.
    let crosshair_id = lpr.read::<i32>(offs.m_i_ident_index).unwrap_or(0);
    // ── v0.4: modes hitbox du triggerbot ──
    // m_iIDEntIndex ne dit PAS quelle hitbox est visée (n'importe où sur le
    // modèle compte). Mode 0 (tout) = m_iIDEntIndex tel quel; les modes
    // ciblés (tête/corps/membres) sont résolus APRÈS le walk d'entités (les
    // os du cache squelette sont requis) — voir la projection normalisée
    // en bas de snapshot_f.
    let crosshair_enemy_any = if trig_hitbox == 0 {
        crosshair_id > 0 && {
            let mut enemy = false;
            if let Some(ent) = list_get(mem, el, crosshair_id as u64, stride).filter(|e| looks_like_ptr(*e)) {
                let team = mem.read::<u8>(ent + offs.m_i_team_num).map(|t| t as i32).unwrap_or(-1);
                let health = mem.read::<i32>(ent + offs.m_i_health).unwrap_or(-1);
                let life = mem.read::<u8>(ent + offs.m_life_state).unwrap_or(1);
                if (2..=3).contains(&team) && (1..=200).contains(&health) && life == 0 {
                    // ressemble à un pawn vivant
                    enemy = team != local_team;
                } else if let Some(h) = mem.read::<u32>(ent + offs.m_h_player_pawn) {
                    // peut-être un contrôleur → suis son handle
                    if (h & 0x7FFF) != 0 {
                        if let Some(pawn) = list_get(mem, el, (h & 0x7FFF) as u64, stride).filter(|p| looks_like_ptr(*p)) {
                            let t2 = mem.read::<u8>(pawn + offs.m_i_team_num).map(|t| t as i32).unwrap_or(-1);
                            let h2 = mem.read::<i32>(pawn + offs.m_i_health).unwrap_or(-1);
                            let l2 = mem.read::<u8>(pawn + offs.m_life_state).unwrap_or(1);
                            enemy = (2..=3).contains(&t2) && (1..=200).contains(&h2) && l2 == 0 && t2 != local_team;
                        }
                    }
                }
            }
            enemy
        }
    } else {
        false // modes ciblés: résolu après le walk (os requis)
    };

    let mut players = Vec::new();
    for slot in 1usize..=64 {
        // ── contrôleur ──
        let ctrl = match list_get(mem, el, slot as u64, stride) {
            Some(c) if looks_like_ptr(c) => c,
            _ => {
                if verbose_entities {
                    dlog!("[ent ] slot {slot}: pas de contrôleur (menu / spectateur)");
                }
                continue;
            }
        };
        // ── handle → pawn ──
        let handle = match mem.read::<u32>(ctrl + offs.m_h_player_pawn) {
            Some(h) if (h & 0x7FFF) != 0 => h,
            _ => {
                if verbose_entities {
                    dlog!("[ent ] slot {slot}: ctrl {:#x} sans pawn (handle nul)", ctrl);
                }
                continue;
            }
        };
        let pawn = match list_get(mem, el, (handle & 0x7FFF) as u64, stride) {
            Some(p) if looks_like_ptr(p) => p,
            _ => {
                if verbose_entities {
                    dlog!("[ent ] slot {slot}: handle {:#x} → pawn introuvable", handle);
                }
                continue;
            }
        };

        // ── champs sur le pawn (v0.8: BULK — 1 RPM au lieu de ~8) ──
        let bulk = Bulk::read_block(mem, pawn, PAWN_BULK);
        let pr = PawnReader::new(mem, pawn, bulk.as_ref());
        let health = pr.read::<i32>(offs.m_i_health).unwrap_or(-1);
        let team = pr.read::<u8>(offs.m_i_team_num).map(|t| t as i32).unwrap_or(-1);
        let life = pr.read::<u8>(offs.m_life_state).unwrap_or(1);
        // v0.7.1 — DORMANCE: une entité hors PVS (loin, hors champ de QUI que
        // ce soit) n'est plus mise à jour par le serveur: son origin et ses os
        // restent figés à la DERNIÈRE position connue. L'aimbot verrouillait
        // dessus → "vise des cibles inexistantes". Lu sur CEntityInstance
        // (base du pawn), pin 0x8C stable — déjà dans le bulk v0.8 (0 RPM en
        // plus) — surcharge offsets.json si ça bouge.
        let d_raw = pr.read::<u8>(offs.m_b_dormant).unwrap_or(0);
        if d_raw > 1 {
            if crate::debug::once_per_sec() {
                dwarn!("slot {slot}: m_bDormant @ {:#x} = {d_raw} (attendu 0/1) → offset STALE, re-dump le schema", pawn + offs.m_b_dormant);
            }
        }
        let dormant = d_raw == 1;
        let node = pr.read::<u64>(offs.m_p_game_scene_node).unwrap_or(0);
        let origin = if looks_like_ptr(node) {
            mem.read::<Vec3>(node + offs.m_vec_abs_origin).unwrap_or_default()
        } else {
            Vec3::default()
        };

        // ── validation: le champ fautif est NOMMÉ précisément ──
        if !(0..=3).contains(&team) {
            if crate::debug::once_per_sec() {
                dwarn!("slot {slot}: m_iTeamNum @ {:#x} = {team} (attendu 0..3) → offset STALE, re-dump le schema", pawn + offs.m_i_team_num);
            }
        }
        if health < -1 || health > 200 {
            if crate::debug::once_per_sec() {
                dwarn!("slot {slot}: m_iHealth @ {:#x} = {health} (attendu 0..100) → offset STALE, re-dump le schema", pawn + offs.m_i_health);
            }
        }

        let alive = health > 0 && life == 0;
        if verbose_entities {
            dlog!("[ent ] slot {slot}: ctrl {:#x} → pawn {:#x} | hp={} team={} life={} dormant={} pos=({:.1},{:.1},{:.1})",
                ctrl, pawn, health, team, life, dormant, origin.x, origin.y, origin.z);
            dmem!(pawn + offs.m_i_health, format!("slot{slot} m_iHealth"), format!("{} ← écrire 100 ici = full vie", health));
            dmem!(pawn + offs.m_i_team_num, format!("slot{slot} m_iTeamNum"), format!("{} ← écrire 2/3 ici = changer de camp", team));
            dmem!(pawn + offs.m_p_game_scene_node, format!("slot{slot} m_pGameSceneNode"), format!("{:#x} → +{:#x} = position", node, offs.m_vec_abs_origin));
        }
        // v0.5: plus de `continue` sur les morts — ils restent dans la liste
        // (alive=false) pour la résolution des spectateurs; le rendu ESP et
        // l'aimbot filtrent sur p.alive.

        let name = read_name(mem, ctrl, offs.m_sanitized_name, slot);
        // bones: lus seulement au besoin (esp squelette) — sinon None.
        // v0.5: les MORTS restent dans la liste (alive=false) pour la liste
        // des spectateurs — le rendu ESP filtre sur p.alive.
        let velocity = if alive {
            pr.vec3(offs.m_vec_velocity).unwrap_or_default()
        } else {
            Vec3::default()
        };
        // v0.5 — services spectateur (mort → observe quelqu'un)
        let (observer_mode, observer_target) = if !alive {
            let svcs = pr.read::<u64>(offs.m_p_observer_services).unwrap_or(0);
            if looks_like_ptr(svcs) {
                (
                    mem.read::<i32>(svcs + offs.m_i_observer_mode).unwrap_or(0),
                    mem.read::<u32>(svcs + offs.m_h_observer_target).unwrap_or(0),
                )
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };
        let bones = if want_bones && pawn != local_pawn && alive {
            read_bones(mem, pawn, node, origin, offs)
        } else {
            None
        };
        // v0.9.6 — POSTURE (m_fFlags bit 1 FL_DUCKING): sans os (cache
        // indisponible une frame), le repli de visée de l'aimbot suit la
        // posture de l'ennemi (66 u debout / 50 u accroupi) au lieu de
        // survoler la tête d'un accroupi. Déjà dans le bulk v0.8 — zéro
        // lecture mémoire en plus.
        let ducking = alive && (pr.read::<i32>(offs.m_f_flags).unwrap_or(0) & 2) != 0;
        players.push(Player { slot, controller: ctrl, pawn, name, health, team, origin, alive, dormant, ducking, velocity, observer_mode, observer_target, bones });
    }

    // ── v0.5: QUI TE REGARDE — spectateurs ──
    // pour chaque joueur mort en mode spectateur dont la cible résout vers
    // TON pawn ou TON contrôleur → son nom remonte. La cible du handle peut
    // être un pawn OU un contrôleur: on teste les deux + le handle→pawn.
    let mut spectators = Vec::new();
    for p in &players {
        if p.alive || p.observer_mode <= 0 || (p.observer_target & 0x7FFF) == 0 {
            continue;
        }
        let idx = (p.observer_target & 0x7FFF) as u64;
        let watches_me = list_get(mem, el, idx, stride)
            .map(|ent| {
                if !looks_like_ptr(ent) {
                    return false;
                }
                if ent == local_pawn {
                    return true;
                }
                // la cible est peut-être un contrôleur → regarde son pawn
                if let Some(h) = mem.read::<u32>(ent + offs.m_h_player_pawn) {
                    if (h & 0x7FFF) != 0 {
                        if let Some(target_pawn) = list_get(mem, el, (h & 0x7FFF) as u64, stride) {
                            if target_pawn == local_pawn {
                                return true;
                            }
                        }
                    }
                }
                false
            })
            .unwrap_or(false);
        if watches_me {
            spectators.push(p.name.clone());
        }
    }

    // ── v0.4: triggerbot modes ciblés — projection normalisée des os ──
    // On projette chaque os candidat en écran NORMALISÉ (0..1) et on le
    // compare au centre (0.5, 0.5) avec un rayon PROPORTIONNEL à la hauteur
    // projetée du joueur — le test s'auto-échelonne avec la distance, sans
    // dépendre de la résolution.
    let crosshair_enemy = if trig_hitbox == 0 {
        crosshair_enemy_any
    } else if matrix_ok {
        let mut hit = false;
        'hb: for p in &players {
            if !p.alive || p.team == local_team || !(2..=3).contains(&p.team) {
                continue; // ennemis VIVANTS uniquement (v0.5: les morts traînent dans la liste)
            }
            let Some(bones) = &p.bones else { continue };
            let (Some((_, fy)), Some((_, ty))) = (
                matrix.w2s(p.origin, 1.0, 1.0),
                matrix.w2s(p.origin.z_up(PLAYER_HEIGHT), 1.0, 1.0),
            ) else { continue };
            let h = (fy - ty).abs(); // hauteur projetée normalisée
            if h < 1e-4 {
                continue;
            }
            // (os à tester, rayon relatif) — tête serrée, corps généreux,
            // membres précis (mains/pieds visés volontairement fines)
            let (check, factor): (&[usize], f32) = match trig_hitbox {
                1 => (&[B_HEAD], 0.11),
                2 => (&[23, 4, 3, B_PELVIS], 0.17),
                _ => (&[9, 10, 11, 13, 14, 15, 17, 18, 19, 20, 21, 22], 0.07),
            };
            for &bi in check {
                if let Some(b) = bones.get(bi) {
                    if let Some((bx, by)) = matrix.w2s(*b, 1.0, 1.0) {
                        let dx = bx - 0.5;
                        let dy = by - 0.5;
                        if (dx * dx + dy * dy).sqrt() <= factor * h {
                            hit = true;
                            break 'hb;
                        }
                    }
                }
            }
        }
        hit
    } else {
        false
    };

    // v0.8 — horodatage APRÈS les lectures: l'âge exact des données pour
    // l'interpolation de rendu (le compteur démarre quand la lecture finit).
    Some(Snapshot {
        taken_at: Instant::now(),
        local_pawn,
        local_team,
        local_origin,
        local_alive,
        local_eye,
        local_eye_ok,
        local_view,
        local_view_ok,
        local_view_src,
        local_sens,
        local_punch,
        local_flags,
        shots_fired,
        buy_menu,
        crosshair_id,
        crosshair_enemy,
        matrix,
        matrix_ok,
        players,
        spectators,
    })
}

// ── lecture des bones d'un pawn ─────────────────────────────────────────────
// node (CGameSceneNode d'un pawn = CSkeletonInstance) + m_modelState (0x140,
// EMBARQUÉ) → pointeur du cache d'os à +bone_array_off (0x80) → N_BONES
// entrées de bone_stride octets: position = 3 premiers f32.
// Les positions sont MONDE absolu (cache post-skinning) — validées contre
// l'origine du joueur (±300 u) pour écarter tout cache pourri.
//
// v0.4 — TOLÉRANCE PAR OS: avant, UN seul os invalide (f32 pourri, os hors
// sanité) rejetait le squelette ENTIERS → le squelette clignotait/
// disparaissait de façon aléatoire selon les frames. Désormais chaque os
// est validé individuellement; un os manquant retombe sur le BASSIN (le
// segment concerné se rétracte au lieu de faire disparaître le joueur).
// Seuls bassin + tête restent obligatoires (un squelette sans colonne
// n'a pas de sens).
//
// v0.4 — STRIDE CONFIGURABLE: si un patch CS2 change la taille des entrées
// du cache (0x20 aujourd'hui), pose "bone_stride": "0x30" dans
// offsets.json — plus besoin de recompiler. Un stride faux brouille
// l'assignation des os (membres croisés) sans rien casser d'autre.
fn read_bones(mem: &Mem, _pawn: u64, node: u64, origin: Vec3, offs: &Offsets) -> Option<Vec<Vec3>> {
    if !looks_like_ptr(node) {
        return None;
    }
    let model = node + offs.skeleton_model_state;
    let base = mem.read::<u64>(model + offs.bone_array_off).filter(|b| looks_like_ptr(*b))?;
    // une SEULE lecture pour tous les os (30 × 0x20 = 960 octets par défaut)
    let stride = offs.bone_stride.max(0x10) as usize;
    let mut buf = vec![0u8; stride * N_BONES];
    if !mem.read_arr(base, &mut buf) {
        return None;
    }
    let f32at = |o: usize| -> f32 {
        f32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
    };
    let mut out: Vec<Option<Vec3>> = (0..N_BONES).map(|_| None).collect();
    for i in 0..N_BONES {
        let o = i * stride;
        if o + 12 > buf.len() {
            break;
        }
        let p = Vec3 { x: f32at(o), y: f32at(o + 4), z: f32at(o + 8) };
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            continue; // CET os est pourri — pas les autres
        }
        // sanité: chaque os reste près du joueur (±300 u de l'origine)
        if (p.x - origin.x).abs() > 300.0 || (p.y - origin.y).abs() > 300.0 {
            continue;
        }
        if !(origin.z - 40.0..origin.z + 140.0).contains(&p.z) {
            continue;
        }
        out[i] = Some(p);
    }
    // bassin + tête obligatoires — signature d'un squelette bien résolu
    let pelvis = out[B_PELVIS]?;
    let head = out[B_HEAD]?;
    if head.z <= pelvis.z {
        return None;
    }
    Some(out.into_iter().map(|o| o.unwrap_or(pelvis)).collect())
}

// ── SONDE EMPIRIQUE squelette+angles (mode --dump) ──────────────────────────
// Balaye le modelState à la recherche du pointeur dont l'entrée 0 est un vrai
// bassin (x/y ≈ origin, z entre les pieds et ~+80), au format TRANSFORM
// (0x20/o, position en tête). Le gagnant est nommé → à piner dans offsets.rs.
pub fn probe_skeleton(mem: &Mem, offs: &Offsets) {
    let lp_slot = offs.local_pawn_slot(&mem.client);
    let Some(pawn) = mem.read::<u64>(lp_slot).filter(|p| looks_like_ptr(*p)) else {
        debug::emit("[skel] pas de local pawn — lance le probe en jeu".into());
        return;
    };
    let Some(node) = mem.read::<u64>(pawn + offs.m_p_game_scene_node).filter(|n| looks_like_ptr(*n)) else {
        debug::emit("[skel] node illisible".into());
        return;
    };
    let origin = mem.read::<Vec3>(node + offs.m_vec_abs_origin).unwrap_or_default();
    debug::emit(format!("[skel] node={:#x} origin=({:.1}, {:.1}, {:.1})", node, origin.x, origin.y, origin.z));

    // 1) angles de vue — QAngle en DEGRÉS (pitch ±89, yaw ±180)
    if let Some(a) = mem.read::<[f32; 2]>(pawn + offs.m_ang_eye_angles) {
        debug::emit(format!("[skel] m_angEyeAngles (pitch, yaw) = ({:.2}, {:.2})", a[0], a[1]));
    }

    // 2) crosshair (triggerbot) — m_iIDEntIndex brut
    if let Some(id) = mem.read::<i32>(pawn + offs.m_i_ident_index) {
        debug::emit(format!("[skel] m_iIDEntIndex = {id} (0 = rien sous le viseur)"));
    }

    // 3) balayage du modelState: cherche le pointeur vers le cache d'os
    let model = node + offs.skeleton_model_state;
    debug::emit(format!("[skel] modelState = node + {:#x} = {:#x}", offs.skeleton_model_state, model));
    let mut found = 0usize;
    'scan: for off in (0x40..0x220).step_by(8) {
        let Some(p) = mem.read::<u64>(model + off) else { continue };
        if !looks_like_ptr(p) || p % 8 != 0 {
            continue;
        }
        // hypothèse v0.3: transform 0x20/o, position = f[0..3]
        let Some(e0) = mem.read::<[f32; 8]>(p) else { continue };
        if !e0.iter().all(|v| v.is_finite()) {
            continue;
        }
        let (tx, ty, tz) = (e0[0], e0[1], e0[2]);
        if (tx - origin.x).abs() > 40.0 || (ty - origin.y).abs() > 40.0 {
            continue;
        }
        if !(origin.z - 10.0..origin.z + 90.0).contains(&tz) {
            continue;
        }
        // entrée 0 plausible = bassin. Vérifie la tête: l'os le plus haut
        // doit culminer vers origin.z + 55..75.
        let mut top_dz = -999.0f32;
        for i in 1..N_BONES {
            if let Some(ei) = mem.read::<[f32; 8]>(p + BONE_STRIDE * i as u64) {
                if ei.iter().all(|v| v.is_finite()) {
                    top_dz = top_dz.max(ei[2] - origin.z);
                }
            }
        }
        if !(45.0..90.0).contains(&top_dz) {
            continue;
        }
        found += 1;
        debug::emit(format!("[skel] CANDIDAT bone_array_off = {:#x} (pointeur {:#x}) — bone0 dz={:.1}, top dz={:.1}",
            off, p, tz - origin.z, top_dz));
        if found == 1 {
            // table complète relative au bassin — pour vérifier les indices
            debug::emit("[skel] table bones (rel au bassin bone0, (dx, dy, dz)):".into());
            for i in 0..N_BONES {
                if let Some(ei) = mem.read::<[f32; 8]>(p + BONE_STRIDE * i as u64) {
                    if !ei.iter().all(|v| v.is_finite()) {
                        break;
                    }
                    debug::emit(format!("[skel]   bone {:>2}: ({:7.2}, {:7.2}, {:7.2})", i, ei[0] - tx, ei[1] - ty, ei[2] - tz));
                } else {
                    break;
                }
            }
            break 'scan;
        }
    }
    if found == 0 {
        debug::emit("[skel] AUCUN candidat — re-balayage manuel nécessaire (offset non trouvé dans 0x40..0x220)".into());
    }
}

// dump complet console (mode --dump ou touche F4)
pub fn dump_verbose(snap: &Snapshot, mem: &Mem, offs: &Offsets) {
    debug::emit("════════════════ dump entités ════════════════".into());
    debug::emit(format!("local pawn   {:#018x}  team {}  pos ({:.1}, {:.1}, {:.1})", snap.local_pawn, snap.local_team, snap.local_origin.x, snap.local_origin.y, snap.local_origin.z));
    debug::emit(format!("view matrix  {:#018x}  (f32×12)  →  {}", offs.view_matrix(&mem.client), if snap.matrix_ok { "OK" } else { "INVALIDE / NULLE — boîtes non traçables, entités quand même listées" }));
    debug::emit(format!("vue locale   pitch {:.1}° · yaw {:.1}° ({})", snap.local_eye.0, snap.local_eye.1, if snap.local_eye_ok { "valide" } else { "bruit" }));
    debug::emit(format!("entity list  {:#018x}  (stride {:#x})", offs.entity_list(&mem.client), offs.list_entry_stride));
    debug::emit(format!("trigger      m_iIDEntIndex = {}{}", snap.crosshair_id, if snap.crosshair_enemy { " → ENNEMI sous le viseur" } else { "" }));
    for p in &snap.players {
        let side = if p.team == 2 { "T" } else if p.team == 3 { "CT" } else { "?" };
        let bones = if p.bones.is_some() { "os OK" } else { "—" };
        debug::emit(format!("─ slot {:>2} {:<20} [{}]  hp {:>3}  squelette: {}", p.slot, p.name, side, p.health, bones));
        debug::emit(format!("    pawn             {:#018x}", p.pawn));
        debug::emit(format!("    m_iHealth        {:#018x} = {:>3}   ← écris 100 ici pour pleine vie", p.pawn + offs.m_i_health, p.health));
        debug::emit(format!("    m_iTeamNum       {:#018x} = {:>3}   ← écris 2 (T) / 3 (CT) ici pour changer de camp", p.pawn + offs.m_i_team_num, p.team));
        debug::emit(format!("    m_pGameSceneNode {:#018x}", p.pawn + offs.m_p_game_scene_node));
        debug::emit(format!("    position         ({:.1}, {:.1}, {:.1})   ← écris f32 x/y/z via le node+m_vecAbsOrigin pour téléporter", p.origin.x, p.origin.y, p.origin.z));
    }
    debug::emit("═══════════════════════════════════════════════".into());
}

// mode --dump: carte + sonde brute + entités puis sortie propre
pub fn dump_and_exit(mem: &Mem, offs: &Offsets) {
    offs.print_resolution(mem);
    let vm_addr = offs.view_matrix(&mem.client);
    let lp_slot = offs.local_pawn_slot(&mem.client);
    // ── sonde projection: projette l'œil (origin + 64) — doit tomber ~centre
    let origin = mem
        .read::<u64>(lp_slot)
        .filter(|p| looks_like_ptr(*p))
        .and_then(|pawn| mem.read::<u64>(pawn + offs.m_p_game_scene_node))
        .filter(|n| looks_like_ptr(*n))
        .and_then(|node| mem.read::<Vec3>(node + offs.m_vec_abs_origin));
    match origin {
        Some(o) => debug::emit(format!("[probe] origin local ≈ ({:.1}, {:.1}, {:.1})", o.x, o.y, o.z)),
        None => debug::emit("[probe] origin local illisible — node/origin stale ?".into()),
    }
    match (mem.read::<[f32; 12]>(vm_addr), origin) {
        (Some(m), Some(o)) => {
            let sane = m.iter().all(|v| v.is_finite());
            let zn = (m[8] * m[8] + m[9] * m[9] + m[10] * m[10]).sqrt();
            debug::emit(format!("[probe] matrice 3×4 — finie={sane} norme-Z={zn:.3}"));
            if sane {
                let eye = Vec3 { x: o.x, y: o.y, z: o.z + 64.0 };
                let vm = ViewMatrix { m };
                match vm.w2s(eye, 1920.0, 1080.0) {
                    Some((sx, sy)) => debug::emit(format!("[probe] œil → ({:.0}, {:.0}) px @1920×1080 — attendu ~centré (960, 540)", sx, sy)),
                    None => debug::emit("[probe] œil → derrière la caméra / hors champ (w2s a refusé)".into()),
                }
            }
        }
        (None, _) => debug::emit(format!("[probe] matrice @ {:#x} ILLISIBLE — RPM échec", vm_addr)),
        (_, None) => debug::emit("[probe] projection impossible — pas d'origin".into()),
    }
    match mem.read::<u64>(lp_slot) {
        Some(v) if looks_like_ptr(v) => debug::emit(format!("[probe] local pawn @ {:#x} → {:#x} VALIDE", lp_slot, v)),
        Some(v) => debug::emit(format!("[probe] local pawn @ {:#x} = {:#x} INVALIDE — pas spawné / offset stale", lp_slot, v)),
        None => debug::emit(format!("[probe] local pawn @ {:#x} ILLISIBLE — RPM échec", lp_slot)),
    }
    match snapshot(mem, offs, true) {
        Some(s) => dump_verbose(&s, mem, offs),
        None => debug::emit("[!] snapshot impossible — pas en partie ou offsets stale (regarde les warns au-dessus)".into()),
    }
    probe_skeleton(mem, offs);
    debug::emit("[*] mode dump terminé.".into());
}
