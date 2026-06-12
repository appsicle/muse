//! The two palettes (PLAN §8), the `Theme` global, and token interpolation.

use gpui::{App, Global, Hsla, rgb};
use serde::{Deserialize, Serialize};

use crate::custom::muse_from;
use crate::oklch::lerp_hsla;

/// Which of the two palettes is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    /// Warm light palette.
    #[default]
    Paper,
    /// Warm dark palette.
    Dusk,
}

impl Appearance {
    /// The palette for this appearance.
    #[must_use]
    pub fn tokens(self) -> Tokens {
        match self {
            Appearance::Paper => paper(),
            Appearance::Dusk => dusk(),
        }
    }

    /// The other appearance — what the theme toggle switches to.
    #[must_use]
    pub fn toggled(self) -> Appearance {
        match self {
            Appearance::Paper => Appearance::Dusk,
            Appearance::Dusk => Appearance::Paper,
        }
    }
}

/// The complete color vocabulary of Muse. All UI reads these; nothing reads
/// hex literals. Tokens are plain colors so the whole set can be
/// interpolated for the animated theme crossfade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Window background.
    pub bg: Hsla,
    /// Raised surfaces: cards, popovers, pills.
    pub surface: Hsla,
    /// Surfaces that float above other surfaces. In Paper this equals
    /// `surface` (elevation comes from `shadow`); in Dusk it is one step
    /// lighter, since Dusk elevates with lifted surfaces + hairlines
    /// instead of shadows.
    pub surface_lifted: Hsla,
    /// Primary text.
    pub ink: Hsla,
    /// Secondary text: labels, inactive controls.
    pub ink_secondary: Hsla,
    /// Tertiary text: date labels, placeholders, relative times.
    pub ink_tertiary: Hsla,
    /// 1px borders and dividers.
    pub hairline: Hsla,
    /// The one accent — stamp crimson. Caret, primary actions, selection base.
    pub accent: Hsla,
    /// Text-selection highlight (accent with baked-in alpha).
    pub selection: Hsla,
    /// The muse's lavender — orb, margin notes, response blocks.
    pub muse: Hsla,
    /// Moss green — the fourth selectable ink color.
    pub moss: Hsla,
    /// Shadow color (alpha baked in) for the soft two-layer elevation
    /// shadow. Fully transparent in Dusk by design.
    pub shadow: Hsla,
}

impl Tokens {
    /// The four curated swatches for range-level text color, in display
    /// order: default ink, crimson accent, lavender muse, moss green.
    #[must_use]
    pub fn ink_palette(&self) -> [Hsla; 4] {
        [self.ink, self.accent, self.muse, self.moss]
    }
}

