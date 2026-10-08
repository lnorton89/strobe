// ============================================================================
// Module:       strobe_audio::spectrum
// Description:  Builds the syn_Spectrum channels (raw, juiced, smooth,
//               waveform) and the Shadertoy audio rows from one FFT.
//
// Dependencies: crate::frame (AudioFrame, Layout), crate::smoothing
// ============================================================================

//! Spectrum texture data.
//!
//! Four views of one spectrum, because a scene wants different things from
//! it. *Raw* is the FFT as measured, for scenes that draw the truth.
//! *Juiced* is what a viewer wants to look at: frequency on a log axis (so
//! a bass note and an octave above it are the same distance apart), a
//! treble tilt (music falls roughly 3 dB per octave, so a flat display
//! would show only bass), smoothing across neighbours and over time. *Smooth*
//! is the juiced curve blurred to a few broad humps. *Waveform* is the
//! signal itself.
//!
//! Every texture is as wide as the [`Layout`] says. Each texel is built
//! from a *span* of FFT bins computed once at construction, so changing the
//! width changes data, not code, and nothing here assumes 256 or 512.
//!
//! All buffers are allocated in [`Spectra::new`]; [`Spectra::update`] only
//! indexes into them.

use crate::frame::{AudioFrame, Layout};
use crate::smoothing::blend;

/// The loudest and quietest levels the float textures show, in dB relative
/// to a full-scale sine. Music sits around -40 to -70 dB per bin, so an
/// 80 dB window puts it mid-scale.
const DB_RANGE: (f32, f32) = (-100.0, -20.0);

/// The browser analyser's decibel window, which the Shadertoy texture
/// reproduces so shaders written against it see familiar numbers.
const SHADERTOY_DB_RANGE: (f32, f32) = (-100.0, -30.0);

/// The browser analyser scales its FFT by 1/N where this crate scales a
/// sine to its amplitude, so its numbers are half ours: -6.02 dB.
const SHADERTOY_DB_OFFSET: f32 = -6.0206;

/// Lowest frequency on the juiced axis.
const JUICED_LOW_HZ: f32 = 30.0;

/// Highest frequency on the juiced axis (clamped under Nyquist).
const JUICED_HIGH_HZ: f32 = 16_000.0;

/// Treble tilt in dB per octave above [`TILT_START_HZ`].
const TILT_DB_PER_OCTAVE: f32 = 3.0;

/// Where the treble tilt begins.
const TILT_START_HZ: f32 = 250.0;

/// Juiced rise and fall time constants: quick to light up, slow to drop.
const JUICED_ATTACK: f32 = 0.03;
/// See [`JUICED_ATTACK`].
const JUICED_RELEASE: f32 = 0.2;

/// The Shadertoy rows are smoothed like the browser analyser's 0.8
/// smoothing constant at about 43 fps: roughly 0.1 s.
const SHADERTOY_TAU: f32 = 0.1;

/// Which FFT bins feed one texel.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Span {
    /// The peak over bins `first..last`.
    Peak {
        /// First bin, inclusive.
        first: usize,
        /// One past the last bin.
        last: usize,
    },
    /// A linear blend of two neighbouring bins, for texels narrower than one
    /// bin (the low end of a log axis).
    Blend {
        /// The lower bin; the upper is the next one.
        bin: usize,
        /// Weight of the upper bin.
        t: f32,
    },
}

impl Span {
    /// The span covering fractional bin positions `low..high`, kept inside
    /// `0..bins`.
    fn covering(low: f32, high: f32, bins: usize) -> Self {
        let top = bins.saturating_sub(1);
        if high - low < 1.0 {
            let centre = (0.5 * (low + high)).clamp(0.0, top as f32);
            let bin = (centre.floor() as usize).min(top.saturating_sub(1));
            return Self::Blend {
                bin,
                t: (centre - bin as f32).clamp(0.0, 1.0),
            };
        }
        let first = (low.ceil() as usize).min(top);
        let last = (high.ceil() as usize).clamp(first + 1, bins);
        Self::Peak { first, last }
    }

    /// Reads this span from a decibel-per-bin array.
    fn read(self, db: &[f32]) -> f32 {
        match self {
            Self::Peak { first, last } => db.get(first..last).map_or(DB_RANGE.0, |bins| {
                bins.iter().copied().fold(f32::MIN, f32::max)
            }),
            Self::Blend { bin, t } => {
                let a = db.get(bin).copied().unwrap_or(DB_RANGE.0);
                let b = db.get(bin + 1).copied().unwrap_or(a);
                a + (b - a) * t
            }
        }
    }
}

