// aimbot — visée automatique externe v0.5 (Vantyx.cc).
//
// MÉCANIQUE (zéro écriture mémoire — le projet reste READ-ONLY):
//   1. position caméra extraite de la dwViewMatrix (c = -Rᵀ·t), validée
//      contre l'origine locale
//   2. cible = ennemi vivant dont la hitbox choisie est la plus proche
//      ANGULAIREMENT du viseur courant, dans le FOV
//   3. delta (pitch, yaw) → compteurs souris via m_yaw (0.022°/count ×
//      sensibilité) → SendInput(MOUSEEVENTF_MOVE) — le jeu voit un vrai
//      mouvement de souris
//
// ═══ v0.5 — LE FIX DU BUG "l'ennemi se cale sur le bord du cercle" ═══
// La vue en mémoire (m_angEyeAngles du pawn, ET dwViewAngles) n'est
// rafraîchie par le jeu qu'au FRAMERATE (60-400 fps). La boucle tourne à
// ~1 kHz: l'ancien code renvoyait jusqu'à ~15× LE MÊME delta calculé sur
// une vue périmée — chaque envoi multipliait le mouvement voulu → overshoot
// en éventail: l'ennemi traversait le viseur, ressortait du FOV, revenait…
// sans jamais se poser au centre.
//
//   FIX 1 — vue source: dwViewAngles (CSGOInput, suit le frame de RENDU,
//           plus frais que le m_angEyeAngles du pawn qui traîne d'un tick)
//   FIX 2 — GATE VUE FRAÎCHE: un mouvement n'est envoyé QUE si la vue lue
//           a CHANGÉ depuis le dernier envoi. Le taux effectif devient le
//           framerate du jeu, chaque delta est calculé sur des données
//           fraîches → convergence exponentielle propre, zéro overshoot.
//   FIX 3 — sensibilité AUTO lue dans la mémoire du jeu (dwSensitivity):
//           la conversion degrés→counts ne dépend plus d'un réglage manuel
//           qui, s'il était faux, décalait systématiquement l'équilibre.
//
// ═══ v0.6.2 — LE FIX du bug "l'ennemi se cale sur le bord du cercle" ═══
// AXE X INVERSÉ: SendInput dx POSITIF tourne à DROITE = yaw DÉCROÎT en
// convention Source (même math que le RCS — modules.rs, dx = -Δyaw).
// L'ancien code envoyait dx = +Δyaw: la visée FUYAIT la cible en horizontal
// jusqu'à ce qu'elle sorte du cône FOV → équilibre stable CIBLE SUR LE BORD
// du cercle, hauteur de tête OK (l'axe Y, lui, était correct). Un seul signe.
//
//   FIX 4 — SIGNE X: dx = -Δyaw/deg_per_count (aligné sur le RCS, vérifié
//           en live par la compensation qui marchait déjà).
//   FIX 5 — RÉSIDU SUB-COUNT (vraiment, cette fois): le carry était RESET à
//           CHAQUE frame dans le prologue d'update() — l'accumulation
//           prévue en v0.6.1 était morte-née, le dernier demi-count n'était
//           jamais envoyé. Le reset ne se fait plus que sur les sorties
//           INACTIVES (désactivé/touche relâchée/mort/pas de snapshot), le
//           résidu persiste entre les frames tant que la cible est la même.
//
// ═══ v0.8.2 — LE FIX DU BUG "vise les CÔTÉS de la tête, jamais le centre" ═══
// CAM_POS: le dés-échelonnage v0.6.2 était ALGÉBRIQUEMENT FAUX. La matrice
// est [S·R | t] avec S = diag(s0, s1, 1), s0 = fx ≈ 0.75 et s1 = fy ≈ 1.33
// en 16:9/74° vfov (PAS 1 !). La caméra exacte est c = -Rᵀ·S⁻¹·t. La v0.6.2
// divisait CHAQUE ligne du produit par la même échelle au lieu de
// dés-échelonner R et t séparément → les termes croisés partaient avec la
// mauvaise échelle. Erreur mesurée (validation numérique): NULLE à pitch 0,
// mais 90-290 u (1.7-5.5 m !) à pitch ±45/60° selon le yaw — et l'ancienne
// sanité (±300 u) LAISSAIT PASSER ces caméras fausses quand l'erreur
// restait sous 300 → l'aimbot verrouillait des angles calculés depuis une
// caméra décalée → le viseur se POSE À CÔTÉ de la tête (souvent sur le
// côté: l'erreur est majoritairement latérale), différemment à chaque
// angle. Quand l'erreur dépassait 300 u le repli origin+64 reprenait → le
// comportement BASCULAIT en bougeant la vue: "sous certains angles ça tire
// pas, c'est buggy et injouable". Fix: R et t dés-échelonnés PUIS
// c = -Rᵀ·t (exact à ±0.001 u) + sanité RESSERRÉE (±24 u) + repli
// accordéon (œil 46 u si FL_DUCKING).
// DEADZONE: 0.05° → 0.012° — 0.05° à 40 m = 3.7 u ≈ le RAYON d'une tête:
// l'aim s'arrêtait à mi-chemin du bord au lieu de finir au centre.
// RCS: interlock aimbot (modules.rs) — pendant un spray il ANNULAIT les
// corrections de l'aimbot en les prenant pour du recul.
//
// RÉGLAGES: touche (AUTO = permanent), hitbox (tête/cou/poitrine/corps),
// FOV en degrés, lissage (divise le delta), sensibilité (auto ou manuelle).
//
// ═══ v0.9 — PRÉCISION DE GROUPE + CIBLES MOBILES (algorithmes: aimcore.rs) ═══
//   · VERROU AVEC HYSTÉRÉSIS: la cible est un LOCK, pas un recompte à chaque
//     frame. Un candidat doit être 30 % meilleur (score angle×distance) pour
//     voler le lock → fini le flip-flop "stresse en groupe". La cible
//     verrouillée garde en plus un FOV de grâce (1.5×) + 150 ms de grâce
//     quand elle devient momentanément invalide (dormant un frame).
//   · PRÉDICTION: la hitbox visée = hitbox + m_vecVelocity × lead. Le lead
//     = lerp client + ½ tick serveur + âge du snapshot + coût du lissage +
//     réglage utilisateur (slider "prédiction", 0-150 ms). Les strafers se
//     prennent la balle DANS la tête, plus derrière l'oreille.
//   · GAIN ADAPTATIF: le delta est multiplié par un gain qui monte avec
//     l'erreur (1 → 4 au-delà de 15°) et le plafond de counts par envoi passe
//     de 40 (près) à 240 (loin). Acquisition ~4× plus rapide, précision
//     finale inchangée (deadzone 0.012° + carry sub-count).
//   · Tous les algorithmes vivent dans aimcore.rs (PUR, testé hors Windows).
//
// ═══ v0.9.1 — "QUAND LA PERSONNE BOUGE ELLE GAGNE" + VISUELS DE VISÉE ═══
//   · SPEED (slider COMBAT "vitesse de visée", 1-10, défaut 7): facteur
//     multiplicatif sur le gain adaptatif ET le plafond de counts. À 5°
//     d'erreur de strafe, le gain effectif passe de 1.78 (v0.9.0) à
//     5.79 × 1.55 ≈ 9.0 — le tracking suit le strafer au lieu de traîner.
//   · FLICK suivi: div 0.25 → 0.12 (le maintien après le snap corrige la
//     dérive deux fois plus vite).
//   · aim_point / pred_point exposés à l'overlay: la tête visée (brute) et
//     le point PRÉDIT (là où la balle partira) → marqueur tête + ligne de
//     visée + point de lead dessinés à l'écran.
//
// ═══ v0.9.2 — "AVEC SON RETARD JE NE TOUCHE JAMAIS LA TÊTE" ═══════════════
//   · MODÈLE DE MOUVEMENT ADAPTATIF (aimcore::MotionPredictor): filtre alpha-
//     beta-gamma sur la position pieds (vitesse + ACCÉLÉRATION à faible
//     retard, immunisée contre le retard de m_vecVelocity) + mini-modèle
//     linéaire à deux poids entraîné EN LIGNE par gradient sur l'erreur de
//     prédiction. Le retard résiduel (interp + tick + snapshot + lissage) est
//     APPRIS au lieu d'être supposé: la balle va là où la cible SERA, y
//     compris sur un counter-strafe où la vitesse brute pointe l'ancienne
//     trajectoire.
//   · un prédicteur par pawn, tous les joueurs observés à chaque frame; le
//     point visé = hitbox + déplacement du modèle. Poids bornés [0.5, 2.0],
//     accélération clampée, déplacement plafonné 300 u — zéro explosion.
//   · prédiction + visuel `pred_point` utilisent le MÊME point: ce que tu vois
//     en cyan est exactement là où la balle part.
//
// ═══ v0.9.3 — "RECALCULE LES BALLES, LE VISEUR EST AU-DESSUS DE LA TÊTE" ══
//   · RECUL DANS LA VISÉE (aimcore::punch_compensated): pendant une rafale la
//     balle part vers vue + punch (convention modules::PUNCH_SIGN = -1). On
//     vise donc tête − punch → le projectile ARRIVE dans la tête même quand
//     la vue monte sous le recul. Gain réglable 0-1.5 (slider COMBAT).
//   · MODE RAFALE (m_iShotsFired > 0): gain × 1.35, diviseur × 0.55 (plancher
//     0.25), deadzone 0.012° → 0.004°. Chaque balle régénère l'erreur: on la
//     rattrape à la balle SUIVANTE au lieu de laisser le viseur flotter.
//   · le recalage est permanent (pas un one-shot): à chaque frame la visée est
//     recalculée depuis la tête + le punch COURANTS — le viseur se remet tout
//     seul sur la tête dès que la cible ou le recul bougent.
//
// ═══ v0.9.4 — "IL ARRÊTE DE BOUGER, LA CIBLE PART ET IL NE SUIT PAS" ══════
//   · la GATE VUE FRAÎCHE pouvait se transformer en verrou mort: la branche
//     deadzone marquait `last_sent_view` sans avoir rien envoyé → la vue ne
//     changeait plus (aucun input) → la gate attendait à vie. Fix: la deadzone
//     ne marque plus rien, et l'attente est BORNÉE à GATE_MAX_WAIT_S (35 ms).
//   · conséquence: après une convergence, l'aimbot reste réactif et se
//     re-colle sur la cible au frame suivant, même si la vue mémoire gèle.
//
// ═══ v0.9.6 — "IL NE RECALCULE PAS LES TÊTES / IL VISE LE CORPS" ══════════
//   · LA TÊTE EST RECENTRÉE: l'os 7 ("head") est la BASE du crâne — visé tel
//     quel, la balle tombe sur la limite cou/nuque → hitgroup NECK = dégâts
//     POITRINE ("des fois il vise le corps"). Le point de visée tête = os 7
//     + HEAD_Z_NUDGE (2.5 u), jamais sous l'os cou + 4 u (le cou est un
//     hitgroup POITRINE, pas tête).
//   · REPLI DUCK-AWARE: sans os (cache indisponible une frame), l'ancien
//     repli fixe origin+64 SURVOlAIT de ~14 u la tête d'un ennemi accroupi →
//     "il ne se remet pas sur la cible". Le repli lit m_fFlags de CHAQUE
//     ennemi (bit FL_DUCKING, déjà dans le bulk v0.8 — zéro lecture en plus)
//     et suit la posture: 66 u debout / 50 u accroupi.
//   · LE TROU D'HYSTÉRÉSIS DU VERROU EST BOUCHÉ: une cible verrouillée qui
//     devenait invalide UNE frame (dormance qui blipe, os illisibles, bord
//     de FOV de grâce) se faisait VOLER le lock par un autre candidat SANS
//     la marge de 30 % — et l'ancienne cible ne pouvait plus revenir (il lui
//     fallait 30 % + elle se battait contre le FOV de grâce du voleur).
//     Maintenant le lock SURVIT 150 ms d'absence: on VISE le meilleur
//     candidat disponible (jamais de visée figée) mais le lock n'est
//     réattribué qu'après expiration de la grâce — la cible qui revient
//     AVANT reprend sa place instantanément.
//   · GARDE-FOU PUNCH RESSERRÉ (aimcore): 15 % de l'écart tête→poitrine +
//     cap ABSOLU 0.6° — à courte portée l'ancien 30 % autorisait 1.5-3° sous
//     le centre de la tête = le menton/le cou.
//   · GATE TÊTE PHYSIQUE (head_shot_ready): à chaque frame, pour la cible
//     choisie, on calcule l'écart angulaire du TIR RÉEL (vue + punch) vers
//     le centre de la tête prédite, et on le compare au rayon angulaire
//     RÉEL de la hitbox tête à cette distance (aimcore::head_radius_deg).
//     PRÊT ⟺ la balle qui part MAINTENANT atterrit DANS la tête. C'est ce
//     signal que consomme le triggerbot v3: « à chaque tir je touche la tête ».
//
// DÉTECTION: les mouvements SendInput sont statistiquement repérables par le
// VAC (patterns inhumains) et l'Overwatch voit les flicks — assumé par spec.
// Le lissage + un FOV bas + la touche tenue (pas AUTO) réduisent le profil.
// Aucune écriture dans le process du jeu: lecture seule + événements souris.
//
// language: Rust, file: aimbot.rs, target: Windows 11, windows-rs 0.58

