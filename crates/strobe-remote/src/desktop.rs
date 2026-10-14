// ============================================================================
// Module:       strobe_remote::desktop
// Description:  Exclusive desktop-control lease, bounded UI inputs and latest
//               captured control-window frame, independent of any UI toolkit.
//
// Dependencies: serde; std::sync; crate::ServerMessage
// ============================================================================

//! Full desktop access is off by default and separate from phone access.
//! One authenticated connection owns input at a time. Disconnect, revocation
//! and disabling release the lease and request an input reset on the app.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

/// Maximum edge events queued, including text. Overflow releases control
/// instead of silently dropping a key or button release.
const QUEUE: usize = 128;
/// Largest capture dimension. Bounds CPU readback and decoded image memory.
pub const MAX_EDGE: u32 = 4096;
/// Largest encoded JPEG, before base64.
pub const MAX_JPEG: usize = 2 * 1024 * 1024;
/// Maximum JSON server frame, including base64 overhead.
pub const MAX_FRAME: usize = 3 * 1024 * 1024;
/// What a full desktop pairing's label starts with, in the approval
/// request and the paired list, so the operator can tell it from a phone.
pub const LABEL_PREFIX: &str = "Full desktop: ";

/// Keyboard modifiers accompanying each input event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modifiers {
    /// Alt / Option.
    pub alt: bool,
    /// Control.
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Command (on macOS).
    pub mac_cmd: bool,
}

/// Toolkit-independent inputs. Coordinates are normalized to the image,
/// so different controller DPI and letterboxing do not change the target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    /// Pointer position in the captured host window.
    Move {
        /// Horizontal fraction of the host window.
        x: f32,
        /// Vertical fraction of the host window.
        y: f32,
    },
    /// Pointer button 0 primary, 1 secondary, 2 middle, at this position.
    Button {
        /// Horizontal fraction of the host window.
        x: f32,
        /// Vertical fraction of the host window.
        y: f32,
        /// Primary, secondary or middle button (0, 1, 2).
        button: u8,
        /// Press rather than release.
        down: bool,
        /// Held modifier keys.
        modifiers: Modifiers,
    },
    /// Scroll in logical points.
    Scroll {
        /// Horizontal logical points.
        x: f32,
        /// Vertical logical points.
        y: f32,
        /// Held modifier keys.
        modifiers: Modifiers,
    },
    /// An egui key name; text is sent separately.
    Key {
        /// Stable toolkit key name.
        key: String,
        /// Press rather than release.
        down: bool,
        /// Held modifier keys.
        modifiers: Modifiers,
    },
    /// Committed Unicode text, including IME input.
    Text {
        /// Committed Unicode text.
        text: String,
    },
    /// Clipboard paste into the focused host widget.
    Paste {
        /// Unicode clipboard text.
        text: String,
    },
    /// Release held inputs when the viewer loses focus.
    Reset {},
    /// Copy the focused widget's selection.
    Copy {},
    /// Cut the focused widget's selection.
    Cut {},
}

/// Validate before enqueueing, including nonfinite and oversized input.
pub fn valid_inputs(events: &[Input]) -> bool {
    let point = |x: f32, y: f32| {
        x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)
    };
    !events.is_empty()
        && events.len() <= 32
        && events.iter().all(|event| match event {
            Input::Move { x, y } => point(*x, *y),
            Input::Button { x, y, button, .. } => point(*x, *y) && *button <= 2,
            Input::Scroll { x, y, .. } => {
                x.is_finite() && y.is_finite() && x.abs() <= 4096.0 && y.abs() <= 4096.0
            }
            Input::Key { key, .. } => {
                !key.is_empty()
                    && key.len() <= 24
                    && key.bytes().all(|b| (0x21..=0x7e).contains(&b))
            }
            Input::Text { text } | Input::Paste { text } => {
                text.len() <= 2048 && !text.contains('\0')
            }
            Input::Reset {} | Input::Copy {} | Input::Cut {} => true,
        })
}

#[derive(Debug, Default)]
struct State {
    enabled: bool,
    owner: Option<u64>,
    inputs: VecDeque<Input>,
    reset: bool,
    revision: u64,
    frame: Option<Arc<crate::ServerMessage>>,
    clipboard: Option<(u64, Arc<crate::ServerMessage>)>,
    clipboard_revision: u64,
}

/// Shared desktop bridge. All operations are short and never perform I/O.
#[derive(Clone, Debug, Default)]
pub struct Desktop(Arc<Mutex<State>>);

