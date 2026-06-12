//! Floating overlays: the format pill that blooms above a settled
//! selection, and the margin-note card. Both render through
//! `deferred(anchored(...))` so they never affect layout — nothing shifts.

use gpui::{
    Action, Animation, AnimationExt as _, AnyElement, Context, Corner, Div, FontWeight,
    MouseButton, SharedString, Window, anchored, deferred, div, point, prelude::*, px,
};
use muse_core::{Ink, InlineStyle};
use muse_theme::{ActiveTheme, Tokens, fonts, layout as metrics, motion};
use muse_ui::{IconName, card, icon, pill};

use crate::notes::{AnnotationTone, reveal_prefix};
use crate::{Editor, notes};

/// Half of the pill's approximate width, used to center it over the
/// selection before its real layout is known.
const PILL_HALF_W: f32 = 150.0;

/// Build the format pill overlay, if it should be visible.
pub(crate) fn render_pill(
    editor: &Editor,
    _window: &mut Window,
    cx: &mut Context<Editor>,
) -> Option<AnyElement> {
    if !editor.pill_shown {
        return None;
    }
    let range = editor.sel.range();
    if range.is_empty() {
        return None;
    }
    let snap = editor.snapshot.clone()?;
    let tokens = cx.theme().tokens;
    let palette = tokens.ink_palette();

    let tiles = editor.doc.spans().runs_in(range.clone());
    let entire = |get: fn(&InlineStyle) -> bool| -> bool {
        !tiles.is_empty() && tiles.iter().all(|(_, style)| get(style))
    };
    let bold = entire(|s| s.bold);
    let italic = entire(|s| s.italic);
    let underline = entire(|s| s.underline);
    let strike = entire(|s| s.strike);
    // The selection's uniform ink, if it has one.
    let uniform_ink: Option<Option<Ink>> = tiles
        .first()
        .map(|(_, style)| style.ink)
        .filter(|first| tiles.iter().all(|(_, style)| style.ink == *first));

    let (start_x, start_y) = snap.caret_point(range.start);
    let anchor_at = snap.to_window((start_x, start_y));
    let position = point(anchor_at.x - px(PILL_HALF_W), anchor_at.y - px(10.0));

    let ink_active = |ink: Option<Ink>| uniform_ink == Some(ink);
    let style_row = div()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(glyph_toggle("pill-bold", "B", bold, muse_commands::Bold, &tokens, |el| {
            el.font_weight(FontWeight::BOLD)
        }))
        .child(glyph_toggle("pill-italic", "I", italic, muse_commands::Italic, &tokens, |el| {
            el.italic()
        }))
        .child(glyph_toggle(
            "pill-underline",
            "U",
            underline,
            muse_commands::Underline,
            &tokens,
            |el| el.underline(),
        ))
        .child(glyph_toggle(
            "pill-strike",
            "S",
            strike,
            muse_commands::Strikethrough,
            &tokens,
            |el| el.line_through(),
        ))
        .child(seperator(&tokens))
        .child(ink_dot(0, palette[0], ink_active(None), &tokens))
        .child(ink_dot(1, palette[1], ink_active(Some(Ink::Rose)), &tokens))
        .child(ink_dot(2, palette[2], ink_active(Some(Ink::Lavender)), &tokens))
        .child(ink_dot(3, palette[3], ink_active(Some(Ink::Moss)), &tokens))
        .child(seperator(&tokens))
        .child(clear_ink_button(&tokens));

    let body = pill().child(
        div()
            .flex()
            .flex_col()
            .child(style_row.pb(px(3.)))
            .child(
                div()
                    .h(px(1.))
                    .mx(px(2.))
                    .bg(tokens.hairline.opacity(0.6)),
            )
            .child(voice_row(editor.doc.voice(), &tokens).pt(px(3.))),
    );

    let bloom = div().occlude().child(body).with_animation(
        "pill-bloom",
        Animation::new(motion::FADE).with_easing(motion::ease_out_quint),
        |el, t| el.opacity(t).mt(px(3.0 * (1.0 - t))),
    );

    Some(
        deferred(
            anchored()
                .position(position)
                .anchor(Corner::BottomLeft)
                .snap_to_window_with_margin(px(8.0))
                .child(bloom),
        )
        .with_priority(1)
        .into_any_element(),
    )
}

