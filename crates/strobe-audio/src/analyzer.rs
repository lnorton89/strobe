// ============================================================================
// Module:       strobe_audio::analyzer
// Description:  The deterministic analysis engine: mono samples in, one
//               AudioFrame of SSF uniforms and texture data out.
//
// Dependencies: realfft; strobe-scene (SCALARS); sibling modules bands,
//               frame, onset, signal, smoothing, spectrum, tempo
// ============================================================================

//! The analyser.
//!
//! Pure DSP: no I/O, no threads, no clock. Feed it mono samples with
//! [`Analyzer::push`] and advance it with [`Analyzer::analyze`]. Given the
//! same samples and the same sequence of `dt`s it produces the same bits,
//! which is what makes the offline tuning harness and the tests possible.
//!
//! # Two clocks
//!
//! Analysis runs on *audio time*: every 512 samples, one hop, the newest
//! 2048 samples are windowed and transformed, and the band, flux, onset and
//! tempo trackers update with the hop's duration (`512 / sample_rate`).
//! Presentation runs on the caller's `dt`: levels glide, hits decay, clocks
//! integrate and the beat grid advances by the seconds the caller says
//! passed. A hop's results wait as *targets* until the next
//! [`Analyzer::analyze`], which keeps the peak of any hit that arrived
//! between two calls so a slow render loop cannot miss a kick. The upshot
//! is frame-rate independence: smoothing is `1 - exp(-dt / tau)`, so 30 and
//! 60 calls a second settle to the same values.
//!
//! # No allocation per call
//!
//! Everything [`Analyzer::analyze`] touches is allocated in
//! [`Analyzer::with_layout`]: the FFT buffers, the history ring, the
//! spectrum scratch, the trail ring, the output frame. The per-call path
//! only indexes into them, which is why it returns a reference to the
//! analyser's own frame rather than a new one. The `frame_storage_is_stable`
//! test checks that the frame's buffers never move or grow.

use std::f32::consts::TAU;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use crate::bands::{BAND_COUNT, BandLayout};
use crate::downbeat::{BANDS, Downbeat};
use crate::frame::{AudioFrame, Layout};
use crate::onset::BeatPicker;
use crate::signal::Signal;
use crate::smoothing::{DcBlocker, Follower, Logistic, PeakTracker, blend, decay};
use crate::spectrum::Spectra;
use crate::tempo::{BPM_DEFAULT, BeatGrid, Tempo};

/// FFT size: 2048 points is 43 ms at 48 kHz, enough to resolve a kick's
/// pitch from the bass band's bins (23 Hz apart) yet short enough to catch
/// a drum hit.
pub const FFT_LEN: usize = 2048;

/// Samples between transforms: a quarter of the window, so each transform
/// overlaps the last by 75% and onsets are located to about 11 ms.
pub const HOP: usize = 512;

/// Channels the per-channel families carry: the whole spectrum, then the
/// four bands.
const CHANNELS: usize = BAND_COUNT + 1;

/// Samples of input history kept. A power of two so indexing is a mask.
/// Large enough that a caller who falls 0.3 s behind (at 48 kHz) still has
/// every sample the pending hops need.
const HISTORY_LEN: usize = 16_384;

/// Mask for indexing the history ring.
const HISTORY_MASK: usize = HISTORY_LEN - 1;

/// Samples of the latest waveform the textures draw from.
const WAVE_LEN: usize = 1024;

/// Scales an FFT bin so that a full-scale sine reads 1.0 at its peak bin:
/// the Hann window's coherent gain is `FFT_LEN / 2`, and a real sine puts
/// half its amplitude in the positive-frequency bin.
const MAG_SCALE: f32 = 4.0 / FFT_LEN as f32;

/// The longest `dt` honoured. A stalled caller must not make clocks leap or
/// followers snap on resuming.
const MAX_DT: f32 = 0.25;

/// RMS below this is never amplified to full scale by normalisation
/// (about -60 dBFS).
const RMS_FLOOR: f32 = 1.0e-3;

/// Spectral flux below this is never amplified to a full hit: the summed
/// magnitude rise of a -54 dBFS sine.
const FLUX_FLOOR: f32 = 2.0e-3;

/// Seconds for the running RMS maximum to forget a loud passage.
const RMS_PEAK_TAU: f32 = 10.0;

/// Seconds for the running flux maximum to forget a loud transient.
const FLUX_PEAK_TAU: f32 = 6.0;