impl Desktop {
    /// Whether the host's local full-desktop switch is enabled.
    pub fn enabled(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .enabled
    }
    /// Explicit local opt-in. Disabling discards queued inputs and frames.
    pub fn enable(&self, enabled: bool) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.enabled = enabled;
        if !enabled {
            state.owner = None;
            state.inputs.clear();
            state.frame = None;
            state.clipboard = None;
            state.reset = true;
        }
    }

    /// Grant one connection exclusive input; never steal another lease.
    pub fn acquire(&self, connection: u64) -> bool {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.enabled || state.owner.is_some_and(|owner| owner != connection) {
            return false;
        }
        state.owner = Some(connection);
        true
    }

    /// Drop the connection's lease and release its held input on the app.
    pub fn release(&self, connection: u64) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.owner == Some(connection) {
            state.owner = None;
            state.inputs.clear();
            state.frame = None;
            state.clipboard = None;
            state.reset = true;
        }
    }

    /// Whether this connection still owns desktop access.
    pub fn owns(&self, connection: u64) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).owner == Some(connection)
    }

    /// Whether the host should capture a frame.
    pub fn active(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .owner
            .is_some()
    }

    /// Submit a validated batch. Overflow fails closed, releasing held input.
    pub fn submit(&self, connection: u64, events: Vec<Input>) -> bool {
        if !valid_inputs(&events) {
            return false;
        }
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.enabled || state.owner != Some(connection) {
            return false;
        }
        if state.inputs.len() + events.len() > QUEUE {
            state.owner = None;
            state.inputs.clear();
            state.frame = None;
            state.clipboard = None;
            state.reset = true;
            return false;
        }
        state.inputs.extend(events);
        true
    }

    /// Drain on the app thread. Reset precedes events from a new owner.
    pub fn drain(&self, out: &mut Vec<Input>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if std::mem::take(&mut state.reset) {
            out.push(Input::Reset {});
        }
        out.extend(state.inputs.drain(..));
    }

    /// Publish only the latest image; slow readers never accumulate frames.
    pub fn publish(&self, size: [u32; 2], jpeg: String) {
        if size.contains(&0)
            || size.iter().any(|edge| *edge > MAX_EDGE)
            || jpeg.len() > MAX_FRAME - 1024
        {
            return;
        }
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.owner.is_none() {
            return;
        }
        state.revision = state.revision.saturating_add(1);
        state.frame = Some(Arc::new(crate::ServerMessage::DesktopFrame {
            revision: state.revision,
            size,
            jpeg,
        }));
    }

    /// Report capture failure to the owner instead of leaving a stale image
    /// interactive. The controller closes and releases its lease.
    pub fn capture_failed(&self, reason: &str) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.owner.is_none() {
            return;
        }
        state.revision = state.revision.saturating_add(1);
        state.frame = Some(Arc::new(crate::ServerMessage::error(
            crate::ErrorCode::Unavailable,
            reason,
        )));
    }

    /// Latest frame and revision; image bytes are shared rather than copied.
    pub fn frame(&self) -> Option<(u64, Arc<crate::ServerMessage>)> {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .frame
            .as_ref()
            .map(|frame| (state.revision, Arc::clone(frame)))
    }

    /// Forward only text explicitly copied by a host UI widget. Never reads
    /// the host OS clipboard; bounded separately from the window stream.
    pub fn copy_text(&self, text: &str) {
        if text.len() > 64 * 1024 {
            return;
        }
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.owner.is_none() {
            return;
        }
        state.clipboard_revision = state.clipboard_revision.saturating_add(1);
        state.clipboard = Some((
            state.clipboard_revision,
            Arc::new(crate::ServerMessage::DesktopClipboard {
                text: text.to_owned(),
            }),
        ));
    }

    /// Latest explicitly copied text, shared with the lease holder.
    pub fn clipboard(&self) -> Option<(u64, Arc<crate::ServerMessage>)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clipboard
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_is_explicit_exclusive_and_resets_on_release() {
        let desktop = Desktop::default();
        assert!(!desktop.acquire(1), "off by default");
        desktop.enable(true);
        assert!(desktop.acquire(1), "first controller");
        assert!(!desktop.acquire(2), "exclusive");
        assert!(
            !desktop.submit(2, vec![Input::Reset {}]),
            "nonowner refused"
        );
        assert!(
            desktop.submit(
                1,
                vec![Input::Text {
                    text: "shader".into()
                }]
            ),
            "owner queued"
        );
        desktop.release(1);
        let mut events = Vec::new();
        desktop.drain(&mut events);
        assert_eq!(
            events,
            vec![Input::Reset {}],
            "discard queued work and release input"
        );
        assert!(desktop.acquire(2), "next controller");
        desktop.enable(false);
        assert!(!desktop.owns(2) && !desktop.active(), "kill switch");
    }

    #[test]
    fn overflow_and_invalid_input_cannot_stick_a_key() {
        let desktop = Desktop::default();
        desktop.enable(true);
        assert!(desktop.acquire(1), "lease");
        for _ in 0..4 {
            assert!(
                desktop.submit(1, vec![Input::Reset {}; 32]),
                "bounded batch"
            );
        }
        assert!(
            !desktop.submit(1, vec![Input::Reset {}]),
            "full releases lease"
        );
        assert!(!desktop.active(), "released");
        assert!(
            !valid_inputs(&[Input::Move {
                x: f32::NAN,
                y: 0.0
            }]),
            "nonfinite"
        );
        assert!(
            !valid_inputs(&[Input::Button {
                x: 0.0,
                y: 0.0,
                button: 9,
                down: true,
                modifiers: Modifiers::default()
            }]),
            "unknown button"
        );
        assert!(
            !valid_inputs(&[Input::Text {
                text: "x".repeat(2049)
            }]),
            "text bound"
        );
    }
}
