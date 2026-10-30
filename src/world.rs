// world — walk du MONDE (v0.5 · Vantyx.cc): armes au sol + projectiles de
// grenades, identifiés par designerName via la chaîne d'identité:
//
//   entité + m_pEntity (0x10) → CEntityIdentity → m_designerName (0x20, char*)
//
// Les designerName CS2 des armes au sol sont les noms prefab ("ak47", "awp",
// "hegrenade_projectile"…) — matching par sous-chaîne, table de mapping vers
// des noms propres affichables.
//
// CADENCE: le scan complet tourne à ~8 Hz (frame_i % 125 de la boucle 1 kHz)
// avec un CACHE de noms par slot d'entité (le ptr identity est stable tant
// que l'entité vit → l'ascii n'est relu que s'il change). Les positions des
// PROJECTILES sont rafraîchies au rythme du rendu (~166 Hz) pour des
// trajectoires fluides; les armes au sol ne bougent pas → 8 Hz suffit.
//
// language: Rust, file: world.rs, target: Windows 11, windows-rs 0.58

use std::collections::HashMap;
use std::time::Instant;

use crate::entity::{list_get, looks_like_ptr};
use crate::math::Vec3;
use crate::mem::Mem;
use crate::offsets::Offsets;
use crate::dlog;

/// arme au sol (ou objet ramassable) — position figée au moment du scan
#[derive(Clone, Debug)]
pub struct WorldItem {
    pub pos: Vec3,
    pub name: String, // nom affichable ("AK-47")
    /// v0.8 — slot d'origine (héritage du compteur d'échecs entre scans)
    pub slot: u64,
    /// v0.8 — scans consécutifs sans validation de node/position. 2 échecs
    /// de suite → retrait: une node périmée (arme détruite côté serveur mais
    /// slot zombie) est tuée en ~250 ms, une vraie arme ne rate jamais deux
    /// scans de suite à 8 Hz.
    pub misses: u8,
}

/// grenade en vol — trajectoire suivie
#[derive(Clone, Debug)]
pub struct Projectile {
    pub entity: u64,
    pub kind: GrenadeKind,
    pub last_pos: Vec3,
    pub trail: Vec<(Vec3, Instant)>, // points monde horodatés
    pub dead_since: Option<Instant>, // entité disparue → la traîne s'estompe
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GrenadeKind {
    He,
    Smoke,
    Flash,
    Molotov,
    Decoy,
}

impl GrenadeKind {
    /// couleur de la traîne [r,g,b] — code fonctionnel stable
    pub fn color(self) -> [f32; 4] {
        match self {
            GrenadeKind::He => [1.00, 0.55, 0.15, 1.0],
            GrenadeKind::Smoke => [0.72, 0.76, 0.82, 1.0],
            GrenadeKind::Flash => [1.00, 0.90, 0.30, 1.0],
            GrenadeKind::Molotov => [1.00, 0.30, 0.20, 1.0],
            GrenadeKind::Decoy => [0.35, 0.60, 1.00, 1.0],
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            GrenadeKind::He => "HE",
            GrenadeKind::Smoke => "SMOKE",
            GrenadeKind::Flash => "FLASH",
            GrenadeKind::Molotov => "MOLOTOV",
            GrenadeKind::Decoy => "DECOY",
        }
    }
}

/// état monde persistant entre les scans
pub struct WorldState {
    pub items: Vec<WorldItem>,
    pub projectiles: Vec<Projectile>,
    /// slot entité → (identity ptr, nom brut) — l'ascii n'est relu que si le
    /// ptr d'identity change (les strings prefab sont internées par le jeu)
    name_cache: HashMap<u64, (u64, String)>,
    last_scan: Option<Instant>,
    /// v0.8 — compteur de rounds (m_totalRoundsPlayed): un changement = reset
    /// du monde → PURGE complète des items + cache (les armes du round
    /// précédent sont détruites côté serveur, leurs slots traînent parfois
    /// avec des nodes périmées — c'était les "objets fantômes").
    last_round: Option<i32>,
}

impl WorldState {
    pub fn new() -> Self {
        WorldState { items: Vec::new(), projectiles: Vec::new(), name_cache: HashMap::new(), last_scan: None, last_round: None }
    }

