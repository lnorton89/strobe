// ============================================================================
// Module:       strobe_script::shared
// Description:  State shared between the host and the native functions it
//               installs: name tables, queued writes, handlers, console.
//
// Dependencies: strobe_scene (ControlBank, names, audio), crate::{events,
//               output, uniforms}
// ============================================================================

//! Shared host state.
//!
//! JavaScript calls native functions (`setControl`, `setUniform`, …) that
//! must reach the host's state, and the host must read what they wrote
//! once the script returns. Both hold an `Rc<Shared>`. The immutable half,
//! [`Names`], is built once from the control bank; the mutable half,
//! [`State`], sits behind one `RefCell` that is only ever borrowed for the
//! length of a single native call or host step, never across a call into
//! JavaScript, so a native function that re-enters the host (registering a
//! handler from inside a handler) cannot hit a double borrow.
//!
//! ## Queued control writes
//!
//! A native function cannot hold `&mut ControlBank`: the bank belongs to
//! the caller of `frame`, and keeping a pointer to it across the
//! interpreter would need `unsafe`. Natives push a [`Command`] instead and
//! the host applies the queue, in order, before `frame` returns, so the
//! write still lands in the frame that made it.

use crate::events::{Handler, Source};
use crate::output::{ConsoleLevel, ConsoleLine, MetaWrite};
use crate::uniforms::UniformStore;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;
use strobe_scene::names::normalize;

/// Console lines kept per frame. A script printing in a tight loop for its
/// whole budget would otherwise hand the app thousands of lines.
const MAX_CONSOLE_LINES: usize = 200;

/// Distinct one-time messages remembered before further ones are dropped,
/// so a script generating unique error text cannot grow the set forever.
const MAX_REMEMBERED_MESSAGES: usize = 256;

/// A control write waiting to be applied to the bank.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Command {
    /// `setControl` / `setControlNormalized`.
    Set {
        /// The control's index in the bank.
        index: usize,
        /// The `dimension` option.
        dimension: Option<usize>,
        /// The values; only the first `len` are meaningful.
        values: [f32; 3],
        /// How many of `values` were given.
        len: usize,
        /// True when the values are `0..1`.
        normalized: bool,
    },
    /// `randomizeControl`, or one member of `randomizeGroup`.
    Randomize(usize),
    /// `defaultControl`, or one member of `defaultGroup`.
    Default(usize),
}

/// The name tables, built once from the bank and never changed.
#[derive(Debug)]
pub(crate) struct Names {
    /// Normalised control name to bank index.
    pub(crate) controls: HashMap<String, usize>,
    /// Control names exactly as declared; a script uniform with one of
    /// these names would redeclare the control's uniform in GLSL.
    pub(crate) control_exact: HashSet<String>,
    /// Normalised `UI_GROUP` to the bank indices in it.
    pub(crate) groups: HashMap<String, Vec<usize>>,
    /// Normalised input name to what an event handler would watch.
    pub(crate) sources: HashMap<String, Source>,
}

impl Names {
    /// Builds the tables for `bank`.
    pub(crate) fn build(bank: &ControlBank) -> Self {
        let mut controls = HashMap::new();
        let mut control_exact = HashSet::new();
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        let mut sources = HashMap::new();
        for (index, name) in SCALARS.iter().enumerate() {
            sources.insert(normalize(name), Source::Audio(index));
        }
        sources.insert(normalize("TIME"), Source::Time);
        sources.insert(normalize("FRAMECOUNT"), Source::FrameCount);
        for (index, def) in bank.defs().iter().enumerate() {
            let key = normalize(&def.name);
            controls.insert(key.clone(), index);
            sources.insert(key, Source::Control(index));
            control_exact.insert(def.name.clone());
            if let Some(group) = &def.group {
                groups.entry(normalize(group)).or_default().push(index);
            }
        }
        Self {
            controls,
            control_exact,
            groups,
            sources,
        }
    }
}