use crate::aimcore::{self, PRED_MIN_SPEED};
use crate::entity::{Player, Snapshot};
use crate::math::{Vec3, ViewMatrix, wrap180};
use crate::dlog;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT,
};

// m_yaw Source: 1 count souris = 0.022° de rotation vue
const M_YAW: f32 = 0.022;
// indices d'os (table communautaire — voir entity.rs)
const B_HEAD: usize = 7;
const B_NECK: usize = 6;
const B_CHEST: usize = 23;
const B_SPINE: usize = 4;
const EYE_Z: f32 = 64.0; // hauteur d'œil debout si les os sont indisponibles
const EYE_Z_DUCK: f32 = 46.0; // v0.8.2 — hauteur d'œil ACCROUPI (repli)
const FL_DUCKING: i32 = 2; // v0.8.2 — bit 1 de m_fFlags (Source/CS2)
// v0.9.6 — RECENTRAGE HITBOX TÊTE: l'os 7 est la BASE du crâne (limite
// cou/nuque — le hitgroup NECK prend les dégâts POITRINE). Le centre de la
// hitbox tête est ~2.5 u AU-DESSUS de l'os. Viser os+2.5 u, jamais sous
// cou+4 u: la balle part pour le CRÂNE, pas pour le cou.
const HEAD_Z_NUDGE: f32 = 2.5;