    /// vraie position d'une entité (node + m_vecAbsOrigin) — v0.8 VALIDÉE:
    /// une node périmée (arme détruite, slot zombie) retourne souvent (0,0,0)
    /// ou une position hors map: ces valeurs sont rejetées ici, l'item ne
    /// reçoit QUE des positions physiquement plausibles (bounds carte CS2:
    /// |x|,|y| < 4000 u, |z| < 3500 u — les maps les plus grandes restent
    /// sous ces bornes; l'origine exacte (0,0,0) n'existe nulle part).
    fn entity_pos(mem: &Mem, ent: u64, offs: &Offsets) -> Option<Vec3> {
        let node = mem.read::<u64>(ent + offs.m_p_game_scene_node)?;
        if !looks_like_ptr(node) {
            return None;
        }
        let p = mem.read::<Vec3>(node + offs.m_vec_abs_origin)?;
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return None;
        }
        if p.x.abs() > 4000.0 || p.y.abs() > 4000.0 || p.z.abs() > 3500.0 {
            return None; // hors des bounds de toute map CS2
        }
        if p.x.abs() < 1.0 && p.y.abs() < 1.0 && p.z.abs() < 1.0 {
            return None; // origine exacte = node non initialisée / périmée
        }
        Some(p)
    }

    /// scan complet (~8 Hz) — `el` = pointeur CGameEntitySystem déjà déréférencé
    pub fn scan(&mut self, mem: &Mem, offs: &Offsets, el: u64) {
        // throttle intégré: pas plus d'un scan complet / 100 ms
        if let Some(t) = self.last_scan {
            if t.elapsed().as_millis() < 100 {
                return;
            }
        }
        self.last_scan = Some(Instant::now());

        // ── v0.8 COUCHE 1 — PURGE PAR ROUND (le reset demandé) ──────────────
        // m_totalRoundsPlayed (C_CSGameRules +0x88) INCRÉMENTE au reset du
        // round: les armes au sol du round précédent sont détruites côté
        // serveur — tout ce qui traîne dans nos items est un zombie. Purge
        // totale des items + du cache de noms; le walk repart propre.
        if let Some(gr) = mem
            .read::<u64>(offs.game_rules_slot(&mem.client))
            .filter(|p| looks_like_ptr(*p))
        {
            let round = mem.read::<i32>(gr + offs.m_total_rounds_played).unwrap_or(-1);
            if let Some(prev) = self.last_round {
                if round != prev {
                    if !self.items.is_empty() {
                        dlog!("[wrld] round {} → {} : PURGE monde ({} armes) — reset du round", prev, round, self.items.len());
                    }
                    self.items.clear();
                    self.name_cache.clear();
                }
            }
            self.last_round = Some(round);
        }

        let stride = offs.list_entry_stride;
        let highest = mem
            .read::<i32>(el + offs.m_n_highest_entity)
            .map(|h| h.clamp(1, 1400) as u64)
            .unwrap_or(64);

        let mut items: Vec<WorldItem> = Vec::new();
        let mut live_projectiles: Vec<(u64, GrenadeKind)> = Vec::new();
        let mut cache = std::mem::take(&mut self.name_cache);
        // v0.8 — ancien état par slot: héritage du compteur d'échecs (TTL)
        let mut prev: HashMap<u64, WorldItem> =
            self.items.drain(..).map(|it| (it.slot, it)).collect();

        for slot in 1..=highest {
            let Some(ent) = list_get(mem, el, slot, stride).filter(|e| looks_like_ptr(*e)) else {
                continue;
            };
            let Some(name) = entity_name(mem, ent, slot, offs, &mut cache) else {
                continue;
            };
            let lname = name.to_ascii_lowercase();

            // projectile de grenade en vol ?
            if let Some(kind) = classify_grenade(&lname) {
                live_projectiles.push((ent, kind));
                continue;
            }
            // arme au sol / objet ramassable ?
            if let Some((display, _short)) = classify_weapon(&lname) {
                // v0.6 — FILTRE ANTI-FANTÔMES: m_hOwnerEntity (C_BaseEntity
                // 0x520, dump 14193). handle 0xFFFFFFFF = SANS propriétaire
                // → VRAIMENT au sol; tout autre handle = tenue/inventaire
                // d'un joueur → suivait son porteur et s'affichait comme un
                // "endroit fantôme". On ne saute que sur une lecture POSITIVE
                // d'un owner — un échec RPM n'efface pas un drop réel.
                match mem.read::<u32>(ent + offs.m_h_owner_entity) {
                    Some(owner) if owner != 0xFFFF_FFFF => {
                        prev.remove(&slot); // possédée: plus au sol, zéro grâce
                        continue;
                    }
                    _ => {} // libre (0xFFFFFFFF) ou lecture échouée → garde
                }
                // ── v0.8 COUCHE 2+3 — VALIDATION + TTL ──
                // position validée (bounds, pas (0,0,0)) → item frais (misses=0)
                // OU héritage: node devenue illisible → on garde la DERNIÈRE
                // position valide + misses+1; 2 échecs de suite → retrait.
                match Self::entity_pos(mem, ent, offs) {
                    Some(pos) => {
                        prev.remove(&slot);
                        items.push(WorldItem { pos, name: display.to_string(), slot, misses: 0 });
                    }
                    None => {
                        if let Some(mut old) = prev.remove(&slot) {
                            old.misses += 1;
                            if old.misses <= 2 {
                                items.push(old); // ~250 ms de grâce, position figée
                            }
                            // > 2 échecs → l'item meurt (slot zombie confirmé)
                        }
                        // jamais vu à ce slot avec une position valide → rien
                    }
                }
            }
        }
        // v0.8 COUCHE 3 (suite) — slots DISPARUS du walk: une arme ramassée
        // quitte la liste d'entités. 2 scans de grâce (~250 ms) avant retrait
        // — un vrai drop ne disparaît JAMAIS du walk tant qu'il est au sol.
        for (_, old) in prev {
            let mut old = old;
            old.misses += 1;
            if old.misses <= 2 {
                items.push(old);
            }
        }
        self.name_cache = cache;
        self.items = items;

        // ── réconciliation des projectiles ──
        // nouveaux: seed avec la position courante; disparus: marqués (la
        // traîne s'estompe encore ~3.5 s puis l'entrée meurt).
        let mut next: Vec<Projectile> = Vec::new();
        for (ent, kind) in live_projectiles {
            if let Some(existing) = self.projectiles.iter_mut().find(|p| p.entity == ent) {
                // recyclé vivant
                existing.dead_since = None;
                next.push(existing.clone());
            } else if let Some(pos) = Self::entity_pos(mem, ent, offs) {
                next.push(Projectile {
                    entity: ent,
                    kind,
                    last_pos: pos,
                    trail: vec![(pos, Instant::now())],
                    dead_since: None,
                });
            }
        }
        // traînées résiduelles des projectiles disparus
        for p in &self.projectiles {
            if next.iter().any(|n| n.entity == p.entity) {
                continue;
            }
            let gone = p.dead_since.unwrap_or_else(Instant::now);
            if gone.elapsed().as_millis() < 3500 {
                let mut ghost = p.clone();
                ghost.dead_since = Some(gone);
                next.push(ghost);
            }
        }
        next.retain(|p| p.trail.len() <= 64);
        self.projectiles = next;
    }

