// ─────────────────────────────────────────────────────────────────────────────
// OFFSETS CS2 v0.5 — pins build 14193 + override par offsets.json · Vantyx.cc
//
// CS2 (Source 2) expose deux familles d'adresses:
//
//  1) SIGNATURES (relatives à client.dll, sauf dwBuildNumber → engine2.dll):
//     dwLocalPlayerPawn  — slot contenant le pointeur vers TON pawn
//     dwEntityList       — slot du CGameEntitySystem (déréférencer 1×)
//     dwViewMatrix       — matrice monde→écran 3×4 row-major (12 f32)
//     dwBuildNumber      — build du jeu (engine2.dll) → vérif d'alignement
//
//  2) CHAMPS SCHEMA (relatifs à l'entité qui les porte):
//     pawn : m_pGameSceneNode → node (+ m_vecAbsOrigin = position aux pieds)
//            m_iHealth, m_iTeamNum (2=T, 3=CT), m_lifeState (0 = vivant)
//            m_angEyeAngles (QAngle), m_iIDEntIndex (entité sous le viseur →
//            triggerbot), m_bIsBuyMenuOpen (menu d'achat ouvert)
//     ctrl : m_hPlayerPawn (handle vers le pawn), m_sSanitizedPlayerName
//     bone : node EST un CSkeletonInstance → m_modelState (0x140, EMBARQUÉ)
//            → pointeur du cache d'os à +0x80 ; entrée = transform 0x20 avec
//            la position (x,y,z) DANS LES 3 PREMIERS f32 (vérifié: python
//            burakacar6 + IMXNOOBX/cs2-external-esp — PAS des matrices 3×4).
//
//  3) v0.5 — MODULES (tous minés du dump local build 14193, 29.10.2026):
//     dwCSGOInput + dwViewAngles — vue LIVE du CSGOInput (rafraîchie au frame
//            de rendu — c'est LA source pour l'aimbot/RCS, m_angEyeAngles
//            du pawn traîne d'un tick et causait l'overshoot en éventail)
//     dwSensitivity (+0x58) — sensibilité RÉELLE lue dans le jeu (la
//            conversion degrés→counts ne dépend plus d'un réglage manuel)
//     dwPlantedC4 + dwGameRules — bomb timer (m_flC4Blow / timer / defuse)
//     m_fFlags (bunnyhop), m_vecVelocity (sound ESP), m_iShotsFired (RCS,
//            accuracy lab), m_pObserverServices/m_hObserverTarget (spectateurs)
//     m_pEntity(0x10) → CEntityIdentity → m_designerName(0x20, char*) —
//            walk du MONDE: armes au sol + projectiles de grenades
//
//  MISE À JOUR SANS RECOMPILER: pose un `offsets.json` à côté de l'exe.
//  Deux formats acceptés:
//    a) PLAT (généré par scripts/gen_offsets_json.py):
//       { "dwEntityList": "0x271EB08", "m_iHealth": "0x34C", ... }
//    b) BRUT du dumper a2x/cs2-dumper (offsets.json d'origine):
//       { "client.dll": { "dwEntityList": 41020168, ... },
//         "engine2.dll": { "dwBuildNumber": 7053352, ... } }
//       → les signatures se mettent à jour; les champs schema manquants
//         gardent les pins (recompile si le walk casse).
//  Le build du jeu est lu au boot et comparé → un warn nomme le problème.
//
//  Pins actuels: dump LOCAL cs2-dumper lancé sur le jeu de dj,
//  build 14193 — 29.10.2026 09:27 — analysis 5.89 s (tools/cs2-dumper).
// ─────────────────────────────────────────────────────────────────────────────

use crate::debug;
use crate::mem::{Mem, ModuleInfo};
use crate::{dlog, dwarn};

