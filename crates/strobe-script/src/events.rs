// ============================================================================
// Module:       strobe_script::events
// Description:  The event system: the five handler kinds, the 0.5 threshold,
//               and the per-handler previous-value bookkeeping.
//
// Dependencies: std only
// ============================================================================

//! Event handlers.
//!
//! `onChange`, `onOffToOn`, `onOnToOff`, `whileOn` and `whileOff` each
//! watch one input and name a JavaScript function to call. This module is
//! only the decision of *whether* to call; the host does the calling.
//!
//! "On" is `value >= 0.5` and "off" is anything below, so every value is
//! one or the other. The documentation says "greater than" and "less than"
//! and leaves exactly 0.5 undefined; counting it as on keeps a toggle
//! whose `MAX` is 1 and a half-way slider consistent with each other.
//!
//! A handler has no previous value on its first frame, so it takes the
//! current one: `onChange` and the edge handlers do not fire just because
//! the scene started, while `whileOn` and `whileOff` do fire on frame one
//! if the input is already in that state.

use std::rc::Rc;

/// The value at and above which an input counts as on.
pub(crate) fn is_on(value: f32) -> bool {
    value >= 0.5
}

/// Which condition a handler waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventKind {
    /// The value differs from last frame's.
    Change,
    /// Off last frame, on now.
    OffToOn,
    /// On last frame, off now.
    OnToOff,
    /// On this frame.
    WhileOn,
    /// Off this frame.
    WhileOff,
}

impl EventKind {
    /// Whether the transition `previous` to `current` triggers this kind.
    pub(crate) fn fires(self, previous: f32, current: f32) -> bool {
        match self {
            // NaN != NaN, so a NaN input would "change" every frame.
            Self::Change => previous != current && !(previous.is_nan() && current.is_nan()),
            Self::OffToOn => !is_on(previous) && is_on(current),
            Self::OnToOff => is_on(previous) && !is_on(current),
            Self::WhileOn => is_on(current),
            Self::WhileOff => !is_on(current),
        }
    }
}

/// What a handler watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// The audio scalar at this index of `strobe_scene::audio::SCALARS`.
    Audio(usize),
    /// `TIME`.
    Time,
    /// `FRAMECOUNT`.
    FrameCount,
    /// The output of the control at this index; for a vector control, its
    /// first component.
    Control(usize),
}

/// One registered handler.
#[derive(Debug, Clone)]
pub(crate) struct Handler {
    /// The condition.
    pub(crate) kind: EventKind,
    /// The input watched.
    pub(crate) source: Source,
    /// The global function to call.
    pub(crate) callback: Rc<str>,
    /// The input's value last frame; `None` before the first frame.
    previous: Option<f32>,
}

impl Handler {
    /// A handler that has not seen a frame yet.
    pub(crate) fn new(kind: EventKind, source: Source, callback: Rc<str>) -> Self {
        Self {
            kind,
            source,
            callback,
            previous: None,
        }
    }

    /// Whether `other` is the same registration, so a script that
    /// re-registers every frame does not stack up duplicates.
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.kind == other.kind && self.source == other.source && self.callback == other.callback
    }

    /// Feeds this frame's value. Returns the previous value to pass to the
    /// callback when it should be called.
    pub(crate) fn step(&mut self, current: f32) -> Option<f32> {
        let previous = self.previous.unwrap_or(current);
        self.previous = Some(current);
        self.kind.fires(previous, current).then_some(previous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_fire_only_on_the_crossing() {
        assert!(EventKind::OffToOn.fires(0.0, 1.0), "up through 0.5");
        assert!(
            !EventKind::OffToOn.fires(1.0, 1.0),
            "staying on is not an edge"
        );
        assert!(EventKind::OnToOff.fires(1.0, 0.0), "down through 0.5");
        assert!(
            !EventKind::OnToOff.fires(0.0, 0.0),
            "staying off is not an edge"
        );
    }

    #[test]
    fn half_counts_as_on() {
        assert!(EventKind::WhileOn.fires(0.0, 0.5), "0.5 is on");
        assert!(!EventKind::WhileOff.fires(0.0, 0.5), "so it is not off");
    }

    #[test]
    fn a_handler_does_not_fire_on_its_first_frame_for_edges() {
        let mut handler = Handler::new(EventKind::OffToOn, Source::Time, Rc::from("cb"));
        assert_eq!(handler.step(1.0), None, "already on at start is no edge");
        assert_eq!(handler.step(0.0), None, "falling is not rising");
        assert_eq!(
            handler.step(1.0),
            Some(0.0),
            "rising passes the previous value"
        );
    }

    #[test]
    fn nan_is_not_a_perpetual_change() {
        assert!(!EventKind::Change.fires(f32::NAN, f32::NAN), "NaN to NaN");
        assert!(
            EventKind::Change.fires(0.0, f32::NAN),
            "a real change to NaN"
        );
    }
}