pub struct AimBot {
    /// verrou actif sur une cible (affiché menu + indicateur)
    pub active: bool,
    /// nom de la cible verrouillée
    pub target: String,
    /// distance à la cible (m)
    pub dist_m: f32,
    /// écart angulaire restant au viseur (°)
    pub delta_deg: f32,
    /// v0.8 — MODE: 0 = SMOOTH (convergence lissée), 1 = FLICK (snap tête).
    /// FLICK = rising edge de la touche → delta complet en BURST SendInput
    /// (2 events si gros) puis suivi smooth rapide tant que la touche est
    /// tenue. Pur SendInput: dispo en LEGIT comme en RAGE.
    pub flicks: u32,
    /// v0.8 — dernier état de la touche (détection rising edge)
    last_key: bool,
    /// v0.5 — dernière vue sur laquelle un mouvement a été envoyé.
    /// La vue mémoire suit le FRAMERATE du jeu, pas notre boucle 1 kHz.
    last_sent_view: Option<(f32, f32)>,
    /// v0.9.4 — instant du dernier envoi RÉEL: la gate vue fraîche n'attend
    /// jamais plus de GATE_MAX_WAIT_S (sinon la visée se fige à vie).
    last_send_t: Option<std::time::Instant>,
    /// v0.5 — envois consécutifs SANS amélioration mesurable (diagnostic)
    pub send_rate: f32,
    sends: u32,
    sends_window: Option<std::time::Instant>,
    /// v0.6.1 — RÉSIDU SUB-COUNT: la part fractionnaire des mouvements trop
    /// petits pour un count entier est REPORTÉE au frame suivant au lieu
    /// d'être jetée. Avant: à haute sensibilité le dernier delta (< 0.5 count)
    /// arrondissait à 0 → envoi (0,0) → la visée CAIT juste à côté de la
    /// tête sans jamais terminer. Le résidu s'accumule jusqu'à 1 count.
    carry_x: f32,
    carry_y: f32,
    // ── v0.9 — VERROU DE CIBLE (anti stress de groupe) ──
    /// pawn de la cible verrouillée (identité stable — le nom peut dupliquer)
    lock_pawn: Option<u64>,
    /// instant où la cible verrouillée est devenue indisponible (grace 150 ms)
    lock_lost: Option<std::time::Instant>,
    /// changements de cible cette session (diagnostic "stresse")
    pub switches: u32,
    // ── v0.9.1 — VISUELS DE VISÉE (consommés par l'overlay) ──
    /// hitbox BRUTE de la cible visée (tête par défaut) — monde, pas écran:
    /// l'overlay projette avec la matrice du frame de rendu
    pub aim_point: Option<Vec3>,
    /// point PRÉDIT (hitbox + v·lead) — là où la balle partira réellement.
    /// ≠ aim_point seulement si la cible est mobile (> 40 u/s)
    pub pred_point: Option<Vec3>,
    // ── v0.9.3 — ÉTAT "VISEUR SUR LA TÊTE" (diagnostic live) ──
    /// vrai si l'erreur angulaire résiduelle est DANS la deadzone courante
    /// (0.004° en rafale, 0.012° sinon): le viseur est réellement sur la tête
    pub aim_on_head: bool,
    /// v0.9.5 — compensation de recul RÉELLEMENT appliquée (degrés, après le
    /// garde-fou tête). Sert au diagnostic menu; 0 = visée brute.
    pub punch_deg: f32,
    // ── v0.9.6 — GATE TÊTE PHYSIQUE (couplage triggerbot) ──
    /// écart angulaire du TIR RÉEL (vue + punch) vers le centre de la tête
    /// prédite de la cible choisie (°) — INFINI si pas de cible
    pub head_err_deg: f32,
    /// PRÊT TÊTE: la balle partant maintenant atterrit DANS la hitbox tête
    /// (écart ≤ rayon angulaire réel à cette distance) — signal du triggerbot
    pub head_shot_ready: bool,
    // ── v0.9.2 — MODÈLE DE MOUVEMENT ADAPTATIF (prédiction apprise) ──
    /// trajectoire (ABG + mini-modèle 2 poids) par pawn. Observe TOUS les
    /// joueurs à chaque frame: le modèle apprend même hors verrou.
    motion: std::collections::HashMap<u64, aimcore::MotionPredictor>,
    /// epoch monotone pour l'horloge du modèle (secondes)
    t0: std::time::Instant,
    /// échantillons d'apprentissage cumulés (diagnostic menu)
    pub model_samples: u32,
    /// poids de lead appris, cible verrouillée (diagnostic menu)
    pub model_w_lead: f32,
    /// dernière erreur de prédiction observée (u, diagnostic menu)
    pub model_err_u: f32,
}

/// âge du snapshot au moment de la boucle (s) — alimente le temps d'avance
fn snapshot_age(s: &Snapshot) -> f32 {
    s.taken_at.elapsed().as_secs_f32()
}

