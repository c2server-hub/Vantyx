// aimcore — ALGORITHMES DE VISÉE v0.9 · Vantyx.cc
//
// MODULE PUR: zéro import windows, zéro accès mémoire — uniquement des
// fonctions numériques. C'est la boîte noire testable de l'aimbot: le harnais
// de test host (scripts/test_aimcore.rs) exécute EXACTEMENT ce fichier sur
// n'importe quelle machine, Windows ou pas.
//
// LES TROIS BUGS QUE CETTE VERSION RÈGLE (retours de dj):
//   1. "il stresse quand y a un groupe de personnes" → HYSTÉRÉSIS de
//      verrouillage: la cible ne change que si le candidat est franchement
//      meilleur (30 %). Avant: deux ennemis angulairement proches se
//      volaient le lock à CHAQUE frame → tremblement/flip-flop.
//   2. "quand un perso bouge l'aimbot est trop lent et ne tire pas dans la
//      tête" → PRÉDICTION: on vise hitbox + v·lead (lerp client + demi-tick
//      serveur + âge du snapshot + coût du lissage). La balle part vers là
//      où l'ennemi SERA, pas où il était il y a 40 ms.
//   3. "toujours plus précis" → GAIN ADAPTATIF + plafond dynamique: près de
//      la cible le gain tombe à 1 (convergence fine, carry sub-count, dead-
//      zone 0.012°) ; loin il monte à 4 avec un plafond de counts qui passe
//      de 40 à 240 — l'acquisition des grosses erreurs est ~4× plus rapide
//      sans rien perdre en précision finale.
//
// language: Rust, file: aimcore.rs, target: toute plateforme (pure)

// ── constantes réseau CS2 ───────────────────────────────────────────────────
/// tick serveur (64 tick)
pub const TICK_S: f32 = 1.0 / 64.0;
/// interpolation client standard: 2 ticks de buffer
pub const LERP_S: f32 = 2.0 * TICK_S;
/// vitesse (u/s) au-dessus de laquelle on considère la cible MOBILE
/// (en dessous: prédire n'apporte que du bruit — strafe stop, micro-jitter)
pub const PRED_MIN_SPEED: f32 = 40.0;

// ── 1. score de cible + hystérésis (anti "stresse en groupe") ──────────────

/// score d'une cible — PLUS PETIT = MIEUX.
/// Composition: écart angulaire (°) dominant + distance pondérée à 15 %
/// (à angle égal, un ennemi à 10 m est un chouïa plus intéressant qu'à 90 m:
/// sa hitbox angulaire est plus grosse → hit ratio plus haut).
pub fn score(ang_deg: f32, dist_m: f32) -> f32 {
    let d = (dist_m / 100.0).clamp(0.0, 1.0);
    ang_deg * (1.0 + 0.15 * d)
}

/// marge d'hystérésis: le candidat doit être 30 % meilleur pour VOLER le lock.
/// 0.30 = assez raide pour tuer le flip-flop entre deux cibles serrées,
/// assez souple pour re-prioriser quand une tête franchement plus proche
/// apparaît (rush dans le FOV).
pub const SWITCH_MARGIN: f32 = 0.30;

/// faut-il abandonner la cible verrouillée pour le candidat ?
pub fn should_switch(current_score: f32, candidate_score: f32) -> bool {
    candidate_score < current_score * (1.0 - SWITCH_MARGIN)
}

/// FOV de grâce: une cible VERROUILLÉE qui sort du FOV garde son lock dans
/// une zone élargie 1.5× — un strafe qui frôle le bord ne coupe pas la visée.
pub fn grace_fov_deg(fov_deg: f32) -> f32 {
    fov_deg * 1.5
}

/// durée de grâce quand la cible devient momentanément invalide (dormant un
/// frame, os illisibles): 150 ms — un peek/strafe ne reset pas le lock.
pub const GRACE_S: f32 = 0.15;

// ── 2. prédiction (cibles mobiles) ──────────────────────────────────────────

/// temps d'avance total (secondes) à appliquer à la position de la hitbox:
///   lerp client (2 ticks)  — le rendu/simulation traîne de 2 ticks
///   + ½ tick serveur       — le usercmd est snappé au tick supérieur
///   + âge du snapshot      — nos données ont déjà ce retard
///   + coût du lissage      — chaque cran de smooth ≈ 2 ms de retard
///   + réglage utilisateur  — prédiction additionnelle (slider, ms)
/// Tout est borné: la prédiction d'un externe ne doit JAMAIS dépasser ~250 ms
/// (au-delà on vise un fantôme quand l'ennemi inverse son strafe).
/// (borne dure appliquée en fin de fonction)
pub fn lead_time_s(snapshot_age_s: f32, smooth: f32, cfg_ms: f32) -> f32 {
    let base = LERP_S
        + TICK_S * 0.5
        + snapshot_age_s.clamp(0.0, 0.05)
        + smooth.clamp(0.0, 20.0) * 0.002;
    (base + cfg_ms.max(0.0) / 1000.0).min(0.25)
}

/// point prédit = position + vitesse × temps d'avance.
/// tuples [x, y, z] — pas de dépendance au type Vec3 du crate.
/// v0.9.2 — conservé comme primitive/test; l'aimbot passe par MotionPredictor.
#[allow(dead_code)]
pub fn predicted(pos: [f32; 3], vel: [f32; 3], lead_s: f32) -> [f32; 3] {
    [
        pos[0] + vel[0] * lead_s,
        pos[1] + vel[1] * lead_s,
        pos[2] + vel[2] * lead_s,
    ]
}

// ── 3. gain adaptatif + plafond dynamique ("plus rapide ET plus précis") ───
//
// v0.9.1 — RÉGLAGE "IL EST LENT, LA CIBLE BOUGE ET GAGNE":
//   · rampe err/15° → err/8°: une erreur de STRAFE (2-6°) prend enfin un
//     vrai gain (avant: gain(5°)=1.78 — trop mou pour suivre un strafer;
//     maintenant: gain(5°)=5.79)
//   · gain max 4 → 8: l'acquisition des grosses erreurs est ~2× plus sec
//   · plafond 40+200·ramp → 60+340·ramp (max 400 counts/envoi)
//   · SPEED_FACTOR (slider "vitesse" COMBAT): multiplie gain ET plafond —
//     v7 par défaut (facteur 1.55) — le tracking suit les strafers
//   La précision finale ne bouge PAS: deadzone 0.012° + carry sub-count +
//   gate vue fraîche restent les patrons de la convergence.