    /// rafraîchit les positions des projectiles ACTIFS (~au frame de rendu)
    /// et étend les traînées. Appelé avant le dessin si grenade_trails actif.
    pub fn refresh_projectiles(&mut self, mem: &Mem, offs: &Offsets) {
        for p in self.projectiles.iter_mut() {
            if p.dead_since.is_some() {
                continue; // fantôme: la traîne s'estompe, pas de nouveau point
            }
            let Some(node) = mem.read::<u64>(p.entity + offs.m_p_game_scene_node).filter(|n| looks_like_ptr(*n)) else {
                p.dead_since = Some(Instant::now());
                continue;
            };
            let Some(pos) = mem.read::<Vec3>(node + offs.m_vec_abs_origin) else {
                continue;
            };
            if !pos.x.is_finite() || !pos.y.is_finite() || !pos.z.is_finite() {
                continue;
            }
            p.last_pos = pos;
            let fresh = match p.trail.last() {
                Some((_, at)) => at.elapsed().as_millis() >= 24,
                None => true,
            };
            if fresh {
                p.trail.push((pos, Instant::now()));
                if p.trail.len() > 64 {
                    p.trail.remove(0);
                }
            }
        }
    }
}

/// designerName → type de grenade (projectiles uniquement)
fn classify_grenade(lname: &str) -> Option<GrenadeKind> {
    if !lname.contains("projectile") {
        return None;
    }
    if lname.contains("smoke") {
        Some(GrenadeKind::Smoke)
    } else if lname.contains("flash") {
        Some(GrenadeKind::Flash)
    } else if lname.contains("molotov") || lname.contains("inc") {
        Some(GrenadeKind::Molotov)
    } else if lname.contains("decoy") {
        Some(GrenadeKind::Decoy)
    } else if lname.contains("he") || lname.contains("grenade") {
        Some(GrenadeKind::He)
    } else {
        None
    }
}

