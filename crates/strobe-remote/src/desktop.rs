// ============================================================================
// Module:       strobe_remote::desktop
// Description:  Exclusive desktop-control lease, bounded UI inputs and latest
//               captured control-window frame, and the controls as data with
//               their edits, independent of any UI toolkit.
//
// Dependencies: serde, serde_json; std::sync; crate::ServerMessage
// ============================================================================

//! Full desktop access is off by default and separate from phone access.
//! One authenticated connection owns input at a time. Disconnect, revocation
//! and disabling release the lease and request an input reset on the app.
//!
//! The owner may also ask for the controls as data (D513): the host
//! publishes its banks of controls ([`SyncedBank`]) and the controller draws
//! them itself, sending back only what it changed ([`ControlEdit`]). Moving
//! a slider then costs a few bytes each way instead of a picture of the
//! window.

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
/// What a pairing made by trust file (D288) starts with, so the paired
/// list can say "trusted".
pub const TRUSTED_LABEL_PREFIX: &str = "Full desktop: trusted ";

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

/// Most control edits in one message (D513).
pub const MAX_CONTROL_EDITS: usize = 32;

/// Most controls in one synced bank: an edit naming a later one is not a
/// control (D513).
pub const MAX_SYNCED_CONTROLS: usize = 256;

/// One change the controller made to a control it draws itself (D513):
/// the control's new target in its own units, as the host's
/// `ControlBank::set` takes it. No values is a bang's press.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEdit {
    /// The bank, by the id the host gave it in `desktop_controls`.
    pub bank: u64,
    /// The control's place in that bank.
    pub index: u32,
    /// The new target, one value per component (at most three); none for
    /// a press.
    pub values: Vec<f32>,
}

/// Whether `edits` may be queued: 1 to [`MAX_CONTROL_EDITS`] of them, each
/// naming a control within [`MAX_SYNCED_CONTROLS`] with at most three
/// finite values.
pub fn valid_edits(edits: &[ControlEdit]) -> bool {
    !edits.is_empty()
        && edits.len() <= MAX_CONTROL_EDITS
        && edits.iter().all(|edit| {
            (edit.index as usize) < MAX_SYNCED_CONTROLS
                && edit.values.len() <= 3
                && edit.values.iter().all(|value| value.is_finite())
        })
}

/// One bank of controls as the host publishes it (D513): the playing
/// scene's, or the output's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncedBank {
    /// Changes whenever the controls themselves do (another scene, an
    /// edit to the scene), so an edit made against old controls is
    /// dropped rather than applied to whatever now sits at its index.
    pub id: u64,
    /// What the controller heads the bank with.
    pub title: String,
    /// The controls as `scene.json` entries. Sent the first time a
    /// connection sees this id, and left out after, when only values move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defs: Option<Vec<serde_json::Value>>,
    /// Every control's target, in order.
    pub values: Vec<[f32; 3]>,
}

/// Most control rows one picture places (D515).
pub const MAX_CONTROL_ROWS: usize = 512;

/// Where the host drew one control row in the picture that carries the
/// same revision (D515), so the controller can draw its own copy over it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRow {
    /// The bank, by the id the host gave it in `desktop_controls`.
    pub bank: u64,
    /// The control's place in that bank.
    pub index: u32,
    /// Left, top, width and height, as fractions of the picture.
    pub rect: [f32; 4],
    /// Drawn at the Controls page's scale rather than the side panel's.
    pub touch: bool,
}

/// Whether `rows` may be sent: at most [`MAX_CONTROL_ROWS`], each naming a
/// control within [`MAX_SYNCED_CONTROLS`] with a rectangle inside the
/// picture.
pub fn valid_rows(rows: &[ControlRow]) -> bool {
    rows.len() <= MAX_CONTROL_ROWS
        && rows.iter().all(|row| {
            let [x, y, w, h] = row.rect;
            (row.index as usize) < MAX_SYNCED_CONTROLS
                && row.rect.iter().all(|value| value.is_finite())
                && x >= 0.0
                && y >= 0.0
                && w > 0.0
                && h > 0.0
                && x + w <= 1.0 + 1e-3
                && y + h <= 1.0 + 1e-3
        })
}

