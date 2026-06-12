//! Floating containers: the selection-toolbar pill and the margin-note
//! card. Both elevate the way the active palette wants — Paper casts the
//! soft two-layer shadow, Dusk lifts the surface and keeps the hairline
//! (its shadow token is transparent, so the same recipe renders both).

use gpui::{AnyElement, App, BoxShadow, Window, div, point, prelude::*, px};
use muse_theme::{ActiveTheme, Tokens, layout};
use smallvec::SmallVec;

/// The soft two-layer elevation shadow from PLAN §8:
/// `0 1px 3px` at the token's alpha plus `0 8px 24px` slightly fainter.
/// Renders as nothing when `tokens.shadow` is transparent (Dusk).
#[must_use]
pub fn soft_shadow(tokens: &Tokens) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: tokens.shadow,
            offset: point(px(0.), px(1.)),
            blur_radius: px(3.),
            spread_radius: px(0.),
        },
        // PLAN specifies .06 and .05 alphas; the second layer is the token
        // at 5/6 strength so both follow the token during crossfades.
        BoxShadow {
            color: tokens.shadow.opacity(5.0 / 6.0),
            offset: point(px(0.), px(8.)),
            blur_radius: px(24.),
            spread_radius: px(0.),
        },
    ]
}

/// The floating pill: a horizontal capsule for the selection toolbar.
/// Surface background, large radius, hairline border, soft shadow.
#[derive(IntoElement)]
pub struct Pill {
    children: SmallVec<[AnyElement; 2]>,
}

/// Build an empty [`Pill`]; add content with
/// [`ParentElement::child`]/[`children`](ParentElement::children).
pub fn pill() -> Pill {
    Pill {
        children: SmallVec::new(),
    }
}

impl ParentElement for Pill {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Pill {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.theme().tokens;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .px(px(6.))
            .py(px(4.))
            .bg(tokens.surface_lifted)
            .border_1()
            .border_color(tokens.hairline)
            .rounded(px(layout::RADIUS_LG))
            .shadow(soft_shadow(&tokens))
            .children(self.children)
    }
}

/// The margin-note card shell: a vertical surface with medium radius,
/// hairline border, and the soft shadow.
#[derive(IntoElement)]
pub struct Card {
    children: SmallVec<[AnyElement; 2]>,
}

/// Build an empty [`Card`]; add content with
/// [`ParentElement::child`]/[`children`](ParentElement::children).
pub fn card() -> Card {
    Card {
        children: SmallVec::new(),
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Card {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.theme().tokens;
        // The scrap sits a touch heavier than the pill: the soft shadow at
        // 4/3 strength (still nothing in Dusk, whose token is transparent)
        // and an ink-based hairline instead of the surface hairline.
        let shadow = soft_shadow(&tokens)
            .into_iter()
            .map(|mut layer| {
                layer.color = layer.color.alpha((layer.color.a * 4.0 / 3.0).min(1.0));
                layer
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .p(px(12.))
            .bg(tokens.surface_lifted)
            .border_1()
            .border_color(tokens.ink.alpha(0.10))
            .rounded(px(layout::RADIUS_MD))
            .shadow(shadow)
            .children(self.children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muse_theme::{dusk, paper};

    #[test]
    fn soft_shadow_layers_follow_the_token() {
        let layers = soft_shadow(&paper());
        assert_eq!(layers.len(), 2);
        assert!((layers[0].color.a - 0.06).abs() < 1e-3);
        assert!((layers[1].color.a - 0.05).abs() < 1e-3);
    }

    #[test]
    fn dusk_shadow_is_invisible() {
        for layer in soft_shadow(&dusk()) {
            assert!(layer.color.a.abs() < 1e-4);
        }
    }
}