/// Seconds over which average flux is tracked. Hits are measured above it,
/// so steady-state flicker does not read as percussion.
const FLUX_AVG_TAU: f32 = 0.5;

/// Whole-spectrum RMS above which the room counts as not silent
/// (about -66 dBFS).
const ACTIVE_RMS: f32 = 5.0e-4;

/// Seconds of quiet before the music is considered to have stopped.
const SILENCE_HOLD: f32 = 2.5;

/// Seconds without a single new hop before the input is considered
/// stalled and the targets fall to zero.
const STARVE_AFTER: f32 = 0.3;

/// Level rise time constant: quick enough to show a kick.
const LEVEL_ATTACK: f32 = 0.03;
/// Level fall time constant: slow enough not to flicker.
const LEVEL_RELEASE: f32 = 0.2;

/// Hit decay time constant.
const HIT_TAU: f32 = 0.1;

/// `syn_OnBeat` fall time constant: "quickly falls back to zero".
const ONBEAT_TAU: f32 = 0.07;

/// Presence averaging time constant; an exponential average with this
/// constant behaves like a window of about twice as long, ~4 s.
const PRESENCE_TAU: f32 = 2.0;

/// How strongly hits push the time clocks beyond what level alone does,
/// so a kick makes `syn_BassTime` surge.
const HIT_TIME_BOOST: f32 = 2.0;

/// Gain of the convex clock: at full level it runs this many times real
/// time, and at half level an eighth of that.
const CURVED_TIME_GAIN: f32 = 4.0;

/// Weights of each band's hit strength in the tempo envelope. Bass leads;
/// highs are left out so a hat pattern at 8th or 16th notes does not pull
/// the estimate to double time.
const ONSET_WEIGHTS: [f32; BAND_COUNT] = [0.6, 0.3, 0.1, 0.0];

/// The published tempo confidence below which the beat grid is not fed to
/// the downbeat estimate: the grid is still finding the beat.
const DOWNBEAT_MIN_TEMPO_CONFIDENCE: f32 = 0.3;

/// Tempo smoothing time constant for the published BPM.
const BPM_TAU: f32 = 0.4;
/// Confidence smoothing time constant.
const CONFIDENCE_TAU: f32 = 0.8;

/// Time constant of the gate that fades beat oscillators in and out as
/// music starts and stops.
const GATE_TAU: f32 = 0.3;

/// `syn_FadeInOut` rise and fall time constants.
const FADE_RISE: f32 = 2.5;
/// See [`FADE_RISE`].
const FADE_FALL: f32 = 4.0;

/// `syn_Intensity` rise and fall time constants: it accumulates over tens
/// of seconds.
const INTENSITY_RISE: f32 = 12.0;
/// See [`INTENSITY_RISE`].
const INTENSITY_FALL: f32 = 25.0;

/// Steepness of the exponential jump in `syn_BPMTwitcher`'s fractional part.
const TWITCH_SHARPNESS: f32 = 6.0;

/// Largest manual gain accepted for reactivity and audio speed.
const MAX_GAIN: f32 = 4.0;

/// Hit strengths and levels a hop leaves for the next [`Analyzer::analyze`].
#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    /// Normalised RMS per channel, from the latest hop.
    level: [f32; CHANNELS],
    /// Hit strength per channel: the peak over hops since the last analyze.
    hit: [f32; CHANNELS],
    /// Beats detected since the last analyze.
    beats: u32,
    /// Raw RMS of the whole signal and the bass band, from the latest hop.
    energy: [f32; 2],
}

/// The audio analyser.
pub struct Analyzer {
    /// Input rate in Hz.
    sample_rate: f32,
    /// Seconds per hop.
    hop_dt: f32,
    /// Texture sizes.
    layout: Layout,
    /// Band edges as bin ranges at this sample rate.
    bands: BandLayout,

    /// Newest input samples, indexed by absolute position masked.
    history: Vec<f32>,
    /// Total samples pushed.
    written: u64,
    /// Total samples covered by hops processed so far.
    hop_end: u64,
    /// Removes a constant input offset before anything measures it.
    dc: DcBlocker,