/// courbe commune: smoothstep sur err/8° (0° → 0.0, 8°+ → 1.0)
fn ramp(err_deg: f32) -> f32 {
    let t = (err_deg / 8.0).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// gain multiplicatif du delta: 1.0 à ≤0° (précision maximale) → 8.0 à ≥8°.
/// Le gain ne s'applique qu'au DELTA — la convergence finale reste pilotée par
/// le carry sub-count + la deadzone, donc la précision au degré près est
/// strictement identique à v0.8.2 ; seule l'ACQUISITION accélère.
pub fn adaptive_gain(err_deg: f32) -> f32 {
    1.0 + 7.0 * ramp(err_deg)
}

/// plafond de counts par envoi: 60 près de la cible → 400 loin.
/// (l'ancien clamp fixe ±60 bridait les grosses erreurs: "l'aimbot est trop
/// lent" — un 180° à 60 counts/envoi × sens 2.5 met ~1.2 s à converger)
pub fn count_ceiling(err_deg: f32) -> f32 {
    60.0 + 340.0 * ramp(err_deg)
}

/// slider "vitesse de visée" du menu COMBAT (1–10) → facteur multiplicatif
/// appliqué au gain ET au plafond. v7 (défaut) → 1.55× — le tracking gagne
/// contre un strafer; v1 → 0.65× (finesse maximale, presque v0.8);
/// v10 → 2.0× (acquisition brutale, pour RAGE assumé).
pub fn speed_factor(speed: f32) -> f32 {
    (0.5 + 0.15 * speed.clamp(1.0, 10.0)).clamp(0.5, 2.0)
}

// ── 4. MODÈLE DE MOUVEMENT ADAPTATIF (v0.9.2) ──────────────────────────────
//
// LE RETARD QUE LA VITESSE BRUTE NE CORRIGE PAS:
// m_vecVelocity est une vitesse INSTANTANÉE, quantifiée au tick (64 Hz) et
// lissée par le moteur. Au moment d'un changement de direction (counter-
// strafe, jiggle), elle pointe encore l'ANCIENNE trajectoire pendant 1-2
// ticks → l'aimbot vise derrière l'épaule. Un simple v·t ne voit pas non plus
// l'ACCÉLÉRATION: la cible « tourne » pendant le temps de vol de la balle et
// le point d'avance linéaire tombe à côté.
//
// Ici deux étages:
//   1. FILTRE ALPHA-BETA-GAMMA sur la position PIEDS — équivalent stationnaire
//      d'un Kalman à accélération constante, mais sans matrice. Il sort une
//      vitesse et une ACCÉLÉRATION à faible retard, ré-ancrées sur les vraies
//      positions lues (donc immunisées contre le retard de m_vecVelocity).
//   2. MINI-MODÈLE LINÉAIRE À DEUX POIDS (w_lead, w_acc) entraîné EN LIGNE
//      par descente de gradient sur l'erreur de prédiction observée à
//      l'échéance. C'est la « mini-réseau »: deux neurones linéaires qui
//      apprennent le retard résiduel systématique (interp client + tick +
//      âge snapshot + temps de convergence) au lieu de le supposer constant.

/// accélération max plausible d'un joueur (u/s²). Un counter-strafe brutal
/// passe de +250 à -250 u/s en ~1 tick — on borne la contribution prédictive
/// à une valeur qui reste saine même sur une lecture bruitée.
pub const ACC_CLAMP: f32 = 6000.0;
/// bornes des poids appris — jamais un modèle qui extrapole dans le décor.
pub const W_MIN: f32 = 0.5;
pub const W_MAX: f32 = 2.0;
/// plafond dur du déplacement prédit (u) — ~5 têtes. Au-delà, c'est un strafe
/// inversé et on viserait un fantôme.
pub const DELTA_MAX_U: f32 = 300.0;
/// v0.9.5 — plafond du lead VERTICAL (u) et part conservée. Une cible qui
/// saute, tombe ou monte un escalier a une vitesse Z énorme et bruitée:
/// projetée telle quelle sur la hitbox elle fait glisser la visée vers la
/// poitrine. La composante verticale n'aide pas à toucher → réduite + bornée.
pub const LEAD_Z_MAX_U: f32 = 3.0;
pub const LEAD_Z_SCALE: f32 = 0.35;

#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    t_target: f32,
    predicted: [f32; 3],
    f_lead: [f32; 3],
    f_acc: [f32; 3],
}

/// Trajectoire d'une cible + mini-modèle appris. Une instance par pawn.
#[derive(Clone, Debug)]
pub struct MotionPredictor {
    pos: [f32; 3],
    vel: [f32; 3],
    acc: [f32; 3],
    last_meas: [f32; 3],
    t: f32,
    primed: bool,
    /// v0.9.6 — échantillons frais consécutifs où la cible est immobile
    /// (vitesse brute < 18 u/s ET vitesse filtrée < 60): à 3+, le lead est
    /// quasi tué — une cible ARRÊTÉE ne se « prédit » plus.
    slow_streak: u8,
    /// poids du mini-modèle linéaire, appris en ligne
    pub w_lead: f32,
    pub w_acc: f32,
    pending: Option<Pending>,
    /// échantillons d'apprentissage accumulés (diagnostic)
    pub samples: u32,
    /// dernière erreur de prédiction observée (u) — diagnostic
    pub last_err_u: f32,
}

impl MotionPredictor {
    pub fn new() -> Self {
        MotionPredictor {
            pos: [0.0; 3],
            vel: [0.0; 3],
            acc: [0.0; 3],
            last_meas: [0.0; 3],
            t: 0.0,
            primed: false,
            slow_streak: 0,
            w_lead: 1.0,
            w_acc: 1.0,
            pending: None,
            samples: 0,
            last_err_u: 0.0,
        }
    }

