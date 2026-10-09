// ============================================================================
// Module:       strobe_script::budget
// Description:  The limits a script runs under: time per frame, time to load,
//               memory, stack, and the seed for randomize.
//
// Dependencies: std only
// ============================================================================

//! Execution budget.
//!
//! The defaults are the ones the design settled on: a script gets 4 ms of
//! a 16.6 ms frame, a 64 MiB heap and a 1 MiB stack. They are fields
//! rather than constants so a test can shrink them to provoke a breach
//! quickly, and so a slow machine can be given more time without a
//! rebuild.

use std::time::Duration;

/// The limits one [`crate::ScriptHost`] runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Wall-clock time for everything one call to `frame` runs: event
    /// handlers and `update` together. Past it the script is interrupted.
    pub frame: Duration,
    /// Wall-clock time for evaluating `script.js` and running `setup()`.
    /// Longer than a frame, because setup may build tables, and it is paid
    /// once while the scene loads rather than while it is on screen.
    pub load: Duration,
    /// Heap cap in bytes; an allocation past it throws inside the script.
    pub memory_limit: usize,
    /// Native stack cap in bytes; deeper recursion throws inside the script.
    pub max_stack: usize,
    /// Seed for `randomizeControl` and `randomizeGroup`. The same seed and
    /// the same calls give the same values, which is what makes a
    /// randomising scene testable.
    pub seed: u64,
}

impl Default for Budget {
    fn default() -> Self {
        const MIB: usize = 1024 * 1024;
        Self {
            frame: Duration::from_millis(4),
            load: Duration::from_millis(250),
            memory_limit: 64 * MIB,
            max_stack: MIB,
            seed: 0x5EED_5EED_5EED_5EED,
        }
    }
}
