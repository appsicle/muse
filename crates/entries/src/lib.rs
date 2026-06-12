//! muse-entries — the left sidebar: entry list state and soft delete.
//!
//! Owns the [`Sidebar`] view: one calm, flat list of entries, most recently
//! touched first (the caller's order). It renders entirely from state the
//! app pushes in ([`Sidebar::set_entries`], [`Sidebar::set_selected`],
//! [`Sidebar::set_sync_glyph`]) and reports intent back out through
//! [`SidebarEvent`]. It must not know about storage handles, documents, or
//! the agent — the app owns every mutation.

mod age;

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, ClickEvent, Context, EventEmitter, MouseButton,
    SharedString, Window, div, prelude::*, px, svg,
};
use muse_storage::EntrySummary;
use muse_theme::{ActiveTheme as _, layout, motion};
use muse_ui::{IconName, icon_button};

/// Top padding above the header row, clearing the inset traffic lights.
const TOP_PAD: f32 = 46.0;
/// Fixed height of one entry row — generous, single line (sized for the
/// `UI_BODY` title; identical in every state, nothing ever shifts).
const ROW_H: f32 = 30.0;
/// Horizontal margin around rows, and half the edge padding of headers.
const ROW_MARGIN_X: f32 = 8.0;
/// Horizontal padding inside a row.
const ROW_PAD_X: f32 = 10.0;
/// Width of every row's fixed right slot. It shows the age label at rest
/// and the trash button on hover, so neither ever reflows the title.
const RIGHT_SLOT: f32 = 32.0;

/// Sync indicator state for the tiny passive glyph in the sidebar footer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncGlyph {
    /// Local-only: nothing is shown.
    #[default]
    Local,
    /// A save or sync is in flight: a quiet cloud.
    Saving,
    /// Everything is safely stored: a quiet check.
    Saved,
}

/// What the sidebar asks the app to do. The sidebar itself never mutates
/// entries — the app decides, then pushes a fresh list back in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarEvent {
    /// A row was clicked: open this entry.
    Open(String),
    /// The row's trash button was clicked. The app performs the soft delete
    /// and shows the undo toast.
    DeleteRequested(String),
    /// The plus button was clicked: start a new entry.
    NewEntry,
}

/// One precomputed display row. Labels are built once per
/// [`Sidebar::set_entries`] so rendering stays allocation-free.
struct Row {
    id: SharedString,
    title: SharedString,
    /// The entry's first line is empty; `title` holds the placeholder and
    /// renders tertiary.
    untitled: bool,
    /// Compact relative age ("now", "5m", "3h", "5d", "2w", "8mo", "1y").
    age: SharedString,
    /// Unique id for the hover-fade animation wrapper (which sits outside
    /// the row's own id scope).
    hover_anim_id: SharedString,
    /// Unique group name tying the trash button's hover to its icon tint.
    trash_group: SharedString,
}

impl Row {
    fn build(entry: &EntrySummary, now_millis: i64) -> Row {
        let untitled = entry.title.trim().is_empty();
        Row {
            id: SharedString::from(entry.id.clone()),
            title: if untitled {
                SharedString::new_static("New entry")
            } else {
                SharedString::from(entry.title.clone())
            },
            untitled,
            age: SharedString::from(age::age_label(entry.touched_at, now_millis)),
            hover_anim_id: SharedString::from(format!("hover-{}", entry.id)),
            trash_group: SharedString::from(format!("trash-{}", entry.id)),
        }
    }
}