#[derive(Clone, Copy, Debug)]
pub struct Offsets {
    // signatures client.dll
    pub dw_entity_list: u64,
    pub dw_view_matrix: u64,
    pub dw_local_pawn: u64,
    pub dw_local_controller: u64,
    // signature engine2.dll
    pub dw_build_number: u64,
    // build attendu pour ces offsets (i32 lu dans le jeu)
    pub expected_build: i32,
    // marche de la liste d'entités (0x70 ici — 0x78 avant le patch d'oct. 2026)
    pub list_entry_stride: u64,
    // champs — pawn (C_CSPlayerPawn)
    pub m_p_game_scene_node: u64,
    pub m_i_health: u64,
    pub m_i_team_num: u64,
    pub m_life_state: u64,
    pub m_b_dormant: u64,          // v0.7.1 — CEntityInstance 0x8C: entité dormante (hors PVS → données PÉRIMÉES)
    pub m_ang_eye_angles: u64,   // QAngle (pitch, yaw) en degrés — info/debug
    pub m_i_ident_index: u64,    // CEntityIndex sous le viseur — triggerbot
    pub m_buy_menu_open: u64,    // bool — menu d'achat ouvert
    // champs — contrôleur (CCSPlayerController)
    pub m_h_player_pawn: u64,
    pub m_sanitized_name: u64,
    // champs — CGameSceneNode
    pub m_vec_abs_origin: u64,
    // champs — CSkeletonInstance (le node d'un pawn EST un CSkeletonInstance)
    pub skeleton_model_state: u64, // m_modelState embarqué
    pub bone_array_off: u64,       // pointeur du cache d'os DANS le modelState
    pub bone_stride: u64,          // octets par entrée d'os (0x20 = pos + quaternion) — v0.4 surchargeable
    // ── v0.5 — signatures modules (client.dll sauf mention) ──
    pub dw_csgo_input: u64,        // slot du pointeur CSGOInput
    pub dw_view_angles: u64,       // offset du champ QAngle DANS CSGOInput (voir read_view_angles)
    pub dw_planted_c4: u64,        // slot → C_PlantedC4 quand la bombe est posée
    pub dw_game_rules: u64,        // slot → C_CSGameRules (m_bBombPlanted)
    pub dw_sensitivity: u64,       // slot → structure sensibilité
    pub dw_sensitivity_field: u64, // offset du f32 sensibilité dans la structure
    // ── v0.9 — caméra 3e personne (CCSGOInput, PAS un champ schema) ──
    /// offset de m_bCameraInThirdPerson DANS l'instance CSGOInput.
    /// ⚠ PAR BUILD — absent des dumps a2x (classe C++ pure). Pin = valeur
    /// publique du build 9640 (UC "Third Person External", mai 2023) — a
    /// sûrement bougé depuis. Re-dérivation sans recompile:
    ///   1. serveur local: sv_cheats 1; thirdperson
    ///   2. diff mémoire de [client+dwCSGOInput .. +0x8000] avant/après
    ///   3. le champ 0 → 256 près de la vue = LE flag
    ///   4. offsets.json: { "m_bCameraInThirdPerson": "0xNNN" }
    /// 0 = module désactivé proprement.
    pub m_b_camera_in_third_person: u64,
    // ── v0.5 — champs modules ──
    pub m_f_flags: u64,            // C_BaseEntity — FL_ONGROUND = bit 0 (bunnyhop)
    pub m_vec_velocity: u64,       // C_BaseEntity — vitesse (sound ESP)
    pub m_i_shots_fired: u64,      // C_CSPlayerPawn — compteur de rafale (RCS / lab)
    pub m_p_observer_services: u64, // C_BasePlayerPawn — spectateurs
    pub m_h_observer_target: u64,  // CPlayer_ObserverServices — handle de la cible observée
    pub m_i_observer_mode: u64,    // CPlayer_ObserverServices — >0 = en spectateur
    pub m_fl_c4_blow: u64,         // C_PlantedC4 — timestamp d'explosion (serv.)
    pub m_fl_timer_length: u64,    // C_PlantedC4 — durée de la mèche (40 s)
    pub m_fl_defuse_countdown: u64, // C_PlantedC4 — restant de désamorçage
    pub m_b_bomb_defused: u64,     // C_PlantedC4 — bool
    pub m_b_bomb_planted_rules: u64, // C_CSGameRules — bool bombe posée (validation)
    pub m_total_rounds_played: u64, // v0.8 — C_CSGameRules m_totalRoundsPlayed (purge monde / round)
    pub m_entity_identity: u64,    // CEntityInstance::m_pEntity → CEntityIdentity
    pub m_designer_name: u64,      // CEntityIdentity::m_designerName (char*)
    pub m_p_weapon_services: u64,  // C_BasePlayerPawn → armes
    pub m_h_active_weapon: u64,    // CPlayer_WeaponServices — handle arme courante
    pub m_n_highest_entity: u64,   // CGameEntitySystem — index max d'entité
    pub m_h_owner_entity: u64,     // v0.6 — C_BaseEntity 0x520: handle du propriétaire (0xFFFFFFFF = au sol)
    pub dw_global_vars: u64,       // v0.6 — client.dll GlobalVars (curtime serveur)
    // ── v0.7 — no-recoil feed-forward (recul balistique CS2) ──
    pub m_p_aim_punch_services: u64, // C_CSPlayerPawn → CCSPlayer_AimPunchServices
    pub punch_pred_angle: u64,       // QAngle dans le services: base ANGLE prédictible
    pub punch_unpred_angle: u64,     // QAngle dans le services: base ANGLE non prédictible
}

