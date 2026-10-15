// ============================================================================
// Module:       strobe_app::audio
// Description:  The app's audio input: live capture from strobe-audio when a
//               device exists, the synthetic track when it does not, and the
//               settings the UI changes (device, reactivity, speed, tap).
//
// Dependencies: strobe-audio (Capture, AudioFrame); strobe-scene (synthetic);
//               strobe-diag (failures); crate::ui::model (AudioPanel)
// ============================================================================

//! Audio input.
//!
//! One [`AudioInput`] lives for the whole run, not per scene: switching
//! scenes must not restart capture or lose a tapped tempo.
//!
//! ## Which source is active
//!
//! ```text
//! start ── background thread ──▶ Capture::start_resilient (may take seconds)
//!            │                          │
//!   synthetic until it arrives          ▼
//!                      Running ────────▶ live    (frames from the analyser)
//!                      SilentFallback ─▶ synthetic (strobe_scene::synthetic)
//!                      Recovering ─────▶ keep whichever was active
//! ```
//!
//! Each change is logged once, and shown in the status bar. A device that
//! is lost *after* having worked is reported as a structured `AUDIO`
//! failure; a machine that simply has no device logs a single warning,
//! because that is not a fault. `strobe-audio` keeps retrying in the
//! background, so a device plugged in later takes over by itself.
//!
//! ## Never blocking the render thread
//!
//! Everything that can wait runs on its own thread: opening the device
//! (WASAPI can take seconds) and enumerating devices. The render thread
//! polls both with `try_recv`. Per frame it does one atomic state read and
//! one wait-free snapshot copy into a buffer that is reused; the capture's
//! `status()` takes a lock and is read at most twice a second, from the
//! control window's path.
//!
//! ## Synthetic textures
//!
//! The synthetic source also fills `syn_Spectrum` and `syn_LevelTrail`
//! ([`strobe_scene::synthetic::fill_textures`]), so a scene that draws the
//! spectrum is not dead without a device.

use crate::ui::model::AudioPanel;
use std::ops::RangeInclusive;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};
use strobe_audio::capture::{
    Capture, CaptureConfig, CaptureError, CaptureState, CaptureStatus, DeviceChoice, DeviceInfo,
    list_devices,
};
use strobe_audio::{AudioFrame, Layout};
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_scene::audio::SCALARS;

/// The range of the reactivity and audio-speed gains (the analyser clamps
/// to the same range).
pub const GAIN: RangeInclusive<f32> = 0.0..=4.0;

/// How often the capture's status (which takes a lock) is re-read.
const STATUS_PERIOD: Duration = Duration::from_millis(500);

/// Where the frames in use come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The fixed 120 BPM pattern.
    Synthetic,
    /// A capture device through the analyser.
    Live,
}

/// The result of opening capture on its own thread.
type Started = Result<Capture, CaptureError>;

/// The result of listing devices on its own thread.
type Listed = Result<Vec<DeviceInfo>, String>;

/// The app's audio input and its settings.
pub struct AudioInput {
    /// Whether capture was asked for at all (`--audio synthetic` says no).
    wants_capture: bool,
    capture: Option<Capture>,
    starting: Option<Receiver<Started>>,
    devices: Vec<DeviceInfo>,
    listing: Option<Receiver<Listed>>,
    choice: DeviceChoice,
    reactivity: f32,
    speed: f32,
    /// The meta controls' reactivity and audio speed (D184). The effective
    /// gain is the panel's times this.
    meta_gain: [f32; 2],
    /// The effective gains last sent to the capture, so it is told on change.
    pushed_gain: [f32; 2],
    /// The frame handed to the renderer; its vectors are sized once.
    frame: AudioFrame,
    trail_seconds: f32,
    bpm_index: Option<usize>,
    source: Source,
    /// The last source that was logged, so each change is said once.
    announced: Option<Source>,
    /// Whether a device has ever delivered audio in this run.
    was_live: bool,
    status: CaptureStatus,
    status_read: Option<Instant>,
    label: String,
    /// `--audio ambient`: the synthetic track made calm, if asked for.
    ambient: Option<Ambient>,
}