/// The left sidebar: header ("Entries" + plus), the flat entry list, and a
/// passive sync glyph in the footer. The workspace owns the 260px slot and
/// the show/hide animation; this view simply fills its container.
pub struct Sidebar {
    rows: Vec<Row>,
    selected: Option<String>,
    hovered: Option<SharedString>,
    sync: SyncGlyph,
    /// Muse is reading/considering; the footer shows the typing dots.
    thinking: bool,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    /// An empty sidebar; the app pushes entries in right after creation.
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Sidebar {
            rows: Vec::new(),
            selected: None,
            hovered: None,
            sync: SyncGlyph::default(),
            thinking: false,
        }
    }

    /// Replace the list with a fresh snapshot (the app calls this after
    /// every save, delete, restore, or switch). Age labels are computed
    /// once, against the clock at this moment — there are no timers
    /// refreshing rows. Order is the caller's order (storage already sorts
    /// by `touched_at` descending).
    pub fn set_entries(&mut self, entries: Vec<EntrySummary>, cx: &mut Context<Self>) {
        let now_millis = jiff::Timestamp::now().as_millisecond();
        self.rows = entries
            .iter()
            .map(|entry| Row::build(entry, now_millis))
            .collect();
        cx.notify();
    }

    /// Mark the entry shown in the editor; its row renders lifted.
    pub fn set_selected(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id;
            cx.notify();
        }
    }

    /// Update the passive sync glyph state (no longer rendered; saves are
    /// silent — kept so the app's wiring stays stable).
    pub fn set_sync_glyph(&mut self, state: SyncGlyph, cx: &mut Context<Self>) {
        if self.sync != state {
            self.sync = state;
            cx.notify();
        }
    }

    /// Whether Muse is currently reading or considering; drives the
    /// iMessage-style typing dots in the footer.
    pub fn set_thinking(&mut self, thinking: bool, cx: &mut Context<Self>) {
        if self.thinking != thinking {
            self.thinking = thinking;
            cx.notify();
        }
    }

    /// The header IS the titlebar strip: traffic lights live in the left
    /// 76px, the new-entry and toggle buttons sit right — one row, nothing
    /// below it. Double-click on empty chrome zooms, like the topbar.
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let _tokens = cx.theme().tokens;
        div()
            .flex_none()
            .h(px(TOP_PAD))
            .pl(px(76.))
            .pr(px(ROW_MARGIN_X))
            .flex()
            .items_center()
            .justify_end()
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        icon_button("sidebar-new-entry", IconName::Plus)
                            .icon_size(px(14.))
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(SidebarEvent::NewEntry)
                            })),
                    )
                    .child(
                        icon_button("sidebar-toggle", IconName::PanelLeft)
                            .icon_size(px(14.))
                            .on_click(|_, window, cx| {
                                window
                                    .dispatch_action(Box::new(muse_commands::ToggleSidebar), cx);
                            }),
                    ),
            )
    }

    /// The hover-revealed trash button: explicit icon tints (gpui's `svg`
    /// paints only with a color set on the svg itself, never the parent's
    /// `text_color`), brightening to accent while the button is hovered.
    fn render_trash(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.theme().tokens;
        let delete_id = row.id.clone();
        div()
            .child(
                div()
                    .id("entry-delete")
                    .group(row.trash_group.clone())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(24.))
                    .rounded(px(layout::RADIUS_SM))
                    .cursor_pointer()
                    .hover(move |style| style.bg(tokens.hairline.opacity(0.8)))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        cx.emit(SidebarEvent::DeleteRequested(delete_id.to_string()));
                    }))
                    .child(
                        svg()
                            .flex_none()
                            .size(px(14.))
                            .path(IconName::Trash.path())
                            .text_color(tokens.ink_secondary)
                            .group_hover(row.trash_group.clone(), |style| {
                                style.text_color(tokens.accent)
                            }),
                    ),
            )
            // Mounted fresh on each hover, so the fade replays.
            .with_animation(
                "trash-fade",
                Animation::new(motion::FADE).with_easing(motion::ease_out_quint),
                |el, delta| el.opacity(delta),
            )
            .into_any_element()
    }

    fn render_row(&self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.theme().tokens;
        let selected = self.selected.as_deref() == Some(row.id.as_ref());
        let hovered = self.hovered.as_ref() == Some(&row.id);

        let open_id = row.id.clone();
        let hover_id = row.id.clone();

        // A fixed right slot: the age label at rest, the trash button on
        // hover. Same width either way, so nothing ever shifts.
        let right_slot = div()
            .flex_none()
            .w(px(RIGHT_SLOT))
            .h_full()
            .flex()
            .items_center()
            .justify_end()
            .child(if hovered {
                self.render_trash(row, cx)
            } else {
                div()
                    .text_size(px(layout::UI_HEADER))
                    .text_color(tokens.ink_tertiary)
                    .child(row.age.clone())
                    .into_any_element()
            });

        let title = div()
            .flex_1()
            .min_w(px(0.))
            .text_size(px(layout::UI_TEXT))
            .text_color(if row.untitled {
                tokens.ink_tertiary
            } else {
                tokens.ink
            })
            .truncate()
            .child(row.title.clone());

        let row_el = div()
            .id(row.id.clone())
            .flex_none()
            .h(px(ROW_H))
            .mx(px(ROW_MARGIN_X))
            .px(px(ROW_PAD_X))
            .rounded(px(layout::RADIUS_SM))
            .when(selected, |el| {
                // Sticker-ish lift: a faint accent wash plus a crimson
                // stamp-edge tab on the left.
                el.bg(tokens.accent.alpha(0.10))
                    .border_l_2()
                    .border_color(tokens.accent)
            })
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, entered: &bool, _, cx| {
                if *entered {
                    if this.hovered.as_ref() != Some(&hover_id) {
                        this.hovered = Some(hover_id.clone());
                        cx.notify();
                    }
                } else if this.hovered.as_ref() == Some(&hover_id) {
                    this.hovered = None;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(SidebarEvent::Open(open_id.to_string()));
            }))
            .child(title)
            .child(right_slot);

        if hovered && !selected {
            // Remounted at hover start, fading the tint in over FADE; the
            // tint vanishes instantly on leave.
            let hover_bg = tokens.hairline.opacity(0.35);
            row_el
                .with_animation(
                    row.hover_anim_id.clone(),
                    Animation::new(motion::FADE).with_easing(motion::ease_out_quint),
                    move |el, delta| el.bg(hover_bg.opacity(delta)),
                )
                .into_any_element()
        } else {
            row_el.into_any_element()
        }
    }

    /// The footer: a quiet Settings button on the left, and three bouncing
    /// dots while Muse is reading or thinking — like a friend typing.
    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.theme().tokens;
        let hover_bg = tokens.hairline.opacity(0.5);
        let mut footer = div()
            .flex_none()
            .h(px(36.))
            .px(px(ROW_MARGIN_X))
            .flex()
            .items_center()
            .gap(px(3.))
            .child(
                div()
                    .id("sidebar-settings")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(ROW_PAD_X))
                    .py(px(4.))
                    .rounded(px(layout::RADIUS_SM))
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover_bg))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(muse_commands::OpenSettings), cx);
                    })
                    .child(
                        svg()
                            .flex_none()
                            .size(px(13.))
                            .path(IconName::Settings.path())
                            .text_color(tokens.ink_secondary),
                    )
                    .child(
                        div()
                            .text_size(px(layout::UI_SMALL))
                            .text_color(tokens.ink_secondary)
                            .child("Settings"),
                    ),
            )
            .child(div().flex_1());
        if self.thinking {
            for phase in 0..3u64 {
                let dot = div()
                    .flex_none()
                    .size(px(6.))
                    .rounded_full()
                    .bg(tokens.accent.alpha(0.75));
                footer = footer.child(
                    dot.with_animation(
                        ("muse-typing-dot", phase as usize),
                        Animation::new(Duration::from_millis(900)).repeat(),
                        move |el, t| {
                            // Staggered bounce: each dot leads the next by a
                            // sixth of the cycle, rising 4px at its peak.
                            let local = (t + phase as f32 / 6.0).fract();
                            let lift = (local * std::f32::consts::PI).sin().max(0.0) * 4.0;
                            el.mb(px(lift)).opacity(0.45 + 0.55 * (lift / 4.0))
                        },
                    ),
                );
            }
        }
        footer
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.theme().tokens;

        let mut list = div()
            .id("entry-list")
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .pt(px(4.))
            .pb(px(ROW_MARGIN_X));

        for row in &self.rows {
            list = list.child(self.render_row(row, cx));
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(tokens.bg)
            .border_r_1()
            .border_color(tokens.hairline)
            .child(self.render_header(cx))
            .child(list)
            .child(self.render_footer(cx))
    }
}