    /// oublie la trajectoire (cible perdue / téléport) mais GARDE les poids
    /// appris: le modèle du retard reste valable d'une cible à l'autre.
    pub fn reset_state(&mut self) {
        self.primed = false;
        self.pending = None;
        self.vel = [0.0; 3];
        self.acc = [0.0; 3];
        self.slow_streak = 0;
    }

    fn same(a: &[f32; 3], b: &[f32; 3]) -> bool {
        (a[0] - b[0]).abs() < 1e-4
            && (a[1] - b[1]).abs() < 1e-4
            && (a[2] - b[2]).abs() < 1e-4
    }

    /// ingère une mesure: position PIEDS + vitesse brute m_vecVelocity (u/s)
    /// et l'horloge en secondes.
    pub fn observe(&mut self, pos: [f32; 3], vel_raw: [f32; 3], now: f32) {
        if !self.primed {
            self.pos = pos;
            self.vel = vel_raw;
            self.acc = [0.0; 3];
            self.last_meas = pos;
            self.t = now;
            self.primed = true;
            return;
        }
        let dt_raw = now - self.t;
        if dt_raw < 0.001 {
            return; // échantillon dupliqué (boucle 1 kHz entre deux ticks)
        }
        // v0.9.6 — LE VÉRITABLE BUG « IL NE RECALCULE PAS »: l'ancienne porte
        // rejetait toute donnée IDENTIQUE pendant 80 ms. Or notre boucle
        // (1 kHz) re-polle la même donnée ~15× ENTRE deux ticks serveur
        // (64 Hz) — c'est CE doublon-là qu'il faut jeter (sinon le filtre
        // ré-intègre le même échantillon et explose). Mais un joueur qui
        // S'ARRÊTE produit des positions identiques TICK APRÈS TICK,
        // légitimement: l'ancienne porte les rejetait TOUTES → la vitesse
        // FILTRÉE restait à 250 u/s pour toujours → le point prédit restait
        // DEVANT la cible arrêtée, la visée glissait à côté de la tête sans
        // jamais se recaler. La vraie cloison est TEMPORELLE: identique à
        // moins d'un DEMI-TICK (8 ms) = re-poll du même tick → jeter;
        // identique après un demi-tick = un VRAI nouveau tick d'une cible
        // arrêtée → ACCEPTER, le filtre voit l'arrêt et décroît.
        const HALF_TICK_S: f32 = TICK_S * 0.5;
        if dt_raw < HALF_TICK_S && Self::same(&pos, &self.last_meas) {
            return; // re-poll du même tick serveur: rien de neuf
        }
        // APPRENDRE SEULEMENT SUR DONNÉE FRAÎCHE: comparer une prédiction à une
        // position périmée ferait croire à un sur-shot systématique et ferait
        // dériver w_lead vers le bas.
        self.learn(pos, now);
        let dt = dt_raw.clamp(0.001, 0.1);
        // alpha-beta-gamma, gains stables pour 64 Hz
        let (al, be, ga) = (0.55f32, 0.28f32, 0.06f32);
        for i in 0..3 {
            let pred = self.pos[i] + self.vel[i] * dt + 0.5 * self.acc[i] * dt * dt;
            let e = pos[i] - pred;
            self.pos[i] = pred + al * e;
            self.vel[i] += be * e / dt;
            self.acc[i] = (self.acc[i] + 2.0 * ga * e / (dt * dt)).clamp(-ACC_CLAMP, ACC_CLAMP);
        }
        // v0.9.6 — streak d'immobilité: vitesse BRUTÉE < 18 u/s (un arrêt net
        // tombe à 0 dès le premier tick arrêté) ET filtrée < 60 (garde-fou
        // contre une lecture brute isolée à 0 pendant un mouvement). 3
        // échantillons frais (~48 ms) → la cible est ARRÊTÉE.
        let h_raw = (vel_raw[0] * vel_raw[0] + vel_raw[1] * vel_raw[1]).sqrt();
        let h_flt = (self.vel[0] * self.vel[0] + self.vel[1] * self.vel[1]).sqrt();
        self.slow_streak = if h_raw < 18.0 && h_flt < 60.0 {
            self.slow_streak.saturating_add(1)
        } else {
            0
        };
        self.last_meas = pos;
        self.t = now;
    }

    /// entraînement: la prédiction émise a une échéance; quand une position
    /// réelle la dépasse, l'erreur projetée met à jour les deux poids.
    fn learn(&mut self, pos: [f32; 3], now: f32) {
        let Some(p) = self.pending else { return };
        if now + 1e-4 < p.t_target - 0.004 {
            return; // échéance pas encore atteinte
        }
        self.pending = None;
        // donnée trop tardive par rapport à l'échéance → pas de signal fiable
        if now - p.t_target > 0.05 {
            return;
        }
        let e = [
            pos[0] - p.predicted[0],
            pos[1] - p.predicted[1],
            pos[2] - p.predicted[2],
        ];
        let e2 = e[0] * e[0] + e[1] * e[1] + e[2] * e[2];
        // erreur > 40 u ≈ téléport / mauvaise lecture: pas un signal d'apprentissage
        if !e2.is_finite() || e2 > 1600.0 {
            return;
        }
        self.last_err_u = e2.sqrt();
        // descente de gradient sur les deux poids, normalisée par l'énergie
        // des features (pas de divergence quand le lead est petit)
        let gl = e[0] * p.f_lead[0] + e[1] * p.f_lead[1] + e[2] * p.f_lead[2];
        let ga = e[0] * p.f_acc[0] + e[1] * p.f_acc[1] + e[2] * p.f_acc[2];
        let nl = (p.f_lead[0] * p.f_lead[0]
            + p.f_lead[1] * p.f_lead[1]
            + p.f_lead[2] * p.f_lead[2])
            .max(1.0);
        let na = (p.f_acc[0] * p.f_acc[0]
            + p.f_acc[1] * p.f_acc[1]
            + p.f_acc[2] * p.f_acc[2])
            .max(1.0);
        self.w_lead = (self.w_lead + 0.12 * gl / nl).clamp(W_MIN, W_MAX);
        self.w_acc = (self.w_acc + 0.08 * ga / na).clamp(W_MIN, W_MAX);
        self.samples = self.samples.saturating_add(1);
    }