/// One of the B/I/U/S toggles: the glyph rendered in its own format.
fn glyph_toggle<A: Action + Clone>(
    id: &'static str,
    label: &'static str,
    active: bool,
    action: A,
    tokens: &Tokens,
    format: impl FnOnce(Div) -> Div,
) -> impl IntoElement {
    let hover_bg = tokens.hairline.opacity(0.6);
    let color = if active { tokens.accent } else { tokens.ink_secondary };
    let shell = format(
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(22.0))
            .h(px(22.0))
            .rounded(px(metrics::RADIUS_SM))
            .text_size(px(metrics::UI_SMALL))
            .font_family(fonts::FONT_UI)
            .text_color(color),
    );
    shell
        .id(id)
        .cursor_pointer()
        .when(active, |el| el.bg(tokens.hairline.opacity(0.5)))
        .hover(move |style| style.bg(hover_bg))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
        .child(label)
}

/// One of the four ink swatches; a quiet ring marks the active one.
fn ink_dot(index: usize, color: gpui::Hsla, active: bool, tokens: &Tokens) -> impl IntoElement {
    let ring = tokens.ink_secondary;
    div()
        .id(("pill-ink", index))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(16.0))
        .h(px(16.0))
        .rounded_full()
        .cursor_pointer()
        .when(active, |el| el.border_1().border_color(ring))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            window.dispatch_action(
                Box::new(muse_commands::SetInk {
                    ink: Some(index as u8),
                }),
                cx,
            );
        })
        .child(div().w(px(10.0)).h(px(10.0)).rounded_full().bg(color))
}

/// The X that clears ink back to the default. The icon carries an explicit
/// tint (gpui svg elements never inherit the parent's text color).
fn clear_ink_button(tokens: &Tokens) -> impl IntoElement {
    let hover_bg = tokens.hairline.opacity(0.6);
    div()
        .id("pill-clear-ink")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(20.0))
        .h(px(22.0))
        .rounded(px(metrics::RADIUS_SM))
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.dispatch_action(Box::new(muse_commands::SetInk { ink: None }), cx);
        })
        .child(icon(IconName::X).size(px(11.0)).color(tokens.ink_tertiary))
}

/// The pill's second row: the entry voice — family, size, weight.
fn voice_row(voice: muse_core::Voice, tokens: &Tokens) -> Div {
    use muse_core::FontFamily;

    let families = [
        (0u8, FontFamily::Literata, fonts::FONT_SERIF),
        (1, FontFamily::Inter, fonts::FONT_SANS),
        (2, FontFamily::Quattro, fonts::FONT_QUATTRO),
        (3, FontFamily::Mono, fonts::FONT_MONO),
    ];
    let weights = [300u16, 400, 700];

    let mut row = div().flex().items_center().gap(px(1.));
    for (index, family, font_name) in families {
        let active = voice.family == family;
        let hover_bg = tokens.hairline.opacity(0.6);
        row = row.child(
            div()
                .id(("pill-family", index as usize))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(24.0))
                .h(px(22.0))
                .rounded(px(metrics::RADIUS_SM))
                .text_size(px(metrics::UI_SMALL))
                .font_family(font_name)
                .text_color(if active { tokens.accent } else { tokens.ink_tertiary })
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.dispatch_action(
                        Box::new(muse_commands::SetFamily { family: index }),
                        cx,
                    );
                })
                .child("Aa"),
        );
    }

    row = row
        .child(seperator(tokens))
        .child(size_step("pill-size-down", "−", false, tokens))
        .child(
            div()
                .flex_none()
                .w(px(18.0))
                .text_size(px(metrics::UI_SMALL))
                .font_family(fonts::FONT_UI)
                .text_color(tokens.ink_secondary)
                .text_center()
                .child(SharedString::from(format!("{:.0}", voice.size))),
        )
        .child(size_step("pill-size-up", "+", true, tokens))
        .child(seperator(tokens));

    for weight in weights {
        let active = voice.weight == weight;
        let hover_bg = tokens.hairline.opacity(0.6);
        row = row.child(
            div()
                .id(("pill-weight", weight as usize))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(20.0))
                .h(px(22.0))
                .rounded(px(metrics::RADIUS_SM))
                .text_size(px(metrics::UI_SMALL))
                .font_family(fonts::FONT_UI)
                .font_weight(FontWeight(f32::from(weight)))
                .text_color(if active { tokens.accent } else { tokens.ink_tertiary })
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window
                        .dispatch_action(Box::new(muse_commands::SetWeight { weight }), cx);
                })
                .child("A"),
        );
    }
    row
}

/// One of the −/+ size steppers.
fn size_step(id: &'static str, label: &'static str, up: bool, tokens: &Tokens) -> impl IntoElement {
    let hover_bg = tokens.hairline.opacity(0.6);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(18.0))
        .h(px(22.0))
        .rounded(px(metrics::RADIUS_SM))
        .text_size(px(metrics::UI_SMALL))
        .font_family(fonts::FONT_UI)
        .text_color(tokens.ink_tertiary)
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if up {
                window.dispatch_action(Box::new(muse_commands::IncreaseSize), cx);
            } else {
                window.dispatch_action(Box::new(muse_commands::DecreaseSize), cx);
            }
        })
        .child(label)
}

