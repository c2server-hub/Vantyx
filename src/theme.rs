// theme — Vantyx.cc · v0.6 — thèmes LIVE + couleurs CUSTOM complètes.
//
// v0.6: le thème n'est plus un choix figé parmi 5 — c'est un SYSTÈME:
//   · 5 presets (Void Purple, Crimson, Arctic, Toxic, Titanium)
//   · 1 slot CUSTOM (index 5): chaque rôle éditable en RGB dans le menu
//     (onglet THÈME → studio de couleurs), persisté dans vantyx.ini,
//     appliqué LIVE (menu, radar, watermark, ESP monde — tout dérive de lui)
//
// Chaque thème définit 6 rôles:
//   bg       fond principal (panneau menu / pill watermark / radar)
//   surface  cartes, sidebar, éléments posés sur le fond
//   primary  accent principal (dégradé start, LED, indicateur, weapon ESP)
//   light    accent lumineux (dégradé end, texte watermark, anneaux sound)
//   text     texte principal
//   border   bordures vitrées (alpha faible inclus dans la couleur)
//
// Le dégradé accent du Renderer est RECRÉÉ au changement (set_accent) —
// primary → light. Les couleurs d'ÉQUIPE (T/CT/toi) restent dans
// menu::PALETTES: le thème habille l'interface, la palette habille les
// joueurs.
//
// language: Rust, file: theme.rs, target: Windows 11, windows-rs 0.58

// [r, g, b, a] — hex → /255, alpha du border = canal du hex préfixé (0x33 etc.)
#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub vibe: &'static str, // une ligne d'ambiance — affichée dans l'onglet THÈME
    pub bg: [f32; 4],
    pub surface: [f32; 4],
    pub primary: [f32; 4],
    pub light: [f32; 4],
    pub text: [f32; 4],
    pub border: [f32; 4],
    #[allow(dead_code)] // codes affichables — consommés par le studio CUSTOM à venir
    pub hex: [&'static str; 6], // affichage des codes (bg, surface, primary, light, text, border)
}

pub const THEMES: [Theme; 5] = [
    // 0 — Void Purple (choix de dj — DÉFAUT)
    Theme {
        name: "Void Purple",
        vibe: "mystérieux · agressif · haut de gamme",
        bg: [0.0314, 0.0314, 0.0510, 1.0],      // #08080D noir abyssal
        surface: [0.0667, 0.0667, 0.1059, 1.0], // #11111B noir violacé
        primary: [0.5451, 0.3608, 0.9647, 1.0], // #8B5CF6 violet électrique
        light: [0.7255, 0.6275, 1.0000, 1.0],   // #B9A0FF lavande
        text: [0.9608, 0.9529, 1.0000, 1.0],    // #F5F3FF blanc froid
        border: [0.5451, 0.3608, 0.9647, 0.20], // #8B5CF633
        hex: ["#08080D", "#11111B", "#8B5CF6", "#B9A0FF", "#F5F3FF", "#8B5CF633"],
    },
    // 1 — Crimson
    Theme {
        name: "Crimson",
        vibe: "agressif · compétitif · underground",
        bg: [0.0314, 0.0314, 0.0314, 1.0],      // #080808
        surface: [0.0784, 0.0627, 0.0627, 1.0], // #141010
        primary: [0.9569, 0.2471, 0.3686, 1.0], // #F43F5E
        light: [1.0000, 0.4431, 0.5412, 1.0],   // #FF718A
        text: [0.9608, 0.9608, 0.9608, 1.0],    // #F5F5F5
        border: [0.9569, 0.2471, 0.3686, 0.19], // #F43F5E30
        hex: ["#080808", "#141010", "#F43F5E", "#FF718A", "#F5F5F5", "#F43F5E30"],
    },
    // 2 — Arctic Blue
    Theme {
        name: "Arctic Blue",
        vibe: "technologique · précis · froid",
        bg: [0.0275, 0.0431, 0.0706, 1.0],      // #070B12
        surface: [0.0549, 0.0863, 0.1412, 1.0], // #0E1624
        primary: [0.2196, 0.7412, 0.9725, 1.0], // #38BDF8
        light: [0.4039, 0.9098, 0.9765, 1.0],   // #67E8F9
        text: [0.9412, 0.9765, 1.0000, 1.0],    // #F0F9FF
        border: [0.2196, 0.7412, 0.9725, 0.19], // #38BDF830
        hex: ["#070B12", "#0E1624", "#38BDF8", "#67E8F9", "#F0F9FF", "#38BDF830"],
    },
    // 3 — Toxic Green
    Theme {
        name: "Toxic Green",
        vibe: "rebelle · technique · identifiable",
        bg: [0.0275, 0.0353, 0.0314, 1.0],      // #070908
        surface: [0.0627, 0.0824, 0.0706, 1.0], // #101512
        primary: [0.6392, 1.0000, 0.0706, 1.0], // #A3FF12
        light: [0.2902, 0.8706, 0.5020, 1.0],   // #4ADE80
        text: [0.9412, 0.9922, 0.9569, 1.0],    // #F0FDF4
        border: [0.6392, 1.0000, 0.0706, 0.15], // #A3FF1225
        hex: ["#070908", "#101512", "#A3FF12", "#4ADE80", "#F0FDF4", "#A3FF1225"],
    },
    // 4 — Titanium
    Theme {
        name: "Titanium",
        vibe: "luxe · sobre · élégant",
        bg: [0.0314, 0.0353, 0.0431, 1.0],      // #08090B
        surface: [0.0745, 0.0824, 0.0980, 1.0], // #131519
        primary: [0.7529, 0.7686, 0.8000, 1.0], // #C0C4CC
        light: [0.9608, 0.9608, 0.9608, 1.0],   // #F5F5F5
        text: [0.9098, 0.9176, 0.9294, 1.0],    // #E8EAED
        border: [1.0000, 1.0000, 1.0000, 0.09], // #FFFFFF18
        hex: ["#08090B", "#131519", "#C0C4CC", "#F5F5F5", "#E8EAED", "#FFFFFF18"],
    },
];

