//! The workspace — the one stateful glue entity. Owns the editor, sidebar,
//! and topbar views; the entries flow (open / new / delete / undo); layout
//! (the sanctioned sidebar slide); the theme crossfade; and toasts. Agent
//! orchestration lives in `muse_flow`, the settings pane in `settings`,
//! storage-facing helpers in `persistence`.
//!
//! Invariant: overlays (toast, settings) are absolutely positioned and
//! never affect layout; the sidebar slide is the only layout animation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, AnyElement, ClickEvent, Context, ElementId, Entity,
    Focusable as _, SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use muse_agent::{Chattiness, NoteRecord, TriggerEngine};
use muse_api::ApiHandle;
use muse_commands as cmd;
use muse_core::Document;
use muse_editor::{Editor, EditorEvent};
use muse_entries::{Sidebar, SidebarEvent};
use muse_storage::Store;
use muse_theme::{
    ActiveTheme as _, Appearance, FONT_UI, Theme, ThemePair, Tokens, layout, lerp_tokens, motion,
};
use muse_topbar::Topbar;
use muse_ui::{TextField, pill, text_button};
use ulid::Ulid;

use crate::persistence::{Boot, date_label};

/// How often the agent poll loop ticks.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How long a toast stays before auto-dismissing.
const TOAST_LIFE: Duration = Duration::from_secs(5);
/// Frame pacing for the theme crossfade driver.
const CROSSFADE_FRAME: Duration = Duration::from_millis(8);

/// The sidebar slot's in-flight width animation.
/// Sidebar drag-resize bounds.
const MIN_SIDEBAR_W: f32 = 200.0;
const MAX_SIDEBAR_W: f32 = 380.0;

struct Slide {
    from: f32,
    to: f32,
    start: Instant,
}

/// The single visible toast — "Entry deleted" with its undo affordance.
pub(crate) struct Toast {
    /// The message shown in the pill.
    message: SharedString,
    /// The soft-deleted entry the Undo button restores.
    entry_id: String,
}

/// The root entity: every other view hangs off it, every workspace action
/// lands here.
pub struct Workspace {
    pub(crate) store: Arc<Store>,
    pub(crate) api: ApiHandle,
    /// The on-device brain. Spawning is cheap; the model loads lazily inside
    /// the crate on first request.
    pub(crate) local: muse_local::LocalHandle,
    pub(crate) editor: Entity<Editor>,
    pub(crate) sidebar: Entity<Sidebar>,
    pub(crate) topbar: Entity<Topbar>,

    /// Ulid string of the entry in the editor.
    pub(crate) current_entry: String,
    pub(crate) dirty: bool,
    pub(crate) save_generation: u64,
    pub(crate) glyph_generation: u64,

    entries_open: bool,
    slide: Option<Slide>,
    /// User-chosen sidebar width (drag the divider), persisted.
    sidebar_w: f32,
    /// A divider drag is in progress.
    resizing_sidebar: bool,
    theme_generation: u64,
    /// The light/dark palettes the app currently dresses in.
    pub(crate) theme_pair: ThemePair,

    pub(crate) muted: bool,
    pub(crate) key_missing: bool,
    pub(crate) chattiness: Chattiness,

    // ── Settings pane ──────────────────────────────────────────────────────
    pub(crate) settings_open: bool,
    /// The theme dropdown in settings is expanded.
    pub(crate) theme_menu_open: bool,
    /// Custom-theme hex fields: light accent/bg/fg, then dark accent/bg/fg.
    pub(crate) custom_fields: [Entity<TextField>; 6],
    pub(crate) api_field: Entity<TextField>,
    /// Briefly true after a successful key save (the tiny check).
    pub(crate) api_saved: bool,
    pub(crate) api_saved_generation: u64,
    /// Guards the settings pane's 250ms download poll, like the pane's
    /// other timers.
    pub(crate) download_poll_generation: u64,