/// An opaque color from a `0xRRGGBB` literal.
fn hx(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// A color from a `0xRRGGBB` literal with an explicit alpha.
fn hxa(hex: u32, alpha: f32) -> Hsla {
    let mut rgba = rgb(hex);
    rgba.a = alpha;
    Hsla::from(rgba)
}

/// The Paper (light) palette, exactly as specified in PLAN §8 — except the
/// muse ink, which is now derived from the accent (one hue story per theme).
#[must_use]
pub fn paper() -> Tokens {
    let accent = hx(0xD7263D);
    let bg = hx(0xFAF8F5);
    let ink = hx(0x26221C);
    Tokens {
        bg,
        surface: hx(0xFFFFFF),
        surface_lifted: hx(0xFFFFFF),
        ink,
        ink_secondary: hx(0x6F6A61),
        ink_tertiary: hx(0xA8A296),
        hairline: hx(0xECE8E1),
        accent,
        selection: accent.alpha(0.18),
        muse: muse_from(accent, bg, ink),
        moss: hx(0x5F7A5A),
        shadow: hxa(0x1C1914, 0.06),
    }
}

/// The Dusk (dark) palette, exactly as specified in PLAN §8.
#[must_use]
pub fn dusk() -> Tokens {
    let accent = hx(0xE4485C);
    let bg = hx(0x171512);
    let ink = hx(0xEDE9E2);
    Tokens {
        bg,
        surface: hx(0x1F1C18),
        surface_lifted: hx(0x2A2622),
        ink,
        ink_secondary: hx(0xA39D92),
        ink_tertiary: hx(0x6E695F),
        hairline: hx(0x2A2722),
        accent,
        selection: accent.alpha(0.22),
        muse: muse_from(accent, bg, ink),
        moss: hx(0x8FAE89),
        // Dusk elevates with lifted surfaces + hairlines, not shadows
        // (PLAN §8); same shadow hue at zero alpha keeps the crossfade a
        // pure alpha fade.
        shadow: hxa(0x1C1914, 0.0),
    }
}

/// Interpolate every token between two palettes in OKLCH. `t = 0.0` yields
/// `a`, `t = 1.0` yields `b`. This powers the 240ms theme crossfade: each
/// frame stores `lerp_tokens(&from, &to, eased_t)` into the [`Theme`]
/// global and every view repaints from it.
#[must_use]
pub fn lerp_tokens(a: &Tokens, b: &Tokens, t: f32) -> Tokens {
    Tokens {
        bg: lerp_hsla(a.bg, b.bg, t),
        surface: lerp_hsla(a.surface, b.surface, t),
        surface_lifted: lerp_hsla(a.surface_lifted, b.surface_lifted, t),
        ink: lerp_hsla(a.ink, b.ink, t),
        ink_secondary: lerp_hsla(a.ink_secondary, b.ink_secondary, t),
        ink_tertiary: lerp_hsla(a.ink_tertiary, b.ink_tertiary, t),
        hairline: lerp_hsla(a.hairline, b.hairline, t),
        accent: lerp_hsla(a.accent, b.accent, t),
        selection: lerp_hsla(a.selection, b.selection, t),
        muse: lerp_hsla(a.muse, b.muse, t),
        moss: lerp_hsla(a.moss, b.moss, t),
        shadow: lerp_hsla(a.shadow, b.shadow, t),
    }
}

/// The app-wide theme, stored as a GPUI global. `tokens` usually equals
/// `appearance.tokens()`, except mid-crossfade when it holds interpolated
/// values.
#[derive(Clone, Debug)]
pub struct Theme {
    /// The active (or, mid-crossfade, the target) appearance.
    pub appearance: Appearance,
    /// The colors every view paints with this frame.
    pub tokens: Tokens,
}

impl Theme {
    /// A theme resting at the given appearance's palette.
    #[must_use]
    pub fn new(appearance: Appearance) -> Self {
        Theme {
            appearance,
            tokens: appearance.tokens(),
        }
    }
}

impl Global for Theme {}

/// Read the current theme from any context that dereferences to
/// [`gpui::App`] — views do `cx.theme().tokens.bg`.
pub trait ActiveTheme {
    /// The current [`Theme`] global.
    ///
    /// Panics if the app has not set the `Theme` global yet; the
    /// composition root installs it before the first window opens.
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Rgba;

    fn close(a: Hsla, b: Hsla) -> bool {
        let a = Rgba::from(a);
        let b = Rgba::from(b);
        (a.r - b.r).abs() < 2e-3
            && (a.g - b.g).abs() < 2e-3
            && (a.b - b.b).abs() < 2e-3
            && (a.a - b.a).abs() < 2e-3
    }

    #[test]
    fn palettes_match_plan_hex_values() {
        let p = paper();
        assert!(close(p.bg, hx(0xFAF8F5)));
        assert!(close(p.ink, hx(0x26221C)));
        assert!(close(p.accent, hx(0xD7263D)));
        assert!((p.selection.a - 0.18).abs() < 1e-4);

        let d = dusk();
        assert!(close(d.bg, hx(0x171512)));
        assert!(close(d.ink, hx(0xEDE9E2)));
        assert!(close(d.accent, hx(0xE4485C)));
        assert!((d.selection.a - 0.22).abs() < 1e-4);
        assert!((d.shadow.a).abs() < 1e-4);
    }

    #[test]
    fn ink_palette_order_is_ink_rose_lavender_moss() {
        let p = paper();
        let [ink, rose, lavender, moss] = p.ink_palette();
        assert!(close(ink, p.ink));
        assert!(close(rose, p.accent));
        assert!(close(lavender, p.muse));
        assert!(close(moss, p.moss));
    }

    #[test]
    fn lerp_tokens_endpoints_round_trip() {
        let a = paper();
        let b = dusk();
        let at_zero = lerp_tokens(&a, &b, 0.0);
        let at_one = lerp_tokens(&a, &b, 1.0);
        assert!(close(at_zero.bg, a.bg) && close(at_zero.muse, a.muse));
        assert!(close(at_one.bg, b.bg) && close(at_one.muse, b.muse));
    }

    #[test]
    fn midpoint_stays_warm_not_gray() {
        // The crossfade midpoint of the two backgrounds should keep a
        // little warm chroma rather than collapsing to a neutral gray.
        let mid = lerp_tokens(&paper(), &dusk(), 0.5).bg;
        let rgba = Rgba::from(mid);
        assert!(rgba.r > rgba.b, "midpoint lost its warmth: {rgba:?}");
    }

    #[test]
    fn appearance_toggles() {
        assert_eq!(Appearance::Paper.toggled(), Appearance::Dusk);
        assert_eq!(Appearance::Dusk.toggled(), Appearance::Paper);
        assert_eq!(Appearance::default(), Appearance::Paper);
    }
}