/// Where one picture has its control rows, and the size of the window it
/// shows in the host's points (D515, D519). The controller lays its copies
/// out at that size and scales them as it scales the picture, so a row
/// drawn there fits the picture's row exactly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlLayout {
    /// The host's window, in its points.
    pub screen: [f32; 2],
    /// The rows drawn and visible.
    pub rows: Vec<ControlRow>,
}

impl ControlLayout {
    /// Whether it may be sent: a real window size, and [`valid_rows`].
    pub fn valid(&self) -> bool {
        self.screen
            .iter()
            .all(|edge| edge.is_finite() && *edge > 0.0 && *edge <= MAX_EDGE as f32)
            && valid_rows(&self.rows)
    }
}

#[derive(Debug, Default)]
struct State {
    enabled: bool,
    owner: Option<u64>,
    inputs: VecDeque<Input>,
    reset: bool,
    revision: u64,
    frame: Option<Arc<crate::ServerMessage>>,
    /// Where the frame of the same revision has its control rows (D515).
    layout: Option<Arc<crate::ServerMessage>>,
    clipboard: Option<(u64, Arc<crate::ServerMessage>)>,
    clipboard_revision: u64,
    /// The owner asked for the controls as data (D513).
    controls_wanted: bool,
    /// The latest published banks, every one with its definitions.
    controls: Option<Arc<Vec<SyncedBank>>>,
    controls_revision: u64,
    edits: VecDeque<ControlEdit>,
}

impl State {
    /// Forgets everything the last owner had: its queued input and edits,
    /// the frame and controls published for it, and its wish for them.
    fn clear_owner(&mut self) {
        self.owner = None;
        self.inputs.clear();
        self.frame = None;
        self.layout = None;
        self.clipboard = None;
        self.reset = true;
        self.controls_wanted = false;
        self.controls = None;
        self.edits.clear();
    }
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
            state.clear_owner();
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
            state.clear_owner();
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
    ///
    /// A pointer move that follows another move replaces it: only the latest
    /// position matters to the host's next pass, and a controller moving its
    /// mouse while the host's frame is held up (a scene compiling) would
    /// otherwise fill the queue with positions and lose the lease. Every
    /// other event, a button or key edge above all, keeps its own slot.
    pub fn submit(&self, connection: u64, events: Vec<Input>) -> bool {
        if !valid_inputs(&events) {
            return false;
        }
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.enabled || state.owner != Some(connection) {
            return false;
        }
        for event in events {
            if matches!(event, Input::Move { .. })
                && let Some(last @ Input::Move { .. }) = state.inputs.back_mut()
            {
                *last = event;
                continue;
            }
            if state.inputs.len() >= QUEUE {
                state.clear_owner();
                return false;
            }
            state.inputs.push_back(event);
        }
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
        self.publish_with_rows(size, jpeg, ControlLayout::default());
    }

    /// As [`Self::publish`], with where the picture has its control rows
    /// (D515). The rows go to an owner that asked for the controls, under
    /// the frame's revision; rows that are not valid are not sent.
    pub fn publish_with_rows(&self, size: [u32; 2], jpeg: String, layout: ControlLayout) {
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
        state.layout =
            (state.controls_wanted && layout.valid() && !layout.rows.is_empty()).then(|| {
                Arc::new(crate::ServerMessage::DesktopLayout {
                    revision: state.revision,
                    screen: layout.screen,
                    rows: layout.rows,
                })
            });
        state.frame = Some(Arc::new(crate::ServerMessage::DesktopFrame {
            revision: state.revision,
            size,
            jpeg,
        }));
    }

    /// Where the latest frame has its control rows, under its revision.
    pub fn layout(&self) -> Option<(u64, Arc<crate::ServerMessage>)> {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .layout
            .as_ref()
            .map(|layout| (state.revision, Arc::clone(layout)))
    }

    /// Report capture failure to the owner instead of leaving a stale image
    /// interactive. The controller closes and releases its lease.
    pub fn capture_failed(&self, reason: &str) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.owner.is_none() {
            return;
        }
        state.revision = state.revision.saturating_add(1);
        state.layout = None;
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

