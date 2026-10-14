// ============================================================================
// Module:       strobe_audio::frame
// Description:  The analyser's output: one AudioFrame holding every SSF scalar
//               and the data behind the spectrum, level-trail and Shadertoy
//               textures, plus the Layout that sizes them.
//
// Dependencies: strobe-scene (the audio uniform names and silent values)
// ============================================================================

//! What the analyser hands to the renderer.
//!
//! An [`AudioFrame`] is plain data: the render thread copies or uploads it
//! and never touches analyser internals. Its textures are sized by a
//! [`Layout`], not by constants baked into the shapes. The exact texel
//! layout real Synesthesia scenes expect is still unverified (see
//! `docs/OPEN_QUESTIONS.md`), so texture width is a setting and the
//! meaning of a texel's channel is documented here and nowhere else.

use strobe_scene::audio::{SCALARS, silent_value};

/// How wide each texture is, and how much time the level trail covers.
///
/// The spectrum's default is Synesthesia's 1024 texels, one per FFT bin
/// below Nyquist (D269); the others are the widths the design plan names.
/// Nothing downstream assumes them: a frame's vectors carry their own
/// lengths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Texels in `syn_Spectrum`.
    pub spectrum_len: usize,
    /// Texels in `syn_LevelTrail`.
    pub trail_len: usize,
    /// Seconds of history the whole level trail spans.
    pub trail_seconds: f32,
    /// Columns of the Shadertoy audio texture (two rows: spectrum, wave).
    pub shadertoy_width: usize,
}

impl Layout {
    /// The narrowest texture accepted. Below this the smoothing kernels
    /// have nothing to work with.
    pub const MIN_LEN: usize = 8;
    /// The widest texture accepted: the FFT has 1024 distinct bins, so
    /// more texels than that only repeat data.
    pub const MAX_LEN: usize = 2048;

    /// This layout with every size forced into the accepted range.
    pub fn clamped(self) -> Self {
        let clamp = |n: usize| n.clamp(Self::MIN_LEN, Self::MAX_LEN);
        let seconds = if self.trail_seconds.is_finite() {
            self.trail_seconds.clamp(0.5, 60.0)
        } else {
            Self::default().trail_seconds
        };
        Self {
            spectrum_len: clamp(self.spectrum_len),
            trail_len: clamp(self.trail_len),
            trail_seconds: seconds,
            shadertoy_width: clamp(self.shadertoy_width),
        }
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            spectrum_len: 1024,
            trail_len: 256,
            trail_seconds: 4.0,
            shadertoy_width: 512,
        }
    }
}

/// One snapshot of the analysis, ready for the shader.
///
/// All storage is allocated once, in [`AudioFrame::silent`]. The analyser
/// overwrites it in place, so a frame's vectors never grow or move.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioFrame {
    /// One value per name in [`SCALARS`], in the same order.
    pub scalars: Vec<f32>,
    /// `syn_Spectrum`, one `[r, g, b, a]` per texel, `layout.spectrum_len`
    /// long, all 0..1.
    ///
    /// - `r` raw FFT: linear frequency, 0 Hz to Nyquist, peak per texel,
    ///   fixed decibel scale (-100 dB to -20 dB mapped to 0..1).
    /// - `g` juiced: log frequency from 30 Hz to 16 kHz, high-boosted
    ///   3 dB per octave above 250 Hz, smoothed across texels and in time.
    /// - `b` smooth: the juiced curve blurred to a few broad humps.
    /// - `a` waveform: the latest samples, 0.5 at zero, 0 and 1 at full
    ///   scale.
    pub spectrum: Vec<[f32; 4]>,
    /// `syn_LevelTrail`, `layout.trail_len` long. Index 0 is now, higher
    /// indices are older. Channels: whole, bass, mid (mean of Mid and
    /// MidHigh), high.
    pub level_trail: Vec<[f32; 4]>,
    /// The Shadertoy audio texture as `2 * shadertoy_width` bytes of R8:
    /// row 0 is the spectrum in the browser analyser's decibel mapping,
    /// row 1 is the waveform with 128 at zero.
    pub shadertoy: Vec<u8>,
    /// The latest hop's RMS of the whole signal and of the bass band,
    /// **not** normalised to a recent peak, 0 when no audio arrives. The
    /// scalars adapt to the music within seconds, which would lift a long
    /// breakdown back up; song-structure events compare these raw values
    /// with their own averages instead (`events`, D226). Not a uniform.
    pub energy: [f32; 2],
    /// The downbeat: `[offset, confidence]`, where a bar starts when the
    /// beat counter `syn_BeatTime` reaches `4n + offset`, and confidence is
    /// 0 to 1. Confidence 0 means no estimate; bars are then counted from
    /// the counter's zero (`downbeat`, D233). Not a uniform.
    pub downbeat: [f32; 2],
}

impl AudioFrame {
    /// A frame at silence, sized by `layout`.
    pub fn silent(layout: Layout) -> Self {
        let layout = layout.clamped();
        Self {
            scalars: SCALARS.iter().map(|name| silent_value(name)).collect(),
            spectrum: vec![[0.0, 0.0, 0.0, 0.5]; layout.spectrum_len],
            level_trail: vec![[0.0; 4]; layout.trail_len],
            shadertoy: [
                vec![0; layout.shadertoy_width],
                vec![128; layout.shadertoy_width],
            ]
            .concat(),
            energy: [0.0; 2],
            downbeat: [0.0; 2],
        }
    }

    /// The value of the scalar called `name`, if there is one.
    pub fn scalar(&self, name: &str) -> Option<f32> {
        let index = SCALARS.iter().position(|candidate| *candidate == name)?;
        self.scalars.get(index).copied()
    }

    /// Columns of the Shadertoy texture (half its length: two rows).
    pub fn shadertoy_width(&self) -> usize {
        self.shadertoy.len() / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_frame_holds_the_scene_crates_silent_values() {
        let frame = AudioFrame::silent(Layout::default());
        assert_eq!(frame.scalars.len(), SCALARS.len(), "one slot per scalar");
        assert_eq!(
            frame.scalar("syn_BPM"),
            Some(120.0),
            "silence reads 120 BPM"
        );
        assert_eq!(frame.scalar("syn_Level"), Some(0.0), "and zero level");
        assert_eq!(frame.scalar("syn_Nope"), None, "unknown names are None");
    }

    #[test]
    fn layout_is_clamped_into_range() {
        let layout = Layout {
            spectrum_len: 1,
            trail_len: 1_000_000,
            trail_seconds: f32::NAN,
            shadertoy_width: 512,
        }
        .clamped();
        assert_eq!(layout.spectrum_len, Layout::MIN_LEN, "too small is raised");
        assert_eq!(layout.trail_len, Layout::MAX_LEN, "too large is lowered");
        assert!(layout.trail_seconds.is_finite(), "NaN seconds are replaced");
    }

    #[test]
    fn frame_sizes_follow_the_layout() {
        let frame = AudioFrame::silent(Layout {
            spectrum_len: 64,
            trail_len: 32,
            trail_seconds: 2.0,
            shadertoy_width: 128,
        });
        assert_eq!(frame.spectrum.len(), 64, "spectrum width is configurable");
        assert_eq!(frame.level_trail.len(), 32, "trail width is configurable");
        assert_eq!(frame.shadertoy.len(), 256, "two rows of 128");
        assert_eq!(frame.shadertoy_width(), 128, "width reads back");
    }
}