    /// Forward transform plan.
    fft: Arc<dyn RealToComplex<f32>>,
    /// Hann window, periodic.
    window: Vec<f32>,
    /// Transform input; the transform uses it as scratch.
    fft_in: Vec<f32>,
    /// Transform output.
    fft_out: Vec<Complex<f32>>,
    /// Transform scratch.
    fft_scratch: Vec<Complex<f32>>,
    /// This hop's magnitudes.
    magnitude: Vec<f32>,
    /// The previous hop's magnitudes.
    previous_magnitude: Vec<f32>,
    /// The latest waveform samples.
    wave: Vec<f32>,

    /// Running RMS maximum per channel.
    rms_peak: [PeakTracker; CHANNELS],
    /// Running flux maximum per channel.
    flux_peak: [PeakTracker; CHANNELS],
    /// Running flux average per channel.
    flux_average: [f32; CHANNELS],
    /// What the hops have left for the output stage.
    pending: Pending,
    /// Bass-onset peak picker.
    picker: BeatPicker,
    /// Tempo tracking.
    tempo: Tempo,
    /// Seconds since the input was last above the activity threshold.
    quiet_for: f32,
    /// Whether anything above the threshold has ever been heard.
    ever_heard: bool,
    /// Seconds since a hop was last processed.
    starved: f32,

    /// Smoothed levels, before reactivity.
    level: [Follower; CHANNELS],
    /// Decaying hits, before reactivity.
    hit: [f32; CHANNELS],
    /// Final levels after reactivity.
    out_level: [f32; CHANNELS],
    /// Final hits after reactivity.
    out_hit: [f32; CHANNELS],
    /// Presence per channel.
    presence: [f32; CHANNELS],
    /// Level-driven clocks per channel, in f64 so hours of accumulation
    /// keep their precision.
    times: [f64; CHANNELS],
    /// The convex clock.
    curved: f64,
    /// `syn_OnBeat`.
    on_beat: f32,
    /// Beats counted.
    beat_count: u64,
    /// Toggle state and its smoothed value.
    toggled: bool,
    /// `syn_ToggleOnBeat`.
    toggle: Logistic,
    /// `syn_RandomOnBeat`.
    random: Logistic,
    /// Random generator state; xorshift32, never zero.
    rng: u32,
    /// Beat-locked phase.
    grid: BeatGrid,
    /// Which grid beat starts a bar (D233).
    downbeat: Downbeat,
    /// The beat counter minus the grid's beat number at the last detected
    /// beat: turns the estimate's grid offset into the counter's, which is
    /// what `syn_BeatTime` and the playlist clock count.
    counter_lead: i64,
    /// Whether hops feed the downbeat estimate (the grid is following).
    downbeat_live: bool,
    /// Hops run so far in this `analyze` call, to place each hop on the
    /// grid between frames.
    frame_hops: u32,
    /// Published tempo.
    bpm: f32,
    /// Published confidence.
    confidence: Follower,
    /// 0 in silence, 1 with music; fades the beat oscillators.
    gate: f32,
    /// `syn_FadeInOut`.
    fade: Follower,
    /// `syn_Intensity`.
    intensity: Follower,

    /// Gain on levels and hits.
    reactivity: f32,
    /// Gain on the time clocks.
    audio_speed: f32,

    /// Level history ring, newest at `trail_head`.
    trail: Vec<[f32; 4]>,
    /// Index of the newest trail entry.
    trail_head: usize,
    /// Seconds accrued toward the next trail entry.
    trail_accrued: f32,

    /// Spectrum texture builder.
    spectra: Spectra,
    /// How to fill each scalar slot.
    signals: Vec<Signal>,
    /// The output.
    frame: AudioFrame,
}

impl Analyzer {
    /// An analyser for mono audio at `sample_rate`, with default texture
    /// sizes.
    pub fn new(sample_rate: u32) -> Self {
        Self::with_layout(sample_rate, Layout::default())
    }