// ce que load() rapporte (affiché dans STATUT/INFO)
#[derive(Clone, Debug)]
pub struct LoadReport {
    pub source: String, // "pins" | "offsets.json (N clés)" | "offsets.json a2x (signatures)"
}

impl Offsets {
    // pins = dump local build 14193 (29.10.2026) — tout est vérifié en live
    pub fn pinned() -> Self {
        Offsets {
            dw_entity_list: 0x271EB08,
            dw_view_matrix: 0x256F1D0,
            dw_local_pawn: 0x25698C8,
            dw_local_controller: 0x25413F8,
            dw_build_number: 0x6BA028,
            expected_build: 14193,
            list_entry_stride: 0x70,

            m_p_game_scene_node: 0x330,
            m_i_health: 0x34C,
            m_i_team_num: 0x3E7,
            m_life_state: 0x354,
            m_b_dormant: 0x8C,         // v0.7.1 — CEntityInstance 140 (stable depuis 2024)
            //   ↑ si un warn "m_bDormant" apparaît en console: le build a bougé
            //     le champ → pose la valeur à jour dans offsets.json (clé m_bDormant)
            m_ang_eye_angles: 0x3600,
            m_i_ident_index: 0x36DC,
            m_buy_menu_open: 0x15EA,

            m_h_player_pawn: 0x92C,
            m_sanitized_name: 0x878,

            m_vec_abs_origin: 0xC8,

            skeleton_model_state: 0x140,
            bone_array_off: 0x80,
            bone_stride: 0x20,

            // v0.6 — CORRECTION (dump-actual 29.10.2026, build 14193): 4
            //   signatures étaient mal transcrites et tuaient le bomb timer
            //   (dwGameRules/dwPlantedC4), la sens auto (dwSensitivity) et la
            //   lecture de vue (dwCSGOInput):
            //   dwCSGOInput  0x257F940 → 0x257F980 (dump)
            //   dwPlantedC4  0x24CB6E0 → 0x24CFB60 (dump)
            //   dwGameRules  0x25706B8 → 0x2569378 (dump)
            //   dwSensitivity 0x256DF28 → 0x2566BC8 (dump)
            dw_csgo_input: 0x257F980,
            dw_view_angles: 0x2580008, // adresse DIRECTE du champ (a2x) — voir entity::read_view_angles
            // v0.9 — 3e personne: pin build 9640 (UC mai 2023), à re-dériver
            // pour le build courant — voir la doc du champ + offsets.json
            m_b_camera_in_third_person: 0x41BC,
            dw_planted_c4: 0x24CFB60,
            dw_game_rules: 0x2569378,
            dw_sensitivity: 0x2566BC8,
            dw_sensitivity_field: 0x58, // dwSensitivity_sensitivity (88)
            dw_global_vars: 0x2234108, // v0.6 — GlobalVars (horloge serveur, bomb timer)

            m_f_flags: 0x3F4,            // C_BaseEntity 1012
            m_vec_velocity: 0x430,       // C_BaseEntity 1072
            m_i_shots_fired: 0x1EB8,     // C_CSPlayerPawn 7864
            m_p_observer_services: 0x1308, // C_BasePlayerPawn 4872
            m_h_observer_target: 0x4C,   // ObserverServices 76
            m_i_observer_mode: 0x48,     // ObserverServices 72
            m_fl_c4_blow: 0x12B8,       // C_PlantedC4 4792
            m_fl_timer_length: 0x12C0,  // C_PlantedC4 4800
            m_fl_defuse_countdown: 0x12D8, // C_PlantedC4 4824
            m_b_bomb_defused: 0x12DC,   // C_PlantedC4 4828
            m_b_bomb_planted_rules: 0x8C7, // C_CSGameRules 2247
            m_total_rounds_played: 0x88,   // v0.8 — dump client_dll.json C_CSGameRules m_totalRoundsPlayed
            m_entity_identity: 0x10,    // CEntityInstance 16
            m_designer_name: 0x20,      // CEntityIdentity 32
            m_p_weapon_services: 0x12F0, // C_BasePlayerPawn 4848
            m_h_active_weapon: 0x60,    // CPlayer_WeaponServices 96
            m_n_highest_entity: 0x2120, // CGameEntitySystem 8480
            m_h_owner_entity: 0x520,   // v0.6 — C_BaseEntity 1312 (filtre armes au sol)

            // v0.7 — AimPunchServices (dump-actual build 14193, 29.10.2026):
            //   m_pAimPunchServices        @ 0x1598 (CCSPlayer_AimPunchServices*)
            //   m_predictableBaseAngle    @ 0x50 (QAngle — part prédite du recul)
            //   m_unpredictableBaseAngle  @ 0xA4 (QAngle — part aléatoire)
            // la deviation balistique ≈ pred + unpred (voir modules.rs NoRecoil)
            m_p_aim_punch_services: 0x1598, // 5528
            punch_pred_angle: 0x50,        // 80
            punch_unpred_angle: 0xA4,      // 164
        }
    }