/// designerName → (nom affichable, label court overlay)
/// matching par sous-chaîne: "ak47" matche "ak47" ET "weapon_ak47".
fn classify_weapon(lname: &str) -> Option<(&'static str, &'static str)> {
    Some(match lname {
        _ if lname.contains("ak47") => ("AK-47", "AK-47"),
        _ if lname.contains("m4a1_silencer") => ("M4A1-S", "M4A1-S"),
        _ if lname.contains("m4a1") => ("M4A4", "M4A4"),
        _ if lname.contains("awp") => ("AWP", "AWP"),
        _ if lname.contains("deagle") => ("Desert Eagle", "Deagle"),
        _ if lname.contains("glock") => ("Glock-18", "Glock"),
        _ if lname.contains("usp_silencer") => ("USP-S", "USP-S"),
        _ if lname.contains("p250") => ("P250", "P250"),
        _ if lname.contains("tec9") => ("Tec-9", "Tec-9"),
        _ if lname.contains("fiveseven") => ("Five-SeveN", "5-7"),
        _ if lname.contains("elite") => ("Dual Berettas", "Berettas"),
        _ if lname.contains("hkp2000") => ("P2000", "P2000"),
        _ if lname.contains("cz75a") => ("CZ75-Auto", "CZ75"),
        _ if lname.contains("revolver") => ("R8 Revolver", "R8"),
        _ if lname.contains("mac10") => ("MAC-10", "MAC-10"),
        _ if lname.contains("mp9") => ("MP9", "MP9"),
        _ if lname.contains("mp7") => ("MP7", "MP7"),
        _ if lname.contains("ump45") => ("UMP-45", "UMP-45"),
        _ if lname.contains("p90") => ("P90", "P90"),
        _ if lname.contains("mp5sd") => ("MP5-SD", "MP5-SD"),
        _ if lname.contains("galilar") => ("Galil AR", "Galil"),
        _ if lname.contains("famas") => ("FAMAS", "FAMAS"),
        _ if lname.contains("sg556") => ("SG 553", "SG 553"),
        _ if lname.contains("aug") => ("AUG", "AUG"),
        _ if lname.contains("ssg08") => ("SSG 08", "SSG 08"),
        _ if lname.contains("scar20") => ("SCAR-20", "SCAR-20"),
        _ if lname.contains("g3sg1") => ("G3SG1", "G3SG1"),
        _ if lname.contains("nova") => ("Nova", "Nova"),
        _ if lname.contains("xm1014") => ("XM1014", "XM1014"),
        _ if lname.contains("mag7") => ("MAG-7", "MAG-7"),
        _ if lname.contains("m249") => ("M249", "M249"),
        _ if lname.contains("negev") => ("Negev", "Negev"),
        _ if lname.contains("bizon") => ("PP-Bizon", "Bizon"),
        _ if lname.contains("taser") => ("Zeus x27", "Zeus"),
        _ if lname.contains("kevlar") || lname.contains("assaultsuit") => ("Kevlar", "Kevlar"),
        _ if lname.contains("hegrenade") => ("Grenade HE", "HE"),
        _ if lname.contains("smokegrenade") => ("Fumigène", "Smoke"),
        _ if lname.contains("flashbang") => ("Flashbang", "Flash"),
        _ if lname.contains("molotov") => ("Molotov", "Molotov"),
        _ if lname.contains("incgrenade") => ("Incendiaire", "Incend."),
        _ if lname.contains("decoy") => ("Leurre", "Leurre"),
        _ if lname.contains("c4") => ("C4", "C4"),
        _ if lname.contains("knife") || lname.contains("bayonet") || lname.contains("karambit")
            || lname.contains("m9_bayonet") || lname.contains("butterfly") || lname.contains("talon") => ("Couteau", "Couteau"),
        _ => return None,
    })
}