/// Maps decibels into 0..1 across `range`.
fn unit_db(db: f32, range: (f32, f32)) -> f32 {
    ((db - range.0) / (range.1 - range.0)).clamp(0.0, 1.0)
}

/// Mean of the waveform samples that fall under texel `i` of `len`.
fn wave_mean(wave: &[f32], len: usize, i: usize) -> f32 {
    let n = wave.len();
    let first = (i * n / len).min(n.saturating_sub(1));
    let last = ((i + 1) * n / len).clamp(first + 1, n);
    wave.get(first..last)
        .map_or(0.0, |s| s.iter().sum::<f32>() / s.len().max(1) as f32)
}

/// Spectrum-texture state: spans, smoothing memory and scratch space.
#[derive(Debug)]
pub(crate) struct Spectra {
    /// Linear-frequency spans for the raw channel.
    raw: Vec<Span>,
    /// Log-frequency spans for the juiced channel.
    juiced: Vec<Span>,
    /// Treble tilt per juiced texel, dB.
    tilt: Vec<f32>,
    /// Linear-frequency spans for the Shadertoy row.
    toy: Vec<Span>,
    /// Decibels per FFT bin for the current frame.
    db: Vec<f32>,
    /// Juiced values after temporal smoothing.
    juiced_state: Vec<f32>,
    /// Scratch for the spatial pass.
    scratch: Vec<f32>,
    /// Scratch for the smooth channel.
    scratch_b: Vec<f32>,
    /// Shadertoy spectrum values after temporal smoothing.
    toy_state: Vec<f32>,
    /// Box-blur radius for the smooth channel, in texels.
    blur_radius: usize,
}

impl Spectra {
    /// Builds the spans for `layout` at `sample_rate` with `bins` FFT bins.
    pub(crate) fn new(layout: Layout, sample_rate: f32, bins: usize) -> Self {
        let layout = layout.clamped();
        let fft_len = (bins.saturating_sub(1) * 2).max(2) as f32;
        let bin_hz = sample_rate / fft_len;
        // The last bin is Nyquist alone; the linear textures span the rest.
        let linear_bins = bins.saturating_sub(1).max(1);
        let linear = |len: usize| -> Vec<Span> {
            (0..len)
                .map(|i| {
                    let low = (i * linear_bins) as f32 / len as f32;
                    let high = ((i + 1) * linear_bins) as f32 / len as f32;
                    Span::covering(low, high, linear_bins)
                })
                .collect()
        };
        let top_hz = JUICED_HIGH_HZ.min(0.95 * 0.5 * sample_rate);
        let ratio = (top_hz / JUICED_LOW_HZ).max(1.01);
        let edge = |i: usize| JUICED_LOW_HZ * ratio.powf(i as f32 / layout.spectrum_len as f32);
        let mut juiced = Vec::with_capacity(layout.spectrum_len);
        let mut tilt = Vec::with_capacity(layout.spectrum_len);
        for i in 0..layout.spectrum_len {
            let (low, high) = (edge(i), edge(i + 1));
            juiced.push(Span::covering(low / bin_hz, high / bin_hz, bins));
            let centre = (low * high).sqrt();
            tilt.push(TILT_DB_PER_OCTAVE * (centre / TILT_START_HZ).log2().max(0.0));
        }
        Self {
            raw: linear(layout.spectrum_len),
            juiced,
            tilt,
            toy: linear(layout.shadertoy_width),
            db: vec![DB_RANGE.0; bins],
            juiced_state: vec![0.0; layout.spectrum_len],
            scratch: vec![0.0; layout.spectrum_len],
            scratch_b: vec![0.0; layout.spectrum_len],
            toy_state: vec![0.0; layout.shadertoy_width],
            blur_radius: (layout.spectrum_len / 40).max(1),
        }
    }