    // ═══ charge pins + offsets.json s'il existe (override clé par clé) ═══
    pub fn load() -> (Offsets, LoadReport) {
        let mut o = Offsets::pinned();
        let mut applied = 0usize;
        let mut nested_sigs = false;
        let Some((path, text)) = find_offsets_json() else {
            return (o, LoadReport { source: "pins".into() });
        };
        let clean = text.trim_start_matches('\u{feff}').to_string();

        if clean.trim_start().starts_with('{') && depth_looks_nested(&clean) {
            // format a2x brut: {"client.dll": {...}, "engine2.dll": {...}}
            let client = extract_block(&clean, "client.dll");
            let engine = extract_block(&clean, "engine2.dll");
            for (k, v) in parse_pairs(client.as_deref()) {
                match k.as_str() {
                    "dwEntityList" => { o.dw_entity_list = v; applied += 1; }
                    "dwViewMatrix" => { o.dw_view_matrix = v; applied += 1; }
                    "dwLocalPlayerPawn" => { o.dw_local_pawn = v; applied += 1; }
                    "dwLocalPlayerController" => { o.dw_local_controller = v; applied += 1; }
                    "dwCSGOInput" => { o.dw_csgo_input = v; applied += 1; }
                    "dwViewAngles" => { o.dw_view_angles = v; applied += 1; }
                    "dwPlantedC4" => { o.dw_planted_c4 = v; applied += 1; }
                    "dwGameRules" => { o.dw_game_rules = v; applied += 1; }
                    "dwSensitivity" => { o.dw_sensitivity = v; applied += 1; }
                    "dwGlobalVars" => { o.dw_global_vars = v; applied += 1; }
                    "m_hOwnerEntity" => { o.m_h_owner_entity = v; applied += 1; }
                    _ => {}
                }
            }
            for (k, v) in parse_pairs(engine.as_deref()) {
                if k == "dwBuildNumber" {
                    o.dw_build_number = v;
                    applied += 1;
                }
            }
            nested_sigs = true;
            if applied > 0 {
                dwarn!("[offs] offsets.json au format a2x: {applied} signatures mises à jour — les champs schema gardent les pins (génère un json PLAT pour tout couvrir, scripts/gen_offsets_json.py)");
            }
        } else {
            for (k, v) in parse_pairs(Some(&clean)) {
                match k.as_str() {
                    "dwEntityList" => { o.dw_entity_list = v; applied += 1; }
                    "dwViewMatrix" => { o.dw_view_matrix = v; applied += 1; }
                    "dwLocalPlayerPawn" => { o.dw_local_pawn = v; applied += 1; }
                    "dwLocalPlayerController" => { o.dw_local_controller = v; applied += 1; }
                    "dwBuildNumber" => { o.dw_build_number = v; applied += 1; }
                    "expected_build" | "build_number" => { o.expected_build = v as i32; applied += 1; }
                    "list_entry_stride" => { o.list_entry_stride = v; applied += 1; }
                    "m_pGameSceneNode" => { o.m_p_game_scene_node = v; applied += 1; }
                    "m_iHealth" => { o.m_i_health = v; applied += 1; }
                    "m_iTeamNum" => { o.m_i_team_num = v; applied += 1; }
                    "m_lifeState" => { o.m_life_state = v; applied += 1; }
                    "m_bDormant" => { o.m_b_dormant = v; applied += 1; }
                    "m_angEyeAngles" => { o.m_ang_eye_angles = v; applied += 1; }
                    "m_iIDEntIndex" => { o.m_i_ident_index = v; applied += 1; }
                    "m_bIsBuyMenuOpen" => { o.m_buy_menu_open = v; applied += 1; }
                    "m_hPlayerPawn" => { o.m_h_player_pawn = v; applied += 1; }
                    "m_sSanitizedPlayerName" => { o.m_sanitized_name = v; applied += 1; }
                    "m_vecAbsOrigin" => { o.m_vec_abs_origin = v; applied += 1; }
                    "m_modelState" => { o.skeleton_model_state = v; applied += 1; }
                    "bone_array_off" => { o.bone_array_off = v; applied += 1; }
                    "bone_stride" => { o.bone_stride = v.max(0x10); applied += 1; }
                    // v0.5 — modules
                    "dwCSGOInput" => { o.dw_csgo_input = v; applied += 1; }
                    "dwViewAngles" => { o.dw_view_angles = v; applied += 1; }
                    "dwPlantedC4" => { o.dw_planted_c4 = v; applied += 1; }
                    "dwGameRules" => { o.dw_game_rules = v; applied += 1; }
                    "dwSensitivity" => { o.dw_sensitivity = v; applied += 1; }
                    "m_bCameraInThirdPerson" => { o.m_b_camera_in_third_person = v; applied += 1; } // v0.9 — 3e personne
                    "dwGlobalVars" => { o.dw_global_vars = v; applied += 1; }
                    "dwSensitivity_sensitivity" => { o.dw_sensitivity_field = v; applied += 1; }
                    "m_fFlags" => { o.m_f_flags = v; applied += 1; }
                    "m_vecVelocity" => { o.m_vec_velocity = v; applied += 1; }
                    "m_iShotsFired" => { o.m_i_shots_fired = v; applied += 1; }
                    "m_pObserverServices" => { o.m_p_observer_services = v; applied += 1; }
                    "m_hObserverTarget" => { o.m_h_observer_target = v; applied += 1; }
                    "m_iObserverMode" => { o.m_i_observer_mode = v; applied += 1; }
                    "m_flC4Blow" => { o.m_fl_c4_blow = v; applied += 1; }
                    "m_flTimerLength" => { o.m_fl_timer_length = v; applied += 1; }
                    "m_flDefuseCountDown" => { o.m_fl_defuse_countdown = v; applied += 1; }
                    "m_bBombDefused" => { o.m_b_bomb_defused = v; applied += 1; }
                    "m_bBombPlanted" => { o.m_b_bomb_planted_rules = v; applied += 1; }
                    "m_totalRoundsPlayed" => { o.m_total_rounds_played = v; applied += 1; }
                    "m_pEntity" => { o.m_entity_identity = v; applied += 1; }
                    "m_designerName" => { o.m_designer_name = v; applied += 1; }
                    "m_pWeaponServices" => { o.m_p_weapon_services = v; applied += 1; }
                    "m_hActiveWeapon" => { o.m_h_active_weapon = v; applied += 1; }
                    "m_hOwnerEntity" => { o.m_h_owner_entity = v; applied += 1; }
                    "dwGameEntitySystem_highestEntityIndex" => { o.m_n_highest_entity = v; applied += 1; }
                    // v0.7 — punch (recul balistique)
                    "m_pAimPunchServices" => { o.m_p_aim_punch_services = v; applied += 1; }
                    "punch_pred_angle" => { o.punch_pred_angle = v; applied += 1; }
                    "punch_unpred_angle" => { o.punch_unpred_angle = v; applied += 1; }
                    _ => {}
                }
            }
        }

        let source = if applied == 0 {
            dwarn!("[offs] offsets.json présent ({}) mais AUCUNE clé reconnue — pins conservés", path.display());
            "pins (json illisible)".into()
        } else if nested_sigs {
            format!("offsets.json a2x ({applied} sig.)")
        } else {
            dlog!("[offs] offsets.json: {applied} clés appliquées — {}", path.display());
            format!("offsets.json ({applied} clés)")
        };
        (o, LoadReport { source })
    }