/// index du slot thème personnalisé (après les 5 presets)
pub const CUSTOM_IDX: usize = 5;
/// nombre total de choix (presets + custom)
pub const THEME_COUNT: usize = 6;

// ═══ v0.6 — COULEURS CUSTOM (persistées dans vantyx.ini) ═══════════════════
/// Chaque rôle est un RGBA 0..1. L'alpha du border reste par défaut (0.20),
/// les autres restent opaques — le studio n'édite que le RGB.
#[derive(Clone, Copy, PartialEq)]
pub struct CustomColors {
    pub bg: [f32; 4],
    pub surface: [f32; 4],
    pub primary: [f32; 4],
    pub light: [f32; 4],
    pub text: [f32; 4],
    pub border: [f32; 4],
}

impl Default for CustomColors {
    fn default() -> Self {
        // départ = Void Purple (dj repart de son identité pour sculpter)
        CustomColors {
            bg: THEMES[0].bg,
            surface: THEMES[0].surface,
            primary: THEMES[0].primary,
            light: THEMES[0].light,
            text: THEMES[0].text,
            border: THEMES[0].border,
        }
    }
}

impl CustomColors {
    /// rôle par index (0..6) — bg, surface, primary, light, text, border
    pub fn role(&self, i: usize) -> [f32; 4] {
        match i {
            0 => self.bg,
            1 => self.surface,
            2 => self.primary,
            3 => self.light,
            4 => self.text,
            _ => self.border,
        }
    }
    /// modifie le RGB d'un rôle (alpha préservé)
    pub fn set_role(&mut self, i: usize, c: [f32; 4]) {
        let a = self.role(i)[3];
        let v = match i {
            0 => &mut self.bg,
            1 => &mut self.surface,
            2 => &mut self.primary,
            3 => &mut self.light,
            4 => &mut self.text,
            _ => &mut self.border,
        };
        *v = [c[0], c[1], c[2], a];
    }
    pub const ROLES: [&'static str; 6] = ["FOND", "CARTES", "ACCENT", "LIGHT", "TEXTE", "BORDS"];
}

/// thème EFFECTIF: preset si 0..4, couleurs custom si CUSTOM_IDX.
/// C'est LA source unique du menu ET de l'overlay (tout dérive d'ici).
pub fn resolve(idx: usize, custom: &CustomColors) -> Theme {
    if idx >= CUSTOM_IDX {
        Theme {
            name: "Custom",
            vibe: "ta signature · sculptée dans le studio",
            bg: custom.bg,
            surface: custom.surface,
            primary: custom.primary,
            light: custom.light,
            text: custom.text,
            border: custom.border,
            hex: ["", "", "", "", "", ""], // le studio affiche le hex calculé
        }
    } else {
        THEMES[idx]
    }
}

/// thème preset (index clampé) — conservé pour les usages internes presets
pub fn theme(idx: usize) -> &'static Theme {
    &THEMES[idx.min(THEMES.len() - 1)]
}

/// texte secondaire — text tiré vers bg (48 %)
pub fn dim(t: &Theme) -> [f32; 4] {
    [
        t.text[0] * 0.52 + t.bg[0] * 0.48,
        t.text[1] * 0.52 + t.bg[1] * 0.48,
        t.text[2] * 0.52 + t.bg[2] * 0.48,
        1.0,
    ]
}

/// verre principal du panneau (bg + alpha 0.86)
pub fn glass_bg(t: &Theme, a: f32) -> [f32; 4] {
    [t.bg[0], t.bg[1], t.bg[2], 0.86 * a]
}

/// barre de titre (bg légèrement remonté + alpha 0.72)
pub fn glass_title(t: &Theme, a: f32) -> [f32; 4] {
    [
        t.bg[0] * 0.85 + t.surface[0] * 0.15,
        t.bg[1] * 0.85 + t.surface[1] * 0.15,
        t.bg[2] * 0.85 + t.surface[2] * 0.15,
        0.72 * a,
    ]
}

/// carte glass posée sur le panneau (surface + alpha 0.55)
pub fn glass_card(t: &Theme, a: f32) -> [f32; 4] {
    [t.surface[0], t.surface[1], t.surface[2], 0.55 * a]
}

/// piste de slider / fond de checkbox (surface éclaircie)
pub fn glass_well(t: &Theme, a: f32) -> [f32; 4] {
    [
        (t.surface[0] + 0.04).min(1.0),
        (t.surface[1] + 0.04).min(1.0),
        (t.surface[2] + 0.04).min(1.0),
        (0.55 + 0.20 * a) * 0.9,
    ]
}

// ═══ helpers hex (persistance ini + affichage studio) ══════════════════════
/// [r,g,b,a] → "#RRGGBB" (alpha ignoré — il est géré par rôle)
pub fn to_hex(c: [f32; 4]) -> String {
    let r = (c[0] * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (c[1] * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (c[2] * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// "#8B5CF6" | "8B5CF6" | "0x8B5CF6" → RGB 0..1 (alpha = 1)
pub fn from_hex(s: &str) -> Option<[f32; 4]> {
    let t = s.trim().trim_start_matches('#').trim_start_matches("0x").trim_start_matches("0X");
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(t, 16).ok()?;
    Some([
        (v >> 16 & 0xFF) as f32 / 255.0,
        (v >> 8 & 0xFF) as f32 / 255.0,
        (v & 0xFF) as f32 / 255.0,
        1.0,
    ])
}