/// The mutable half of the shared state.
#[derive(Debug)]
pub(crate) struct State {
    /// Uniforms the script has published.
    pub(crate) uniforms: UniformStore,
    /// Console output since the last frame was returned.
    pub(crate) console: Vec<ConsoleLine>,
    /// Control writes not yet applied.
    pub(crate) commands: Vec<Command>,
    /// Meta-control writes not yet returned.
    pub(crate) meta: Vec<MetaWrite>,
    /// Registered event handlers, in registration order.
    pub(crate) handlers: Vec<Handler>,
    /// Messages already logged once.
    once: HashSet<String>,
    /// The final-pass downsample, or empty when there is none.
    pub(crate) final_pass: Vec<u8>,
    /// Generator state for randomize.
    rng: u64,
}

impl State {
    /// Fresh state seeded with `seed`.
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            uniforms: UniformStore::default(),
            console: Vec::new(),
            commands: Vec::new(),
            meta: Vec::new(),
            handlers: Vec::new(),
            once: HashSet::new(),
            final_pass: Vec::new(),
            // xorshift cannot leave the all-zero state.
            rng: seed | 1,
        }
    }

    /// Appends a console line, truncating a flood.
    pub(crate) fn log(&mut self, level: ConsoleLevel, text: impl Into<String>) {
        let count = self.console.len();
        if count > MAX_CONSOLE_LINES {
            return;
        }
        let text = if count == MAX_CONSOLE_LINES {
            "(console output truncated for this frame)".to_string()
        } else {
            text.into()
        };
        self.console.push(ConsoleLine { level, text });
    }

    /// Logs `text` the first time it is seen and never again. This is what
    /// stops a bad name inside `update` from printing sixty times a second.
    pub(crate) fn log_once(&mut self, level: ConsoleLevel, text: &str) {
        if self.once.contains(text) || self.once.len() >= MAX_REMEMBERED_MESSAGES {
            return;
        }
        self.once.insert(text.to_string());
        self.log(level, text);
    }

    /// The next random number in `0..1` (xorshift64*).
    pub(crate) fn next_unit(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let mixed = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // The top 24 bits fill an f32 mantissa exactly.
        (mixed >> 40) as f32 / 16_777_216.0
    }
}

/// Everything a native function and the host both touch.
#[derive(Debug)]
pub(crate) struct Shared {
    /// Immutable name tables.
    pub(crate) names: Names,
    /// Mutable state.
    pub(crate) state: RefCell<State>,
    /// When the running call must stop; `None` outside a call.
    pub(crate) deadline: Rc<Cell<Option<Instant>>>,
}

impl Shared {
    /// Shared state for a script driving `bank`.
    pub(crate) fn new(bank: &ControlBank, seed: u64, deadline: Rc<Cell<Option<Instant>>>) -> Self {
        Self {
            names: Names::build(bank),
            state: RefCell::new(State::new(seed)),
            deadline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_stays_in_unit_range_and_moves() {
        let mut state = State::new(7);
        let draws: Vec<f32> = (0..1000).map(|_| state.next_unit()).collect();
        assert!(
            draws.iter().all(|d| (0.0..1.0).contains(d)),
            "every draw is in 0..1"
        );
        assert!(
            draws.windows(2).any(|pair| pair[0] != pair[1]),
            "the sequence is not constant"
        );
    }

    #[test]
    fn a_console_flood_is_truncated() {
        let mut state = State::new(1);
        for _ in 0..(MAX_CONSOLE_LINES * 3) {
            state.log(ConsoleLevel::Log, "x");
        }
        assert_eq!(
            state.console.len(),
            MAX_CONSOLE_LINES + 1,
            "the cap plus one truncation notice"
        );
    }

    #[test]
    fn a_message_is_logged_once() {
        let mut state = State::new(1);
        state.log_once(ConsoleLevel::Error, "no such control");
        state.log_once(ConsoleLevel::Error, "no such control");
        assert_eq!(state.console.len(), 1, "the repeat was swallowed");
    }
}
