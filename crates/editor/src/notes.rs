//! Margin annotations: presentation-neutral note types, their anchored
//! lifecycle (bloom → rest → wither), and the word-by-word reveal used by
//! note cards and the coda.
//!
//! The editor knows nothing about who wrote a note — the app maps agent
//! output into [`Annotation`]s.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::SharedString;
use muse_core::AnchorId;
use unicode_segmentation::UnicodeSegmentation;

/// How long a note whose anchor text was deleted takes to fade out before
/// being dropped silently.
pub(crate) const WITHER: Duration = Duration::from_millis(400);

/// Duration of the word-by-word text reveal in note cards and the coda.
pub(crate) const REVEAL: Duration = Duration::from_millis(600);

/// A margin note anchored to a passage. `range` is the anchor range at the
/// time the note was handed to the editor; thereafter the editor's document
/// anchor tracks it through edits.
#[derive(Debug, Clone)]
pub struct Annotation {
    /// Stable id the app uses to correlate dismissals.
    pub id: u64,
    /// Anchored byte range at creation time.
    pub range: Range<usize>,
    /// What kind of note this is; labels the card.
    pub tone: AnnotationTone,
    /// The note text. May be empty for a pure emoji reaction.
    pub body: SharedString,
    /// Reaction emoji. When present the anchored text renders as an
    /// iMessage-style highlight with the emoji badged on it; when absent the
    /// note is a quiet citation caret that reveals a card on hover.
    pub emoji: Option<SharedString>,
}

/// The register of a margin note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnnotationTone {
    /// An observation worth keeping.
    Insight,
    /// A question back at the writer.
    Question,
    /// Warmth, no agenda.
    Encouragement,
    /// Something looks wrong (the one accent-tinted tone).
    Correction,
    /// A pointer worth chasing.
    Reference,
}

impl AnnotationTone {
    /// The typewriter-stamp label on the bloomed card.
    pub(crate) fn label(self) -> &'static str {
        match self {
            AnnotationTone::Insight => "A THOUGHT",
            AnnotationTone::Question => "A QUESTION",
            AnnotationTone::Encouragement => "ENCOURAGEMENT",
            AnnotationTone::Correction => "A CATCH",
            AnnotationTone::Reference => "WORTH A LOOK",
        }
    }
}

/// One annotation as the editor tracks it.
pub(crate) struct NoteSlot {
    pub ann: Annotation,
    pub anchor: AnchorId,
    /// When the marker appeared (drives the bloom / reaction pop).
    pub appeared: Instant,
    /// Set the moment the anchor resolves to `None`; the marker fades for
    /// [`WITHER`] and is then dropped without an event.
    pub withering: Option<Instant>,
    /// Content-coordinate center of the hover/click marker from the last
    /// layout (the emoji badge for reactions, the caret for notes) — it
    /// keeps its place while withering (the anchor is already gone).
    pub last_center: Option<(f32, f32)>,
    /// Highlight rectangles for the anchored text, from the last layout.
    /// Non-empty only for reactions.
    pub last_rects: Vec<crate::layout::SelectionRect>,
}

/// The longest prefix of `text` containing `ceil(t × words)` words, for the
/// client-side typewriter reveal. `t` is the eased animation delta in
/// `0.0..=1.0`; whitespace and punctuation attach to the word they follow.
pub(crate) fn reveal_prefix(text: &str, t: f32) -> &str {
    if t >= 1.0 {
        return text;
    }
    let bounds: Vec<(usize, &str)> = text
        .split_word_bound_indices()
        .filter(|(_, seg)| seg.chars().any(|c| c.is_alphanumeric()))
        .collect();
    let total = bounds.len();
    if total == 0 {
        return if t > 0.0 { text } else { "" };
    }
    let shown = ((t.max(0.0) * total as f32).ceil() as usize).min(total);
    if shown == 0 {
        return "";
    }
    if shown == total {
        return text;
    }
    let (idx, seg) = bounds[shown - 1];
    &text[..idx + seg.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_walks_word_by_word() {
        let text = "two roads diverged in a wood";
        assert_eq!(reveal_prefix(text, 0.0), "");
        assert_eq!(reveal_prefix(text, 1.0), text);
        // Six words: t = 0.5 shows ceil(3) = 3 words.
        assert_eq!(reveal_prefix(text, 0.5), "two roads diverged");
        // Monotonic: later t never shows less.
        let mut prev = 0;
        for i in 0..=20 {
            let p = reveal_prefix(text, i as f32 / 20.0).len();
            assert!(p >= prev);
            prev = p;
        }
    }

    #[test]
    fn reveal_handles_punctuation_and_empty() {
        assert_eq!(reveal_prefix("", 0.5), "");
        assert_eq!(reveal_prefix("…", 0.5), "…");
        let text = "well, yes — maybe.";
        assert_eq!(reveal_prefix(text, 1.0), text);
        // One word in: punctuation before the next word stays hidden.
        assert_eq!(reveal_prefix(text, 0.34), "well, yes");
    }
}
