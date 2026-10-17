// ============================================================================
// Module:       strobe_audio::capture
// Description:  Live audio: device enumeration, a cpal input stream, an
//               analysis thread, and a wait-free handle for the render loop.
//
// Dependencies: cpal, rtrb, arc-swap, thiserror, log; crate::analyzer
// ============================================================================

//! Live capture.
//!
//! ```text
//! audio callback  --rtrb ring-->  analysis thread  --ArcSwap-->  render thread
//! (cpal, mono)                    (~100 Hz Analyzer)             (Capture::latest)
//! ```
//!
//! The render thread never waits on audio. [`Capture::latest`] is one
//! atomic pointer load; a stale frame for a tick is preferable to a late
//! frame on the projector. The audio callback only converts, averages
//! channels and pushes into a lock-free ring (see [`device`]). Everything
//! else happens on the analysis thread (see the `worker` module), which
//! also recovers from device loss.
//!
//! # No device is not an error to live with
//!
//! A laptop with the microphone disabled, a docked machine with the dock
//! gone, a CI runner: none of them may stop the show. [`Capture::start`]
//! reports a missing device as an `Err` so a caller that wants to tell the
//! user can; [`Capture::start_resilient`] always returns a handle, serves
//! silent frames, and keeps retrying in the background.
//!
//! # Controls
//!
//! Reactivity, audio speed, tap tempo, nudge, and device selection are
//! methods on [`Capture`]. They write atomics the worker reads on its next
//! pass, so a UI thread can call them freely.

mod device;
mod watchdog;
mod worker;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;

pub use device::{DeviceChoice, DeviceInfo, DeviceKind, list_devices};

use crate::analyzer::Analyzer;
use crate::frame::{AudioFrame, Layout};
use worker::Shared;

/// How long [`Capture::start`] waits for the first open attempt. WASAPI can
/// take a second or two to activate a device.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// A tap time value meaning "no tap pending".
const NO_TAP: u64 = u64::MAX;

/// Why capture could not start.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// There is no capture device on this machine.
    #[error("no audio capture device is available")]
    NoDevice,
    /// The named device is not present.
    #[error("audio device `{0}` was not found")]
    DeviceNotFound(String),
    /// The audio host could not enumerate devices.
    #[error("audio host error: {0}")]
    Host(String),
    /// The device delivers a sample format this crate does not read.
    #[error("unsupported sample format {0}")]
    UnsupportedFormat(String),
    /// The stream could not be built or started.
    #[error("could not open the audio stream: {0}")]
    Stream(String),
    /// The analysis thread could not be started.
    #[error("could not start the analysis thread: {0}")]
    Thread(String),
}

/// Where the capture pipeline is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureState {
    /// A device is open and delivering samples.
    Running,
    /// Opening or re-opening a device.
    Recovering,
    /// No device; serving silence and retrying.
    #[default]
    SilentFallback,
}

impl CaptureState {
    /// The state as stored in an atomic.
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Recovering => 1,
            Self::SilentFallback => 2,
        }
    }

    /// The state from its atomic encoding; anything unknown is the safe
    /// fallback.
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Running,
            1 => Self::Recovering,
            _ => Self::SilentFallback,
        }
    }
}

/// A snapshot of the pipeline for a settings panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureStatus {
    /// Pipeline state.
    pub state: CaptureState,
    /// Name of the open device, if any.
    pub device: Option<String>,
    /// Its sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Why the last open attempt failed, if it did.
    pub last_error: Option<String>,
}

/// What to capture and how the output is sized.
#[derive(Clone, Debug, Default)]
pub struct CaptureConfig {
    /// The device to open.
    pub device: DeviceChoice,
    /// Texture sizes of the published frames.
    pub layout: Layout,
}

/// Operator controls shared with the worker.
pub(crate) struct Controls {
    /// Reactivity gain, as `f32` bits.
    reactivity: AtomicU32,
    /// Audio-speed gain, as `f32` bits.
    audio_speed: AtomicU32,
    /// Microseconds since the capture epoch of an unprocessed tap.
    tap_micros: AtomicU64,
    /// BPM nudge accumulated since the worker last looked, in hundredths.
    nudge_centi: AtomicI32,
    /// Set to hand the tempo back to the estimator.
    clear_manual: AtomicBool,
    /// The instant tap times are measured from.
    epoch: Instant,
}

impl Controls {
    /// Neutral controls.
    fn new() -> Self {
        Self {
            reactivity: AtomicU32::new(1.0_f32.to_bits()),
            audio_speed: AtomicU32::new(1.0_f32.to_bits()),
            tap_micros: AtomicU64::new(NO_TAP),
            nudge_centi: AtomicI32::new(0),
            clear_manual: AtomicBool::new(false),
            epoch: Instant::now(),
        }
    }

