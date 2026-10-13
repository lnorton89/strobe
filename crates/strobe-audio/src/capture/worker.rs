// ============================================================================
// Module:       strobe_audio::capture::worker
// Description:  The analysis thread: owns the stream, drains the ring at
//               ~100 Hz, runs the Analyzer, publishes the latest frame, and
//               recovers from device loss.
//
// Dependencies: arc-swap, log; crate::analyzer, crate::capture (shared state)
// ============================================================================

//! The analysis thread.
//!
//! One thread does everything that is not the audio callback and not the
//! render loop. It owns the `cpal` stream (some backends require a stream
//! to stay on the thread that built it), so a device change is handled
//! here too. Every pass it applies the control values the UI has set,
//! drains whatever samples have arrived, runs the analyser, and swaps the
//! new frame into the slot the render thread reads.
//!
//! # Device state machine
//!
//! ```text
//! Running --stream error--> Recovering --opened--> Running
//!                               |
//!                            failed
//!                               v
//!                        SilentFallback --retry timer--> Recovering
//! ```
//!
//! A stream that fails soon after it opened is reopened later each time
//! ([`reopen_delay`]): at once the first time, then after a growing pause
//! up to [`RETRY_EVERY`], so a device that keeps failing costs a few opens
//! a second at most instead of one every pass (D266). Glitches the stream
//! survives are not failures; they are summed and logged once a minute.
//!
//! In `Recovering` and `SilentFallback` the analyser keeps running with no
//! samples, so levels decay to silence instead of freezing, and frames keep
//! being published. The render loop never learns the difference except
//! through [`super::Capture::status`].

use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;

use super::device::{self, DeviceChoice, Opened};
use super::{CaptureError, CaptureState, CaptureStatus, Controls};
use crate::analyzer::Analyzer;
use crate::frame::{AudioFrame, Layout};

/// Time between analysis passes: 100 Hz, finer than any display refresh so
/// the published frame is never more than one pass stale.
const POLL: Duration = Duration::from_millis(10);

/// How long to wait before trying a failed device again.
const RETRY_EVERY: Duration = Duration::from_secs(2);

/// A stream that ran this long before failing is treated as having worked:
/// the next reopen is immediate again.
const STABLE_AFTER: Duration = Duration::from_secs(5);

/// The first pause after a stream fails quickly; it doubles each time.
const FIRST_BACKOFF: Duration = Duration::from_millis(100);

/// How often the count of survived stream glitches is logged.
const GLITCH_REPORT: Duration = Duration::from_secs(60);

/// How long to wait before reopening a device whose stream failed after
/// `quick_failures` failures in a row that each came within
/// [`STABLE_AFTER`] of opening: none for the first, then
/// [`FIRST_BACKOFF`] doubling up to [`RETRY_EVERY`].
pub(crate) fn reopen_delay(quick_failures: u32) -> Duration {
    if quick_failures == 0 {
        return Duration::ZERO;
    }
    let doublings = quick_failures.saturating_sub(1).min(16);
    FIRST_BACKOFF
        .saturating_mul(1_u32 << doublings)
        .min(RETRY_EVERY)
}

/// Sample rate of the silent analyser used while no device is open.
const FALLBACK_RATE: u32 = 48_000;

/// State shared between the handle, the worker and (read-only) the render
/// thread.
pub(crate) struct Shared {
    /// The latest published frame.
    pub(crate) latest: ArcSwap<AudioFrame>,
    /// Asks the worker to exit.
    pub(crate) stop: std::sync::atomic::AtomicBool,
    /// Current [`CaptureState`] as a number.
    pub(crate) state: std::sync::atomic::AtomicU8,
    /// Operator controls.
    pub(crate) controls: Controls,
    /// A device change waiting for the worker.
    pub(crate) pending_choice: Mutex<Option<DeviceChoice>>,
    /// Human-readable status for a settings panel.
    pub(crate) status: Mutex<CaptureStatus>,
}

impl Shared {
    /// Publishes `state` and mirrors it into the status record.
    fn set_state(&self, state: CaptureState) {
        self.state.store(state.to_u8(), Ordering::Release);
        let mut status = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        status.state = state;
    }
}

