// thirdperson — caméra 3e personne EXTERNE · Vantyx.cc v0.9
//
// CE QUE ÇA FAIT: force la caméra du jeu derrière TON pawn — tu vois ton
// perso bouger (et avec le spinbot GHOST: tu le vois spinné pour les ennemis
// pendant que TA caméra reste jouable). C'est la demande "force la caméra en
// 3e personne comme je vois tout normalement et je vois mon perso bouger".
//
// MÉCANIQUE (deux niveaux, technique publique unknowncheats "Third Person
// External" — 0xEB_0xFE, mai 2023, revue pour le build courant):
//
//   NIVEAU 1 — FLAG-WRITE (défaut, robuste):
//     Le CSGOInput porte un bool `m_bCameraInThirdPerson`. Le jeu le reset à
//     chaque frame quand les conditions officielles ne sont pas réunies
//     (sv_cheats) → on le ré-écrit à la cadence de la boucle (~0,5-1 ms):
//     le flag est présent quand la caméra se construit → 3e personne.
//     C'est la version 1 du post UC: ça marche, la caméra peut clipper dans
//     les murs (cam_collision par défaut).
//
//   NIVEAU 2 — PATCH MOTEUR (opt-in, menu):
//     Un `je` (branchement conditionnel) dans client.dll garde la 3e
//     personne derrière le gate sv_cheats. On scanne client.dll au runtime
//     pour le motif `74 ?? ?? ?? ?? 48 ?? BA` (je rel8 + contexte), et si le
//     motif est UNIQUE on remplace le je par un `jmp` court vers la MÊME
//     cible (74→EB: le déplacement est préservé — le saut devient incondi-
//     tionnel, la géométrie du code ne bouge PAS, contrairement au jmp rel32
//     du post d'origine qui était calibré pour le build 9640).
//     UNIQUE SEULEMENT: un motif à 4 jokers sur ~40 Mo peut matcher ailleurs
//     → si plus d'un candidat, on ne patch RIEN et on reste en flag-write.
//     Octets originaux sauvegardés, restaurés à la désactivation + au Drop.
//
// ⚠ OFFSET PAR BUILD — m_bCameraInThirdPerson N'EST PAS dans les dumps a2x
//   (c'est un champ de la classe C++ CSGOInput, pas un champ schema). Le pin
//   par défaut (0x41BC) est la valeur publique du build 9640 (mai 2023) —
//   elle a sûrement bougé depuis. RE-DÉRIVATION (15 min, sans recompile):
//     1. lance un serveur local: `sv_cheats 1; thirdperson`
//     2. dump la zone [CSGOInput .. +0x8000] avant/après (ReClass ou un
//        petit script RPM diff)
//     3. le champ qui passe 0 → 256 (0x100) près de la vue est LE flag
//     4. pose la valeur dans offsets.json: { "m_bCameraInThirdPerson": "0xNNN" }
//   Le module est fail-soft: offset faux = la 3e personne ne s'active pas,
//   rien ne casse (le write reste dans le périmètre de CSGOInput).
//
// VOIE B GATED: comme le spinbot — endormi en LEGIT (mem.write est un no-op
// sans handle RW). Toggle menu RAGE + état live dans la carte.
//
// language: Rust, file: thirdperson.rs, target: Windows 11, windows-rs 0.58

use crate::mem::{Mem, ModuleInfo};
use crate::offsets::Offsets;
use crate::{dlog, dwarn};

/// valeur écrite dans le flag (le jeu teste non-nul; 256 = valeur publique UC)
const TP_FLAG_ON: i32 = 256;
/// valeur de repos du flag
const TP_FLAG_OFF: i32 = 0;

/// motif du gate 3e personne: `74 ?? ?? ?? ?? 48 ?? BA` (x?xxxx?x)
/// — signature publique UC, mai 2023. Elle peut être morte sur ton build:
/// le scan est fail-soft (niveau 2 opt-in, voir en-tête).
const GATE_PATTERN: [u8; 8] = [0x74, 0x00, 0x00, 0x00, 0x00, 0x48, 0x00, 0xBA];
const GATE_MASK: [bool; 8] = [true, false, false, false, false, true, false, true];

/// taille de chunk du scan (2 Mo + recouvrement motif)
const SCAN_CHUNK: usize = 2 * 1024 * 1024;

pub struct ThirdPerson {
    /// écritures du flag réussies ce cycle (statut)
    pub writes: u32,
    /// mode effectif: "flag+patch" | "flag seul" | "off"
    pub mode: &'static str,
    /// patch appliqué: (adresse, 2 octets originaux du je) — restauration
    patch: Option<(u64, [u8; 2])>,
    /// le scan a déjà été tenté (une fois par session)
    scan_done: bool,
    /// dernier état actif connu (détection des fronts)
    was_active: bool,
}

impl ThirdPerson {
    pub fn new() -> Self {
        ThirdPerson { writes: 0, mode: "off", patch: None, scan_done: false, was_active: false }
    }

    /// adresse du flag dans le CSGOInput.
    /// Convention a2x (primaire, celle des pins de ce projet): l'instance
    /// CSGOInput est EMBARQUÉE à client.dll + dwCSGOInput (la vue se lit à
    /// +0x688 en direct — même convention). Repli: pointeur déréférencé.
    fn flag_addr(mem: &Mem, offs: &Offsets) -> Option<u64> {
        if offs.m_b_camera_in_third_person == 0 {
            return None;
        }
        // primaire: instance statique embarquée
        let direct = mem.client.base + offs.dw_csgo_input + offs.m_b_camera_in_third_person;
        Some(direct)
    }