    /// Applies pending controls to `analyzer`. Worker thread only.
    pub(crate) fn apply(&self, analyzer: &mut Analyzer) {
        analyzer.set_reactivity(f32::from_bits(self.reactivity.load(Ordering::Relaxed)));
        analyzer.set_audio_speed(f32::from_bits(self.audio_speed.load(Ordering::Relaxed)));
        if self.clear_manual.swap(false, Ordering::AcqRel) {
            analyzer.clear_manual_tempo();
        }
        let tap = self.tap_micros.swap(NO_TAP, Ordering::AcqRel);
        if tap != NO_TAP {
            analyzer.tap(tap as f64 / 1.0e6);
        }
        let nudge = self.nudge_centi.swap(0, Ordering::AcqRel);
        if nudge != 0 {
            analyzer.nudge_bpm(nudge as f32 / 100.0);
        }
    }
}

/// A running capture pipeline. Dropping it stops the thread and closes the
/// device.
pub struct Capture {
    /// State shared with the worker.
    shared: Arc<Shared>,
    /// The analysis thread.
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    /// Starts capturing, failing if the device cannot be opened.
    ///
    /// Blocks for the duration of the first open attempt (milliseconds on
    /// most hosts, up to [`OPEN_TIMEOUT`]). Call it from a startup or
    /// settings path, never from the render loop.
    pub fn start(config: CaptureConfig) -> Result<Self, CaptureError> {
        Self::spawn(config, true)
    }

    /// Starts the pipeline whether or not a device opens.
    ///
    /// Without a device the handle serves silent frames, its status reads
    /// [`CaptureState::SilentFallback`] with the reason in `last_error`,
    /// and the worker retries every two seconds, so a device plugged in
    /// later is picked up.
    pub fn start_resilient(config: CaptureConfig) -> Result<Self, CaptureError> {
        Self::spawn(config, false)
    }

