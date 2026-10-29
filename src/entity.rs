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

use crate::debug;
use crate::math::{Vec3, ViewMatrix};
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
pub const BONE_STRIDE: u64 = 0x20;

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
    pub bones: Option<Vec<Vec3>>, // N_BONES positions monde (None = pas lu / stale)
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub local_pawn: u64,
    pub local_team: i32,
    pub local_origin: Vec3,
    pub local_alive: bool,
    pub local_eye: (f32, f32), // (pitch, yaw) degrés — debug/sonde
    pub local_eye_ok: bool,
    pub buy_menu: bool,        // m_bIsBuyMenuOpen — coupe l'ESP pendant l'achat
    pub crosshair_id: i32,     // m_iIDEntIndex brut (0 = rien sous le viseur)
    pub crosshair_enemy: bool, // ennemi vivant sous le viseur — triggerbot
    pub matrix: ViewMatrix,
    pub matrix_ok: bool,
    pub players: Vec<Player>,
}

// résolution d'un index dans la liste d'entités → pointeur d'objet
// `system` = *(client.dll + dwEntityList) — le déréférencement est fait par l'appelant
fn list_get(mem: &Mem, system: u64, index: u64, stride: u64) -> Option<u64> {
    let chunk = mem.read::<u64>(system + 8 * (index >> 9) + 0x10)?;
    if !looks_like_ptr(chunk) {
        return None;
    }
    let obj = mem.read::<u64>(chunk + stride * (index & 0x1FF))?;
    Some(obj)
}

fn looks_like_ptr(v: u64) -> bool {
    (0x10000..0x0000_7FFF_FFFF_FFFF).contains(&v)
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
    snapshot_f(mem, offs, verbose_entities, false)
}

pub fn snapshot_full(mem: &Mem, offs: &Offsets, verbose_entities: bool, want_bones: bool) -> Option<Snapshot> {
    snapshot_f(mem, offs, verbose_entities, want_bones)
}

fn snapshot_f(mem: &Mem, offs: &Offsets, verbose_entities: bool, want_bones: bool) -> Option<Snapshot> {
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

    let local_team = mem.read::<u8>(local_pawn + offs.m_i_team_num).map(|t| t as i32).unwrap_or(-1);
    let local_node = mem.read::<u64>(local_pawn + offs.m_p_game_scene_node).unwrap_or(0);
    let local_origin = if looks_like_ptr(local_node) {
        mem.read::<Vec3>(local_node + offs.m_vec_abs_origin).unwrap_or_default()
    } else {
        Vec3::default()
    };
    dmem!(local_pawn + offs.m_i_team_num, "local m_iTeamNum", local_team);
    dmem!(local_node + offs.m_vec_abs_origin, "local m_vecAbsOrigin", format!("({:.1},{:.1},{:.1})", local_origin.x, local_origin.y, local_origin.z));

    // état local: vivant ? menu d'achat ? (degrés: pitch ∈ [-89..89], yaw ∈ [-180..180])
    let local_health = mem.read::<i32>(local_pawn + offs.m_i_health).unwrap_or(-1);
    let local_life = mem.read::<u8>(local_pawn + offs.m_life_state).unwrap_or(1);
    let local_alive = local_health > 0 && local_life == 0;
    let buy_menu = mem.read::<u8>(local_pawn + offs.m_buy_menu_open).unwrap_or(0) != 0;
    let local_eye = mem
        .read::<[f32; 2]>(local_pawn + offs.m_ang_eye_angles)
        .map(|a| (a[0], a[1]))
        .unwrap_or((0.0, 0.0));
    let local_eye_ok = local_eye.0.is_finite() && local_eye.1.is_finite()
        && local_eye.0.abs() <= 90.0 && local_eye.1.abs() <= 180.0;

    // ── triggerbot: entité sous le viseur ──
    // m_iIDEntIndex = index d'entité pointé par le crosshair. On le résout,
    // on lit team/vie; si l'objet est un contrôleur (lecture invalide) on
    // retente via son handle m_hPlayerPawn. Ennemi = équipe adverse vivante.
    let crosshair_id = mem.read::<i32>(local_pawn + offs.m_i_ident_index).unwrap_or(0);
    let crosshair_enemy = crosshair_id > 0 && {
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

        // ── champs sur le pawn ──
        let health = mem.read::<i32>(pawn + offs.m_i_health).unwrap_or(-1);
        let team = mem.read::<u8>(pawn + offs.m_i_team_num).map(|t| t as i32).unwrap_or(-1);
        let life = mem.read::<u8>(pawn + offs.m_life_state).unwrap_or(1);
        let node = mem.read::<u64>(pawn + offs.m_p_game_scene_node).unwrap_or(0);
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
            dlog!("[ent ] slot {slot}: ctrl {:#x} → pawn {:#x} | hp={} team={} life={} pos=({:.1},{:.1},{:.1})",
                ctrl, pawn, health, team, life, origin.x, origin.y, origin.z);
            dmem!(pawn + offs.m_i_health, format!("slot{slot} m_iHealth"), format!("{} ← écrire 100 ici = full vie", health));
            dmem!(pawn + offs.m_i_team_num, format!("slot{slot} m_iTeamNum"), format!("{} ← écrire 2/3 ici = changer de camp", team));
            dmem!(pawn + offs.m_p_game_scene_node, format!("slot{slot} m_pGameSceneNode"), format!("{:#x} → +{:#x} = position", node, offs.m_vec_abs_origin));
        }
        if !alive {
            continue; // morts: pas de boîte (déjà tracés au-dessus en verbeux)
        }

        let name = read_name(mem, ctrl, offs.m_sanitized_name, slot);
        // bones: lus seulement au besoin (esp squelette) — sinon None
        let bones = if want_bones && pawn != local_pawn {
            read_bones(mem, pawn, node, origin, offs)
        } else {
            None
        };
        players.push(Player { slot, controller: ctrl, pawn, name, health, team, origin, alive: true, bones });
    }

    Some(Snapshot {
        local_pawn,
        local_team,
        local_origin,
        local_alive,
        local_eye,
        local_eye_ok,
        buy_menu,
        crosshair_id,
        crosshair_enemy,
        matrix,
        matrix_ok,
        players,
    })
}