    /// déplacement à ajouter à l'ancre courante après `lead_s` secondes.
    /// `pos_now` = position PIEDS au moment de l'appel (ancre de prédiction).
    pub fn predict_delta(&mut self, pos_now: [f32; 3], lead_s: f32) -> [f32; 3] {
        let lead = lead_s.clamp(0.0, 0.25);
        let t2 = 0.5 * lead * lead;
        let f_lead = [self.vel[0] * lead, self.vel[1] * lead, self.vel[2] * lead];
        let f_acc = [self.acc[0] * t2, self.acc[1] * t2, self.acc[2] * t2];
        let mut d = [
            self.w_lead * f_lead[0] + self.w_acc * f_acc[0],
            self.w_lead * f_lead[1] + self.w_acc * f_acc[1],
            self.w_lead * f_lead[2] + self.w_acc * f_acc[2],
        ];
        // v0.9.5 — LE LEAD VERTICAL EST UN PIÈGE: la vitesse Z (saut, chute,
        // escalier, accroupissement) est la plus bruitée du jeu et, projetée
        // sur la hitbox, elle fait DESCENDRE la visée du crâne vers la
        // poitrine ("des fois il vise le corps"). On la réduit et on la borne.
        d[2] = (d[2] * LEAD_Z_SCALE).clamp(-LEAD_Z_MAX_U, LEAD_Z_MAX_U);
        let m = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if m > DELTA_MAX_U {
            let k = DELTA_MAX_U / m;
            for v in &mut d {
                *v *= k;
            }
        }
        // v0.9.6 — CIBLE ARRÊTÉE (3+ échantillons frais immobiles): le lead
        // est quasi tué (15 %). L'ancien code continuait de « prédire » un
        // mouvement mort → la visée restait DEVANT la cible arrêtée au lieu
        // de se poser sur sa tête.
        if self.slow_streak >= 3 {
            for v in &mut d {
                *v *= 0.15;
            }
        }
        // ne PAS écraser une prédiction en attente: elle doit atteindre son
        // échéance pour fournir un signal d'apprentissage (sinon chaque frame
        // remplace la précédente et aucune n'est jamais évaluée).
        if lead > 1e-4 && self.primed && self.pending.is_none() {
            self.pending = Some(Pending {
                t_target: self.t + lead,
                predicted: [pos_now[0] + d[0], pos_now[1] + d[1], pos_now[2] + d[2]],
                f_lead,
                f_acc,
            });
        }
        d
    }

    /// vitesse filtrée (u/s) — diagnostic / seuil de mobilité
    pub fn speed(&self) -> f32 {
        (self.vel[0] * self.vel[0] + self.vel[1] * self.vel[1] + self.vel[2] * self.vel[2]).sqrt()
    }
}

// ── 5. RECUL BALISTIQUE DANS LA VISÉE + MODE RAFALE (v0.9.3) ───────────────
//
// "LE VISEUR EST AU-DESSUS DE LA TÊTE AVEC LE RECUL":
// pendant une rafale, le recul balistique décale la direction des balles de
// l'angle de punch (AimPunchServices, lu dans le snapshot). Viser la tête
// avec la VUE ne suffit donc pas: la balle part vers vue + punch. Pour que le
// projectile arrive dans la tête, la vue doit viser tête − punch.
//
// Convention du projet (voir modules.rs NoRecoil): direction de balle =
// vue + punch ⇒ compensation = view − punch. PUNCH_SIGN = -1.0.
//
// Et parce que chaque balle RÉGÉNÈRE l'erreur, une rafale doit converger plus
// sec et plus près du centre: gain renforcé, diviseur réduit, deadzone quasi
// nulle tant que m_iShotsFired > 0. Cela répond à "qu'il se remette
// automatiquement": à chaque balle il recalcule et re-centre.

/// signe de la compensation de punch — partagé avec modules::PUNCH_SIGN.
pub const AIM_PUNCH_SIGN: f32 = -1.0;

/// angle de visée corrigé du recul: (pitch, yaw) de la tête MOINS le punch.
/// `gain` = 0 désactive, 1 compense intégralement (slider menu).
pub fn punch_compensated(
    head_pitch: f32,
    head_yaw: f32,
    punch: Option<(f32, f32)>,
    gain: f32,
) -> (f32, f32) {
    match punch {
        Some((pp, py)) if gain != 0.0 && pp.is_finite() && py.is_finite() => (
            head_pitch + AIM_PUNCH_SIGN * pp * gain,
            head_yaw + AIM_PUNCH_SIGN * py * gain,
        ),
        _ => (head_pitch, head_yaw),
    }
}

// ── 5b. GARDE-FOU "JAMAIS LA POITRINE" (v0.9.5) ────────────────────────────
//
// "DES FOIS IL ARRÊTE DE VISER LA TÊTE ET VISE LE CORPS":
// la compensation de punch (v0.9.3) déplace la visée VERS LE BAS de l'angle de
// recul. Avec un punch lu gros, une cible proche ou un build qui applique le
// recul différemment, la vue descend de plusieurs degrés — visuellement le
// viseur se pose sur le torse. Le garde-fou ci-dessous borne ce déplacement à
// une FRACTION de l'écart angulaire réel tête→poitrine, quelle que soit la
// valeur du punch: la visée ne peut plus quitter le crâne.

/// fraction maximale de l'écart tête→poitrine que la compensation peut couvrir.
/// v0.9.6 — 0.30 → 0.15 + CAP ABSOLU: à courte portée l'écart angulaire
/// tête→poitrine explose (5-10°) et 30 % = 1.5-3° sous le centre de la tête
/// = le MENTON/LE COU → « des fois il vise le corps ». Le déplacement est
/// désormais borné par le PLUS PETIT des deux: 15 % de l'écart OU 0.6°
/// (0.6° ≈ 1.6 u à 3 m: toujours dans le crâne).
pub const HEAD_GUARD_FRAC: f32 = 0.15;
/// borne ABSOLUE (°) du déplacement autorisé vers la poitrine, quelle que
/// soit la portée — le garde-fou ne peut plus ouvrir à courte distance.
pub const HEAD_GUARD_ABS_DEG: f32 = 0.6;