    // lit le build RÉEL du jeu (engine2.dll + dwBuildNumber) et le compare
    pub fn check_build(&self, mem: &Mem) -> i32 {
        let addr = mem.engine.base + self.dw_build_number;
        match mem.read::<i32>(addr) {
            Some(b) if b > 0 => {
                if b == self.expected_build {
                    dlog!("[ chk] build du jeu {b} = build du dump → offsets ALIGNÉS");
                } else {
                    dwarn!("[ chk] build du jeu {b} ≠ build du dump {} → OFFSETS DÉCALÉS: re-dump (tools/cs2-dumper) puis remplace offsets.json / recompile", self.expected_build);
                }
                b
            }
            _ => {
                dwarn!("[ chk] build illisible @ {:#x} — engine2.dll chargé ?", addr);
                0
            }
        }
    }

    // imprime la carte des adresses absolues — LA table "où regarder / modifier"
    pub fn print_resolution(&self, mem: &Mem) {
        let c = mem.client.base;
        debug::emit("──────── carte mémoire (c'est ici que tu regardes / modifies) ────────".into());
        debug::emit(format!("[addr] client.dll              {:#018x}", c));
        debug::emit(format!("[addr] + dwEntityList      +{:#010x} → {:#018x}  ← slot du système d'entités (déréf. 1×)", self.dw_entity_list, c + self.dw_entity_list));
        debug::emit(format!("[addr] + dwViewMatrix      +{:#010x} → {:#018x}  ← f32×12 (3×4) monde→écran", self.dw_view_matrix, c + self.dw_view_matrix));
        debug::emit(format!("[addr] + dwLocalPlayerPawn +{:#010x} → {:#018x}  ← slot: ptr de TON pawn", self.dw_local_pawn, c + self.dw_local_pawn));
        debug::emit(format!("[addr] + dwLocalPlayerCtrl +{:#010x} → {:#018x}  ← slot: ptr de TON contrôleur", self.dw_local_controller, c + self.dw_local_controller));
        debug::emit(format!("[addr] pawn + m_pGameSceneNode  +{:#010x} → node, puis +m_vecAbsOrigin +{:#010x} = position", self.m_p_game_scene_node, self.m_vec_abs_origin));
        debug::emit(format!("[addr] pawn + m_iHealth         +{:#010x} — écris un i32 ici: 100 = pleine vie", self.m_i_health));
        debug::emit(format!("[addr] pawn + m_iTeamNum        +{:#010x} — écris 2 (T) / 3 (CT): change de camp", self.m_i_team_num));
        debug::emit(format!("[addr] pawn + m_iIDEntIndex     +{:#010x} — entité sous le viseur (triggerbot)", self.m_i_ident_index));
        debug::emit(format!("[addr] ctrl + m_hPlayerPawn     +{:#010x} — handle u32 vers le pawn (this→pawn)", self.m_h_player_pawn));
        debug::emit(format!("[addr] ctrl + m_sSanitizedName  +{:#010x} — nom du joueur", self.m_sanitized_name));
        debug::emit("──────────────────────────────────────────────────────────────────────".into());
    }