// ── lecture des bones d'un pawn ─────────────────────────────────────────────
// node (CGameSceneNode d'un pawn = CSkeletonInstance) + m_modelState (0x140,
// EMBARQUÉ) → pointeur du cache d'os à +bone_array_off (0x80) → N_BONES
// entrées de BONE_STRIDE octets: position = 3 premiers f32.
// Les positions sont MONDE absolu (cache post-skinning) — validées contre
// l'origine du joueur (±300 u) pour écarter tout cache pourri.
fn read_bones(mem: &Mem, _pawn: u64, node: u64, origin: Vec3, offs: &Offsets) -> Option<Vec<Vec3>> {
    if !looks_like_ptr(node) {
        return None;
    }
    let model = node + offs.skeleton_model_state;
    let base = mem.read::<u64>(model + offs.bone_array_off).filter(|b| looks_like_ptr(*b))?;
    // une SEULE lecture pour les 30 os (30 × 0x20 = 960 octets)
    let mut raw = vec![[0f32; 8]; N_BONES];
    if !mem.read_arr(base, &mut raw) {
        return None;
    }
    let mut out = Vec::with_capacity(N_BONES);
    for (_i, entry) in raw.iter().enumerate() {
        let p = Vec3 { x: entry[0], y: entry[1], z: entry[2] };
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return None; // cache pourri pour ce pawn — squelette skippé
        }
        // sanité: chaque os reste près du joueur (±300 u de l'origine)
        if (p.x - origin.x).abs() > 300.0 || (p.y - origin.y).abs() > 300.0 {
            return None;
        }
        if !(origin.z - 40.0..origin.z + 140.0).contains(&p.z) {
            return None;
        }
        out.push(p);
    }
    // tête au-dessus du bassin — signature d'un squelette bien résolu
    if out.get(B_HEAD).map(|h| h.z).unwrap_or(0.0) <= out.get(B_PELVIS).map(|b| b.z).unwrap_or(999.0) {
        return None;
    }
    Some(out)
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