/// borne le pitch visé autour de la tête. `chest_pitch` = pitch de la
/// poitrine (plus bas que la tête: le pitch CROÎT vers le bas).
/// Retourne toujours une valeur du côté de la tête, jamais au-delà de
/// min(15 % de l'écart tête→poitrine, 0.6°) sous la tête.
pub fn head_guard(head_pitch: f32, chest_pitch: f32, aim_pitch: f32) -> f32 {
    if !head_pitch.is_finite() || !chest_pitch.is_finite() || !aim_pitch.is_finite() {
        return head_pitch; // donnée pourrie: on reste pile sur le crâne
    }
    let gap = chest_pitch - head_pitch;
    // écart nul OU INVERSÉ (poitrine au-dessus de la tête = os croisés,
    // lecture pourrie): l'info poitrine ne vaut rien → pile sur la tête
    if gap.abs() < 1e-3 || gap <= 0.0 {
        return head_pitch;
    }
    // v0.9.6 — bande autorisée: [tête … tête + span] VERS la poitrine,
    // span = min(15 % de l'écart, 0.6°) — jamais au-dessus de la tête,
    // jamais au menton même à bout portant.
    let span = (gap * HEAD_GUARD_FRAC).min(HEAD_GUARD_ABS_DEG);
    aim_pitch.clamp(head_pitch, head_pitch + span)
}

// ── 5c. GATE TÊTE PHYSIQUE (v0.9.6) — « À CHAQUE TIR JE TOUCHE LA TÊTE » ──
//
// Le triggerbot ne doit cliquer QUE si la balle, partant MAINTENANT le long
// de (vue + punch), atterrit DANS la hitbox tête. Le test est PHYSIQUE:
// écart angulaire du tir vers le centre de la tête ≤ rayon angulaire de la
// hitbox à cette distance. C'est le couplage exact aimbot→triggerbot: quand
// l'aimbot a convergé (il vise tête − punch), l'écart tend vers 0 et le gate
// s'ouvre — pas avant, pas pour le corps.

/// rayon de la hitbox tête (u) — conservateur: la tête CS2 fait ~3.4 u de
/// rayon; 3.2 garde une marge pour que le gate ne s'ouvre que si la balle
/// est VRAIMENT dans le crâne (le lead résiduel et le bruit de tick
/// mangent la marge).
pub const HEAD_RADIUS_U: f32 = 3.2;
/// conversion mètres → unités Hammer (1 u = 0.019 m ≈ 1.9 cm)
pub const U_PER_M: f32 = 1.0 / 0.019;

/// rayon angulaire (°) de la hitbox tête à `dist_m` mètres.
/// Plancher 0.06°: au-delà de ~50 m l'écart est sous le bruit de mesure —
/// le gate reste utilisable sans exiger l'impossible. Cap 8° (au contact).
pub fn head_radius_deg(dist_m: f32) -> f32 {
    if !dist_m.is_finite() || dist_m <= 0.0 {
        return 8.0; // distance inconnue: on suppose le contact (gate ouvert)
    }
    (HEAD_RADIUS_U / (dist_m * U_PER_M)).atan().to_degrees().clamp(0.06, 8.0)
}

/// le tir partant maintenant (écart `err_deg` vers le centre de la tête à
/// `dist_m`) atterrit-il dans la tête ? — le signal du triggerbot.
pub fn shot_lands_in_head(err_deg: f32, dist_m: f32) -> bool {
    err_deg.is_finite() && err_deg.abs() <= head_radius_deg(dist_m)
}

/// gain multiplicatif de convergence pendant une rafale (m_iShotsFired > 0).
pub fn spray_gain(shots_fired: i32) -> f32 {
    if shots_fired > 0 {
        1.35
    } else {
        1.0
    }
}

/// facteur appliqué au diviseur de lissage pendant une rafale (< 1 = plus vif).
pub fn spray_div_scale(shots_fired: i32) -> f32 {
    if shots_fired > 0 {
        0.55
    } else {
        1.0
    }
}

/// deadzone angulaire (°) — quasi nulle en rafale pour rester DANS la tête.
pub fn deadzone_deg(shots_fired: i32) -> f32 {
    if shots_fired > 0 {
        0.004
    } else {
        0.012
    }
}

/// le viseur est-il "sur la tête" à cette erreur résiduelle ?
pub fn on_target(err_deg: f32, shots_fired: i32) -> bool {
    err_deg.is_finite() && err_deg.abs() <= deadzone_deg(shots_fired)
}

// ── 6. GATE VUE FRAÎCHE — ANTI-BLOCAGE (v0.9.4) ────────────────────────────
//
// LE BUG "IL ARRÊTE DE BOUGER, LA CIBLE PART ET IL NE SE REMET PAS":
// la gate anti-doublon ne renvoie pas un mouvement tant que la vue mémoire n'a
// pas changé depuis le dernier envoi (protection contre l'overshoot en éventail
// quand la vue est périmée). Mais quand PLUS RIEN n'est envoyé, la vue ne
// change plus… et la gate attend indéfiniment: l'aimbot se fige jusqu'à ce que
// le joueur bouge la souris lui-même.
//
// Règle: la gate n'attend que pendant une durée COURTE (≈ 2 frames à 60 Hz).
// Au-delà, on renvoie quand même: la correction est déjà bornée par axe à
// l'erreur restante, donc un envoi sur vue gelée ne peut pas dépasser la tête.
pub const GATE_MAX_WAIT_S: f32 = 0.035;

/// faut-il attendre la prochaine frame de rendu avant de renvoyer ?
/// `same_view` = la vue mémoire n'a pas bougé depuis le dernier envoi.
/// `since_send_s` = temps écoulé depuis cet envoi (s).
pub fn gate_should_wait(same_view: bool, since_send_s: f32) -> bool {
    same_view && since_send_s.is_finite() && since_send_s < GATE_MAX_WAIT_S
}

