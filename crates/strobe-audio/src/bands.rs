// ============================================================================
// Module:       strobe_audio::bands
// Description:  The four analysis bands, their FFT bin ranges for a given
//               sample rate, and per-band energy and spectral flux.
//
// Dependencies: std only
// ============================================================================

//! The four frequency bands every SSF audio family is split into.
//!
//! The edges are our choice (Synesthesia does not publish its own); they
//! follow the design plan: Bass 20-150 Hz, Mid 150 Hz-2 kHz, MidHigh
//! 2-6 kHz, High 6-16 kHz. A bin belongs to the band whose range contains
//! its centre frequency, so adjacent bands never share or drop a bin.
//!
//! Energy is reported as the band's RMS amplitude. Magnitudes arrive scaled
//! so a full-scale sine reads 1.0 at its peak bin; a Hann window spreads a
//! tone over about 1.5 bins of equivalent noise bandwidth, so the power
//! sum is divided by `2 * 1.5` to give the RMS a time-domain meter would
//! show. That keeps a band's number meaningful whatever the FFT size.

use std::ops::Range;

/// How many bands there are.
pub const BAND_COUNT: usize = 4;

/// One of the four analysis bands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    /// 20-150 Hz: kick drums, bass.
    Bass,
    /// 150 Hz-2 kHz: snares, vocals, most instruments.
    Mid,
    /// 2-6 kHz: presence, attack of snares, consonants.
    MidHigh,
    /// 6-16 kHz: hats, cymbals, air.
    High,
}

impl Band {
    /// Every band, lowest first. The position in this array is the band's
    /// index in all per-band storage.
    pub const ALL: [Band; BAND_COUNT] = [Band::Bass, Band::Mid, Band::MidHigh, Band::High];

    /// The band's lower and upper edge in Hz.
    pub const fn edges(self) -> (f32, f32) {
        match self {
            Band::Bass => (20.0, 150.0),
            Band::Mid => (150.0, 2_000.0),
            Band::MidHigh => (2_000.0, 6_000.0),
            Band::High => (6_000.0, 16_000.0),
        }
    }

    /// Position in [`Band::ALL`].
    pub const fn index(self) -> usize {
        match self {
            Band::Bass => 0,
            Band::Mid => 1,
            Band::MidHigh => 2,
            Band::High => 3,
        }
    }
}

/// The equivalent noise bandwidth of a Hann window, in bins.
const HANN_ENBW: f32 = 1.5;

/// Which FFT bins each band covers at one sample rate.
#[derive(Clone, Debug)]
pub struct BandLayout {
    /// Bin range per band, indexed by [`Band::index`].
    ranges: [Range<usize>; BAND_COUNT],
    /// Width of one bin in Hz.
    bin_hz: f32,
}

/// What one hop's spectrum says about each band.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BandReading {
    /// RMS amplitude, where a full-scale sine is about 0.707.
    pub rms: [f32; BAND_COUNT],
    /// Positive spectral flux: summed rise in magnitude since the last hop.
    pub flux: [f32; BAND_COUNT],
}

impl BandLayout {
    /// The layout for `fft_len`-point transforms of audio at `sample_rate`.
    pub fn new(sample_rate: f32, fft_len: usize) -> Self {
        let bin_hz = sample_rate / fft_len as f32;
        let top = fft_len / 2;
        let range_of = |band: Band| {
            let (low, high) = band.edges();
            let start = ((low / bin_hz).ceil() as usize).clamp(1, top.saturating_sub(1).max(1));
            let end = ((high / bin_hz).ceil() as usize).clamp(start + 1, top + 1);
            start..end
        };
        Self {
            ranges: Band::ALL.map(range_of),
            bin_hz,
        }
    }

    /// The bins `band` covers.
    pub fn range(&self, band: Band) -> Range<usize> {
        self.ranges
            .get(band.index())
            .cloned()
            .unwrap_or(Range { start: 1, end: 2 })
    }

    /// Width of one FFT bin in Hz.
    pub fn bin_hz(&self) -> f32 {
        self.bin_hz
    }

    /// Reads every band from this hop's magnitudes and the previous hop's.
    pub fn measure(&self, magnitude: &[f32], previous: &[f32]) -> BandReading {
        let mut reading = BandReading::default();
        for band in Band::ALL {
            let range = self.range(band);
            let (Some(now), Some(before)) = (magnitude.get(range.clone()), previous.get(range))
            else {
                continue;
            };
            let mut power = 0.0_f32;
            let mut flux = 0.0_f32;
            for (m, p) in now.iter().zip(before) {
                power += m * m;
                flux += (m - p).max(0.0);
            }
            let slot = band.index();
            if let (Some(rms), Some(f)) = (reading.rms.get_mut(slot), reading.flux.get_mut(slot)) {
                *rms = (power / (2.0 * HANN_ENBW)).sqrt();
                *f = flux;
            }
        }
        reading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_tile_the_spectrum_without_gaps_or_overlap() {
        for rate in [22_050.0, 44_100.0, 48_000.0, 96_000.0] {
            let layout = BandLayout::new(rate, 2048);
            let mut previous_end = layout.range(Band::Bass).end;
            for band in [Band::Mid, Band::MidHigh, Band::High] {
                let range = layout.range(band);
                assert_eq!(
                    range.start, previous_end,
                    "{band:?} should start where the band below ends at {rate} Hz"
                );
                assert!(range.end > range.start, "{band:?} has at least one bin");
                previous_end = range.end;
            }
        }
    }

    #[test]
    fn bass_covers_the_kick_region_at_48k() {
        let layout = BandLayout::new(48_000.0, 2048);
        let range = layout.range(Band::Bass);
        assert_eq!(range, 1..7, "bins 1 to 6 are 23 Hz to 141 Hz");
    }

    #[test]
    fn a_sine_reads_its_rms_in_its_band() {
        // A full-scale sine leaks over about three bins of a Hann window.
        let layout = BandLayout::new(48_000.0, 2048);
        let mut mags = vec![0.0; 1025];
        for (k, value) in [(2, 0.5), (3, 1.0), (4, 0.5)] {
            if let Some(slot) = mags.get_mut(k) {
                *slot = value;
            }
        }
        let reading = layout.measure(&mags, &vec![0.0; 1025]);
        let rms = reading.rms.first().copied().unwrap_or(0.0);
        assert!(
            (rms - 0.707).abs() < 0.1,
            "a full-scale sine should read near 0.707, got {rms}"
        );
    }

    #[test]
    fn flux_counts_only_rises() {
        let layout = BandLayout::new(48_000.0, 2048);
        let mut now = vec![0.0; 1025];
        let mut before = vec![0.0; 1025];
        if let (Some(a), Some(b)) = (now.get_mut(2), before.get_mut(3)) {
            *a = 0.4;
            *b = 0.9;
        }
        let reading = layout.measure(&now, &before);
        let flux = reading.flux.first().copied().unwrap_or(-1.0);
        assert!(
            (flux - 0.4).abs() < 1e-6,
            "rise of 0.4, fall ignored: {flux}"
        );
    }
}