/// position caméra depuis la matrice de vue (c = -Rᵀ·t).
/// v0.8.2 — ALGÈBRE CORRECTE + SANITÉ RESSERRÉE (voir en-tête de fichier).
/// La matrice 3×4 est [S·R | t], S = diag(s0, s1, 1) les échelles de
/// projection (normes des lignes 0/1 — s0 ≈ 0.75, s1 ≈ 1.33 en 16:9).
/// Étapes: dés-échelonner R (lignes unitaires) et t, PUIS c = -Rᵀ·t.
/// La v0.6.2 divisait chaque ligne du produit par une seule échelle →
/// termes croisés faux → caméra décalée de 90-290 u à pitch ±45/60° selon
/// le yaw → viseur posé À CÔTÉ de la tête, jamais au centre.
/// Repli (matrice incohérente avec l'origine): origin + hauteur d'œil
/// (64 debout / 46 accroupi). Sanité resserrée: la vraie caméra ne
/// s'écarte de origin+œil que de quelques unités (interpolation tick→
/// frame en mouvement) — ±24 u suffit, l'ancienne fenêtre ±300 u
/// laissait passer les caméras fausses silencieusement.
pub(crate) fn cam_pos(m: &ViewMatrix, fallback: &Vec3, ducking: bool) -> Vec3 {
    let mm = &m.m;
    // échelles de projection = normes des lignes 0/1 (ligne 2 unitaire)
    let s0 = (mm[0] * mm[0] + mm[1] * mm[1] + mm[2] * mm[2]).sqrt().max(1e-6);
    let s1 = (mm[4] * mm[4] + mm[5] * mm[5] + mm[6] * mm[6]).sqrt().max(1e-6);
    // rotation dés-échelonnée (lignes unitaires) + translation dés-échelonnée
    let r = [
        [mm[0] / s0, mm[1] / s0, mm[2] / s0],
        [mm[4] / s1, mm[5] / s1, mm[6] / s1],
        [mm[8], mm[9], mm[10]],
    ];
    let t = [mm[3] / s0, mm[7] / s1, mm[11]];
    // c = -Rᵀ·t : composante i = -( r0[i]·t0 + r1[i]·t1 + r2[i]·t2 )
    let c = Vec3 {
        x: -(r[0][0] * t[0] + r[1][0] * t[1] + r[2][0] * t[2]),
        y: -(r[0][1] * t[0] + r[1][1] * t[1] + r[2][1] * t[2]),
        z: -(r[0][2] * t[0] + r[1][2] * t[1] + r[2][2] * t[2]),
    };
    let eye_z = if ducking { EYE_Z_DUCK } else { EYE_Z };
    // origin exactement (0,0,0) = lecture ratée du node → la comparaison
    // n'a pas de sens, on fait CONFIANCE à la matrice (déjà validée par
    // ViewMatrix::sane()) au lieu de viser depuis le (0,0) du monde.
    let origin_unread = fallback.x == 0.0 && fallback.y == 0.0 && fallback.z == 0.0;
    let sane = origin_unread
        || ((c.x - fallback.x).abs() < 24.0
            && (c.y - fallback.y).abs() < 24.0
            && (fallback.z + eye_z - 20.0..fallback.z + eye_z + 20.0).contains(&c.z));
    if sane { c } else { fallback.z_up(eye_z) }
}

/// angles (pitch, yaw) en degrés pour viser `to` depuis `from`
/// (convention Source: pitch + = vers le bas, yaw 0 = +X, atan2(y, x))
pub(crate) fn angles_to(from: Vec3, to: Vec3) -> (f32, f32) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let dz = to.z - from.z;
    let horiz = (dx * dx + dy * dy).sqrt();
    (-dz.atan2(horiz).to_degrees(), dy.atan2(dx).to_degrees())
}

/// point à viser selon la hitbox choisie (repli hauteur d'œil sans os).
/// v0.9.6 — LA TÊTE EST RECENTRÉE + REPLI DUCK-AWARE:
///   · l'os 7 ("head") est la BASE du crâne, pas le centre de la hitbox:
///     visé tel quel, la balle tombe sur la limite cou/nuque → hitgroup NECK
///     = dégâts POITRINE ("des fois il vise le corps"). On vise l'os +
///     HEAD_Z_NUDGE (2.5 u), jamais sous l'os cou + 4 u.
///   · sans os (cache indisponible une frame), le repli suit la POSTURE de
///     l'ennemi (m_fFlags FL_DUCKING): 66 u debout / 50 u accroupi —
///     l'ancien repli fixe 64 u survolait la tête d'un accroupi de ~14 u.
pub(crate) fn hitbox_pos(
    bones: &Option<Vec<Vec3>>,
    hitbox: usize,
    origin: &Vec3,
    ducking: bool,
) -> Vec3 {
    // (index d'os, hauteur repli debout, hauteur repli accroupi)
    let (bi, z_stand, z_duck) = match hitbox {
        0 => (B_HEAD, 66.0, 50.0),  // tête — centre de la hitbox
        1 => (B_NECK, 60.0, 47.0), // cou
        2 => (B_CHEST, 52.0, 42.0), // poitrine
        _ => (B_SPINE, 48.0, 38.0), // corps (centre de masse)
    };
    let z_fallback = if ducking { z_duck } else { z_stand };
    let mut p = bones
        .as_ref()
        .and_then(|b| b.get(bi).cloned())
        .unwrap_or_else(|| origin.z_up(z_fallback));
    if hitbox == 0 {
        if let Some(b) = bones.as_ref() {
            // v0.9.6 — recentrage hitbox tête: os + nudge, jamais sous le
            // cou + 4 u (le cou est un hitgroup POITRINE), jamais plus de
            // 7 u au-dessus de la base (sanité: un vrai crâne).
            if let Some(head) = b.get(B_HEAD) {
                p.z = (p.z + HEAD_Z_NUDGE).min(head.z + 7.0);
            }
            if let Some(neck) = b.get(B_NECK) {
                p.z = p.z.max(neck.z + 4.0);
            }
        }
    }
    p
}

/// mouvement souris relatif SYNTHÉTIQUE (SendInput) — même canal que le
/// triggerbot pour les clics. Le jeu avec raw input consomme ces counts.
fn send_move(dx: i32, dy: i32) {
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

/// v0.8 — FLICK: burst de 2 événements dans UN SEUL SendInput. Les deltas
/// massifs d'un coup passent mal dans le raw input (vitesse plafonnée par
/// event) — deux mouvements consécutifs dans le même appel sont consommés
/// proprement par le jeu: la vue SAUTE à la cible en un frame.
fn send_move2(dx1: i32, dy1: i32, dx2: i32, dy2: i32) {
    unsafe {
        let mk = |dx: i32, dy: i32| {
            let mut i = INPUT::default();
            i.r#type = INPUT_MOUSE;
            i.Anonymous = INPUT_0 {
                mi: MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: MOUSEEVENTF_MOVE, time: 0, dwExtraInfo: 0 },
            };
            i
        };
        SendInput(&[mk(dx1, dy1), mk(dx2, dy2)], std::mem::size_of::<INPUT>() as i32);
    }
}