/// The synthetic track without a beat (`--audio ambient`): hits and beat
/// triggers held at zero, levels a slow swell, the textures damped. The
/// clocks and the smooth tempo shapes keep running, so scenes still move.
/// Indices are resolved once; per frame it only writes into the frame.
#[derive(Debug)]
struct Ambient {
    /// Scalars held at zero: hits, beat triggers, the twitcher.
    zero: Vec<usize>,
    /// Level scalars, replaced by the swell.
    levels: Vec<usize>,
}

impl Ambient {
    /// How much of the synthetic textures' amplitude is kept.
    const TEXTURE_GAIN: f32 = 0.35;
    /// The swell's period in seconds.
    const SWELL_PERIOD: f32 = 16.0;

    fn new() -> Self {
        const ZERO: &[&str] = &[
            "syn_Hits",
            "syn_BassHits",
            "syn_MidHits",
            "syn_MidHighHits",
            "syn_HighHits",
            "syn_OnBeat",
            "syn_ToggleOnBeat",
            "syn_RandomOnBeat",
            "syn_BPMTwitcher",
        ];
        const LEVELS: &[&str] = &[
            "syn_Level",
            "syn_BassLevel",
            "syn_MidLevel",
            "syn_MidHighLevel",
            "syn_HighLevel",
        ];
        let find = |names: &[&str]| {
            names
                .iter()
                .filter_map(|name| SCALARS.iter().position(|scalar| scalar == name))
                .collect()
        };
        Self {
            zero: find(ZERO),
            levels: find(LEVELS),
        }
    }

    /// Calms a frame the synthetic track just filled for scene time `time`.
    fn calm(&self, time: f32, frame: &mut AudioFrame) {
        let phase = time * std::f32::consts::TAU / Self::SWELL_PERIOD;
        for &index in &self.zero {
            if let Some(value) = frame.scalars.get_mut(index) {
                *value = 0.0;
            }
        }
        for (band, &index) in self.levels.iter().enumerate() {
            // Each band a little out of step with the others.
            let swell = 0.35 + 0.1 * (phase + band as f32 * 0.7).sin();
            if let Some(value) = frame.scalars.get_mut(index) {
                *value = swell;
            }
        }
        for texel in frame
            .spectrum
            .iter_mut()
            .chain(frame.level_trail.iter_mut())
        {
            for channel in texel.iter_mut() {
                *channel *= Self::TEXTURE_GAIN;
            }
        }
    }
}

