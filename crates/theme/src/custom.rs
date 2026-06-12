//! Custom themes: hex parsing, three-color token derivation, the active
//! light/dark token pair, and the bundled presets.
//!
//! A whole palette is derived from just an accent, a background, and a
//! foreground using the OKLCH machinery in [`crate::oklch`], so every
//! custom theme inherits the same warmth rules as Paper and Dusk.

use gpui::{Hsla, Rgba, rgb};

use crate::oklch::{lerp_hsla, lightness, with_chroma_scaled, with_hue_rotated, with_lightness};
use crate::tokens::{Appearance, Tokens, dusk, paper};

/// A custom theme is "light" when its background's OKLCH lightness clears
/// this bar; this picks the selection alpha and the shadow treatment.
const LIGHT_BG_LIGHTNESS: f32 = 0.6;

/// Parse `#RGB` or `#RRGGBB` (case-insensitive, leading `#` optional) into
/// an opaque color. Returns `None` for anything else.
#[must_use]
pub fn hsla_from_hex(s: &str) -> Option<Hsla> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let (r, g, b) = match s.len() {
        3 => {
            let mut it = s.chars().map(|c| {
                let d = c.to_digit(16).unwrap_or(0);
                (d * 16 + d) as f32 / 255.0
            });
            // Length is exactly 3, so all three pulls succeed.
            (it.next()?, it.next()?, it.next()?)
        }
        6 => {
            let byte = |i: usize| {
                u8::from_str_radix(s.get(i..i + 2).unwrap_or("00"), 16)
                    .map(|b| f32::from(b) / 255.0)
            };
            (byte(0).ok()?, byte(2).ok()?, byte(4).ok()?)
        }
        _ => return None,
    };
    Some(Rgba { r, g, b, a: 1.0 }.into())
}

/// Format an opaque color as `#RRGGBB` (alpha is ignored).
#[must_use]
pub fn hex_from_hsla(c: Hsla) -> String {
    let rgba = Rgba::from(c);
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02X}{:02X}{:02X}",
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b)
    )
}

/// Derive a full token set from three colors: the accent, the window
/// background, and the primary ink. Everything else is mixed, nudged, or
/// hue-rotated from those in OKLCH, following the same scheme as Paper and
/// Dusk (shadows only on light backgrounds, lifted surfaces on dark ones).
#[must_use]
pub fn derive_tokens(accent: Hsla, bg: Hsla, fg: Hsla) -> Tokens {
    let light = lightness(bg) >= LIGHT_BG_LIGHTNESS;
    let bg_l = lightness(bg);
    // Surfaces step away from the foreground's side of the lightness axis.
    let step = if light { 0.018 } else { 0.035 };
    let surface = with_lightness(bg, bg_l + step);
    let surface_lifted = with_lightness(bg, bg_l + 2.0 * step);

    Tokens {
        bg,
        surface,
        surface_lifted,
        ink: fg,
        ink_secondary: lerp_hsla(bg, fg, 0.55),
        ink_tertiary: lerp_hsla(bg, fg, 0.35),
        hairline: fg.alpha(0.12),
        accent,
        selection: accent.alpha(if light { 0.18 } else { 0.22 }),
        muse: muse_from(accent, bg, fg),
        moss: with_chroma_scaled(with_hue_rotated(accent, -80.0), 0.6),
        shadow: {
            let mut shadow = rgb(0x1C1914);
            shadow.a = if light { 0.06 } else { 0.0 };
            Hsla::from(shadow)
        },
    }
}

/// Muse's annotation ink: a close kin of the accent — same family, a
/// whisper of hue drift and slightly softer chroma — kept legible against
/// the page. One hue story per theme; the margin never clashes with the
/// caret or the selection.
#[must_use]
pub fn muse_from(accent: Hsla, bg: Hsla, fg: Hsla) -> Hsla {
    legible_against(
        with_chroma_scaled(with_hue_rotated(accent, 14.0), 0.92),
        bg,
        fg,
    )
}

/// Annotation ink must stay readable on the page: when `color` sits within
/// 0.28 OKLCH lightness of the background (a gray accent, a black-on-black
/// rotation), pull it toward the foreground until it clears the bar.
fn legible_against(color: Hsla, bg: Hsla, fg: Hsla) -> Hsla {
    const MIN_CONTRAST_L: f32 = 0.28;
    if (lightness(color) - lightness(bg)).abs() >= MIN_CONTRAST_L {
        return color;
    }
    let target = if lightness(fg) > lightness(bg) {
        (lightness(bg) + MIN_CONTRAST_L + 0.08).min(1.0)
    } else {
        (lightness(bg) - MIN_CONTRAST_L - 0.08).max(0.0)
    };
    with_lightness(color, target)
}

/// The light and dark palettes the app is currently dressed in. The theme
/// toggle crossfades within this pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemePair {
    /// Tokens used in the Paper (light) appearance.
    pub light: Tokens,
    /// Tokens used in the Dusk (dark) appearance.
    pub dark: Tokens,
}

impl ThemePair {
    /// The pair's palette for the given appearance.
    #[must_use]
    pub fn tokens_for(&self, appearance: Appearance) -> Tokens {
        match appearance {
            Appearance::Paper => self.light,
            Appearance::Dusk => self.dark,
        }
    }
}

impl Default for ThemePair {
    fn default() -> Self {
        ThemePair {
            light: paper(),
            dark: dusk(),
        }
    }
}

