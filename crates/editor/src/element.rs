//! The one custom element: lays out and paints the date label, paragraphs,
//! selection, caret, margin dots, and coda, owns vertical scrolling, and
//! registers IME + mouse handling. Generalizes gpui's `examples/input.rs`
//! from a single shaped line to cached multi-paragraph wrapped text.

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Context, Corners, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, Font, FontFeatures, FontStyle, FontWeight, GlobalElementId,
    Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent, SharedString, Style,
    TextAlign, TextRun, Window, WrappedLine, fill, point, px, quad, relative, size,
};
use muse_core::{InlineStyle, Voice};
use muse_theme::{ActiveTheme, fonts, layout as metrics, motion};

use crate::layout::{CodaSnap, LINE_HEIGHT_FACTOR, SnapDot, SnapPara, Snapshot};
use crate::notes::{self, NoteSlot};
use crate::{Editor, EditorEvent, anim, runs};
/// Minimum horizontal margin between the column and the window edge.
const H_MARGIN: f32 = 24.0;
/// Breathing room below the last line when scrolled to the bottom.
const BOTTOM_PAD: f32 = 160.0;
/// Keep the caret at least this far from the viewport edges on autoscroll.
const SCROLL_MARGIN: f32 = 48.0;
/// Drag-selection autoscroll engages within this distance of an edge.
const DRAG_EDGE: f32 = 28.0;
/// Duration of the reaction pop — louder and longer than the quiet bloom.
const REACT_POP: std::time::Duration = std::time::Duration::from_millis(520);
/// Content y of the date label (inside the fixed 96px top pad).
const DATE_Y: f32 = 44.0;
/// Vertical gap above the coda divider and between divider and body.
const CODA_GAP: f32 = 28.0;

/// The editor's content element. Fills its container; everything inside is
/// painted by hand from the entity's state.
pub(crate) struct EditorElement {
    editor: Entity<Editor>,
}

impl EditorElement {
    pub(crate) fn new(editor: Entity<Editor>) -> Self {
        Self { editor }
    }
}