    /// One trigger engine per entry visited this session.
    pub(crate) engines: HashMap<String, TriggerEngine>,
    /// Notes visible in the margin of the current entry.
    pub(crate) notes: Vec<NoteRecord>,
    pub(crate) next_note_id: u64,
    pub(crate) last_len_chars: usize,
    pub(crate) considering: bool,
    pub(crate) orb_generation: u64,

    toast: Option<Toast>,
    toast_leaving: Option<Toast>,
    toast_generation: u64,

    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

impl Workspace {
    /// Build the workspace: child views, event wiring, the agent poll loop,
    /// and the startup entry restoration.
    pub fn new(
        store: Arc<Store>,
        api: ApiHandle,
        boot: Boot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| Editor::new(Document::new(Ulid::new()), window, cx));
        let sidebar = cx.new(Sidebar::new);
        let topbar = cx.new(Topbar::new);

        let mut subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            cx.subscribe_in(&sidebar, window, Self::on_sidebar_event),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.flush(cx);
                }
            }),
        ];
        subscriptions.push(cx.on_app_quit(|this, cx| {
            this.flush(cx);
            std::future::ready(())
        }));
        // Closing the window must not outrun the 400ms autosave debounce.
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| this.flush(cx)).ok();
            true
        });

        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if this.update(cx, |this, cx| this.agent_tick(cx)).is_err() {
                    break;
                }
            }
        });

        // Key resolution is a quick sync check (env, then Keychain); a key
        // saved in Settings re-resolves without a relaunch.
        let key_missing = muse_api::resolve_api_key().is_none();

        let custom_fields = [
            cx.new(|cx| TextField::new(window, cx, "#D7263D")),
            cx.new(|cx| TextField::new(window, cx, "#FAF8F5")),
            cx.new(|cx| TextField::new(window, cx, "#26221C")),
            cx.new(|cx| TextField::new(window, cx, "#E4485C")),
            cx.new(|cx| TextField::new(window, cx, "#171512")),
            cx.new(|cx| TextField::new(window, cx, "#EDE9E2")),
        ];
        let api_field = cx.new(|cx| TextField::new(window, cx, "sk-ant-…").masked(true));

        let mut this = Workspace {
            store,
            api,
            local: muse_local::spawn(),
            editor,
            sidebar,
            topbar,
            current_entry: String::new(),
            dirty: false,
            save_generation: 0,
            glyph_generation: 0,
            entries_open: true,
            slide: None,
            sidebar_w: layout::SIDEBAR_W,
            resizing_sidebar: false,
            theme_generation: 0,
            theme_pair: boot.pair,
            muted: boot.muted,
            key_missing,
            chattiness: boot.chattiness,
            settings_open: false,
            theme_menu_open: false,
            custom_fields,
            api_field,
            api_saved: false,
            api_saved_generation: 0,
            download_poll_generation: 0,
            engines: HashMap::new(),
            notes: Vec::new(),
            next_note_id: 1,
            last_len_chars: 0,
            considering: false,
            orb_generation: 0,
            toast: None,
            toast_leaving: None,
            toast_generation: 0,
            _subscriptions: subscriptions,
            _poll: poll,
        };

        this.sidebar_w = this
            .store
            .setting("sidebar_width")
            .ok()
            .flatten()
            .and_then(|raw| raw.parse::<f32>().ok())
            .map_or(layout::SIDEBAR_W, |w| w.clamp(MIN_SIDEBAR_W, MAX_SIDEBAR_W));

        // First-launch onboarding: with no API key and no on-device model,
        // quietly start fetching the light model in the background. By the
        // time the first entry has a few paragraphs, Muse can think — no
        // setup screen, no download button, no fanfare.
        if this.key_missing && muse_local::installed_model().is_none() {
            tracing::info!("no brain available; auto-downloading the light on-device model");
            this.local.start_download(muse_local::LocalModel::Light);
        }

        let muted = this.muted;
        let entries_open = this.entries_open;
        this.topbar.update(cx, |topbar, cx| {
            topbar.set_appearance(boot.appearance, cx);
            topbar.set_muted(muted, cx);
            topbar.set_sidebar_open(entries_open, cx);
        });

        this.startup_open(window, cx);

        this
    }

    // ── Entries flow ───────────────────────────────────────────────────────

    /// Launch restoration: list entries (creating one if the store is
    /// empty), then open the last-open entry, falling back to the most
    /// recent.
    fn startup_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut entries = self.store.list_entries().unwrap_or_else(|err| {
            tracing::error!(%err, "failed to list entries at launch");
            Vec::new()
        });
        if entries.is_empty() {
            self.create_entry_record();
            entries = self.store.list_entries().unwrap_or_default();
        }
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_entries(entries.clone(), cx);
        });

        let target = self
            .store
            .last_open_entry()
            .unwrap_or_else(|err| {
                tracing::error!(%err, "failed to read last-open entry");
                None
            })
            .filter(|id| entries.iter().any(|entry| &entry.id == id))
            .or_else(|| entries.first().map(|entry| entry.id.clone()));
        if let Some(id) = target {
            self.open_entry(&id, window, cx);
        }
    }

    /// Open an entry: flush the old one, load and swap the document, then
    /// restore voice, date label, agent state, and focus. Opening never
    /// touches `touched_at` — only edits reorder the sidebar.
    pub(crate) fn open_entry(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        // A pending debounce for the old entry must not flush the new one.
        self.save_generation = self.save_generation.wrapping_add(1);
        self.dirty = false;

        let ulid = Ulid::from_string(id).unwrap_or_else(|err| {
            tracing::error!(%err, id, "entry id is not a ulid; minting a fresh one");
            Ulid::new()
        });
        let doc = match self.store.load_doc(id) {
            Ok(Some(json)) => Document::from_json(ulid, &json).unwrap_or_else(|err| {
                tracing::error!(%err, id, "stored document failed to parse; starting fresh");
                Document::new(ulid)
            }),
            Ok(None) => Document::new(ulid),
            Err(err) => {
                tracing::error!(%err, id, "failed to load document; starting fresh");
                Document::new(ulid)
            }
        };

        self.current_entry = id.to_string();
        self.editor
            .update(cx, |editor, cx| editor.replace_document(doc, cx));
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_selected(Some(id.to_string()), cx);
        });
        if let Err(err) = self.store.set_last_open_entry(id) {
            tracing::error!(%err, "failed to remember last-open entry");
        }


        let label = self
            .store
            .list_entries()
            .ok()
            .and_then(|entries| {
                entries
                    .into_iter()
                    .find(|entry| entry.id == id)
                    .map(|entry| entry.created_at)
            })
            .and_then(date_label);
        self.editor
            .update(cx, |editor, cx| editor.set_date_label(label, cx));

        self.restore_agent_state(cx);

        window.focus(&self.editor.focus_handle(cx));
    }

    /// ⌘N and the sidebar plus button: flush, mint a record (so the sidebar
    /// shows it instantly), and open it.
    pub(crate) fn new_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        let id = self.create_entry_record();
        self.refresh_sidebar(cx);
        self.open_entry(&id, window, cx);
    }

    /// Soft-delete with a 5s undo toast. Deleting the open entry moves to
    /// the most recent remaining one (or a fresh entry if none remain).
    fn delete_entry(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        if let Err(err) = self.store.soft_delete(&id) {
            tracing::error!(%err, "failed to delete entry");
            return;
        }
        self.refresh_sidebar(cx);
        if id == self.current_entry {
            // The deleted entry's edits are already flushed; don't re-flush
            // them while switching away.
            self.dirty = false;
            let next = self
                .store
                .list_entries()
                .unwrap_or_default()
                .first()
                .map(|entry| entry.id.clone());
            match next {
                Some(next_id) => self.open_entry(&next_id, window, cx),
                None => self.new_entry(window, cx),
            }
        }
        self.show_toast(
            Toast {
                message: SharedString::new_static("Entry deleted"),
                entry_id: id,
            },
            cx,
        );
    }

    fn undo_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(err) = self.store.restore(id) {
            tracing::error!(%err, "failed to restore entry");
        }
        self.refresh_sidebar(cx);
        self.dismiss_toast(cx);
    }

    // ── Child-view events ──────────────────────────────────────────────────

    fn on_editor_event(
        &mut self,
        _editor: &Entity<Editor>,
        event: &EditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::Edited => self.on_edited(cx),
            EditorEvent::VoiceChanged => {
            }
            EditorEvent::ScrollChanged => {
                let scrolled = self.editor.read(cx).is_scrolled();
                self.topbar
                    .update(cx, |topbar, cx| topbar.set_scrolled(scrolled, cx));
            }
            EditorEvent::AnnotationDismissed { id } => self.on_annotation_dismissed(*id, cx),
            EditorEvent::SelectionChanged => {}
        }
    }

    fn on_sidebar_event(
        &mut self,
        _sidebar: &Entity<Sidebar>,
        event: &SidebarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SidebarEvent::Open(id) => {
                // Re-opening the visible entry would only reset the caret.
                if id != &self.current_entry {
                    self.open_entry(&id.clone(), window, cx);
                }
            }
            SidebarEvent::DeleteRequested(id) => self.delete_entry(id.clone(), window, cx),
            SidebarEvent::NewEntry => self.new_entry(window, cx),
        }
    }

    // ── Workspace actions ──────────────────────────────────────────────────

    fn act_new_entry(&mut self, _: &cmd::NewEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.new_entry(window, cx);
    }

    fn act_toggle_sidebar(
        &mut self,
        _: &cmd::ToggleSidebar,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = self.sidebar_width_now();
        self.entries_open = !self.entries_open;
        let to = if self.entries_open {
            self.sidebar_w
        } else {
            0.0
        };
        self.slide = Some(Slide {
            from,
            to,
            start: Instant::now(),
        });
        let open = self.entries_open;
        self.topbar
            .update(cx, |topbar, cx| topbar.set_sidebar_open(open, cx));
        cx.notify();
    }

    fn act_toggle_theme(
        &mut self,
        _: &cmd::ToggleTheme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_theme(cx);
    }

    fn act_muse_now(&mut self, _: &cmd::MuseNow, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.brain_available() {
            return;
        }
        let chattiness = self.chattiness;
        self.engines
            .entry(self.current_entry.clone())
            .or_insert_with(|| TriggerEngine::new(chattiness))
            .force();
        cx.notify();
    }

    fn act_toggle_muted(
        &mut self,
        _: &cmd::ToggleMuseMuted,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.muted = !self.muted;
        self.persist_setting("muse_muted", if self.muted { "true" } else { "false" });
        let muted = self.muted;
        self.topbar
            .update(cx, |topbar, cx| topbar.set_muted(muted, cx));
    }

    fn act_quit(&mut self, _: &cmd::Quit, _window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        cx.quit();
    }

    fn act_about(&mut self, _: &cmd::About, _window: &mut Window, _cx: &mut Context<Self>) {
        tracing::info!("Muse {}", env!("CARGO_PKG_VERSION"));
    }

    /// Workspace-level Escape: only fires when the editor isn't handling
    /// it; closes the settings pane, then the `Aa` popover.
    fn act_cancel(&mut self, _: &cmd::Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            self.close_settings(window, cx);
            return;
        }
        self.topbar
            .update(cx, |topbar, cx| topbar.dismiss_popover(window, cx));
    }

    // ── Theme crossfade ────────────────────────────────────────────────────

    /// Toggle Paper/Dusk within the current theme pair.
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let target = cx.theme().appearance.toggled();
        self.set_appearance(target, cx);
    }

    /// Switch to the given appearance (no-op if already there mid-rest),
    /// persisting it and crossfading to that side of the theme pair.
    pub(crate) fn set_appearance(&mut self, target: Appearance, cx: &mut Context<Self>) {
        self.persist_setting(
            "appearance",
            match target {
                Appearance::Paper => "paper",
                Appearance::Dusk => "dusk",
            },
        );
        self.topbar
            .update(cx, |topbar, cx| topbar.set_appearance(target, cx));
        self.crossfade_to(target, self.theme_pair.tokens_for(target), cx);
    }

    /// Replace the whole theme pair (preset or custom apply), persist
    /// nothing here — callers own persistence — and crossfade the current
    /// appearance to its new palette.
    pub(crate) fn apply_theme_pair(&mut self, pair: ThemePair, cx: &mut Context<Self>) {
        self.theme_pair = pair;
        let appearance = cx.theme().appearance;
        self.crossfade_to(appearance, pair.tokens_for(appearance), cx);
    }

    /// The 240ms OKLCH crossfade: a generation-guarded driver task lerps the
    /// token global each frame and refreshes every window. Interruptible —
    /// re-targeting mid-fade starts a new fade from the current tokens.
    fn crossfade_to(&mut self, target: Appearance, target_tokens: Tokens, cx: &mut Context<Self>) {
        let from = cx.theme().tokens;
        self.theme_generation = self.theme_generation.wrapping_add(1);
        let generation = self.theme_generation;
        cx.spawn(async move |this, cx| {
            let start = Instant::now();
            loop {
                cx.background_executor().timer(CROSSFADE_FRAME).await;
                let t = (start.elapsed().as_secs_f32()
                    / motion::THEME_FADE.as_secs_f32())
                .min(1.0);
                let proceed = this.update(cx, |this, cx| {
                    if this.theme_generation != generation {
                        return false;
                    }
                    let tokens = if t >= 1.0 {
                        target_tokens
                    } else {
                        lerp_tokens(&from, &target_tokens, motion::ease_in_out(t))
                    };
                    cx.set_global(Theme {
                        appearance: target,
                        tokens,
                    });
                    cx.refresh_windows();
                    t < 1.0
                });
                if !proceed.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    // ── Toasts ─────────────────────────────────────────────────────────────

    /// Show (replacing any current toast) with a 5s auto-dismiss.
    pub(crate) fn show_toast(&mut self, toast: Toast, cx: &mut Context<Self>) {
        self.toast_generation = self.toast_generation.wrapping_add(1);
        let generation = self.toast_generation;
        self.toast = Some(toast);
        self.toast_leaving = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_LIFE).await;
            this.update(cx, |this, cx| {
                if this.toast_generation == generation {
                    this.dismiss_toast(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn dismiss_toast(&mut self, cx: &mut Context<Self>) {
        self.toast_generation = self.toast_generation.wrapping_add(1);
        let Some(toast) = self.toast.take() else {
            return;
        };
        self.toast_leaving = Some(toast);
        let generation = self.toast_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::FADE).await;
            this.update(cx, |this, cx| {
                if this.toast_generation == generation {
                    this.toast_leaving = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ── Layout ─────────────────────────────────────────────────────────────

    /// The sidebar slot's width this instant (mid-flight aware), without
    /// scheduling a frame.
    fn sidebar_width_now(&self) -> f32 {
        match &self.slide {
            Some(slide) => {
                let t = (slide.start.elapsed().as_secs_f32() / motion::MOVE.as_secs_f32())
                    .clamp(0.0, 1.0);
                slide.from + (slide.to - slide.from) * motion::ease_out_quint(t)
            }
            None => {
                if self.entries_open {
                    self.sidebar_w
                } else {
                    0.0
                }
            }
        }
    }

    /// As [`Self::sidebar_width_now`], but drives the animation: requests
    /// the next frame while mid-flight and retires the slide when done.
    fn animated_sidebar_width(&mut self, window: &Window) -> f32 {
        let Some(slide) = &self.slide else {
            return if self.entries_open {
                self.sidebar_w
            } else {
                0.0
            };
        };
        let t = slide.start.elapsed().as_secs_f32() / motion::MOVE.as_secs_f32();
        if t >= 1.0 {
            let to = slide.to;
            self.slide = None;
            to
        } else {
            window.request_animation_frame();
            slide.from + (slide.to - slide.from) * motion::ease_out_quint(t.max(0.0))
        }
    }

    fn render_toast(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (toast, entering) = match (&self.toast, &self.toast_leaving) {
            (Some(toast), _) => (toast, true),
            (None, Some(toast)) => (toast, false),
            (None, None) => return None,
        };
        let tokens = cx.theme().tokens;
        let (message, entry_id) = (toast.message.clone(), toast.entry_id.clone());

        let content = pill().child(
            div()
                .px(px(8.))
                .py(px(2.))
                .text_size(px(layout::UI_BODY))
                .text_color(tokens.ink)
                .child(message),
        );
        // The button stays mounted while the toast fades out (inert),
        // so the pill never changes width mid-fade.
        let mut undo = text_button("toast-undo", "Undo");
        if entering {
            undo = undo.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.undo_delete(&entry_id, cx);
            }));
        }
        let content = content.child(undo);

        Some(
            div()
                .absolute()
                .bottom(px(24.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div().child(content).with_animation(
                        ElementId::NamedInteger(
                            if entering { "toast-in" } else { "toast-out" }.into(),
                            self.toast_generation,
                        ),
                        Animation::new(motion::FADE).with_easing(motion::ease_out_quint),
                        move |el, t| el.opacity(if entering { t } else { 1.0 - t }),
                    ),
                )
                .into_any_element(),
        )
    }

}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.theme().tokens;
        let width = self.animated_sidebar_width(window);

        div()
            .key_context(cmd::WORKSPACE_CONTEXT)
            .on_action(cx.listener(Self::act_new_entry))
            .on_action(cx.listener(Self::act_toggle_sidebar))
            .on_action(cx.listener(Self::act_toggle_theme))
            .on_action(cx.listener(Self::act_muse_now))
            .on_action(cx.listener(Self::act_toggle_muted))
            .on_action(cx.listener(Self::act_quit))
            .on_action(cx.listener(Self::act_about))
            .on_action(cx.listener(Self::act_cancel))
            .on_action(cx.listener(Self::act_open_settings))
            .relative()
            .size_full()
            .flex()
            .bg(tokens.bg)
            .font_family(FONT_UI)
            .text_size(px(layout::UI_TEXT))
            .text_color(tokens.ink)
            .when(self.resizing_sidebar, |this| {
                this.on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    let next = f32::from(event.position.x).clamp(MIN_SIDEBAR_W, MAX_SIDEBAR_W);
                    if (next - this.sidebar_w).abs() > 0.5 {
                        this.sidebar_w = next;
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| {
                        this.resizing_sidebar = false;
                        this.persist_setting("sidebar_width", &format!("{:.0}", this.sidebar_w));
                        cx.notify();
                    }),
                )
            })
            .child(
                // The sidebar slot: the one sanctioned layout animation.
                // The inner panel is fixed-width and slides with the slot's
                // right edge, clipped rather than squished.
                div()
                    .flex_none()
                    .h_full()
                    .w(px(width))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_none()
                            .h_full()
                            .w(px(self.sidebar_w))
                            .ml(px(width - self.sidebar_w))
                            .child(self.sidebar.clone()),
                    ),
            )
            // The divider drag handle: a slim invisible strip over the
            // sidebar's right edge; drag to resize.
            .when(self.entries_open && self.slide.is_none(), |this| {
                this.child(
                    div()
                        .id("sidebar-resize")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(width - 3.0))
                        .w(px(6.))
                        .cursor_col_resize()
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.resizing_sidebar = true;
                                cx.notify();
                            }),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_none()
                            .h(px(layout::TOPBAR_H))
                            .child(self.topbar.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .relative()
                            .child(self.editor.clone())
                            // Scrolling text slips under this fade instead of
                            // hitting a hard hairline.
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .right_0()
                                    .h(px(24.))
                                    .bg(gpui::linear_gradient(
                                        180.,
                                        gpui::linear_color_stop(tokens.bg, 0.),
                                        gpui::linear_color_stop(tokens.bg.opacity(0.), 1.),
                                    )),
                            ),
                    ),
            )
            .children(self.render_settings(window, cx))
            .children(self.render_toast(cx))
    }
}