    /// Builds the shared state and the worker thread.
    fn spawn(config: CaptureConfig, strict: bool) -> Result<Self, CaptureError> {
        let layout = config.layout.clamped();
        let shared = Arc::new(Shared {
            latest: ArcSwap::from_pointee(AudioFrame::silent(layout)),
            stop: AtomicBool::new(false),
            state: AtomicU8::new(CaptureState::SilentFallback.to_u8()),
            controls: Controls::new(),
            pending_choice: Mutex::new(None),
            status: Mutex::new(CaptureStatus::default()),
        });
        let (sender, receiver) = std::sync::mpsc::channel();
        let for_thread = Arc::clone(&shared);
        let device = config.device;
        let thread = std::thread::Builder::new()
            .name("strobe-audio".to_owned())
            .spawn(move || worker::run(&for_thread, device, layout, strict, sender))
            .map_err(|e| CaptureError::Thread(e.to_string()))?;
        let capture = Self {
            shared,
            thread: Some(thread),
        };
        match receiver.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(())) => Ok(capture),
            Ok(Err(error)) if strict => Err(error),
            Ok(Err(_)) | Err(_) => Ok(capture),
        }
    }

    /// The most recent frame. Wait-free; safe to call every render frame.
    pub fn latest(&self) -> Arc<AudioFrame> {
        self.shared.latest.load_full()
    }

    /// Copies the most recent frame into `destination`, reusing its
    /// allocation. Wait-free and allocation-free once `destination` has the
    /// right size.
    pub fn snapshot_into(&self, destination: &mut AudioFrame) {
        destination.clone_from(&self.shared.latest.load());
    }

    /// The pipeline state, without taking a lock.
    pub fn state(&self) -> CaptureState {
        CaptureState::from_u8(self.shared.state.load(Ordering::Acquire))
    }

    /// A fuller status. Takes a lock the worker holds only briefly; meant
    /// for a settings panel, not the render loop.
    pub fn status(&self) -> CaptureStatus {
        self.shared
            .status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Switches to another device. The worker reopens on its next pass.
    pub fn select_device(&self, choice: DeviceChoice) {
        *self
            .shared
            .pending_choice
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(choice);
    }

    /// Sets the gain on levels and hits (see [`Analyzer::set_reactivity`]).
    pub fn set_reactivity(&self, gain: f32) {
        self.shared
            .controls
            .reactivity
            .store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Sets the gain on the time clocks (see [`Analyzer::set_audio_speed`]).
    pub fn set_audio_speed(&self, gain: f32) {
        self.shared
            .controls
            .audio_speed
            .store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Registers a tap-tempo tap, timed now.
    pub fn tap(&self) {
        let micros = self.shared.controls.epoch.elapsed().as_micros();
        let micros = u64::try_from(micros).unwrap_or(NO_TAP - 1);
        self.shared
            .controls
            .tap_micros
            .store(micros, Ordering::Release);
    }

    /// Moves the tempo by `delta` BPM and holds it.
    pub fn nudge_bpm(&self, delta: f32) {
        if delta.is_finite() {
            let centi = (delta * 100.0).round() as i32;
            self.shared
                .controls
                .nudge_centi
                .fetch_add(centi, Ordering::AcqRel);
        }
    }

    /// Hands the tempo back to the automatic estimator.
    pub fn clear_manual_tempo(&self) {
        self.shared
            .controls
            .clear_manual
            .store(true, Ordering::Release);
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            // Waits for the worker to stop; a panic in it already aborted the
            // process in release builds.
            let _ = thread.join();
        }
    }
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    /// Whether this machine has any input device at all.
    fn has_input() -> bool {
        list_devices()
            .map(|devices| devices.iter().any(|d| d.kind == DeviceKind::Input))
            .unwrap_or(false)
    }

    #[test]
    fn listing_devices_never_panics_and_degrades_to_empty() -> Result<()> {
        // In a container this is empty or an error; on a desktop it lists
        // real devices. Either is fine; a panic is not.
        match list_devices() {
            Ok(devices) => {
                for device in &devices {
                    assert!(!device.id.is_empty(), "every device has an id");
                }
            }
            Err(error) => assert!(
                !error.to_string().is_empty(),
                "an enumeration error explains itself"
            ),
        }
        Ok(())
    }

    #[test]
    fn start_reports_a_missing_device_as_an_error() -> Result<()> {
        if has_input() {
            // A real device exists; the failure path is exercised below
            // with a device id that cannot exist.
            return Ok(());
        }
        match Capture::start(CaptureConfig::default()) {
            Err(error) => assert!(
                !error.to_string().is_empty(),
                "the error should say what is missing"
            ),
            Ok(_) => return Err(anyhow!("capture started with no input device")),
        }
        Ok(())
    }

    #[test]
    fn an_unknown_device_id_is_an_error_not_a_panic() -> Result<()> {
        let config = CaptureConfig {
            device: DeviceChoice::ById("no-such-device".to_owned()),
            layout: Layout::default(),
        };
        match Capture::start(config) {
            Err(error) => assert!(
                !error.to_string().is_empty(),
                "the error should name the bad device"
            ),
            Ok(_) => return Err(anyhow!("a bogus device id opened")),
        }
        Ok(())
    }

    #[test]
    fn resilient_start_serves_silence_without_a_device() -> Result<()> {
        let config = CaptureConfig {
            device: DeviceChoice::ById("no-such-device".to_owned()),
            layout: Layout::default(),
        };
        let capture = Capture::start_resilient(config)?;
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            capture.state(),
            CaptureState::SilentFallback,
            "no device means the silent fallback"
        );
        let frame = capture.latest();
        assert_eq!(
            frame.scalar("syn_BPM"),
            Some(120.0),
            "the fallback frame is a silent frame"
        );
        assert!(
            capture.status().last_error.is_some(),
            "the status explains why there is no device"
        );

        // Controls are accepted while in fallback.
        capture.set_reactivity(1.5);
        capture.set_audio_speed(0.5);
        capture.tap();
        capture.nudge_bpm(2.0);
        capture.clear_manual_tempo();
        capture.select_device(DeviceChoice::ById("still-no-such-device".to_owned()));
        std::thread::sleep(Duration::from_millis(50));

        let mut copy = AudioFrame::silent(Layout::default());
        capture.snapshot_into(&mut copy);
        assert_eq!(
            copy.scalars.len(),
            strobe_scene::audio::SCALARS.len(),
            "snapshot_into fills a frame"
        );
        Ok(())
    }

    #[test]
    fn dropping_the_handle_stops_the_thread() -> Result<()> {
        let capture = Capture::start_resilient(CaptureConfig {
            device: DeviceChoice::ById("x".to_owned()),
            layout: Layout::default(),
        })?;
        let started = Instant::now();
        drop(capture);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "drop joins the worker promptly"
        );
        Ok(())
    }

    #[test]
    fn state_round_trips_through_its_encoding() -> Result<()> {
        for state in [
            CaptureState::Running,
            CaptureState::Recovering,
            CaptureState::SilentFallback,
        ] {
            assert_eq!(
                CaptureState::from_u8(state.to_u8()),
                state,
                "{state:?} survives the atomic encoding"
            );
        }
        Ok(())
    }
}