    /// An analyser with explicit texture sizes. The sample rate is clamped
    /// to 8 kHz-192 kHz.
    pub fn with_layout(sample_rate: u32, layout: Layout) -> Self {
        let layout = layout.clamped();
        let sample_rate = sample_rate.clamp(8_000, 192_000) as f32;
        let hop_dt = HOP as f32 / sample_rate;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_LEN);
        let window = (0..FFT_LEN)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT_LEN as f32).cos())
            .collect();
        let bins = FFT_LEN / 2 + 1;
        Self {
            sample_rate,
            hop_dt,
            layout,
            bands: BandLayout::new(sample_rate, FFT_LEN),
            history: vec![0.0; HISTORY_LEN],
            written: 0,
            hop_end: 0,
            dc: DcBlocker::new(sample_rate),
            fft_in: fft.make_input_vec(),
            fft_out: fft.make_output_vec(),
            fft_scratch: fft.make_scratch_vec(),
            fft,
            window,
            magnitude: vec![0.0; bins],
            previous_magnitude: vec![0.0; bins],
            wave: vec![0.0; WAVE_LEN],
            rms_peak: [PeakTracker::new(RMS_FLOOR); CHANNELS],
            flux_peak: [PeakTracker::new(FLUX_FLOOR); CHANNELS],
            flux_average: [0.0; CHANNELS],
            pending: Pending::default(),
            picker: BeatPicker::new(),
            tempo: Tempo::new(1.0 / hop_dt),
            quiet_for: 0.0,
            ever_heard: false,
            starved: 0.0,
            level: [Follower::default(); CHANNELS],
            hit: [0.0; CHANNELS],
            out_level: [0.0; CHANNELS],
            out_hit: [0.0; CHANNELS],
            presence: [0.0; CHANNELS],
            times: [0.0; CHANNELS],
            curved: 0.0,
            on_beat: 0.0,
            beat_count: 0,
            toggled: false,
            toggle: Logistic::at(0.0),
            random: Logistic::at(0.0),
            rng: 0x9E37_79B9,
            grid: BeatGrid::default(),
            downbeat: Downbeat::new(),
            counter_lead: 0,
            downbeat_live: false,
            frame_hops: 0,
            bpm: BPM_DEFAULT,
            confidence: Follower::default(),
            gate: 0.0,
            fade: Follower::default(),
            intensity: Follower::default(),
            reactivity: 1.0,
            audio_speed: 1.0,
            trail: vec![[0.0; 4]; layout.trail_len],
            trail_head: 0,
            trail_accrued: 0.0,
            spectra: Spectra::new(layout, sample_rate, bins),
            signals: Signal::all(),
            frame: AudioFrame::silent(layout),
        }
    }

    /// The input sample rate in Hz.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// The texture sizes this analyser produces.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// The most recent frame, without advancing anything.
    pub fn frame(&self) -> &AudioFrame {
        &self.frame
    }

    /// Appends mono samples. Never blocks, never allocates. Non-finite
    /// samples count as silence, and magnitudes above 8 are clipped so one
    /// bad driver value cannot poison the normalisers. A constant offset is
    /// removed (a 10 Hz high-pass, D327): it is not sound, and it would
    /// read as bass.
    ///
    /// If more than about 0.3 s of samples are pushed without an
    /// [`Analyzer::analyze`], the oldest are skipped rather than queued
    /// without bound.
    pub fn push(&mut self, samples: &[f32]) {
        for sample in samples {
            let finite = if sample.is_finite() {
                sample.clamp(-8.0, 8.0)
            } else {
                0.0
            };
            let clean = self.dc.step(finite).clamp(-8.0, 8.0);
            let slot = (self.written as usize) & HISTORY_MASK;
            if let Some(cell) = self.history.get_mut(slot) {
                *cell = clean;
            }
            self.written += 1;
        }
    }

    /// Sets the gain on levels and hits. 1.0 is neutral; clamped to 0..4.
    /// Non-finite values are ignored.
    pub fn set_reactivity(&mut self, gain: f32) {
        if gain.is_finite() {
            self.reactivity = gain.clamp(0.0, MAX_GAIN);
        }
    }

    /// The gain on levels and hits.
    pub fn reactivity(&self) -> f32 {
        self.reactivity
    }

    /// Sets the gain on the time clocks. 1.0 is neutral; clamped to 0..4.
    /// Non-finite values are ignored.
    pub fn set_audio_speed(&mut self, gain: f32) {
        if gain.is_finite() {
            self.audio_speed = gain.clamp(0.0, MAX_GAIN);
        }
    }

    /// The gain on the time clocks.
    pub fn audio_speed(&self) -> f32 {
        self.audio_speed
    }

    /// Registers a tap at `now` seconds on any monotonic clock the caller
    /// likes. Two or more taps in a row set the tempo manually; every tap
    /// aligns the beat grid so the tap lands on a beat.
    pub fn tap(&mut self, now: f64) {
        self.tempo.tap(now);
        self.grid.on_tap();
    }

    /// Moves the tempo by `delta` BPM and holds it, overriding the
    /// estimator until [`Analyzer::clear_manual_tempo`].
    pub fn nudge_bpm(&mut self, delta: f32) {
        self.tempo.nudge(delta);
    }

    /// Hands the tempo back to the automatic estimator.
    pub fn clear_manual_tempo(&mut self) {
        self.tempo.clear_manual();
    }

    /// Whether the operator's tempo (tap or nudge) is in force.
    pub fn manual_tempo(&self) -> bool {
        self.tempo.is_manual()
    }

    /// Processes the samples pushed since the last call and advances
    /// presentation by `dt` seconds. Returns the analyser's own frame.
    ///
    /// Allocation-free; see the module documentation.
    pub fn analyze(&mut self, dt: f32) -> &AudioFrame {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, MAX_DT)
        } else {
            0.0
        };
        self.frame_hops = 0;
        let hops = self.consume_hops();
        if hops == 0 {
            self.starved += dt;
            self.quiet_for += dt;
        } else {
            self.starved = 0.0;
        }
        self.copy_wave();
        self.advance_outputs(dt);
        self.fill_frame(dt);
        &self.frame
    }

    /// Runs every hop the pushed samples complete; returns how many.
    fn consume_hops(&mut self) -> u32 {
        let backlog_limit = (HISTORY_LEN - FFT_LEN) as u64;
        let backlog = self.written.saturating_sub(self.hop_end);
        if backlog > backlog_limit {
            let skip = (backlog - FFT_LEN as u64) / HOP as u64 * HOP as u64;
            self.hop_end += skip;
        }
        let mut hops = 0;
        while self.hop_end + HOP as u64 <= self.written {
            self.hop_end += HOP as u64;
            self.process_hop();
            hops += 1;
        }
        hops
    }

    /// Copies the newest samples into the waveform buffer.
    fn copy_wave(&mut self) {
        let start = self.written.wrapping_sub(WAVE_LEN as u64);
        for (j, slot) in self.wave.iter_mut().enumerate() {
            let index = (start.wrapping_add(j as u64) as usize) & HISTORY_MASK;
            *slot = self.history.get(index).copied().unwrap_or(0.0);
        }
    }

    /// Transforms the window ending at `hop_end` and updates the trackers.
    fn process_hop(&mut self) {
        let start = self.hop_end.wrapping_sub(FFT_LEN as u64);
        for (j, (slot, w)) in self.fft_in.iter_mut().zip(&self.window).enumerate() {
            let index = (start.wrapping_add(j as u64) as usize) & HISTORY_MASK;
            *slot = self.history.get(index).copied().unwrap_or(0.0) * w;
        }
        let result = self.fft.process_with_scratch(
            &mut self.fft_in,
            &mut self.fft_out,
            &mut self.fft_scratch,
        );
        if result.is_err() {
            return;
        }
        std::mem::swap(&mut self.magnitude, &mut self.previous_magnitude);
        for (m, c) in self.magnitude.iter_mut().zip(&self.fft_out) {
            *m = (c.re * c.re + c.im * c.im).sqrt() * MAG_SCALE;
        }

        let reading = self
            .bands
            .measure(&self.magnitude, &self.previous_magnitude);
        let mut rms = [0.0; CHANNELS];
        let mut flux = [0.0; CHANNELS];
        for band in 0..BAND_COUNT {
            let r = reading.rms.get(band).copied().unwrap_or(0.0);
            let f = reading.flux.get(band).copied().unwrap_or(0.0);
            if let (Some(rs), Some(fs)) = (rms.get_mut(band + 1), flux.get_mut(band + 1)) {
                *rs = r;
                *fs = f;
            }
            rms[0] += r * r;
            flux[0] += f;
        }
        rms[0] = rms[0].sqrt();
        self.pending.energy = [rms[0], rms.get(1).copied().unwrap_or(0.0)];

        let rms_decay = decay(self.hop_dt, RMS_PEAK_TAU);
        let flux_decay = decay(self.hop_dt, FLUX_PEAK_TAU);
        let average_blend = blend(self.hop_dt, FLUX_AVG_TAU);
        let mut hit = [0.0; CHANNELS];
        for c in 0..CHANNELS {
            self.rms_peak[c].update(rms[c], rms_decay);
            let level = rms[c] / self.rms_peak[c].divisor();
            self.pending.level[c] = level.clamp(0.0, 1.0);

            self.flux_peak[c].update(flux[c], flux_decay);
            let average = self.flux_average[c];
            let divisor = self.flux_peak[c].divisor();
            let span = (divisor - average).max(0.25 * divisor);
            hit[c] = ((flux[c] - average) / span).clamp(0.0, 1.0);
            self.pending.hit[c] = self.pending.hit[c].max(hit[c]);
            self.flux_average[c] += (flux[c] - average) * average_blend;
        }

        if rms[0] > ACTIVE_RMS {
            self.quiet_for = 0.0;
            self.ever_heard = true;
        } else {
            self.quiet_for += self.hop_dt;
        }

        let onset: f32 = ONSET_WEIGHTS
            .iter()
            .enumerate()
            .map(|(band, w)| w * hit.get(band + 1).copied().unwrap_or(0.0))
            .sum();
        self.tempo.observe(onset);

        let gap = if self.tempo.confidence() > 0.3 {
            (0.45 * 60.0 / self.tempo.bpm()).clamp(0.15, 0.4)
        } else {
            0.25
        };
        if self.picker.step(hit[1], self.hop_dt, gap) {
            self.pending.beats += 1;
        }

        // The downbeat estimate hears every hop, not a frame's latest: the
        // hops' alignment to the beat drifts over several beats, and a
        // once-a-frame sample of it reads as an accent on some beats. It
        // takes the raw bass energy (the hits are clamped to 1, so a harder
        // kick would read the same) and the four band levels.
        if self.downbeat_live {
            let ahead = self.frame_hops as f32 * self.hop_dt * self.bpm / 60.0;
            let bands: [f32; BANDS] = std::array::from_fn(|band| {
                self.pending.level.get(band + 1).copied().unwrap_or(0.0)
            });
            self.downbeat
                .observe(self.grid.beats() + f64::from(ahead), rms[1], bands);
        }
        self.frame_hops += 1;
    }

    /// Advances levels, hits, beats, clocks and the beat grid by `dt`.
    fn advance_outputs(&mut self, dt: f32) {
        let starving = self.starved > STARVE_AFTER;
        let active = self.ever_heard && self.quiet_for < SILENCE_HOLD;
        let gain = self.reactivity;

        for c in 0..CHANNELS {
            let level_target = if starving { 0.0 } else { self.pending.level[c] };
            let smoothed = self.level[c].step(level_target, dt, LEVEL_ATTACK, LEVEL_RELEASE);
            self.hit[c] = self.pending.hit[c].max(self.hit[c] * decay(dt, HIT_TAU));
            self.out_level[c] = (smoothed * gain).clamp(0.0, 1.0);
            self.out_hit[c] = (self.hit[c] * gain).clamp(0.0, 1.0);
            self.presence[c] += (self.out_level[c] - self.presence[c]) * blend(dt, PRESENCE_TAU);
            let rate = self.out_level[c] + HIT_TIME_BOOST * self.out_hit[c];
            self.times[c] += f64::from(dt * self.audio_speed * rate);
        }
        self.pending.hit = [0.0; CHANNELS];
        let whole = self.out_level[0];
        self.curved += f64::from(dt * self.audio_speed * CURVED_TIME_GAIN * whole * whole * whole);

        // Tempo and the beat grid.
        self.gate += (if active { 1.0 } else { 0.0 } - self.gate) * blend(dt, GATE_TAU);
        if active {
            self.grid.advance(dt, self.bpm);
        } else {
            self.grid.unlock();
            self.downbeat.reset();
            self.tempo.silence();
        }
        // Only a grid that follows the music says anything about bars; the
        // next frame's hops feed the estimate if this one's does.
        self.downbeat_live = active && self.confidence.value > DOWNBEAT_MIN_TEMPO_CONFIDENCE;
        self.bpm += (self.tempo.bpm() - self.bpm) * blend(dt, BPM_TAU);
        let confidence_target = if active { self.tempo.confidence() } else { 0.0 };
        self.confidence
            .step(confidence_target, dt, CONFIDENCE_TAU, CONFIDENCE_TAU);

        // Beats.
        let beats = std::mem::take(&mut self.pending.beats);
        if beats > 0 {
            self.on_beat = 1.0;
            self.beat_count += u64::from(beats);
            if beats % 2 == 1 {
                self.toggled = !self.toggled;
                self.toggle.retarget(if self.toggled { 1.0 } else { 0.0 });
            }
            let value = self.next_random();
            self.random.retarget(value);
            self.grid.on_beat(self.bpm);
            self.counter_lead = self.beat_count as i64 - self.grid.beats().round() as i64;
        } else {
            self.on_beat *= decay(dt, ONBEAT_TAU);
        }
        self.toggle.advance(dt);
        self.random.advance(dt);

        // Long-form followers.
        self.fade
            .step(if active { 1.0 } else { 0.0 }, dt, FADE_RISE, FADE_FALL);
        let drive = if active {
            (self.out_level[0] + 0.5 * self.out_hit[0]).min(1.0)
        } else {
            0.0
        };
        self.intensity
            .step(drive, dt, INTENSITY_RISE, INTENSITY_FALL);

        self.advance_trail(dt);
    }

    /// Pushes level history at the trail's rate.
    fn advance_trail(&mut self, dt: f32) {
        let interval = self.layout.trail_seconds / self.layout.trail_len as f32;
        self.trail_accrued += dt;
        let due = ((self.trail_accrued / interval) as usize).min(self.trail.len());
        self.trail_accrued = (self.trail_accrued - due as f32 * interval).max(0.0);
        let sample = [
            self.out_level[0],
            self.out_level[1],
            0.5 * (self.out_level[2] + self.out_level[3]),
            self.out_level[4],
        ];
        for _ in 0..due {
            self.trail_head = (self.trail_head + 1) % self.trail.len();
            if let Some(slot) = self.trail.get_mut(self.trail_head) {
                *slot = sample;
            }
        }
    }

    /// The next value in 0..1 from a xorshift32 stream.
    fn next_random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / 16_777_216.0
    }

    /// Writes scalars, trail and spectrum textures into the frame.
    fn fill_frame(&mut self, dt: f32) {
        let beats = self.grid.beats();
        let twitch = {
            let whole = beats.floor();
            let part = (beats - whole) as f32;
            let jump = (1.0 - (-TWITCH_SHARPNESS * part).exp()) / (1.0 - (-TWITCH_SHARPNESS).exp());
            (whole + f64::from(jump)) as f32
        };
        let phase = |period: f32| ((beats / f64::from(period)).fract()) as f32;
        let channel = |values: &[f32; CHANNELS], c: usize| values.get(c).copied().unwrap_or(0.0);

        self.frame.energy = if self.starved > STARVE_AFTER {
            [0.0; 2]
        } else {
            self.pending.energy
        };
        self.frame.downbeat = self.downbeat.estimate().map_or([0.0; 2], |bar| {
            let offset = (i64::from(bar.offset) + self.counter_lead).rem_euclid(4);
            [offset as f32, bar.confidence]
        });
        for (slot, signal) in self.frame.scalars.iter_mut().zip(&self.signals) {
            *slot = match *signal {
                Signal::Level(c) => channel(&self.out_level, c),
                Signal::Hits(c) => channel(&self.out_hit, c),
                Signal::Time(c) => self.times.get(c).copied().unwrap_or(0.0) as f32,
                Signal::CurvedTime => self.curved as f32,
                Signal::Presence(c) => channel(&self.presence, c),
                Signal::OnBeat => self.on_beat,
                Signal::ToggleOnBeat => self.toggle.value(),
                Signal::RandomOnBeat => self.random.value(),
                Signal::BeatTime => self.beat_count as f32,
                Signal::Bpm => self.bpm,
                Signal::BpmConfidence => self.confidence.value,
                Signal::BpmTwitcher => twitch,
                Signal::BpmSin(period) => self.gate * (0.5 + 0.5 * (TAU * phase(period)).sin()),
                Signal::BpmTri(period) => self.gate * (1.0 - (2.0 * phase(period) - 1.0).abs()),
                Signal::FadeInOut => self.fade.value,
                Signal::Intensity => self.intensity.value,
                Signal::Constant(value) => value,
            };
        }

        let len = self.trail.len();
        for (i, texel) in self.frame.level_trail.iter_mut().enumerate() {
            let index = (self.trail_head + len - i.min(len - 1)) % len;
            *texel = self.trail.get(index).copied().unwrap_or([0.0; 4]);
        }

        self.spectra
            .update(&self.magnitude, &self.wave, dt, &mut self.frame);
    }
}

impl std::fmt::Debug for Analyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Analyzer")
            .field("sample_rate", &self.sample_rate)
            .field("layout", &self.layout)
            .field("bpm", &self.bpm)
            .finish_non_exhaustive()
    }
}