impl std::fmt::Debug for AudioInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioInput")
            .field("source", &self.source)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl AudioInput {
    /// Audio for a new run. With `capture` true, opening the default input
    /// and listing devices start on background threads; synthetic audio
    /// plays until a device is open.
    pub fn new(capture: bool) -> Self {
        let layout = Layout::default();
        let mut input = Self {
            wants_capture: capture,
            capture: None,
            starting: None,
            devices: Vec::new(),
            listing: None,
            choice: DeviceChoice::default(),
            reactivity: 1.0,
            speed: 1.0,
            meta_gain: [1.0, 1.0],
            pushed_gain: [1.0, 1.0],
            frame: AudioFrame::silent(layout),
            trail_seconds: layout.trail_seconds,
            bpm_index: SCALARS.iter().position(|name| *name == "syn_BPM"),
            source: Source::Synthetic,
            announced: None,
            was_live: false,
            status: CaptureStatus::default(),
            status_read: None,
            label: String::new(),
            ambient: None,
        };
        if capture {
            input.label = "synthetic (opening audio input...)".to_string();
            input.start_capture();
            input.refresh_devices();
        } else {
            input.label = "synthetic (forced)".to_string();
            input.announced = Some(Source::Synthetic);
            log::info!("synthetic source (--audio synthetic)");
        }
        input
    }

    /// With `ambient` true, the synthetic track plays without its beat
    /// (`--audio ambient`). Only meaningful without capture.
    pub fn with_ambient(mut self, ambient: bool) -> Self {
        if ambient {
            self.ambient = Some(Ambient::new());
            self.label = "ambient (no beat)".to_string();
            log::info!("ambient source (--audio ambient): synthetic without hits or beats");
        }
        self
    }

    fn start_capture(&mut self) {
        let config = CaptureConfig {
            device: self.choice.clone(),
            layout: Layout::default(),
        };
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("strobe-audio-open".to_string())
            .spawn(move || {
                // The UI stopped waiting for this open (it was replaced or
                // the app is closing); the capture is dropped with the
                // message.
                let _ = sender.send(Capture::start_resilient(config));
            });
        match spawned {
            Ok(_) => self.starting = Some(receiver),
            Err(error) => {
                self.wants_capture = false;
                failure::report(
                    Failure::new(Subsystem::Audio, "start")
                        .message(error)
                        .action("using synthetic audio"),
                );
            }
        }
    }

    /// Re-reads the list of capturable devices on a background thread.
    pub fn refresh_devices(&mut self) {
        if self.listing.is_some() {
            return;
        }
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("strobe-audio-list".to_string())
            .spawn(move || {
                // The UI stopped waiting for this listing; a newer one is on
                // its way.
                let _ = sender.send(list_devices().map_err(|e| e.to_string()));
            });
        if spawned.is_ok() {
            self.listing = Some(receiver);
        }
    }

    /// Collects whatever the background threads have finished. Only
    /// `try_recv`: never waits.
    fn poll_threads(&mut self) {
        if let Some(receiver) = &self.starting {
            match receiver.try_recv() {
                Ok(Ok(capture)) => {
                    let gains = self.effective_gains();
                    capture.set_reactivity(gains[0]);
                    capture.set_audio_speed(gains[1]);
                    self.pushed_gain = gains;
                    self.capture = Some(capture);
                    self.starting = None;
                }
                Ok(Err(error)) => {
                    self.starting = None;
                    self.wants_capture = false;
                    failure::report(
                        Failure::new(Subsystem::Audio, "start")
                            .message(error)
                            .action("using synthetic audio"),
                    );
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.starting = None,
            }
        }
        if let Some(receiver) = &self.listing {
            match receiver.try_recv() {
                Ok(Ok(devices)) => {
                    log::info!("{} capturable device(s)", devices.len());
                    self.devices = devices;
                    self.listing = None;
                }
                Ok(Err(error)) => {
                    log::warn!("could not list devices: {error}");
                    self.listing = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.listing = None,
            }
        }
    }

    /// The audio for the frame at scene time `time`.
    ///
    /// Live frames are copied from the analyser's latest snapshot;
    /// otherwise the synthetic track is evaluated at `time`. No allocation
    /// and no waiting after the first call.
    pub fn frame_at(&mut self, time: f32) -> &AudioFrame {
        self.poll_threads();
        let desired = match self.capture.as_ref().map(Capture::state) {
            Some(CaptureState::Running) => Some(Source::Live),
            Some(CaptureState::SilentFallback) => Some(Source::Synthetic),
            Some(CaptureState::Recovering) | None => None,
        };
        if let Some(desired) = desired {
            self.source = desired;
            if self.announced != Some(desired) {
                self.announce(desired);
            }
        }
        match (self.source, self.capture.as_ref()) {
            (Source::Live, Some(capture)) => capture.snapshot_into(&mut self.frame),
            (Source::Live | Source::Synthetic, _) => {
                strobe_scene::synthetic::fill(time, &mut self.frame.scalars);
                // The synthetic track has no song structure (D226).
                self.frame.energy = [0.0; 2];
                self.frame.downbeat = [0.0; 2];
                strobe_scene::synthetic::fill_textures(
                    time,
                    &mut self.frame.spectrum,
                    &mut self.frame.level_trail,
                    self.trail_seconds,
                );
                if let Some(ambient) = &self.ambient {
                    ambient.calm(time, &mut self.frame);
                }
            }
        }
        &self.frame
    }

    /// Logs a change of source, once, and refreshes the label.
    fn announce(&mut self, source: Source) {
        self.announced = Some(source);
        self.read_status();
        let device = self
            .status
            .device
            .clone()
            .unwrap_or_else(|| "unknown device".to_string());
        match source {
            Source::Live => {
                self.was_live = true;
                log::info!(
                    "live capture from {device} ({} Hz)",
                    self.status.sample_rate.unwrap_or_default()
                );
            }
            Source::Synthetic => {
                let reason = self
                    .status
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "no input device".to_string());
                if self.was_live {
                    failure::report(
                        Failure::new(Subsystem::Audio, "capture")
                            .asset(&device)
                            .message(&reason)
                            .action("using synthetic audio; capture retries in the background"),
                    );
                } else {
                    log::warn!(
                        "no usable capture device ({reason}); using synthetic audio, \
                         and retrying in the background"
                    );
                }
            }
        }
    }

    /// Re-reads the capture's status for the status bar, at most twice a
    /// second. Call from the control window's path.
    pub fn refresh_status(&mut self, now: Instant) {
        if self.capture.is_none() {
            return;
        }
        if self
            .status_read
            .is_some_and(|read| now.duration_since(read) < STATUS_PERIOD)
        {
            return;
        }
        self.status_read = Some(now);
        self.read_status();
    }

    fn read_status(&mut self) {
        let Some(capture) = &self.capture else {
            return;
        };
        self.status = capture.status();
        self.label = match self.source {
            Source::Live => format!(
                "live: {}",
                self.status.device.as_deref().unwrap_or("input device")
            ),
            Source::Synthetic => match self.status.last_error {
                Some(_) => "synthetic (no input device)".to_string(),
                None => "synthetic (opening audio input...)".to_string(),
            },
        };
    }

    /// The active source.
    pub fn source(&self) -> Source {
        self.source
    }

    /// One line for the status bar.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The tempo in the latest frame, in BPM.
    pub fn bpm(&self) -> f32 {
        self.bpm_index
            .and_then(|index| self.frame.scalars.get(index))
            .copied()
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ settings

    /// Switches the capture device. The analyser reopens on its next pass.
    pub fn select_device(&mut self, choice: DeviceChoice) {
        log::info!("input set to {choice:?}");
        self.choice = choice.clone();
        if let Some(capture) = &self.capture {
            capture.select_device(choice);
        }
    }

    /// Sets the gain on levels and hits.
    pub fn set_reactivity(&mut self, gain: f32) {
        self.reactivity = gain.clamp(*GAIN.start(), *GAIN.end());
        self.push_gains();
    }

    /// Sets the gain on the time clocks.
    pub fn set_speed(&mut self, gain: f32) {
        self.speed = gain.clamp(*GAIN.start(), *GAIN.end());
        self.push_gains();
    }

    /// Sets the meta controls' reactivity and audio speed (D184). They
    /// multiply the audio panel's own gains, so the panel keeps working and
    /// a script's `meta/reactivity` write scales on top of it. Called every
    /// frame; the capture is only told when the product changes.
    pub fn set_meta_gains(&mut self, reactivity: f32, speed: f32) {
        let clean = |gain: f32| if gain.is_finite() { gain.max(0.0) } else { 1.0 };
        let gains = [clean(reactivity), clean(speed)];
        if gains != self.meta_gain {
            self.meta_gain = gains;
            self.push_gains();
        }
    }

    /// The gains the analyser runs with: the panel's times the meta's,
    /// clamped to the analyser's range.
    fn effective_gains(&self) -> [f32; 2] {
        let clamp = |gain: f32| gain.clamp(*GAIN.start(), *GAIN.end());
        [
            clamp(self.reactivity * self.meta_gain[0]),
            clamp(self.speed * self.meta_gain[1]),
        ]
    }

    /// Sends the effective gains to the capture if they changed.
    fn push_gains(&mut self) {
        let gains = self.effective_gains();
        if let Some(capture) = &self.capture
            && gains != self.pushed_gain
        {
            capture.set_reactivity(gains[0]);
            capture.set_audio_speed(gains[1]);
        }
        self.pushed_gain = gains;
    }

    /// The effective reactivity and speed gains, for tests and the panel.
    pub fn effective(&self) -> [f32; 2] {
        self.effective_gains()
    }

    /// Registers a tap-tempo tap. Only a live input has a tempo to set.
    pub fn tap(&mut self) {
        match (&self.capture, self.source) {
            (Some(capture), Source::Live) => capture.tap(),
            (Some(_) | None, Source::Live | Source::Synthetic) => {
                log::info!("tap tempo needs a live audio input (synthetic is fixed at 120 BPM)");
            }
        }
    }

    /// Whether a live capture is the active source.
    pub fn is_live(&self) -> bool {
        self.source == Source::Live
    }

    /// Whether capture was asked for (`--audio synthetic` says no).
    pub fn can_capture(&self) -> bool {
        self.wants_capture
    }

    /// What the audio panel shows and edits this frame.
    pub fn panel(&mut self) -> AudioPanel<'_> {
        let bpm = self.bpm();
        AudioPanel {
            source: &self.label,
            live: self.source == Source::Live,
            can_capture: self.wants_capture,
            devices: &self.devices,
            choice: &self.choice,
            reactivity: &mut self.reactivity,
            speed: &mut self.speed,
            bpm,
            input_rms: self.frame.energy[0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn forced_synthetic_audio_is_a_function_of_time() -> Result<()> {
        let mut input = AudioInput::new(false);
        let at_beat = input.frame_at(0.0).scalars.clone();
        let later = input.frame_at(0.25).scalars.clone();
        let again = input.frame_at(0.0).scalars.clone();
        assert_eq!(at_beat, again, "same time, same frame");
        assert_ne!(at_beat, later, "the track moves");
        assert_eq!(
            input.source(),
            Source::Synthetic,
            "no capture was asked for"
        );
        assert!(
            input.label().contains("synthetic"),
            "label: {}",
            input.label()
        );
        Ok(())
    }

    #[test]
    fn ambient_frames_keep_the_clocks_and_drop_the_beat() -> Result<()> {
        let index = |name: &str| SCALARS.iter().position(|scalar| *scalar == name);
        let (Some(hits), Some(on_beat), Some(level), Some(time), Some(bpm)) = (
            index("syn_BassHits"),
            index("syn_OnBeat"),
            index("syn_BassLevel"),
            index("syn_Time"),
            index("syn_BPM"),
        ) else {
            anyhow::bail!("the scalars this test reads exist");
        };
        let mut plain = AudioInput::new(false);
        let mut calm = AudioInput::new(false).with_ambient(true);
        let mut beat_seen = false;
        for step in 0..240 {
            let t = step as f32 / 60.0;
            let synthetic = plain.frame_at(t).scalars.clone();
            beat_seen |= synthetic[hits] > 0.1 || synthetic[on_beat] > 0.5;
            let frame = calm.frame_at(t);
            assert!(
                frame.scalars[hits] == 0.0 && frame.scalars[on_beat] == 0.0,
                "no hits or beat triggers at {t}"
            );
            assert!(
                (0.2..=0.5).contains(&frame.scalars[level]),
                "levels are a gentle swell at {t}: {}",
                frame.scalars[level]
            );
            assert!(
                frame.scalars[time] == synthetic[time] && frame.scalars[bpm] == synthetic[bpm],
                "clocks and tempo are the synthetic track's at {t}"
            );
        }
        assert!(beat_seen, "the plain synthetic track does have a beat");
        Ok(())
    }

    #[test]
    fn synthetic_frames_carry_textures_and_the_tempo() -> Result<()> {
        let mut input = AudioInput::new(false);
        let frame = input.frame_at(1.3);
        assert!(
            frame.spectrum.iter().flatten().any(|v| *v > 0.0),
            "the synthetic spectrum is not silent"
        );
        assert!(
            frame.level_trail.iter().flatten().any(|v| *v > 0.0),
            "nor is the trail"
        );
        assert!((input.bpm() - 120.0).abs() < 1e-3, "bpm {}", input.bpm());
        Ok(())
    }

    #[test]
    fn gains_are_clamped_to_the_analysers_range() -> Result<()> {
        let mut input = AudioInput::new(false);
        input.set_reactivity(9.0);
        input.set_speed(-1.0);
        let panel = input.panel();
        assert!((*panel.reactivity - 4.0).abs() < f32::EPSILON, "reactivity");
        assert!(panel.speed.abs() < f32::EPSILON, "speed");
        Ok(())
    }

    #[test]
    fn meta_gains_multiply_the_panels_and_stay_in_range() -> Result<()> {
        let mut input = AudioInput::new(false);
        input.set_reactivity(1.5);
        input.set_speed(2.0);
        input.set_meta_gains(2.0, 0.5);
        assert_eq!(input.effective(), [3.0, 1.0], "product of panel and meta");
        input.set_meta_gains(2.0, 9.0);
        assert_eq!(input.effective()[1], 4.0, "clamped to the analyser's range");
        input.set_meta_gains(f32::NAN, -1.0);
        assert_eq!(
            input.effective(),
            [1.5, 0.0],
            "NaN is neutral, negative is zero"
        );
        Ok(())
    }

    #[test]
    fn tapping_without_a_live_input_is_harmless() -> Result<()> {
        let mut input = AudioInput::new(false);
        input.tap();
        assert_eq!(input.source(), Source::Synthetic, "still synthetic");
        Ok(())
    }
}