/// A named light/dark pair offered in the settings pane.
pub struct ThemePreset {
    /// Display name, also the persisted `theme.preset` value.
    pub name: &'static str,
    /// Light-appearance tokens.
    pub light: Tokens,
    /// Dark-appearance tokens.
    pub dark: Tokens,
}

impl ThemePreset {
    /// The preset as a [`ThemePair`].
    #[must_use]
    pub fn pair(&self) -> ThemePair {
        ThemePair {
            light: self.light,
            dark: self.dark,
        }
    }
}

/// A derived preset from six hex literals: light accent/bg/fg, then dark.
fn derived(name: &'static str, l: [u32; 3], d: [u32; 3]) -> ThemePreset {
    let c = |hex: u32| Hsla::from(rgb(hex));
    ThemePreset {
        name,
        light: derive_tokens(c(l[0]), c(l[1]), c(l[2])),
        dark: derive_tokens(c(d[0]), c(d[1]), c(d[2])),
    }
}

/// The bundled presets: the Paper & Dusk defaults plus four bold,
/// magazine-leaning pairs — saturated accents, decisive backgrounds, no
/// pastel hedging. Order is display order.
#[must_use]
pub fn presets() -> Vec<ThemePreset> {
    vec![
        ThemePreset {
            name: "Paper & Dusk",
            light: paper(),
            dark: dusk(),
        },
        // OpenAI Codex's actual palette: blue #0285FF on white with
        // #0D0D0D ink; #339CFF on #181818 with white ink by night.
        derived(
            "Codex",
            [0x0285FF, 0xFFFFFF, 0x0D0D0D],
            [0x339CFF, 0x181818, 0xFFFFFF],
        ),
        // Black on white, white on black. Editorial, no color at all —
        // the boldest move is restraint.
        derived(
            "Magazine",
            [0x111111, 0xFFFFFF, 0x111111],
            [0xFAFAFA, 0x0A0A0A, 0xFAFAFA],
        ),
        // The default crimson pushed onto warm paper neutrals — a postage
        // stamp pressed onto the page.
        derived(
            "Stamp",
            [0xD7263D, 0xFAF8F5, 0x26221C],
            [0xE4485C, 0x171512, 0xEDE9E2],
        ),
        // Loud cobalt on cool paper; blueprint by day, photocopier glow
        // by night.
        derived(
            "Scan",
            [0x2244CC, 0xF7F8FC, 0x14182B],
            [0x4D6FFF, 0x05070F, 0xE6EAF7],
        ),
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn finite(c: Hsla) {
        let rgba = Rgba::from(c);
        for v in [rgba.r, rgba.g, rgba.b, rgba.a] {
            assert!(v.is_finite() && (0.0..=1.0).contains(&v), "channel {v}");
        }
    }

    #[test]
    fn hex_parses_all_accepted_shapes() {
        for s in ["#B86450", "b86450", "#b86450", "B86450"] {
            let c = hsla_from_hex(s).unwrap();
            assert_eq!(hex_from_hsla(c), "#B86450");
        }
        let short = hsla_from_hex("#fa0").unwrap();
        assert_eq!(hex_from_hsla(short), "#FFAA00");
    }

    #[test]
    fn hex_rejects_garbage() {
        for s in ["", "#", "#12", "#12345", "#1234567", "zzz", "#gg0011"] {
            assert!(hsla_from_hex(s).is_none(), "accepted {s:?}");
        }
    }

    #[test]
    fn hex_round_trips() {
        for hex in ["#FAF8F5", "#171512", "#B86450", "#5F7A5A", "#000000", "#FFFFFF"] {
            let c = hsla_from_hex(hex).unwrap();
            assert_eq!(hex_from_hsla(c), *hex);
        }
    }

    #[test]
    fn derive_from_paper_inputs_is_sane() {
        let p = paper();
        let t = derive_tokens(p.accent, p.bg, p.ink);
        for c in [
            t.bg,
            t.surface,
            t.surface_lifted,
            t.ink,
            t.ink_secondary,
            t.ink_tertiary,
            t.hairline,
            t.accent,
            t.selection,
            t.muse,
            t.moss,
            t.shadow,
        ] {
            finite(c);
        }
        assert!((t.selection.a - 0.18).abs() < 1e-4);
        assert!((t.hairline.a - 0.12).abs() < 1e-4);
        assert!((t.shadow.a - 0.06).abs() < 1e-4);
    }

    #[test]
    fn dark_backgrounds_pick_dark_scheme() {
        let d = dusk();
        let t = derive_tokens(d.accent, d.bg, d.ink);
        assert!((t.selection.a - 0.22).abs() < 1e-4);
        assert!(t.shadow.a.abs() < 1e-4);
    }

    #[test]
    fn presets_are_finite_and_distinctly_named() {
        let presets = presets();
        assert_eq!(presets.len(), 5);
        let mut names: Vec<_> = presets.iter().map(|p| p.name).collect();
        names.dedup();
        assert_eq!(names.len(), 5);
        for preset in &presets {
            for tokens in [preset.light, preset.dark] {
                finite(tokens.bg);
                finite(tokens.accent);
                finite(tokens.muse);
            }
        }
    }

    #[test]
    fn theme_pair_defaults_to_paper_and_dusk() {
        let pair = ThemePair::default();
        assert_eq!(pair.tokens_for(Appearance::Paper), paper());
        assert_eq!(pair.tokens_for(Appearance::Dusk), dusk());
    }
}
