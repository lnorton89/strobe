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
            log::warn!("audio device failed; recovering");
            live = None;
            next_retry = now;
        }

        if live.is_none() && now >= next_retry {
            shared.set_state(CaptureState::Recovering);
            match device::open(&choice) {
                Ok(opened) => {
                    if opened.sample_rate != analyzer.sample_rate() as u32 {
                        analyzer = Analyzer::with_layout(opened.sample_rate, layout);
                    }
                    log::info!("capturing `{}` at {} Hz", opened.name, opened.sample_rate);
                    {
                        let mut status =
                            shared.status.lock().unwrap_or_else(PoisonError::into_inner);
                        status.device = Some(opened.name.clone());
                        status.sample_rate = Some(opened.sample_rate);
                        status.last_error = None;
                    }
                    live = Some(opened);
                    shared.set_state(CaptureState::Running);
                    if let Some(tx) = first_attempt.take() {
                        let _ = tx.send(Ok(()));
                    }
                }
                Err(error) => {
                    log::warn!("no audio capture: {error}");
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