    // adresses absolues pratiques
    pub fn entity_list(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_entity_list }
    pub fn view_matrix(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_view_matrix }
    pub fn local_pawn_slot(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_local_pawn }
    pub fn csgo_input_slot(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_csgo_input }
    pub fn planted_c4_slot(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_planted_c4 }
    pub fn game_rules_slot(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_game_rules }
    pub fn global_vars_addr(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_global_vars }
    pub fn dw_view_angles_addr(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_view_angles }
    pub(crate) fn dw_sensitivity_addr(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_sensitivity }
    pub(crate) fn dw_csgo_input_addr(&self, m: &ModuleInfo) -> u64 { m.base + self.dw_csgo_input }
}

// ═══ mini-parseur JSON tolérant (zéro dépendance) ═══════════════════════════

// offsets.json: exe dir → 3 ancêtres → cwd (le zip le pose à côté de l'exe)
fn find_offsets_json() -> Option<(std::path::PathBuf, String)> {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
            let mut cur = dir.to_path_buf();
            for _ in 0..3 {
                if let Some(parent) = cur.parent() {
                    cur = parent.to_path_buf();
                    roots.push(cur.clone());
                }
            }
        }
    }
    roots.push(std::env::current_dir().unwrap_or_default());
    for r in roots {
        let p = r.join("offsets.json");
        if let Ok(text) = std::fs::read_to_string(&p) {
            return Some((p, text));
        }
    }
    None
}