// ═══ tests unitaires (tournent sur TOUTE machine — zéro windows) ═══════════
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_distance_weight() {
        // à angle égal, le plus proche gagne
        assert!(score(5.0, 10.0) < score(5.0, 90.0));
        // l'angle domine: 3° loin bat 10° près
        assert!(score(3.0, 90.0) < score(10.0, 10.0));
        // symétrie zéro
        assert_eq!(score(0.0, 0.0), 0.0);
    }

    #[test]
    fn hysteresis_blocks_group_flicker() {
        // deux cibles quasi identiques: PAS de switch (le bug "stresse")
        assert!(!should_switch(4.20, 4.10));
        // un candidat franchement meilleur: switch
        assert!(should_switch(10.0, 4.0));
        // frontière exacte: 30 % de mieux passe
        assert!(should_switch(10.0, 6.9));
        assert!(!should_switch(10.0, 7.2));
    }

    #[test]
    fn grace_fov_scales() {
        assert_eq!(grace_fov_deg(10.0), 15.0);
        assert_eq!(grace_fov_deg(30.0), 45.0);
    }

    #[test]
    fn lead_time_bounds() {
        // base seule ≈ 31.25 + 7.8 ms ≈ 39 ms
        let base = lead_time_s(0.0, 0.0, 0.0);
        assert!(base > 0.030 && base < 0.050, "base = {base}");
        // composants ajoutent
        assert!(lead_time_s(0.02, 5.0, 10.0) > base);
        // borne haute dure: même délire de config → 250 ms max
        assert_eq!(lead_time_s(1.0, 100.0, 5000.0), 0.25);
        // smooth au-delà de 20 est clampé (le retard ne suit plus le slider)
        assert_eq!(lead_time_s(0.0, 20.0, 0.0), lead_time_s(0.0, 99.0, 0.0));
    }

    #[test]
    fn prediction_moves_point_along_velocity() {
        let p = predicted([0.0, 0.0, 64.0], [250.0, 0.0, 0.0], 0.1);
        assert!((p[0] - 25.0).abs() < 1e-4, "p[0] = {}", p[0]);
        assert!(p[1] == 0.0 && p[2] == 64.0);
        // lead nul → point inchangé
        let q = predicted([1.0, 2.0, 3.0], [500.0, -500.0, 0.0], 0.0);
        assert_eq!(q, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn adaptive_gain_curve() {
        assert!((adaptive_gain(0.0) - 1.0).abs() < 1e-6);
        assert!((adaptive_gain(-5.0) - 1.0).abs() < 1e-6); // err négative impossible mais clampée
        assert!(adaptive_gain(1.0) < 1.4, "petites erreurs restent fines");
        // v0.9.1 — l'erreur de STRAFE (5°) prend un vrai gain: le tracking
        // ne se fait plus distancer par une cible qui bouge
        assert!(adaptive_gain(5.0) > 4.5 && adaptive_gain(5.0) < 6.5,
            "gain strafe = {}", adaptive_gain(5.0));
        assert!((adaptive_gain(8.0) - 8.0).abs() < 1e-6);
        assert!((adaptive_gain(90.0) - 8.0).abs() < 1e-6); // saturé
    }

    #[test]
    fn count_ceiling_curve() {
        assert!((count_ceiling(0.0) - 60.0).abs() < 1e-6);
        assert!(count_ceiling(2.0) < 150.0, "près de la cible: plafond bas");
        assert!((count_ceiling(20.0) - 400.0).abs() < 1e-6);
        // monotone
        assert!(count_ceiling(3.0) <= count_ceiling(4.0));
        assert!(adaptive_gain(3.0) <= adaptive_gain(4.0));
    }

    #[test]
    fn speed_factor_curve() {
        // défaut menu v7 → 1.55: le tracking gagne contre un strafer
        assert!((speed_factor(7.0) - 1.55).abs() < 1e-6);
        // bornes: v1 douce, v10 brutale — clampé des deux côtés
        assert!((speed_factor(1.0) - 0.65).abs() < 1e-6);
        assert!((speed_factor(10.0) - 2.0).abs() < 1e-6);
        assert!((speed_factor(0.0) - 0.65).abs() < 1e-6); // clamp bas
        assert!((speed_factor(99.0) - 2.0).abs() < 1e-6); // clamp haut
        // monotone
        assert!(speed_factor(3.0) < speed_factor(8.0));
    }

    #[test]
    fn pred_min_speed_sane() {
        assert!(PRED_MIN_SPEED > 0.0 && PRED_MIN_SPEED < 100.0);
    }

    // ── v0.9.2: modèle de mouvement adaptatif ──────────────────────────────
    #[test]
    fn motion_estimates_velocity_from_positions() {
        // cible qui avance à 250 u/s sur +X, un échantillon tous les 15.6 ms
        let mut m = MotionPredictor::new();
        let dt = 1.0 / 64.0;
        for k in 0..40 {
            let t = k as f32 * dt;
            let pos = [250.0 * t, 0.0, 0.0];
            m.observe(pos, [250.0, 0.0, 0.0], t);
        }
        // vitesse filtrée ≈ 250 (le filtre ABG converge)
        assert!((m.speed() - 250.0).abs() < 25.0, "speed = {}", m.speed());
        // le déplacement prédit avance dans le bon sens
        let d = m.predict_delta([250.0 * 39.0 * dt, 0.0, 0.0], 0.06);
        assert!(d[0] > 8.0 && d[0] < 25.0, "lead x = {}", d[0]);
        assert!(d.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn motion_learner_accumulates_and_stays_bounded() {
        // Sur une cible à vitesse CONSTANTE, le filtre ABG est déjà exact:
        // le mini-modèle doit rester NEUTRE (w ≈ 1) et ne pas déstabiliser,
        // tout en accumulant des échantillons d'apprentissage.
        let mut m = MotionPredictor::new();
        let dt = 1.0 / 64.0;
        let mut t = 0.0f32;
        for _ in 0..400 {
            let pos = [250.0 * t, 0.0, 0.0];
            m.observe(pos, [250.0, 0.0, 0.0], t);
            let _ = m.predict_delta(pos, 0.05);
            t += dt;
        }
        assert!(m.samples > 50, "samples = {}", m.samples);
        assert!(m.w_lead >= W_MIN && m.w_lead <= W_MAX);
        assert!((m.w_lead - 1.0).abs() < 0.2, "w_lead neutre = {}", m.w_lead);
        assert!(m.last_err_u < 3.0, "err = {}", m.last_err_u);
    }

    #[test]
    fn motion_acceleration_beats_linear_on_strafe() {
        // Courbe: la cible ACCÉLÈRE. Le modèle ACC doit mieux prédire que v·t.
        let mut m = MotionPredictor::new();
        let dt = 1.0 / 64.0;
        let (v0, a) = (250.0f32, -3000.0f32);
        let pos_at = |t: f32| v0 * t + 0.5 * a * t * t;
        for k in 0..60 {
            let t = k as f32 * dt;
            m.observe([pos_at(t), 0.0, 0.0], [v0 + a * t, 0.0, 0.0], t);
        }
        let t_now = 59.0 * dt;
        let lead = 0.06f32;
        let d = m.predict_delta([pos_at(t_now), 0.0, 0.0], lead);
        let pred = pos_at(t_now) + d[0];
        let truth = pos_at(t_now + lead);
        let linear = pos_at(t_now) + (v0 + a * t_now) * lead;
        assert!(
            (pred - truth).abs() <= (linear - truth).abs() + 0.5,
            "acc pred err {:.1} vs linear err {:.1}",
            (pred - truth).abs(),
            (linear - truth).abs()
        );
    }

    #[test]
    fn motion_bounded_and_finite() {
        let mut m = MotionPredictor::new();
        // téléports et valeurs extrêmes: jamais de NaN, jamais > DELTA_MAX_U
        for k in 0..50 {
            let t = k as f32 * 0.016;
            let pos = if k % 7 == 0 { [1e5, -1e5, 3e4] } else { [0.0, 0.0, 0.0] };
            m.observe(pos, [1e6, -1e6, 1e6], t);
            let d = m.predict_delta(pos, 0.25);
            let mag = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            assert!(d.iter().all(|v| v.is_finite()));
            assert!(mag <= DELTA_MAX_U + 1e-3, "delta mag = {mag}");
            assert!(m.w_lead >= W_MIN && m.w_lead <= W_MAX);
        }
    }

    #[test]
    fn motion_reset_keeps_learned_weights() {
        let mut m = MotionPredictor::new();
        m.w_lead = 1.4;
        m.reset_state();
        assert!((m.w_lead - 1.4).abs() < 1e-6);
        assert!(m.speed() == 0.0);
    }

    // ── v0.9.3: recul dans la visée + rafale ────────────────────────────────
    #[test]
    fn punch_compensation_lowers_aim_under_recoil() {
        // tête à (pitch 10, yaw 20), punch qui pousse (pitch +2, yaw -3).
        // Balle = vue + punch ⇒ vue cible = tête − punch.
        let (p, y) = punch_compensated(10.0, 20.0, Some((2.0, -3.0)), 1.0);
        assert!((p - 8.0).abs() < 1e-6, "pitch = {p}");
        assert!((y - 23.0).abs() < 1e-6, "yaw = {y}");
        // vérifie l'invariant: vue cible + punch = tête
        assert!((p + 2.0 - 10.0).abs() < 1e-6);
        assert!((y + (-3.0) - 20.0).abs() < 1e-6);
    }

    #[test]
    fn punch_gain_zero_and_none_are_noops() {
        assert_eq!(punch_compensated(5.0, 6.0, Some((2.0, -3.0)), 0.0), (5.0, 6.0));
        assert_eq!(punch_compensated(5.0, 6.0, None, 1.0), (5.0, 6.0));
        // gain partiel: moitié du punch
        let (p, _) = punch_compensated(10.0, 0.0, Some((4.0, 0.0)), 0.5);
        assert!((p - 8.0).abs() < 1e-6);
    }

    #[test]
    fn spray_mode_tightens_aim() {
        assert!((spray_gain(0) - 1.0).abs() < 1e-6);
        assert!(spray_gain(3) > 1.0);
        assert!((spray_div_scale(0) - 1.0).abs() < 1e-6);
        assert!(spray_div_scale(5) < 1.0);
        assert!(deadzone_deg(1) < deadzone_deg(0));
        // "sur la tête": tolérance plus serrée en rafale
        assert!(on_target(0.008, 0));
        assert!(!on_target(0.008, 1));
        assert!(on_target(0.003, 1));
    }

    // ── v0.9.4: la gate vue fraîche ne peut PAS figer la visée ─────────────
    #[test]
    fn gate_never_freezes_forever() {
        // vue identique + envoi très récent → on attend la frame (anti-doublon)
        assert!(gate_should_wait(true, 0.002));
        assert!(gate_should_wait(true, GATE_MAX_WAIT_S * 0.5));
        // vue identique mais envoi VIEUX → on renvoie (sinon blocage définitif)
        assert!(!gate_should_wait(true, GATE_MAX_WAIT_S + 0.001));
        assert!(!gate_should_wait(true, 1.0));
        // vue fraîche → jamais d'attente
        assert!(!gate_should_wait(false, 0.0));
        // NaN (horloge absente) → on n'attend pas
        assert!(!gate_should_wait(true, f32::NAN));
        // garde-fou: la patience reste sous 3 frames à 60 Hz
        assert!(GATE_MAX_WAIT_S > 0.020 && GATE_MAX_WAIT_S < 0.050);
    }

    // ── v0.9.5: "il vise le corps" — garde-fou tête + lead vertical borné ──
    #[test]
    fn head_guard_keeps_aim_on_skull() {
        // tête pitch 10, poitrine pitch 14 (4° plus bas)
        // punch qui pousse la vue 8° vers le bas → sans garde on finirait
        // sous la poitrine; avec garde on reste au-dessus.
        let a = head_guard(10.0, 14.0, 18.0);
        assert!(a > 10.0 && a <= 10.0 + 4.0 * HEAD_GUARD_FRAC + 1e-6, "a = {a}");
        // punch qui pousse vers le HAUT (cas vu en live v0.9.3) → borné aussi
        let b = head_guard(10.0, 14.0, 2.0);
        assert!((b - 10.0).abs() < 1e-6, "b = {b}");
        // déjà dans la zone autorisée → inchangé
        let c = head_guard(10.0, 14.0, 10.5);
        assert!((c - 10.5).abs() < 1e-6);
        // données pourries → pile sur la tête
        assert_eq!(head_guard(10.0, 14.0, f32::NAN), 10.0);
        assert!(head_guard(f32::NAN, 14.0, 11.0).is_nan());
        // poitrine au-dessus de la tête (os croisés) → reste PILE sur la tête
        assert!((head_guard(10.0, 8.0, 6.0) - 10.0).abs() < 1e-6);
        // le garde-fou ne descend jamais plus loin que 15 % de l'écart,
        // et JAMAIS plus de 0.6° sous la tête (cap absolu v0.9.6)
        assert!(HEAD_GUARD_FRAC > 0.0 && HEAD_GUARD_FRAC <= 0.2);
        assert!((HEAD_GUARD_ABS_DEG - 0.6).abs() < 1e-6);
        // écart ÉNORME (bout portant, 10°): cap absolu → 0.6° max, pas 1.5°
        let close = head_guard(10.0, 20.0, 30.0);
        assert!((close - 10.6).abs() < 1e-6, "close = {close}");
    }

    #[test]
    fn vertical_lead_is_bounded() {
        // cible qui CHUTE à -600 u/s: le lead vertical brut ferait -24 u
        // (poitrine). Le modèle doit rester dans la tête.
        let mut m = MotionPredictor::new();
        let dt = 1.0 / 64.0;
        let mut t = 0.0f32;
        for _ in 0..40 {
            let z = 64.0 - 600.0 * t * t * 0.5;
            m.observe([0.0, 0.0, z], [0.0, 0.0, -600.0], t);
            t += dt;
        }
        let d = m.predict_delta([0.0, 0.0, 40.0], 0.06);
        assert!(d[2].abs() <= LEAD_Z_MAX_U + 1e-6, "d[2] = {}", d[2]);
        assert!(d[2] >= -LEAD_Z_MAX_U - 1e-6);
    }

    // ── v0.9.6: « il ne recalcule pas » + gate physique tête ─────────────
    #[test]
    fn motion_sees_the_stop_and_kills_the_lead() {
        // LE bug "il ne recalcule pas les têtes": un strafer à 250 u/s qui
        // S'ARRÊTE net. Les positions deviennent identiques → l'ancienne
        // porte anti-doublon (position seule) rejetait TOUT → la vitesse
        // filtrée restait à 250 pour toujours → le point prédit restait
        // devant la cible arrêtée. La porte doit accepter l'échantillon
        // (la vitesse brute a changé), décroître, puis tuer le lead.
        let mut m = MotionPredictor::new();
        let dt = 1.0 / 64.0;
        let mut t = 0.0f32;
        // phase 1: strafer constant
        for _ in 0..30 {
            m.observe([250.0 * t, 0.0, 0.0], [250.0, 0.0, 0.0], t);
            t += dt;
        }
        let stop_x = 250.0 * t;
        let pre_stop_speed = m.speed();
        assert!(pre_stop_speed > 200.0, "avant arrêt: {}", pre_stop_speed);
        // phase 2: ARRÊT NET — position figée, vitesse brute 0
        for _ in 0..12 {
            m.observe([stop_x, 0.0, 0.0], [0.0, 0.0, 0.0], t);
            t += dt;
        }
        // le filtre a VU l'arrêt: la vitesse filtrée décroît franchement
        let post_stop_speed = m.speed();
        assert!(
            post_stop_speed < pre_stop_speed * 0.6,
            "le filtre doit décroître: {} → {}",
            pre_stop_speed,
            post_stop_speed
        );
        // et après 3+ échantillons immobiles le lead est QUASI TUÉ
        let d = m.predict_delta([stop_x, 0.0, 0.0], 0.06);
        let mag = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        assert!(mag < 3.0, "lead sur cible arrêtée = {mag} u (attendu < 3)");
        // reprise du mouvement: le streak se réinitialise, le lead revient
        for _ in 0..30 {
            m.observe([stop_x + 250.0 * (t - (30.0 * dt) - 12.0 * dt), 0.0, 0.0], [250.0, 0.0, 0.0], t);
            t += dt;
        }
        let d2 = m.predict_delta([stop_x + 250.0 * 18.0 * dt, 0.0, 0.0], 0.06);
        assert!(d2[0] > 5.0, "lead après reprise = {}", d2[0]);
    }

    #[test]
    fn head_radius_scales_with_distance() {
        // 10 m ≈ 526 u: atan(3.2/526) ≈ 0.35° — une tête fait bien ça
        let r10 = head_radius_deg(10.0);
        assert!(r10 > 0.25 && r10 < 0.45, "r(10m) = {r10}");
        // monotone décroissante avec la distance
        assert!(head_radius_deg(5.0) > r10 && r10 > head_radius_deg(30.0));
        // plancher au très long: le gate reste utilisable mais pas magique
        assert!((head_radius_deg(300.0) - 0.06).abs() < 1e-6);
        // cap au contact
        assert!((head_radius_deg(0.01) - 8.0).abs() < 1e-6);
        // données pourries: gate ouvert (on ne bloque jamais sur NaN)
        assert!((head_radius_deg(f32::NAN) - 8.0).abs() < 1e-6);
        assert!((head_radius_deg(0.0) - 8.0).abs() < 1e-6);
    }

    #[test]
    fn shot_lands_in_head_gate() {
        // à 10 m le rayon ≈ 0.35°: dans la tête, à côté, loin
        let r = head_radius_deg(10.0);
        assert!(shot_lands_in_head(r * 0.9, 10.0));
        assert!(shot_lands_in_head(0.0, 10.0));
        assert!(!shot_lands_in_head(r * 1.1, 10.0));
        assert!(!shot_lands_in_head(2.0, 10.0));
        // très loin: même 0.05° hors gate si sous le plancher… non:
        // le plancher 0.06° garde le gate OUVERT à ~0.05°
        assert!(shot_lands_in_head(0.05, 300.0));
        assert!(!shot_lands_in_head(0.2, 300.0));
        // NaN → jamais (viseur perdu = pas de tir garanti)
        assert!(!shot_lands_in_head(f32::NAN, 10.0));
        assert!(!shot_lands_in_head(f32::INFINITY, 10.0));
        // à bout portant la tête est grosse: 3° passe
        assert!(shot_lands_in_head(3.0, 1.0));
    }
}