/// nom brut (designerName) avec cache par slot — FONCTION LIBRE du module:
/// utilisée par WorldState::scan ET par local_weapon_name (hors impl).
fn entity_name(mem: &Mem, ent: u64, slot: u64, offs: &Offsets, cache: &mut HashMap<u64, (u64, String)>) -> Option<String> {
    let identity = mem.read::<u64>(ent + offs.m_entity_identity)?;
    if !looks_like_ptr(identity) {
        return None;
    }
    if let Some((cached_id, cached_name)) = cache.get(&slot) {
        if *cached_id == identity {
            return Some(cached_name.clone());
        }
    }
    // designerName: char* pointé, sinon inline (le même pattern que read_name)
    let mut name = None;
    if let Some(ptr) = mem.read::<u64>(identity + offs.m_designer_name) {
        if looks_like_ptr(ptr) {
            if let Some(s) = mem.read_ascii(ptr, 32) {
                if !s.is_empty() && s.chars().all(|c| !c.is_control()) {
                    name = Some(s);
                }
            }
        }
    }
    if name.is_none() {
        if let Some(s) = mem.read_ascii(identity + offs.m_designer_name, 32) {
            if !s.is_empty() && s.chars().all(|c| !c.is_control()) {
                name = Some(s);
            }
        }
    }
    let name = name?;
    cache.insert(slot, (identity, name.clone()));
    Some(name)
}

/// résout l'arme ACTIVE du pawn local → label affichable (accuracy lab).
/// chaîne: pawn + m_pWeaponServices → m_hActiveWeapon → entité → designerName
pub fn local_weapon_name(mem: &Mem, offs: &Offsets, local_pawn: u64, el: u64) -> Option<String> {
    let svcs = mem.read::<u64>(local_pawn + offs.m_p_weapon_services)?;
    if !looks_like_ptr(svcs) {
        return None;
    }
    let handle = mem.read::<u32>(svcs + offs.m_h_active_weapon)?;
    let idx = handle & 0x7FFF;
    if idx == 0 {
        return None;
    }
    let ent = list_get(mem, el, idx as u64, offs.list_entry_stride)?;
    if !looks_like_ptr(ent) {
        return None;
    }
    let mut cache = HashMap::new();
    let raw = entity_name(mem, ent, idx as u64, offs, &mut cache)?;
    let (display, _) = classify_weapon(&raw.to_ascii_lowercase())?;
    Some(display.to_string())
}