    /// The owner asks for the controls as data, to draw them itself
    /// (D513). Anyone else is refused.
    pub fn want_controls(&self, connection: u64) -> bool {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.enabled || state.owner != Some(connection) {
            return false;
        }
        state.controls_wanted = true;
        true
    }

    /// Whether the host should publish its controls: an owner asked.
    pub fn controls_wanted(&self) -> bool {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.owner.is_some() && state.controls_wanted
    }

    /// Publishes the banks as they are now, each with its definitions; a
    /// connection leaves out the ones it has already sent.
    pub fn publish_controls(&self, banks: Vec<SyncedBank>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.owner.is_none() || !state.controls_wanted {
            return;
        }
        state.controls_revision = state.controls_revision.saturating_add(1);
        state.controls = Some(Arc::new(banks));
    }

    /// The latest published banks and their revision.
    pub fn controls(&self) -> Option<(u64, Arc<Vec<SyncedBank>>)> {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .controls
            .as_ref()
            .map(|banks| (state.controls_revision, Arc::clone(banks)))
    }

    /// Queues the owner's control edits. A new value for a control whose
    /// last one the app has yet to take replaces it, as a pointer move
    /// does; a press keeps its own slot. Overflow fails closed, as input
    /// does.
    pub fn submit_controls(&self, connection: u64, edits: Vec<ControlEdit>) -> bool {
        if !valid_edits(&edits) {
            return false;
        }
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.enabled || state.owner != Some(connection) || !state.controls_wanted {
            return false;
        }
        for edit in edits {
            if !edit.values.is_empty()
                && let Some(queued) = state.edits.iter_mut().find(|queued| {
                    !queued.values.is_empty()
                        && queued.bank == edit.bank
                        && queued.index == edit.index
                })
            {
                *queued = edit;
                continue;
            }
            if state.edits.len() >= QUEUE {
                state.clear_owner();
                return false;
            }
            state.edits.push_back(edit);
        }
        true
    }

