// math — Vec3 + matrice vue + world→screen
//
// LAYOUT VÉRIFIÉ EN LIVE (build 14193, 29.10.2026) :
//   dwViewMatrix pointe une matrice **3×4 row-major** (12 f32 = 0x30 octets).
//   Les 4 f32 suivants appartiennent à la matrice SUIVANTE de la table — ne
//   jamais les lire comme une 4ᵉ ligne (c'est le piège qui donnait m[15]≈-1990).
//
//   ligne 0 : axe X caméra  (échelle 1/tan(fovx/2) déjà incluse)
//   ligne 1 : axe Y caméra  (échelle 1/tan(fovy/2) déjà incluse)
//   ligne 2 : axe Z caméra  = profondeur vue
//
//   vue = M · (x,y,z,1)  →  ndc = (vue.x/vue.z, vue.y/vue.z)  →  pixels.
//   Test live : ton propre pawn tombe à x=960 exact, les coéquipiers à
//   hauteur d'œil tombent à y≈540, les ennemis se répartissent en largeur.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub fn dist(&self, o: &Vec3) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2) + (self.z - o.z).powi(2)).sqrt()
    }
    /// norme euclidienne (v0.9 — prédiction aimbot: vitesse de la cible)
    pub fn len(&self) -> f32 {
        (self.x.powi(2) + self.y.powi(2) + self.z.powi(2)).sqrt()
    }
    pub fn z_up(&self, dz: f32) -> Vec3 {
        Vec3 { x: self.x, y: self.y, z: self.z + dz }
    }
    /// v0.8 — INTERPOLATION DE RENDU: self + v·t (extrapole la position par
    /// la vélocité lue). Utilisé UNIQUEMENT côté dessin: le serveur tick à
    /// 64 Hz vu à ~166 fps fait avancer les boîtes par pas sinon — la logique
    /// de tir (trigger/aim) reste sur les positions BRUTES du snapshot.
    pub fn add_scaled(&self, v: &Vec3, t: f32) -> Vec3 {
        Vec3 { x: self.x + v.x * t, y: self.y + v.y * t, z: self.z + v.z * t }
    }
}

/// normalise un angle degrés dans (-180..180] (utilisé par l'aimbot v0.4)
pub fn wrap180(a: f32) -> f32 {
    let mut d = a.rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    d
}

// hauteur d'un joueur debout en unités Source — l'origine est aux PIEDS,
// la boîte monte jusqu'à origin.z + 72
pub const PLAYER_HEIGHT: f32 = 72.0;

#[derive(Clone, Copy, Debug)]
pub struct ViewMatrix {
    pub m: [f32; 12], // 3×4 row-major: m[row * 4 + col]
}

impl ViewMatrix {
    // projette un point monde → pixels écran. None = derrière la caméra / hors champ.
    pub fn w2s(&self, p: Vec3, sw: f32, sh: f32) -> Option<(f32, f32)> {
        let m = &self.m;
        // profondeur vue — en dessous de 1 unité le point est dans l'œil
        let z = m[8] * p.x + m[9] * p.y + m[10] * p.z + m[11];
        if !(1.0..f32::MAX).contains(&z) {
            return None;
        }
        let x = m[0] * p.x + m[1] * p.y + m[2] * p.z + m[3];
        let y = m[4] * p.x + m[5] * p.y + m[6] * p.z + m[7];
        let sx = (x / z + 1.0) * 0.5 * sw;
        let sy = (1.0 - y / z) * 0.5 * sh;
        // marge hors-écran: les boîtes coupées au bord restent dessinées
        if !(-sw..sw * 2.0).contains(&sx) || !(-sh..sh * 3.0).contains(&sy) {
            return None;
        }
        Some((sx, sy))
    }

    // sanity check : l'axe Z caméra est un vecteur unitaire (norme ≈ 1).
    // (m[15]≈1 était le test de l'ancien layout 4×4 — faux ici)
    pub fn sane(&self) -> bool {
        if !self.m.iter().all(|v| v.is_finite()) {
            return false;
        }
        let n = (self.m[8] * self.m[8] + self.m[9] * self.m[9] + self.m[10] * self.m[10]).sqrt();
        (0.5..1.5).contains(&n)
    }
}