fn seperator(tokens: &Tokens) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(1.0))
        .h(px(12.0))
        .mx(px(4.0))
        .bg(tokens.hairline.opacity(0.7))
}

/// Build the margin-note card overlay (open card or a dismissed card
/// mid-recede), if any.
pub(crate) fn render_card(
    editor: &Editor,
    _window: &mut Window,
    cx: &mut Context<Editor>,
) -> Option<AnyElement> {
    let tokens = cx.theme().tokens;

    // A dismissed card recedes in place, inert.
    if let Some(closing) = &editor.closing_card {
        let shell = div()
            .max_w(px(280.0))
            .child(card_content(
                closing.tone,
                closing.body.clone(),
                None::<Div>,
                &tokens,
            ))
            .with_animation(
                ("card-recede", closing.id as usize),
                Animation::new(motion::FADE).with_easing(motion::ease_out_quint),
                |el, t| el.opacity(1.0 - t),
            );
        return Some(
            deferred(
                anchored()
                    .position(closing.position + point(px(-6.0), px(-14.0)))
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.0))
                    .child(shell),
            )
            .with_priority(2)
            .into_any_element(),
        );
    }

    let open = editor.card.as_ref()?;
    let slot = editor.notes.iter().find(|slot| slot.ann.id == open.id)?;
    let center = slot.last_center?;
    let snap = editor.snapshot.clone()?;
    let marker_at = snap.to_window(center);
    // The card blooms above the citation caret (never covering the quoted
    // line), overlapping the marker slightly so the pointer can travel
    // marker → card without a dead gap closing it.
    let position = point(marker_at.x - px(14.0), marker_at.y - px(4.0));

    let id = open.id;
    let hover_ink = tokens.ink;
    let dismiss = div()
        .id(("note-dismiss", id as usize))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(18.0))
        .h(px(18.0))
        .rounded(px(metrics::RADIUS_SM))
        .text_color(tokens.ink_tertiary)
        .cursor_pointer()
        .hover(move |style| style.text_color(hover_ink))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |editor, _: &gpui::MouseDownEvent, _window, cx| {
                editor.dismiss_note(id, cx);
            }),
        )
        .child(icon(IconName::X).size(px(10.0)));

    let shell = div()
        .id(("note-card", id as usize))
        .occlude()
        .max_w(px(280.0))
        .on_hover(cx.listener(|editor, hovered: &bool, _window, cx| {
            editor.set_card_hovered(*hovered, cx);
        }))
        .child(card_content(slot.ann.tone, slot.ann.body.clone(), Some(dismiss), &tokens))
        .with_animation(
            ("card-bloom", id as usize),
            Animation::new(motion::NOTE_BLOOM).with_easing(motion::ease_out_quint),
            |el, t| el.opacity(t).mt(px(3.0 * (1.0 - t))),
        );

    Some(
        deferred(
            anchored()
                .position(position)
                .anchor(Corner::BottomLeft)
                .snap_to_window_with_margin(px(8.0))
                .child(shell),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

/// The card itself: a taped scrap — a tape strip across the top edge, a
/// typewriter tone label in accent, an optional dismiss, and the body
/// revealing word by word.
fn card_content(
    tone: AnnotationTone,
    body: SharedString,
    dismiss: Option<impl IntoElement>,
    tokens: &Tokens,
) -> impl IntoElement {
    // The tape strip: a short translucent accent band riding the card's
    // top edge, centered, like a scrap taped into the margin.
    let tape = div()
        .flex()
        .flex_none()
        .justify_center()
        .mt(px(-10.0))
        .mb(px(2.0))
        .child(
            div()
                .w(px(46.0))
                .h(px(8.0))
                .rounded(px(2.0))
                .bg(tokens.accent.alpha(0.18)),
        );

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
            div()
                .text_size(px(metrics::UI_SMALL))
                .font_family(fonts::FONT_MONO)
                .text_color(tokens.accent)
                .child(tone.label()),
        )
        .children(dismiss);

    let ink = tokens.ink;
    let reveal = div()
        .mt(px(4.0))
        .text_size(px(metrics::UI_TEXT))
        .font_family(fonts::FONT_UI)
        .text_color(ink)
        .with_animation(
            "card-reveal",
            Animation::new(notes::REVEAL),
            move |el, t| {
                el.child(SharedString::from(reveal_prefix(&body, t).to_string()))
            },
        );

    card().child(tape).child(header).child(reveal)
}