    /// Rebuilds the spectrum, waveform and Shadertoy data in `frame` from
    /// this hop's magnitudes and the latest samples. `dt` drives the
    /// temporal smoothing. No allocation.
    pub(crate) fn update(
        &mut self,
        magnitude: &[f32],
        wave: &[f32],
        dt: f32,
        frame: &mut AudioFrame,
    ) {
        for (db, m) in self.db.iter_mut().zip(magnitude) {
            *db = 8.685_89 * m.max(1e-9).ln();
        }

        // Juiced: tilt, spatial smoothing, then rise/fall over time.
        let len = self.juiced_state.len();
        for (i, span) in self.juiced.iter().enumerate() {
            let boost = self.tilt.get(i).copied().unwrap_or(0.0);
            if let Some(slot) = self.scratch.get_mut(i) {
                *slot = unit_db(span.read(&self.db) + boost, DB_RANGE);
            }
        }
        for i in 0..len {
            let at = |j: usize| self.scratch.get(j.min(len - 1)).copied().unwrap_or(0.0);
            let spread = 0.25 * at(i.saturating_sub(1)) + 0.5 * at(i) + 0.25 * at(i + 1);
            if let Some(state) = self.juiced_state.get_mut(i) {
                let tau = if spread > *state {
                    JUICED_ATTACK
                } else {
                    JUICED_RELEASE
                };
                *state += (spread - *state) * blend(dt, tau);
            }
        }

        // Smooth: two box-blur passes over the juiced curve.
        self.scratch.copy_from_slice(&self.juiced_state);
        box_blur(&self.scratch, &mut self.scratch_b, self.blur_radius);
        box_blur(&self.scratch_b, &mut self.scratch, self.blur_radius);

        let spectrum_len = frame.spectrum.len();
        for (i, texel) in frame.spectrum.iter_mut().enumerate() {
            let raw = self
                .raw
                .get(i)
                .map_or(0.0, |span| unit_db(span.read(&self.db), DB_RANGE));
            texel[0] = raw;
            texel[1] = self.juiced_state.get(i).copied().unwrap_or(0.0);
            texel[2] = self.scratch.get(i).copied().unwrap_or(0.0);
            texel[3] = (0.5 + 0.5 * wave_mean(wave, spectrum_len, i)).clamp(0.0, 1.0);
        }

        // Shadertoy: row 0 spectrum in the browser's dB window, row 1 wave.
        let width = self.toy_state.len();
        for (i, span) in self.toy.iter().enumerate() {
            let target = unit_db(
                span.read(&self.db) + SHADERTOY_DB_OFFSET,
                SHADERTOY_DB_RANGE,
            );
            if let Some(state) = self.toy_state.get_mut(i) {
                *state += (target - *state) * blend(dt, SHADERTOY_TAU);
            }
        }
        let (spectrum_row, wave_row) = frame.shadertoy.split_at_mut(width);
        for (byte, state) in spectrum_row.iter_mut().zip(&self.toy_state) {
            *byte = (state.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        for (i, byte) in wave_row.iter_mut().enumerate() {
            *byte = ((0.5 + 0.5 * wave_mean(wave, width, i)).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

/// A box blur of radius `radius` with clamped edges, `src` into `dst`.
fn box_blur(src: &[f32], dst: &mut [f32], radius: usize) {
    let n = src.len();
    for (i, out) in dst.iter_mut().enumerate().take(n) {
        let low = i.saturating_sub(radius);
        let high = (i + radius).min(n.saturating_sub(1));
        let mut sum = 0.0;
        let mut count = 0.0;
        for j in low..=high {
            sum += src.get(j).copied().unwrap_or(0.0);
            count += 1.0;
        }
        *out = sum / count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_log_texels_blend_and_wide_ones_take_the_peak() {
        let spans = Spectra::new(Layout::default(), 48_000.0, 1025);
        assert!(
            matches!(spans.juiced.first(), Some(Span::Blend { .. })),
            "30 Hz texels are narrower than a 23 Hz bin"
        );
        assert!(
            matches!(spans.juiced.last(), Some(Span::Peak { .. })),
            "16 kHz texels span many bins"
        );
    }

    #[test]
    fn spans_stay_inside_the_spectrum_at_any_layout() {
        for width in [Layout::MIN_LEN, 100, 256, 512, Layout::MAX_LEN] {
            let layout = Layout {
                spectrum_len: width,
                trail_len: 64,
                trail_seconds: 4.0,
                shadertoy_width: width,
            };
            let spans = Spectra::new(layout, 44_100.0, 1025);
            for span in spans.raw.iter().chain(&spans.juiced).chain(&spans.toy) {
                let ok = match *span {
                    Span::Peak { first, last } => first < last && last <= 1025,
                    Span::Blend { bin, t } => bin + 1 < 1025 && (0.0..=1.0).contains(&t),
                };
                assert!(ok, "span {span:?} escapes the spectrum at width {width}");
            }
        }
    }

    #[test]
    fn box_blur_preserves_a_constant() {
        let src = vec![0.4; 32];
        let mut dst = vec![0.0; 32];
        box_blur(&src, &mut dst, 3);
        assert!(
            dst.iter().all(|v| (v - 0.4).abs() < 1e-6),
            "blurring a constant changes nothing, edges included"
        );
    }
}