// heuristique: le premier "valeur" rencontré est-il un objet (dump a2x) ?
fn depth_looks_nested(s: &str) -> bool {
    if let Some(pos) = s.find(':') {
        let after = s[pos + 1..].trim_start();
        return after.starts_with('{');
    }
    false
}

// extrait le contenu brut d'un bloc "clé": { ... } (accolades équilibrées)
fn extract_block(s: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = s.find(&needle)? + needle.len();
    let rest = &s[start..];
    let brace = rest.find('{')?;
    let mut depth = 0i32;
    for (i, ch) in rest[brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[brace..brace + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

// paires "clé": valeur d'un objet PLAT — hex "0x…", décimal, ou entre quotes
fn parse_pairs(s: Option<&str>) -> Vec<(String, u64)> {
    let Some(s) = s else { return Vec::new() };
    let mut out = Vec::new();
    let bytes: Vec<char> = s.chars().collect();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        if bytes[i] != '"' {
            i += 1;
            continue;
        }
        // clé
        let mut j = i + 1;
        let mut key = String::new();
        while j < n && bytes[j] != '"' {
            key.push(bytes[j]);
            j += 1;
        }
        i = j + 1;
        // jusqu'au ':'
        while i < n && bytes[i] != ':' && bytes[i] != '"' {
            i += 1;
        }
        if i >= n || bytes[i] != ':' {
            continue; // clé sans valeur → boucle
        }
        i += 1;
        while i < n && bytes[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        // valeur: nombre, chaîne, ou objet (ignoré ici)
        let (val, next) = if bytes[i] == '"' {
            let mut j = i + 1;
            let mut v = String::new();
            while j < n && bytes[j] != '"' {
                v.push(bytes[j]);
                j += 1;
            }
            (v, j + 1)
        } else if bytes[i] == '{' {
            let mut depth = 0i32;
            let mut j = i;
            while j < n {
                match bytes[j] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            (String::new(), j)
        } else {
            let mut j = i;
            while j < n && bytes[j] != ',' && bytes[j] != '}' {
                j += 1;
            }
            (bytes[i..j].iter().collect::<String>(), j)
        };
        let vtrim = val.trim().trim_matches('"');
        if let Some(v) = parse_num(vtrim) {
            out.push((key.clone(), v));
        }
        i = next.max(i + 1);
    }
    out
}

// "0x271EB08" | "41020168" | 41020168 → u64
fn parse_num(s: &str) -> Option<u64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok();
    }
    t.parse::<u64>().ok().or_else(|| t.parse::<i64>().ok().map(|v| v as u64))
}