/// Runs the worker until `shared.stop` is set.
///
/// `first_attempt` receives the outcome of the first open, which is how
/// [`super::Capture::start`] learns whether a device exists. With `strict`
/// set the worker exits after a failed first attempt.
pub(crate) fn run(
    shared: &Arc<Shared>,
    mut choice: DeviceChoice,
    layout: Layout,
    strict: bool,
    first_attempt: Sender<Result<(), CaptureError>>,
) {
    let mut analyzer = Analyzer::with_layout(FALLBACK_RATE, layout);
    let mut live: Option<Opened> = None;
    let mut next_retry = Instant::now();
    let mut first_attempt = Some(first_attempt);
    let mut last_pass = Instant::now();
    // The frame the render thread last saw, kept to reuse its allocation.
    let mut spare: Option<Arc<AudioFrame>> = None;
    // The last open failure that was logged at `warn`: a device that stays
    // missing is retried every two seconds, and saying so every time would
    // bury the rest of the log (and the UI console).
    let mut warned: Option<String> = None;
    // When the live stream opened, and how many streams in a row failed
    // soon after opening (for the reopen backoff).
    let mut opened_at = Instant::now();
    let mut quick_failures: u32 = 0;
    // Survived glitches since the last report.
    let mut glitches: u64 = 0;
    let mut glitch_report = Instant::now() + GLITCH_REPORT;

    while !shared.stop.load(Ordering::Acquire) {
        let now = Instant::now();

        let requested = shared
            .pending_choice
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(new_choice) = requested {
            choice = new_choice;
            live = None;
            next_retry = now;
        }
        if live
            .as_ref()
            .is_some_and(|l| l.failed.load(Ordering::Acquire))
        {
            if now.duration_since(opened_at) < STABLE_AFTER {
                quick_failures = quick_failures.saturating_add(1);
            } else {
                quick_failures = 0;
            }
            let delay = reopen_delay(quick_failures);
            if quick_failures <= 1 {
                log::warn!("audio device failed; recovering");
            } else {
                log::debug!(
                    "audio device failed again ({quick_failures} in a row); reopening in {} ms",
                    delay.as_millis()
                );
            }
            live = None;
            next_retry = now + delay;
        }
        if let Some(active) = live.as_ref() {
            glitches += active.glitches.swap(0, Ordering::Relaxed);
        }
        if now >= glitch_report {
            if glitches > 0 {
                log::info!("audio: {glitches} buffer glitch(es) in the last minute, stream kept");
            }
            glitches = 0;
            glitch_report = now + GLITCH_REPORT;
        }

        if live.is_none() && now >= next_retry {
            shared.set_state(CaptureState::Recovering);
            match device::open(&choice) {
                Ok(opened) => {
                    if opened.sample_rate != analyzer.sample_rate() as u32 {
                        analyzer = Analyzer::with_layout(opened.sample_rate, layout);
                    }
                    log::info!("capturing `{}` at {} Hz", opened.name, opened.sample_rate);
                    warned = None;
                    {
                        let mut status =
                            shared.status.lock().unwrap_or_else(PoisonError::into_inner);
                        status.device = Some(opened.name.clone());
                        status.sample_rate = Some(opened.sample_rate);
                        status.last_error = None;
                    }
                    live = Some(opened);
                    opened_at = now;
                    shared.set_state(CaptureState::Running);
                    if let Some(tx) = first_attempt.take() {
                        let _ = tx.send(Ok(()));
                    }
                }
                Err(error) => {
                    let text = error.to_string();
                    if warned.as_deref() == Some(text.as_str()) {
                        log::debug!("still no audio capture: {text}");
                    } else {
                        log::warn!("no audio capture: {text}");
                        warned = Some(text);
                    }
                    {
                        let mut status =
                            shared.status.lock().unwrap_or_else(PoisonError::into_inner);
                        status.device = None;
                        status.last_error = Some(error.to_string());
                    }
                    shared.set_state(CaptureState::SilentFallback);
                    next_retry = now + RETRY_EVERY;
                    if let Some(tx) = first_attempt.take() {
                        let _ = tx.send(Err(error));
                        if strict {
                            return;
                        }
                    }
                }
            }
        }

        shared.controls.apply(&mut analyzer);

        if let Some(active) = live.as_mut() {
            let waiting = active.consumer.slots();
            if waiting > 0
                && let Ok(chunk) = active.consumer.read_chunk(waiting)
            {
                let (head, tail) = chunk.as_slices();
                analyzer.push(head);
                analyzer.push(tail);
                chunk.commit_all();
            }
        }

        let dt = now.duration_since(last_pass).as_secs_f32();
        last_pass = now;
        analyzer.analyze(dt);
        publish(shared, analyzer.frame(), &mut spare);

        let elapsed = now.elapsed();
        if let Some(rest) = POLL.checked_sub(elapsed) {
            std::thread::sleep(rest);
        }
    }
}

/// Swaps a copy of `frame` into the shared slot, reusing the previously
/// published frame's allocation when the render thread has let go of it.
fn publish(shared: &Shared, frame: &AudioFrame, spare: &mut Option<Arc<AudioFrame>>) {
    let next = match spare.take() {
        Some(mut old) => match Arc::get_mut(&mut old) {
            Some(slot) => {
                slot.clone_from(frame);
                old
            }
            None => Arc::new(frame.clone()),
        },
        None => Arc::new(frame.clone()),
    };
    *spare = Some(shared.latest.swap(next));
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_first_failure_reopens_at_once_and_repeats_back_off_to_the_retry_period() -> Result<()> {
        assert_eq!(
            reopen_delay(0),
            Duration::ZERO,
            "a stream that worked reopens at once"
        );
        assert_eq!(
            reopen_delay(1),
            FIRST_BACKOFF,
            "the first quick failure waits a little"
        );
        assert_eq!(
            reopen_delay(2),
            FIRST_BACKOFF * 2,
            "each further quick failure doubles the pause"
        );
        assert_eq!(
            reopen_delay(6),
            RETRY_EVERY,
            "the pause stops at the retry period"
        );
        assert_eq!(
            reopen_delay(u32::MAX),
            RETRY_EVERY,
            "a device failing forever is tried every retry period, never overflowing"
        );
        // At the cap, a device that fails at once is opened about once per
        // RETRY_EVERY, not once per 10 ms pass.
        let per_minute = 60.0 / reopen_delay(100).as_secs_f32();
        assert!(
            per_minute <= 30.0,
            "{per_minute} reopens a minute at the cap"
        );
        Ok(())
    }
}