    /// scan du motif gate dans client.dll — retourne l'adresse si UNIQUE.
    fn scan_gate(mem: &Mem) -> Option<u64> {
        let module: &ModuleInfo = &mem.client;
        let size = module.size as usize;
        let mut candidates: Vec<u64> = Vec::new();
        let mut buf = vec![0u8; SCAN_CHUNK + GATE_PATTERN.len()];
        let mut off = 0usize;
        while off < size {
            let len = (size - off).min(SCAN_CHUNK);
            if len < GATE_PATTERN.len() {
                break;
            }
            if !mem.read_bytes(module.base + off as u64, &mut buf[..len]) {
                break; // page illisible: on s'arrête (le code est en début de module)
            }
            let hay = &buf[..len];
            for i in 0..=hay.len() - GATE_PATTERN.len() {
                let mut hit = true;
                for (j, mb) in GATE_MASK.iter().enumerate() {
                    if *mb && hay[i + j] != GATE_PATTERN[j] {
                        hit = false;
                        break;
                    }
                }
                if hit {
                    candidates.push(module.base + (off + i) as u64);
                    if candidates.len() > 1 {
                        return None; // ambigu → on ne patch jamais à l'aveugle
                    }
                }
            }
            off += len - (GATE_PATTERN.len() - 1); // recouvrement
        }
        candidates.pop() // unique → Some(addr), aucun → None
    }

    /// applique le patch gate (je → jmp court, même cible). Retourne true si
    /// patché. Idempotent.
    fn apply_gate_patch(&mut self, mem: &Mem) -> bool {
        if self.patch.is_some() {
            return true;
        }
        let Some(addr) = Self::scan_gate(mem) else {
            dwarn!("[tp  ] motif gate introuvable (ou ambigu) — reste en flag-write (niveau 1)");
            return false;
        };
        // relis le je présent à l'adresse (sécurité: c'est bien 0x74)
        let mut orig = [0u8; 2];
        if !mem.read_bytes(addr, &mut orig) || orig[0] != 0x74 {
            dwarn!("[tp  ] octets @ gate inattendus ({:02X} {:02X}) — patch annulé", orig[0], orig[1]);
            return false;
        }
        // 74 disp → EB disp: saut inconditionnel vers la MÊME cible
        let patched = [0xEBu8, orig[1]];
        if mem.write_bytes(addr, &patched) {
            self.patch = Some((addr, orig));
            dlog!("[tp  ] gate patché @ {:#018x}: 74 {:02X} → EB {:02X} (je → jmp, même cible)", addr, orig[1], orig[1]);
            true
        } else {
            false
        }
    }

    /// restaure le gate original (désactivation / Drop)
    fn restore_gate_patch(&mut self, mem: &Mem) {
        if let Some((addr, orig)) = self.patch.take() {
            if mem.write_bytes(addr, &orig) {
                dlog!("[tp  ] gate restauré @ {:#018x} ({:02X} {:02X})", addr, orig[0], orig[1]);
            } else {
                dwarn!("[tp  ] restauration gate ÉCHOUÉE @ {:#018x} — jeu en pause ?", addr);
            }
        }
    }

    /// appelé à chaque itération de boucle (~0,5-1 ms) — comme le spinbot.
    /// enabled: toggle menu (ET master RAGE — le caller gate sur rage_on).
    /// ctx_ok: jeu au premier plan, pas de menu/buy. alive: pawn vivant.
    /// want_patch: niveau 2 opt-in (scan + patch moteur — voir en-tête).
    pub fn update(
        &mut self,
        mem: &Mem,
        offs: &Offsets,
        enabled: bool,
        ctx_ok: bool,
        alive: bool,
        want_patch: bool,
    ) {
        self.writes = 0;
        let active = enabled && ctx_ok && alive && mem.write_open();

        // front descendant → tout remettre proprement (flag 0 + gate original)
        if self.was_active && !active {
            if let Some(addr) = Self::flag_addr(mem, offs) {
                let _ = mem.write(addr, &TP_FLAG_OFF);
            }
            self.restore_gate_patch(mem);
            self.mode = "off";
            dlog!("[tp  ] 3e personne OFF — flag 0 + gate restauré");
        }
        self.was_active = active;
        if !active {
            return;
        }

        let Some(addr) = Self::flag_addr(mem, offs) else {
            self.mode = "off";
            if !self.scan_done {
                self.scan_done = true;
                dwarn!("[tp  ] offset m_bCameraInThirdPerson = 0 — module inactif (re-dérive, voir offsets.rs)");
            }
            return;
        };

        // niveau 2 opt-in: le patch moteur (une tentative par session)
        if !self.scan_done && want_patch {
            self.scan_done = true;
            if self.apply_gate_patch(mem) {
                self.mode = "flag+patch";
            }
        }
        if self.patch.is_some() {
            self.mode = "flag+patch";
        } else if self.mode != "flag seul" {
            self.mode = "flag seul";
            dlog!("[tp  ] 3e personne active (flag-write ~1 kHz) — pin 0x{:X} (build 9640: re-dérive si la caméra ne bouge pas, offsets.rs)", offs.m_b_camera_in_third_person);
        }

        // niveau 1: le flag, écrit à CHAQUE itération (le jeu le reset par frame)
        if mem.write(addr, &TP_FLAG_ON) {
            self.writes += 1;
        }
    }
}
