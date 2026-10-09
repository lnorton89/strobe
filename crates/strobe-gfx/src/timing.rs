// ============================================================================
// Module:       strobe_gfx::timing
// Description:  GpuTimer: a small ring of GL timer queries that measures how
//               long the GPU took per frame without ever waiting for it.
//
// Dependencies: crate::gl (Gl, TimerQuery); crate::caps (Caps)
// ============================================================================

//! Measuring the GPU.
//!
//! With vsync on, the CPU's frame time reads one refresh period whether the
//! GPU took 4 ms or 16, so the only honest "how close to the edge are we"
//! is a timer query around the frame's GL work. The danger is how it is
//! read: asking for a result the GPU has not produced yet *blocks* until
//! it does, which turns a measurement into a CPU/GPU synchronisation point
//! and costs exactly the frame time it was meant to protect.
//!
//! ```text
//! frame N     begin(slot a) ... GL work ... end(slot a)
//! frame N+1   begin(slot b) ... poll(): is a ready? yes -> sample, no -> carry on
//! frame N+2   begin(slot c) ... poll(): a, b ...
//! ```
//!
//! [`GpuTimer`] keeps [`SLOTS`] queries in flight. [`GpuTimer::poll`] reads
//! only results the driver says are available, oldest first, and stops at the
//! first that is not. If every slot is still busy, [`GpuTimer::begin`] skips
//! that frame rather than queueing more: a missing sample is harmless to the
//! adaptive controller, a stall is not.
//!
//! `glGetQueryObject` and the ES disjoint flag are driver queries, and this
//! is the one place the per-frame path makes any: the measurement is the
//! point. Capability facts are still decided once ([`Caps`]).

use crate::caps::Caps;
use crate::gl::{Gl, TimerQuery};

/// Queries in flight. Four covers a driver that holds results back for
/// three frames, which a deep render-ahead queue can.
pub const SLOTS: usize = 4;

/// A result above this is a driver glitch, not a frame: a second of GPU
/// time for one frame would have been noticed. Discarded.
const MAX_PLAUSIBLE_MS: f32 = 1000.0;

/// Nanoseconds to milliseconds.
fn milliseconds(nanoseconds: u32) -> f32 {
    nanoseconds as f32 / 1.0e6
}

/// A ring of GPU timer queries.
#[derive(Debug)]
pub struct GpuTimer {
    gl: Gl,
    queries: Vec<TimerQuery>,
    /// The slot the next [`Self::begin`] uses.
    next: usize,
    /// Queries ended and not yet read.
    in_flight: usize,
    /// Whether a query is between `begin` and `end`.
    active: bool,
    /// Whether to ask the ES disjoint flag before trusting results.
    check_disjoint: bool,
}

impl GpuTimer {
    /// A timer, or `None` when this context cannot time the GPU
    /// ([`crate::gl::Info::gpu_timer`]) or the queries could not be made.
    pub fn new(gl: &Gl, caps: &Caps) -> Option<Self> {
        if !caps.info.gpu_timer {
            log::info!("GPU timer queries unavailable on this context");
            return None;
        }
        let queries = (0..SLOTS)
            .map(|_| gl.timer_query())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| log::warn!("GPU timer queries could not be created: {error}"))
            .ok()?;
        Some(Self {
            gl: gl.clone(),
            queries,
            next: 0,
            in_flight: 0,
            active: false,
            check_disjoint: caps.info.parsed.2,
        })
    }

    /// Starts timing. Does nothing if every slot is still waiting for its
    /// result, or a measurement is already open.
    pub fn begin(&mut self) {
        if self.active || self.in_flight >= SLOTS {
            return;
        }
        if let Some(query) = self.queries.get(self.next) {
            self.gl.begin_timer(query);
            self.active = true;
        }
    }

    /// Stops timing and puts the measurement in flight.
    pub fn end(&mut self) {
        if !self.active {
            return;
        }
        self.gl.end_timer();
        self.active = false;
        self.next = (self.next + 1) % SLOTS;
        self.in_flight += 1;
    }

    /// Calls `each` with the GPU milliseconds of every measurement whose
    /// result is ready, oldest first. Never waits: it stops at the first
    /// result that is not available yet.
    pub fn poll(&mut self, mut each: impl FnMut(f32)) {
        let mut samples = [0.0_f32; SLOTS];
        let mut count = 0;
        while self.in_flight > 0 {
            let oldest = (self.next + SLOTS - self.in_flight) % SLOTS;
            let Some(query) = self.queries.get(oldest) else {
                break;
            };
            if !self.gl.timer_ready(query) {
                break;
            }
            let ms = milliseconds(self.gl.timer_nanoseconds(query));
            self.in_flight -= 1;
            if let Some(slot) = samples.get_mut(count) {
                *slot = ms;
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        // The flag clears when read, so this also re-arms it.
        if self.check_disjoint && self.gl.gpu_disjoint() {
            return;
        }
        for ms in samples.iter().take(count).copied() {
            if ms.is_finite() && ms <= MAX_PLAUSIBLE_MS {
                each(ms);
            }
        }
    }

    /// Measurements ended and not yet read.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nanoseconds_become_milliseconds() {
        assert!(
            (milliseconds(16_666_667) - 16.666_667).abs() < 1e-3,
            "60 Hz"
        );
        assert!(milliseconds(0).abs() < f32::EPSILON, "zero");
    }
}