impl AimBot {
    pub fn new() -> Self {
        AimBot {
            active: false,
            target: String::new(),
            dist_m: 0.0,
            delta_deg: f32::INFINITY,
            flicks: 0,
            last_key: false,
            last_sent_view: None,
            last_send_t: None,
            send_rate: 0.0,
            sends: 0,
            sends_window: None,
            carry_x: 0.0,
            carry_y: 0.0,
            lock_pawn: None,
            lock_lost: None,
            switches: 0,
            aim_point: None,
            pred_point: None,
            aim_on_head: false,
            punch_deg: 0.0,
            head_err_deg: f32::INFINITY,
            head_shot_ready: false,
            motion: std::collections::HashMap::new(),
            t0: std::time::Instant::now(),
            model_samples: 0,
            model_w_lead: 1.0,
            model_err_u: 0.0,
        }
    }

    /// appelé à chaque itération de la boucle overlay (~1 kHz). Le snapshot
    /// est rafraîchi au même rythme → la vue dwViewAngles suit le frame de
    /// rendu en temps réel, et la GATE n'envoie que sur vue fraîche.
    /// v0.9 — pred_ms: prédiction additionnelle (ms) · lock_on: verrouillage
    /// de cible avec hystérésis (see aimcore.rs).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        snap: Option<&Snapshot>,
        enabled: bool,
        key_down: bool,
        hitbox: usize,
        fov_deg: f32,
        smooth: f32,
        sens_manual: f32,
        sens_auto: Option<f32>,
        ctx_ok: bool, // jeu au premier plan, pas de menu, vivant
        mode: usize,  // v0.8 — 0 smooth · 1 flick
        pred_ms: f32, // v0.9 — prédiction utilisateur (ms)
        lock_on: bool, // v0.9 — verrouillage cible (hystérésis)
        speed: f32,   // v0.9.1 — slider vitesse de visée (1-10, défaut 7)
        punch_gain: f32, // v0.9.3 — compensation du recul dans la visée (0-1.5)
    ) {
        let rising = key_down && !self.last_key; // front montant de la touche
        self.last_key = key_down;
        self.active = false;
        self.delta_deg = f32::INFINITY;
        self.aim_on_head = false; // v0.9.3 — recalculé à chaque frame
        self.punch_deg = 0.0; // v0.9.5 — idem
        self.head_err_deg = f32::INFINITY; // v0.9.6 — idem
        self.head_shot_ready = false; // v0.9.6 — idem
        // v0.6.2 — PLUS de reset du carry ici: il était remis à zéro À CHAQUE
        // frame, tuant l'accumulation sub-count avant qu'elle n'existe. Les
        // resets vivent désormais dans les sorties inactives ci-dessous.
        if !enabled || !key_down || !ctx_ok {
            self.target.clear();
            self.last_sent_view = None; // reset: la prochaine activation repart propre
            self.last_send_t = None;
            self.carry_x = 0.0;
            self.carry_y = 0.0;
            self.lock_pawn = None; // v0.9 — hors combat: le lock meurt
            self.lock_lost = None;
            self.aim_point = None; // v0.9.1 — plus de cible → plus de marqueur
            self.pred_point = None;
            self.head_shot_ready = false; // v0.9.6
            // v0.9.2 — hors combat: les trajectoires gèlent (les poids appris
            // survivent au reset_state)
            for m in self.motion.values_mut() {
                m.reset_state();
            }
            return;
        }
        let Some(s) = snap else {
            self.target.clear();
            self.carry_x = 0.0;
            self.carry_y = 0.0;
            self.lock_lost = None;
            self.aim_point = None;
            self.pred_point = None;
            self.head_shot_ready = false; // v0.9.6
            return;
        };
        if !s.matrix_ok || !s.local_alive || !s.local_view_ok {
            self.target.clear();
            self.last_sent_view = None; // v0.6.1 — idem: contexte mort → gate propre
            self.last_send_t = None;
            self.carry_x = 0.0;
            self.carry_y = 0.0;
            self.lock_pawn = None;
            self.lock_lost = None;
            self.aim_point = None;
            self.pred_point = None;
            self.head_shot_ready = false; // v0.9.6
            return;
        }

        // v0.5 — FIX 3: sensibilité effective = valeur LUE dans le jeu si
        // dispo (dwSensitivity), sinon la valeur manuelle du menu.
        let sens = sens_auto.filter(|v| (0.01..=10.0).contains(v)).unwrap_or(sens_manual).max(0.05);

        // v0.5 — FIX 1: vue LIVE (dwViewAngles, fallback m_angEyeAngles)
        let view = s.local_view;
        // v0.8.2 — repli caméra accordéon (FL_DUCKING) + algèbre corrigée
        let ducking = s.local_flags & FL_DUCKING != 0;
        let cam = cam_pos(&s.matrix, &s.local_origin, ducking);

        // ── v0.9 — SÉLECTION: score + verrou + prédiction ──
        // v0.9.2 — le lead vient du MODÈLE ADAPTATIF (aimcore): filtre alpha-
        // beta-gamma sur la position pieds (vitesse + ACCÉLÉRATION à faible
        // retard) + mini-modèle à deux poids entraîné en ligne. Il corrige le
        // retard de m_vecVelocity sur les changements de direction ET le virage
        // de la cible pendant le temps de vol.
        let age = snapshot_age(s);
        let lead = aimcore::lead_time_s(age, smooth, pred_ms);
        let grace_fov = aimcore::grace_fov_deg(fov_deg);
        let now = self.t0.elapsed().as_secs_f32();
        // v0.9.3 — rafale en cours ? (m_iShotsFired > 0) → visée plus sèche
        let shots = s.shots_fired.max(0);

        // meilleur candidat ET cible verrouillée courante, en un seul passage:
        // (score, player, dyaw, dpitch, ang, point_prédit)
        let mut best: Option<(f32, &Player, f32, f32, f32, Vec3)> = None;
        let mut locked: Option<(f32, &Player, f32, f32, f32, Vec3)> = None;
        for p in &s.players {
            // v0.7.1 — p.dormant: un joueur hors PVS garde une position PÉRIMÉE en
            // mémoire (le serveur ne met plus son origin/os à jour). Sans ce filtre
            // l'aimbot verrouillait sur des ennemis qui n'étaient PLUS là → la
            // caméra collait un coin vide = "vise des cibles inexistantes".
            if !p.alive || p.dormant || (p.team != 2 && p.team != 3) || p.team == s.local_team || p.pawn == s.local_pawn {
                continue; // ennemis vivants et NON dormants uniquement
            }
            let hitbox_raw = hitbox_pos(&p.bones, hitbox, &p.origin, p.ducking);
            let feet = [p.origin.x, p.origin.y, p.origin.z];
            // observe TOUS les joueurs: le modèle apprend même hors verrou
            let mp = self
                .motion
                .entry(p.pawn)
                .or_insert_with(aimcore::MotionPredictor::new);
            mp.observe(
                feet,
                [p.velocity.x, p.velocity.y, p.velocity.z],
                now,
            );
            // v0.9.2 — mobile si la vitesse BRUTE ou la vitesse FILTRÉE dépasse
            // le seuil: le filtre rattrape une m_vecVelocity périmée en virage.
            let pos = if p.velocity.len() > PRED_MIN_SPEED || mp.speed() > PRED_MIN_SPEED {
                let d = mp.predict_delta(feet, lead);
                Vec3 {
                    x: hitbox_raw.x + d[0],
                    y: hitbox_raw.y + d[1],
                    z: hitbox_raw.z + d[2],
                }
            } else {
                hitbox_raw
            };
            // v0.9.3 — RECUL BALISTIQUE DANS LA VISÉE: la balle part vers
            // vue + punch; pour qu'elle arrive dans la tête on vise tête −
            // punch (voir aimcore::punch_compensated). C'est le "viseur au-
            // dessus de la tête avec le recul" corrigé à la source.
            let (tp0, ty0) = angles_to(cam, pos);
            let (tp_c, ty) =
                aimcore::punch_compensated(tp0, ty0, s.local_punch, punch_gain);
            // v0.9.5 — GARDE-FOU "JAMAIS LA POITRINE": la compensation de
            // recul ne peut pas éloigner la visée du crâne de plus de 30 % de
            // l'écart angulaire tête→poitrine. Punch gros, cible proche ou
            // convention inverse: le viseur reste sur la tête. (Uniquement
            // quand la hitbox choisie EST la tête — sinon on respecte le choix.)
            let tp = if hitbox == 0 {
                let chest = hitbox_pos(&p.bones, 2, &p.origin, p.ducking);
                let (cp0, _) = angles_to(cam, chest);
                let g = aimcore::head_guard(tp0, cp0, tp_c);
                if g.is_finite() { g } else { tp0 }
            } else {
                tp_c
            };
            let dyaw = wrap180(ty - view.1);
            let dpitch = tp - view.0;
            let ang = dyaw.hypot(dpitch);
            let is_locked = lock_on && self.lock_pawn == Some(p.pawn);
            // FOV: plein pour tous, zone de grâce pour la cible verrouillée
            // (un strafe qui frôle le bord du FOV ne coupe pas la visée).
            let lim = if is_locked { grace_fov } else { fov_deg };
            if ang > lim {
                // hors FOV ET hors grâce — la cible verrouillée passe en grace
                // timer, les autres sont juste ignorées
                continue;
            }
            let dist_m = p.origin.dist(&s.local_origin) * 0.019;
            let sc = aimcore::score(ang, dist_m);
            if best.map(|(a, ..)| sc < a).unwrap_or(true) {
                best = Some((sc, p, dyaw, dpitch, ang, pos));
            }
            if is_locked {
                locked = Some((sc, p, dyaw, dpitch, ang, pos));
            }
        }

        // purge des trajectoires des joueurs absents (mémoire bornée)
        self.motion
            .retain(|pawn, _| s.players.iter().any(|p| p.pawn == *pawn));

        // ── v0.9 — RÉSOLUTION DU VERROU (hystérésis) ──
        // v0.9.6 — LE TROU D'HYSTÉRÉSIS EST BOUCHÉ: avant, une cible
        // verrouillée qui devenait invalide UNE frame (dormance qui blipe,
        // os illisibles, bord du FOV de grâce) tombait dans la branche
        // (aucun lock, un candidat) → le lock était VOLÉ sans la marge de
        // 30 %, et l'ancienne cible ne pouvait plus revenir (il lui fallait
        // 30 % + elle se battait contre le FOV de grâce du voleur).
        // Maintenant le lock SURVIT 150 ms d'absence: on VISE le meilleur
        // candidat disponible (jamais de visée figée), mais le lock n'est
        // réattribué qu'après EXPIRATION de la grâce — la cible qui revient
        // avant reprend sa place instantanément.
        let chosen: (f32, &Player, f32, f32, f32, Vec3) = match (locked, best) {
            (Some(l), Some(b)) => {
                if aimcore::should_switch(l.0, b.0) {
                    self.switches += 1;
                    self.lock_pawn = Some(b.1.pawn);
                    self.lock_lost = None;
                    b
                } else {
                    l
                }
            }
            (Some(l), None) => l,
            (None, Some(b)) => {
                if lock_on && self.lock_pawn.is_some() {
                    // la cible verrouillée est absente CE frame: grâce ou mort
                    if let Some(t) = self.lock_lost {
                        if t.elapsed().as_secs_f32() > aimcore::GRACE_S {
                            // grâce expirée: le lock est mort, on le réattribue
                            self.switches += 1;
                            self.lock_pawn = Some(b.1.pawn);
                            self.lock_lost = None;
                        }
                        // sinon: grâce EN COURS — on vise b, le lock attend
                        // le retour de sa cible (≤ 150 ms)
                    } else {
                        // première frame d'absence: le chrono démarre
                        self.lock_lost = Some(std::time::Instant::now());
                    }
                } else if lock_on {
                    if self.lock_pawn.is_some() && self.lock_pawn != Some(b.1.pawn) {
                        self.switches += 1;
                    }
                    self.lock_pawn = Some(b.1.pawn);
                    self.lock_lost = None;
                }
                b
            }
            (None, None) => {
                // plus AUCUNE cible valide: la grace tourne encore 150 ms pour
                // ne pas reset le lock sur un frame de dormance — mais on ne
                // peut pas viser sur des données absentes → sortie propre.
                if let Some(t) = self.lock_lost {
                    if t.elapsed().as_secs_f32() > aimcore::GRACE_S {
                        self.lock_pawn = None;
                        self.lock_lost = None;
                    }
                } else {
                    self.lock_lost = Some(std::time::Instant::now());
                }
                self.target.clear();
                self.carry_x = 0.0;
                self.carry_y = 0.0;
                self.aim_point = None;
                self.pred_point = None;
                self.head_shot_ready = false; // v0.9.6
                self.head_err_deg = f32::INFINITY; // v0.9.6
                return;
            }
        };
        // cible verrouillée VALIDE ce frame → le chrono de grâce se réarme
        if locked.is_some() {
            self.lock_lost = None;
        }
        let (ang, p, dyaw, dpitch) = (chosen.4, chosen.1, chosen.2, chosen.3);

        self.active = true;
        // v0.6.1 — changement de cible → résidu sub-count remis à zéro
        // (le résidu appartient à l'angle vers l'ANCIENNE cible)
        if self.target != p.name {
            self.carry_x = 0.0;
            self.carry_y = 0.0;
        }
        self.target = p.name.clone();
        self.dist_m = p.origin.dist(&s.local_origin) * 0.019;
        self.delta_deg = ang;
        // v0.9.3 — le viseur est-il DANS la tête ? (même deadzone que la boucle)
        self.aim_on_head = aimcore::on_target(ang, shots);

        // ── v0.9.1 — VISUELS: tête visée + point prédit exposés à l'overlay.
        // v0.9.2 — le point prédit est celui DU MODÈLE (chosen.5), plus le
        // v·lead linéaire: l'overlay dessine exactement là où la balle part.
        let raw_pos = hitbox_pos(&p.bones, hitbox, &p.origin, p.ducking);
        self.aim_point = Some(raw_pos);
        self.pred_point = Some(chosen.5);

        // ── v0.9.6 — GATE TÊTE PHYSIQUE (consommé par le triggerbot v3) ──
        // La balle part le long de (vue + punch); elle touche la tête ssi
        // l'angle du tir est à moins du RAYON ANGULAIRE RÉEL de la hitbox
        // tête de la cible choisie, à cette distance. Calculé sur la TÊTE
        // (pas la hitbox du menu): le trigger sait TOUJOURS si CE tir
        // irait dans la tête — la même trajectoire prédite que le point
        // visé est décalée du même delta sur la tête brute.
        {
            let head_raw = hitbox_pos(&p.bones, 0, &p.origin, p.ducking);
            let d_pred = Vec3 {
                x: chosen.5.x - raw_pos.x,
                y: chosen.5.y - raw_pos.y,
                z: chosen.5.z - raw_pos.z,
            };
            let head_pt = Vec3 {
                x: head_raw.x + d_pred.x,
                y: head_raw.y + d_pred.y,
                z: head_raw.z + d_pred.z,
            };
            let (hp0, hy0) = angles_to(cam, head_pt);
            let (pp, py) = s.local_punch.unwrap_or((0.0, 0.0));
            let b_yaw = wrap180(hy0 - (view.1 + py));
            let b_pitch = hp0 - (view.0 + pp);
            self.head_err_deg = b_yaw.hypot(b_pitch);
            self.head_shot_ready =
                aimcore::shot_lands_in_head(self.head_err_deg, self.dist_m);
        }

        // diagnostics du modèle appris (ligne "prédiction" du menu)
        if let Some(mp) = self.motion.get(&p.pawn) {
            self.model_samples = mp.samples;
            self.model_w_lead = mp.w_lead;
            self.model_err_u = mp.last_err_u;
        }

        // v0.9.5 — recul réellement appliqué APRÈS le garde-fou tête: le menu
        // affiche la valeur effective, pas le punch brut lu en mémoire (sinon
        // "punch 8°" alors qu'on n'en applique que 0.6°).
        if hitbox == 0 {
            let head = hitbox_pos(&p.bones, 0, &p.origin, p.ducking);
            let chest = hitbox_pos(&p.bones, 2, &p.origin, p.ducking);
            let (hp, _) = angles_to(cam, head);
            let (cp, _) = angles_to(cam, chest);
            let (ap, _) = aimcore::punch_compensated(hp, 0.0, s.local_punch, punch_gain);
            let g = aimcore::head_guard(hp, cp, ap);
            self.punch_deg = if g.is_finite() { (g - hp).abs() } else { 0.0 };
        }

        // ═══ v0.8 — MODE FLICK: rising edge → SNAP COMPLET en burst ═════════
        // Le delta ENTIER part en UN appel (2 events si gros delta) — la
        // souris fait un mouvement très rapide vers la cible, SANS écriture
        // mémoire. Ensuite la touche tenue suit la cible en smooth rapide
        // (div réduit — les corrections restent vives). Pur SendInput:
        // fonctionne en LEGIT comme en RAGE, zéro surface mémoire ajoutée.
        if mode == 1 && rising {
            let dpc = M_YAW * sens;
            let fx = (-dyaw / dpc).clamp(-400.0, 400.0).round() as i32;
            let fy = (dpitch / dpc).clamp(-400.0, 400.0).round() as i32;
            if fx != 0 || fy != 0 {
                if fx.abs() > 200 || fy.abs() > 200 {
                    // gros delta → burst de 2 events (le raw input plafonne
                    // la vitesse PAR event — deux mi-passent, un seul non)
                    send_move2(fx / 2, fy / 2, fx - fx / 2, fy - fy / 2);
                } else {
                    send_move(fx, fy);
                }
                self.flicks += 1;
                self.last_sent_view = Some(view); // le flick compte comme envoi
                self.last_send_t = Some(std::time::Instant::now());
                self.carry_x = 0.0;
                self.carry_y = 0.0;
                dlog!("[flk ] flick → {} ({:+.1}°, {:+.1}°) · {} counts", p.name, dyaw, dpitch, fx.abs() + fy.abs());
                self.sends += 1;
                let now = std::time::Instant::now();
                match self.sends_window {
                    Some(t) if now.duration_since(t).as_secs_f32() >= 1.0 => {
                        self.send_rate = self.sends as f32 / now.duration_since(t).as_secs_f32();
                        self.sends = 0;
                        self.sends_window = Some(now);
                    }
                    None => self.sends_window = Some(now),
                    _ => {}
                }
            }
            return; // le flick est parti — la vue se rafraîchit au frame jeu
        }

        // ── v0.5 — FIX 2: GATE VUE FRAÎCHE ──
        // la vue mémoire est rafraîchie au FRAMERATE du jeu; si elle n'a pas
        // changé depuis notre dernier envoi, tout nouveau mouvement serait
        // calculé sur des données périmées → on ATTEND la nouvelle frame.
        //
        // v0.9.4 — ANTI-BLOCAGE: cette attente est BORNÉE (GATE_MAX_WAIT_S).
        // Bug constaté en live: après une convergence (plus aucun envoi), la
        // vue ne change plus — la gate attendait donc POUR TOUJOURS. La cible
        // partait, l'aimbot restait figé jusqu'à ce que le joueur bouge sa
        // souris lui-même. Maintenant, après ~2 frames sans rafraîchissement,
        // on renvoie quand même: le pas est déjà borné par axe à l'erreur
        // restante, donc impossible de dépasser la tête.
        if let Some((lp, ly)) = self.last_sent_view {
            let same_view = (lp - view.0).abs() < 1e-4 && (ly - view.1).abs() < 1e-4;
            let since = self
                .last_send_t
                .map(|t| t.elapsed().as_secs_f32())
                .unwrap_or(f32::INFINITY);
            if aimcore::gate_should_wait(same_view, since) {
                return; // vue pas encore rafraîchie par le jeu — ne pas doubler
            }
        }

        // deadzone: déjà aligné → ne pas micro-trembler autour de la cible.
        // v0.8.2 — 0.05° → 0.012°: 0.05° à 40 m = 3.7 u ≈ le RAYON d'une
        // tête — l'aim s'arrêtait à mi-chemin du bord ("sur le côté de la
        // tête" à distance). Le carry sub-count + la gate vue fraîche
        // terminent la convergence sous le count entier.
        // v0.9.3 — en rafale (m_iShotsFired > 0) la deadzone tombe à 0.004°:
        // le recul régénère l'erreur à chaque balle → on reste DANS la tête au
        // lieu de laisser le viseur "flotter" juste à côté.
        let dz = aimcore::deadzone_deg(shots);
        if dyaw.abs() < dz && dpitch.abs() < dz {
            // v0.9.4 — NE PAS marquer la vue comme traitée ici: rien n'a été
            // envoyé. C'était LA cause du gel définitif (vue identique à
            // elle-même → gate fermée à vie) dès qu'on atteignait la tête.
            self.carry_x = 0.0; // aligné: le résidu restant est du bruit
            self.carry_y = 0.0;
            return;
        }

        // ── degrés → counts souris (m_yaw × sensibilité) + lissage ──
        // lissage = diviseur du delta: 0 = snap instant (le plus cramable),
        // 20 = déplacement très progressif (le plus humain).
        // v0.8 — MODE FLICK, touche tenue APRÈS le snap: suivi VIF (div
        // réduit) — le flick initial a fait le gros du trajet, les frames
        // suivantes corrigent la dérive de la cible sans traîner.
        // v0.6.1 — RÉSIDU SUB-COUNT: want arrondi, la fraction est reportée.
        // Si l'arrondi donne (0,0) on ne marque PAS la vue comme traitée:
        // la frame suivante ré-essaie, le résidu grossit jusqu'à 1 count,
        // le dernier degré CONVERGE (avant: visée figée à côté de la tête).
        // v0.6.2 — FIX 4 SIGNES (la source du "bord du cercle"):
        //   convention Source: dx > 0 = souris à droite = yaw DÉCROÎT,
        //   dy > 0 = souris vers le bas = pitch CROÎT (+ = vers le bas).
        //   Pour ATTEINDRE (dyaw, dpitch): dx = -dyaw/…, dy = +dpitch/…
        //   (même convention que le RCS, qui compense correctement en live).
        //   L'ancien code montait dx = +dyaw → la vue s'éloignait de la cible
        //   en horizontal jusqu'au bord du FOV → équilibre "sur le cercle".
        // v0.9 — GAIN ADAPTATIF + PLAFOND DYNAMIQUE (aimcore.rs):
        //   gain 1→8 selon l'erreur (rampe 8°), plafond 60→400 counts/envoi.
        //   v0.9.1 — × SPEED FACTOR (slider "vitesse"): à erreur de strafe
        //   (5°) le gain effectif v0.9.0 était 1.78 → désormais 5.79 × 1.55
        //   ≈ 9.0 — le tracking gagne contre une cible qui bouge. Le plafond
        //   effectif est borné à 500 (le raw input digère ±400/event, le
        //   flick burst 2-events prouve la plage).
        //   La précision finale ne change pas (deadzone + carry inchangés).
        let deg_per_count = M_YAW * sens;
        let spd = aimcore::speed_factor(speed);
        let div = if mode == 1 {
            1.0 + smooth.max(0.0) * 0.12 // v0.9.1 — FLICK suivi: VIF (0.25 → 0.12)
        } else {
            1.0 + smooth.max(0.0) // SMOOTH: le réglage plein
        };
        // v0.9.3 — MODE RAFALE: gain renforcé + diviseur réduit (≥ 0.25) → la
        // correction de recul se rattrape à la balle suivante, pas à la fin
        // de la rafale.
        let div = (div * aimcore::spray_div_scale(shots)).max(0.25);
        let err = dyaw.abs().hypot(dpitch.abs()); // écart angulaire combiné
        let gain = aimcore::adaptive_gain(err) * spd * aimcore::spray_gain(shots);
        let cap = (aimcore::count_ceiling(err) * spd).min(500.0);
        // v0.9.1 — CLAMP À L'ERREUR, PAR AXE: le pas sur un axe ne dépasse
        // jamais l'erreur restante de CET axe — gain élevé + smooth bas =
        // snap direct, mais zéro ringing (le plafond seul autorisait un
        // dépassement ~10%/frame sur les acquisitions → oscillation amortie
        // visible à l'écran). La convergence finale reste pilotée par
        // gain<div (progressif) puis deadzone + carry sub-count.
        let ceil_x = cap.min(dyaw.abs() / deg_per_count);
        let ceil_y = cap.min(dpitch.abs() / deg_per_count);
        let want_x = (-dyaw * gain / deg_per_count / div).clamp(-ceil_x, ceil_x);
        let want_y = (dpitch * gain / deg_per_count / div).clamp(-ceil_y, ceil_y);
        self.carry_x = (self.carry_x + want_x).clamp(-cap * 2.0, cap * 2.0);
        self.carry_y = (self.carry_y + want_y).clamp(-cap * 2.0, cap * 2.0);
        let sx = self.carry_x.round();
        let sy = self.carry_y.round();
        if sx != 0.0 || sy != 0.0 {
            self.carry_x -= sx; // la part envoyée quitte le résidu
            self.carry_y -= sy;
            send_move(sx as i32, sy as i32);
            self.last_sent_view = Some(view); // envoi réel → gate vue fraîche
            self.last_send_t = Some(std::time::Instant::now());

            // diagnostic: taux d'envoi effectif (≈ framerate du jeu attendu)
            self.sends += 1;
            let now = std::time::Instant::now();
            match self.sends_window {
                Some(t) if now.duration_since(t).as_secs_f32() >= 1.0 => {
                    self.send_rate = self.sends as f32 / now.duration_since(t).as_secs_f32();
                    self.sends = 0;
                    self.sends_window = Some(now);
                }
                None => self.sends_window = Some(now),
                _ => {}
            }
        }
    }
}