    /// Takes the queued control edits, on the app thread.
    pub fn drain_controls(&self, out: &mut Vec<ControlEdit>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        out.extend(state.edits.drain(..));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(bank: u64, index: u32, value: f32) -> ControlEdit {
        ControlEdit {
            bank,
            index,
            values: vec![value],
        }
    }

    fn bank(id: u64) -> SyncedBank {
        SyncedBank {
            id,
            title: "Scene".into(),
            defs: Some(Vec::new()),
            values: vec![[0.5; 3]],
        }
    }

    #[test]
    fn controls_go_only_to_the_owner_that_asked_and_end_with_its_lease() {
        let desktop = Desktop::default();
        desktop.enable(true);
        assert!(desktop.acquire(1), "owner");
        assert!(!desktop.want_controls(2), "not the owner");
        desktop.publish_controls(vec![bank(1)]);
        assert!(
            desktop.controls().is_none() && !desktop.controls_wanted(),
            "nothing is published before the owner asks"
        );
        assert!(
            !desktop.submit_controls(1, vec![set(1, 0, 0.1)]),
            "edits before asking are refused"
        );
        assert!(desktop.want_controls(1), "the owner asks");
        desktop.publish_controls(vec![bank(1)]);
        assert!(
            desktop
                .controls()
                .is_some_and(|(_, banks)| banks.len() == 1),
            "published"
        );
        assert!(
            !desktop.submit_controls(2, vec![set(1, 0, 0.1)]),
            "another connection's edits are refused"
        );
        assert!(
            desktop.submit_controls(1, vec![set(1, 0, 0.1)]),
            "owner's edit"
        );
        desktop.release(1);
        let mut edits = Vec::new();
        desktop.drain_controls(&mut edits);
        assert!(
            edits.is_empty() && desktop.controls().is_none() && !desktop.controls_wanted(),
            "the lease's end takes its edits, its controls and its wish with it"
        );
        assert!(desktop.acquire(2), "the next owner");
        assert!(!desktop.controls_wanted(), "the next owner asks for itself");
    }

    #[test]
    fn rows_go_with_their_frame_only_to_an_owner_that_asked_for_the_controls() {
        let desktop = Desktop::default();
        desktop.enable(true);
        assert!(desktop.acquire(1), "owner");
        let row = ControlRow {
            bank: 1,
            index: 0,
            rect: [0.1, 0.1, 0.5, 0.05],
            touch: true,
        };
        let layout = |rows: Vec<ControlRow>, screen: [f32; 2]| ControlLayout { screen, rows };
        desktop.publish_with_rows([8, 8], "jpeg".into(), layout(vec![row.clone()], [8.0, 8.0]));
        assert!(desktop.layout().is_none(), "not asked: no rows");
        assert!(desktop.want_controls(1), "asks");
        desktop.publish_with_rows([8, 8], "jpeg".into(), layout(vec![row.clone()], [8.0, 8.0]));
        let frame = desktop.frame().map(|(revision, _)| revision);
        let sent = desktop.layout().map(|(revision, _)| revision);
        assert!(
            frame.is_some() && sent == frame,
            "the rows under the frame's revision"
        );
        let outside = ControlRow {
            rect: [0.9, 0.1, 0.5, 0.05],
            ..row.clone()
        };
        desktop.publish_with_rows([8, 8], "jpeg".into(), layout(vec![outside], [8.0, 8.0]));
        assert!(
            desktop.layout().is_none() && desktop.frame().is_some(),
            "rows outside the picture are not sent; the frame still is"
        );
        desktop.publish_with_rows([8, 8], "jpeg".into(), layout(vec![row], [0.0, 8.0]));
        assert!(
            desktop.layout().is_none(),
            "nor rows of a window with no size (D519)"
        );
    }

    #[test]
    fn a_newer_value_replaces_an_untaken_one_and_presses_keep_their_slots() {
        let desktop = Desktop::default();
        desktop.enable(true);
        assert!(desktop.acquire(1) && desktop.want_controls(1), "owner asks");
        let press = ControlEdit {
            bank: 1,
            index: 3,
            values: Vec::new(),
        };
        assert!(
            desktop.submit_controls(
                1,
                vec![
                    set(1, 0, 0.1),
                    press.clone(),
                    set(1, 0, 0.2),
                    set(2, 0, 0.9)
                ]
            ),
            "queued"
        );
        assert!(
            desktop.submit_controls(1, vec![press.clone(), set(1, 0, 0.3)]),
            "queued"
        );
        let mut edits = Vec::new();
        desktop.drain_controls(&mut edits);
        assert_eq!(
            edits,
            vec![set(1, 0, 0.3), press.clone(), set(2, 0, 0.9), press],
            "the slider's latest value in its first slot, another bank's kept, both presses"
        );
    }

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
    fn pointer_moves_while_the_host_is_busy_do_not_cost_the_lease() {
        // The host drains once per control frame; a scene compile holds that
        // for a second or more. A controller moving its mouse meanwhile sends
        // a batch of moves every 150 ms, and each used to take a queue slot,
        // so the lease was dropped and the session closed for no lost edge.
        let desktop = Desktop::default();
        desktop.enable(true);
        assert!(desktop.acquire(1), "lease");
        let at = |n: usize| {
            let x = n as f32 / 1000.0;
            Input::Move { x, y: 1.0 - x }
        };
        let press = Input::Button {
            x: 0.5,
            y: 0.5,
            button: 0,
            down: true,
            modifiers: Modifiers::default(),
        };
        for batch in 0..20 {
            let mut events: Vec<Input> = (0..32).map(|i| at(batch * 32 + i)).collect();
            if batch == 10
                && let Some(slot) = events.get_mut(16)
            {
                *slot = press.clone();
            }
            assert!(
                desktop.submit(1, events),
                "batch {batch}: moves never fill the queue"
            );
        }
        assert!(desktop.owns(1), "the lease is kept");
        let mut out = Vec::new();
        desktop.drain(&mut out);
        assert_eq!(
            out,
            vec![at(335), press, at(639)],
            "each run of moves is its last position, and the button keeps its place"
        );
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