/// Hitboxes from prepaint: the whole element (scroll + mouse) and the text
/// column (ibeam cursor).
pub(crate) struct EditorPrepaint {
    hitbox: Hitbox,
    column: Hitbox,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.editor
            .update(cx, |editor, cx| editor.layout_pass(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.editor
            .update(cx, |editor, cx| editor.paint_pass(bounds, prepaint, window, cx));
    }
}

impl Editor {
    /// Layout: shape dirty paragraphs (and only those), stack heights,
    /// clamp scroll, service autoscroll, advance the caret spring, place
    /// margin dots, and publish the snapshot.
    pub(crate) fn layout_pass(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EditorPrepaint {
        let tokens = cx.theme().tokens;
        let voice = self.doc.voice();
        let line_height = voice.size * LINE_HEIGHT_FACTOR;
        let width = f32::from(bounds.size.width);
        let viewport_h = f32::from(bounds.size.height);
        let wrap = (width - 2.0 * H_MARGIN).clamp(120.0, metrics::COLUMN_MAX_W);
        let column_x = ((width - wrap) / 2.0).max(H_MARGIN);
        let palette = tokens.ink_palette();
        let gsig = global_sig(voice, wrap, palette);

        // PERFORMANCE INVARIANT: only paragraphs whose signature (text was
        // already diffed in `reuse_paragraphs`; this covers styles, voice,
        // width, palette, IME marking) changed are re-shaped.
        let marked = self.marked.clone();
        let doc = &self.doc;
        for rec in &mut self.paras {
            let visible = rec.span.visible();
            let tiles: Vec<(Range<usize>, InlineStyle)> = doc
                .spans()
                .runs_in(visible.clone())
                .into_iter()
                .map(|(r, s)| (r.start - visible.start..r.end - visible.start, s))
                .collect();
            let marked_rel = marked.as_ref().and_then(|m| {
                let s = m.start.max(visible.start);
                let e = m.end.min(visible.end);
                (s < e).then(|| s - visible.start..e - visible.start)
            });
            let sig = para_sig(gsig, &tiles, marked_rel.as_ref());
            if rec.shaped.is_some() && rec.sig == sig {
                continue;
            }
            let runs = runs::paragraph_runs(&tiles, visible.len(), voice, marked_rel, palette);
            match window.text_system().shape_text(
                rec.text.clone(),
                px(voice.size),
                &runs,
                Some(px(wrap)),
                None,
            ) {
                Ok(mut lines) if !lines.is_empty() => {
                    let line = lines.swap_remove(0);
                    rec.rows = line.wrap_boundaries().len() + 1;
                    rec.shaped = Some(line);
                }
                Ok(_) => {
                    rec.shaped = None;
                    rec.rows = 1;
                }
                Err(error) => {
                    tracing::error!(%error, "paragraph shaping failed");
                    rec.shaped = None;
                    rec.rows = 1;
                }
            }
            rec.sig = sig;
        }

        // Stack paragraph origins (prefix sums).
        let mut y = metrics::COLUMN_TOP_PAD;
        let mut paras = Vec::with_capacity(self.paras.len());
        for rec in &self.paras {
            let height = rec.rows.max(1) as f32 * line_height;
            paras.push(SnapPara {
                span: rec.span.clone(),
                y,
                height,
                line: rec.shaped.clone(),
            });
            y += height;
        }

        // Date label: shaped fresh each layout (one short line), drawn in
        // the reserved top pad so its presence never shifts the text.
        let date = self.date_label.as_ref().map(|label| {
            let text = SharedString::from(label.to_uppercase());
            let run = TextRun {
                len: text.len(),
                font: date_font(),
                color: tokens.accent.alpha(0.55),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line(text, px(metrics::UI_SMALL), &[run], None);
            (line, DATE_Y)
        });

        // Coda: shaped per-frame only while the typewriter reveal runs;
        // afterwards the full shape is cached against `coda_shaped`.
        let mut coda_snap = None;
        if let Some(coda) = &self.coda {
            let elapsed = coda.since.elapsed();
            let divider_frac = motion::ease_out_quint(
                (elapsed.as_secs_f32() / motion::MOVE.as_secs_f32()).clamp(0.0, 1.0),
            );
            let reveal_t = ((elapsed.as_secs_f32() - motion::MOVE.as_secs_f32())
                / notes::REVEAL.as_secs_f32())
            .clamp(0.0, 1.0);
            let coda_size = (voice.size - 2.0).max(11.0);
            let coda_lh = coda_size * LINE_HEIGHT_FACTOR;
            let csig = coda_sig(gsig, &coda.body);
            let lines = if reveal_t >= 1.0 {
                match &self.coda_shaped {
                    Some((sig, lines)) if *sig == csig => lines.clone(),
                    _ => {
                        let lines =
                            shape_coda(&coda.body, coda_size, voice, tokens.muse, wrap, window);
                        self.coda_shaped = Some((csig, lines.clone()));
                        lines
                    }
                }
            } else {
                let revealed = notes::reveal_prefix(&coda.body, reveal_t);
                shape_coda(revealed, coda_size, voice, tokens.muse, wrap, window)
            };
            let divider_y = y + CODA_GAP;
            let body_y = divider_y + CODA_GAP * 0.75;
            let rows: usize = lines
                .iter()
                .map(|line| line.wrap_boundaries().len() + 1)
                .sum();
            y = body_y + rows.max(1) as f32 * coda_lh;
            coda_snap = Some(CodaSnap {
                divider_y,
                divider_frac,
                body_y,
                line_height: coda_lh,
                lines,
            });
        }
        let content_height = y + BOTTOM_PAD;

        let mut snap = Snapshot {
            bounds,
            column_x,
            wrap_width: wrap,
            line_height,
            scroll: self.scroll,
            paras,
            content_height,
            dots: Vec::new(),
            date,
            coda: coda_snap,
        };

        // Scroll: clamp, then service the pending caret autoscroll with the
        // minimal correction, then drag-selection edge autoscroll.
        let max_scroll = snap.max_scroll(viewport_h);
        let mut scroll = self.scroll.clamp(0.0, max_scroll);
        if let Some(offset) = self.autoscroll_to.take() {
            let (_, caret_y) = snap.caret_point(offset);
            if caret_y < scroll + SCROLL_MARGIN {
                scroll = (caret_y - SCROLL_MARGIN).max(0.0);
            } else if caret_y + line_height > scroll + viewport_h - SCROLL_MARGIN {
                scroll = caret_y + line_height - viewport_h + SCROLL_MARGIN;
            }
            scroll = scroll.clamp(0.0, max_scroll);
        }
        if self.is_selecting
            && let Some(p) = self.drag_point
        {
            let local = f32::from(p.y - bounds.origin.y);
            if local < DRAG_EDGE {
                scroll = (scroll - (DRAG_EDGE - local) * 0.15).max(0.0);
            } else if local > viewport_h - DRAG_EDGE {
                scroll = (scroll + (local - (viewport_h - DRAG_EDGE)) * 0.15).min(max_scroll);
            }
        }
        if (scroll - self.scroll).abs() > 0.01 {
            self.scroll = scroll;
            cx.emit(EditorEvent::ScrollChanged);
        }
        snap.scroll = self.scroll;

        // Annotations: place reaction highlights and citation carets, mark
        // withered anchors, drop the fully faded (silently — the app
        // reconciles via set_annotations). A reaction's marker is the emoji
        // badge at the top-right corner of its first highlight rect; a
        // note's marker is the caret just past the quoted text.
        let now = Instant::now();
        {
            let doc = &self.doc;
            for slot in &mut self.notes {
                match doc.anchor_range(slot.anchor) {
                    Some(range) => {
                        if slot.ann.emoji.is_some() {
                            slot.last_rects = snap.selection_rects(range.clone());
                            if let Some(first) = slot.last_rects.first() {
                                slot.last_center =
                                    Some((first.x + first.w - 2.0, first.y + 2.0));
                            }
                        } else {
                            let (x, y) = snap.caret_point(range.end);
                            slot.last_center = Some((x + 5.0, y + line_height * 0.22));
                        }
                    }
                    None => {
                        if slot.withering.is_none() {
                            slot.withering = Some(now);
                        }
                    }
                }
            }
        }
        let mut kept = Vec::with_capacity(self.notes.len());
        for slot in std::mem::take(&mut self.notes) {
            let dead = slot
                .withering
                .is_some_and(|since| now.duration_since(since) >= notes::WITHER);
            if dead {
                self.doc.release_anchor(slot.anchor);
                if self.card.as_ref().is_some_and(|card| card.id == slot.ann.id) {
                    self.card = None;
                }
                if self.hovered_dot == Some(slot.ann.id) {
                    self.hovered_dot = None;
                }
            } else {
                kept.push(slot);
            }
        }
        self.notes = kept;
        snap.dots = self
            .notes
            .iter()
            .filter_map(|slot| {
                slot.last_center.map(|center| SnapDot {
                    id: slot.ann.id,
                    center,
                })
            })
            .collect();

        // Caret spring: the typed glyph paints this same frame; only the
        // caret eases toward it.
        let (caret_x, caret_y) = snap.caret_point(self.sel.head);
        if self.spring_primed {
            self.spring.retarget(caret_x, caret_y);
        } else {
            self.spring.snap_to(caret_x, caret_y);
            self.spring_primed = true;
        }
        let dt = self
            .last_frame
            .map_or(1.0 / 120.0, |t| now.duration_since(t).as_secs_f32());
        self.last_frame = Some(now);
        self.spring.step(dt);

        self.snapshot = Some(Rc::new(snap));

        // Keep a drag selection tracking the pointer while edge-autoscroll
        // moves the content under it.
        if self.is_selecting
            && let Some(p) = self.drag_point
        {
            self.extend_to_point(p, cx);
        }

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let column_bounds = Bounds::new(
            point(bounds.origin.x + px(column_x), bounds.origin.y),
            size(px(wrap), bounds.size.height),
        );
        let column = window.insert_hitbox(column_bounds, HitboxBehavior::Normal);
        EditorPrepaint { hitbox, column }
    }

    /// Paint everything from the published snapshot, then register input
    /// handling and window mouse listeners for the coming frame.
    pub(crate) fn paint_pass(
        &mut self,
        bounds: Bounds<Pixels>,
        prepaint: &EditorPrepaint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snap) = self.snapshot.clone() else {
            return;
        };
        let tokens = cx.theme().tokens;
        let focused = self.focus_handle.is_focused(window);
        let viewport_h = f32::from(bounds.size.height);
        let now = Instant::now();

        window.handle_input(
            &self.focus_handle,
            ElementInputHandler::new(bounds, cx.entity()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.column);

        let sel_range = self.sel.range();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some((line, date_y)) = &snap.date {
                let origin = snap.to_window((0.0, *date_y));
                if line.paint(origin, px(16.0), window, cx).is_err() {
                    tracing::warn!("date label paint failed");
                }
            }

            // Selection, behind the glyphs: full-line-box rects that tile
            // with no vertical gaps; only the outer silhouette is rounded.
            if !sel_range.is_empty() {
                for sel_rect in snap.selection_rects(sel_range.clone()) {
                    if sel_rect.y + sel_rect.h < snap.scroll
                        || sel_rect.y > snap.scroll + viewport_h
                    {
                        continue;
                    }
                    let rect = Bounds::new(
                        snap.to_window((sel_rect.x, sel_rect.y)),
                        size(px(sel_rect.w), px(sel_rect.h)),
                    );
                    let radius = px(4.0).min(rect.size.width / 2.0);
                    let top = if sel_rect.round_top { radius } else { px(0.0) };
                    let bottom = if sel_rect.round_bottom { radius } else { px(0.0) };
                    let corners = Corners {
                        top_left: top,
                        top_right: top,
                        bottom_left: bottom,
                        bottom_right: bottom,
                    };
                    window.paint_quad(fill(rect, tokens.selection).corner_radii(corners));
                }
            }

            // Reaction highlights, under the glyphs: the anchored text
            // tinted and outlined, flashing in loud when the reaction lands.
            for slot in &self.notes {
                if slot.ann.emoji.is_none() || slot.last_rects.is_empty() {
                    continue;
                }
                let (alpha, pop) = annotation_anim(slot, now);
                if alpha <= 0.01 {
                    continue;
                }
                // The entrance flash: the tint starts hot and settles.
                let flash = 1.0 + 0.9 * (1.0 - pop);
                let tint = tokens.muse.alpha(tokens.muse.a * 0.16 * alpha * flash);
                let edge = tokens.muse.alpha(tokens.muse.a * 0.55 * alpha);
                let single = slot.last_rects.len() == 1;
                for hl in &slot.last_rects {
                    if hl.y + hl.h < snap.scroll || hl.y > snap.scroll + viewport_h {
                        continue;
                    }
                    let rect = Bounds::new(
                        snap.to_window((hl.x - 2.0, hl.y)),
                        size(px(hl.w + 4.0), px(hl.h)),
                    );
                    let radius = px(5.0).min(rect.size.width / 2.0);
                    let top = if hl.round_top { radius } else { px(0.0) };
                    let bottom = if hl.round_bottom { radius } else { px(0.0) };
                    let corners = Corners {
                        top_left: top,
                        top_right: top,
                        bottom_left: bottom,
                        bottom_right: bottom,
                    };
                    if single {
                        window.paint_quad(quad(
                            rect,
                            corners,
                            tint,
                            px(1.0),
                            edge,
                            BorderStyle::default(),
                        ));
                    } else {
                        window.paint_quad(fill(rect, tint).corner_radii(corners));
                    }
                }
            }

            // Paragraphs: only the visible band pays paint cost.
            for para in &snap.paras {
                if para.y + para.height < snap.scroll || para.y > snap.scroll + viewport_h {
                    continue;
                }
                if let Some(line) = &para.line {
                    let origin = snap.to_window((0.0, para.y));
                    if line
                        .paint(origin, px(snap.line_height), TextAlign::default(), None, window, cx)
                        .is_err()
                    {
                        tracing::warn!("paragraph paint failed");
                    }
                }
            }

            // Coda: divider draws in, body reveals beneath.
            if let Some(coda) = &snap.coda {
                let divider_w = snap.wrap_width * coda.divider_frac;
                if divider_w > 0.5 {
                    let rect = Bounds::new(
                        snap.to_window((0.0, coda.divider_y)),
                        size(px(divider_w), px(1.0)),
                    );
                    window.paint_quad(fill(rect, tokens.hairline));
                }
                let mut line_y = coda.body_y;
                for line in &coda.lines {
                    let rows = (line.wrap_boundaries().len() + 1) as f32;
                    let origin = snap.to_window((0.0, line_y));
                    if line
                        .paint(origin, px(coda.line_height), TextAlign::default(), None, window, cx)
                        .is_err()
                    {
                        tracing::warn!("coda paint failed");
                    }
                    line_y += rows * coda.line_height;
                }
            }

            // Annotation markers, over the glyphs: reaction emoji badges
            // popping in with overshoot, citation carets resting just past
            // their quotes (open on hover, like a cited source).
            for slot in &self.notes {
                let Some((marker_x, marker_y)) = slot.last_center else {
                    continue;
                };
                if marker_y + 48.0 < snap.scroll || marker_y - 48.0 > snap.scroll + viewport_h {
                    continue;
                }
                let (alpha, pop) = annotation_anim(slot, now);
                if alpha <= 0.01 {
                    continue;
                }
                let engaged = self.hovered_dot == Some(slot.ann.id)
                    || self.card.as_ref().is_some_and(|card| card.id == slot.ann.id);

                if let Some(emoji) = &slot.ann.emoji {
                    // The badge: a small lifted circle riding the highlight's
                    // top-right corner, scaling in with a springy overshoot.
                    let scale = ease_out_back(pop);
                    if scale <= 0.05 {
                        continue;
                    }
                    let diameter = 26.0 * scale;
                    let center = snap.to_window((marker_x, marker_y - 4.0));
                    let badge = Bounds::new(
                        point(
                            center.x - px(diameter / 2.0),
                            center.y - px(diameter / 2.0),
                        ),
                        size(px(diameter), px(diameter)),
                    );
                    // Sticker treatment: a small offset shadow first, so the
                    // badge looks peeled onto the page, then a near-white
                    // disc with a crisp ink hairline.
                    let shadow_rect = Bounds::new(
                        point(badge.origin.x, badge.origin.y + px(2.0)),
                        badge.size,
                    );
                    window.paint_quad(
                        fill(shadow_rect, tokens.ink.alpha(0.10 * alpha))
                            .corner_radii(px(diameter / 2.0)),
                    );
                    let bg = gpui::white().alpha(alpha);
                    window.paint_quad(quad(
                        badge,
                        px(diameter / 2.0),
                        bg,
                        px(2.0),
                        tokens.ink.alpha(0.10 * alpha),
                        BorderStyle::default(),
                    ));
                    let emoji_size = 13.0 * scale;
                    let run = TextRun {
                        len: emoji.len(),
                        font: date_font(),
                        color: tokens.ink.alpha(alpha),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = window.text_system().shape_line(
                        emoji.clone(),
                        px(emoji_size),
                        &[run],
                        None,
                    );
                    let width = f32::from(line.width);
                    let origin = point(
                        center.x - px(width / 2.0),
                        center.y - px(emoji_size * 0.62),
                    );
                    if line.paint(origin, px(emoji_size * 1.25), window, cx).is_err() {
                        tracing::warn!("reaction emoji paint failed");
                    }
                } else {
                    // The citation caret: a quiet ^ in muse ink, brightening
                    // when hovered or while its card is open.
                    let em = snap.line_height / LINE_HEIGHT_FACTOR;
                    let caret_size = em * 0.72;
                    let strength = if engaged { 1.0 } else { 0.65 };
                    let color = tokens.muse.alpha(tokens.muse.a * alpha * strength);
                    let glyph = SharedString::new_static("^");
                    let run = TextRun {
                        len: glyph.len(),
                        font: date_font(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = window.text_system().shape_line(
                        glyph,
                        px(caret_size),
                        &[run],
                        None,
                    );
                    let width = f32::from(line.width);
                    let origin = snap.to_window((marker_x - width / 2.0, marker_y));
                    if line.paint(origin, px(caret_size), window, cx).is_err() {
                        tracing::warn!("citation caret paint failed");
                    }
                }
            }

            // Caret: 2px rounded quad in accent; eased position, faded blink.
            if focused && sel_range.is_empty() {
                let opacity = if self.is_selecting {
                    1.0
                } else {
                    anim::blink_opacity(now.duration_since(self.blink_reset))
                };
                if opacity > 0.02 {
                    let rect = Bounds::new(
                        snap.to_window((
                            self.spring.x - 1.0,
                            self.spring.y + snap.line_height * 0.05,
                        )),
                        size(px(2.0), px(snap.line_height * 0.9)),
                    );
                    let color = tokens.accent.alpha(tokens.accent.a * opacity);
                    window.paint_quad(fill(rect, color).corner_radii(px(1.0)));
                }
            }
        });

        // Continuous animation: caret spring/blink, dot fades, coda reveal,
        // drag autoscroll. All of these are paint-only damage — the shaping
        // cache guarantees no text is re-shaped for an animation frame.
        let coda_animating = self
            .coda
            .as_ref()
            .is_some_and(|coda| coda.since.elapsed() < motion::MOVE + notes::REVEAL);
        let dots_animating = self
            .notes
            .iter()
            .any(|slot| slot.withering.is_some() || slot.appeared.elapsed() < REACT_POP);
        let needs_frame = !self.spring.settled()
            || (focused && sel_range.is_empty())
            || coda_animating
            || dots_animating
            || (self.is_selecting && self.drag_point.is_some());
        if needs_frame {
            window.request_animation_frame();
        }

        // Window-level mouse listeners, re-registered each frame (the
        // input.rs pattern, hitbox-gated where appropriate).
        let entity = cx.entity();
        window.on_mouse_event::<MouseDownEvent>({
            let entity = entity.clone();
            let hitbox = prepaint.hitbox.clone();
            move |event, phase, window, cx| {
                if phase.bubble()
                    && event.button == MouseButton::Left
                    && hitbox.is_hovered(window)
                {
                    entity.update(cx, |editor, cx| editor.on_mouse_down(event, window, cx));
                }
            }
        });
        window.on_mouse_event::<MouseMoveEvent>({
            let entity = entity.clone();
            move |event, phase, window, cx| {
                if phase.bubble() {
                    entity.update(cx, |editor, cx| editor.on_mouse_move(event, window, cx));
                }
            }
        });
        window.on_mouse_event::<MouseUpEvent>({
            let entity = entity.clone();
            move |event, phase, window, cx| {
                if phase.bubble() {
                    entity.update(cx, |editor, cx| editor.on_mouse_up(event, window, cx));
                }
            }
        });
        window.on_mouse_event::<ScrollWheelEvent>({
            let entity = entity.clone();
            let hitbox = prepaint.hitbox.clone();
            move |event, phase, window, cx| {
                if phase.bubble() && hitbox.is_hovered(window) {
                    entity.update(cx, |editor, cx| editor.on_scroll_wheel(event, window, cx));
                }
            }
        });
    }
}

/// Shape the coda body: Quattro italic, two steps under the voice size,
/// muse-tinted ink.
fn shape_coda(
    text: &str,
    size_pt: f32,
    voice: Voice,
    color: Hsla,
    wrap: f32,
    window: &mut Window,
) -> Vec<WrappedLine> {
    let coda_voice = Voice {
        family: muse_core::FontFamily::Quattro,
        size: size_pt,
        weight: voice.weight,
    };
    let style = InlineStyle {
        italic: true,
        ..InlineStyle::default()
    };
    let shared: SharedString = SharedString::from(text.to_string());
    let run = TextRun {
        len: shared.len(),
        font: runs::run_font(coda_voice, style),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    match window
        .text_system()
        .shape_text(shared, px(size_pt), &[run], Some(px(wrap)), None)
    {
        Ok(lines) => lines.into_vec(),
        Err(error) => {
            tracing::error!(%error, "coda shaping failed");
            Vec::new()
        }
    }
}

/// Appear/wither animation state for one annotation: `(alpha, t)` where
/// `alpha` is the eased entrance opacity folded with the wither fade, and
/// `t` is the raw (unclamped-easing) entrance progress for the scale pop.
fn annotation_anim(slot: &NoteSlot, now: Instant) -> (f32, f32) {
    let t = (now.duration_since(slot.appeared).as_secs_f32() / REACT_POP.as_secs_f32())
        .clamp(0.0, 1.0);
    let wither = slot.withering.map_or(1.0, |since| {
        1.0 - (now.duration_since(since).as_secs_f32() / notes::WITHER.as_secs_f32())
            .clamp(0.0, 1.0)
    });
    (motion::ease_out_quint(t) * wither, t)
}

/// Ease-out-back: overshoots past 1.0 and settles — the reaction pop.
fn ease_out_back(t: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    let u = t - 1.0;
    1.0 + C3 * u * u * u + C1 * u * u
}

/// The typewriter font for the date label — a mono stamp, not a UI label.
fn date_font() -> Font {
    Font {
        family: SharedString::new_static(fonts::FONT_MONO),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

/// Hash of everything besides paragraph text that affects shaping.
fn global_sig(voice: Voice, wrap: f32, palette: [Hsla; 4]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (voice.family as u8).hash(&mut hasher);
    voice.size.to_bits().hash(&mut hasher);
    voice.weight.hash(&mut hasher);
    wrap.to_bits().hash(&mut hasher);
    for color in palette {
        hash_hsla(color, &mut hasher);
    }
    hasher.finish()
}

/// Per-paragraph shaping signature: global inputs + style tiles + IME
/// marked overlap.
fn para_sig(
    gsig: u64,
    tiles: &[(Range<usize>, InlineStyle)],
    marked: Option<&Range<usize>>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    gsig.hash(&mut hasher);
    for (range, style) in tiles {
        range.hash(&mut hasher);
        style.hash(&mut hasher);
    }
    marked.hash(&mut hasher);
    hasher.finish()
}

fn coda_sig(gsig: u64, body: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    gsig.hash(&mut hasher);
    body.hash(&mut hasher);
    hasher.finish()
}

fn hash_hsla(color: Hsla, hasher: &mut impl Hasher) {
    color.h.to_bits().hash(hasher);
    color.s.to_bits().hash(hasher);
    color.l.to_bits().hash(hasher);
    color.a.to_bits().hash(hasher);
}
