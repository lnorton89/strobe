// ============================================================================
// Module:       strobe_playlist::rng
// Description:  SplitMix64, the small seedable generator behind random
//               durations and the playlist generator.
//
// Dependencies: None.
// ============================================================================

//! The playlist crate's random numbers.
//!
//! A generated section has to be the same section tomorrow, on another
//! machine, after a dependency bump: the UI previews it, the owner saves
//! it, and a seed written in a playlist file is a promise. A general
//! purpose crate (`rand`) reserves the right to change its streams between
//! versions, so this crate carries its own generator: **SplitMix64**
//! (Steele, Lea & Flood, "Fast splittable pseudorandom number generators",
//! 2014), the generator Java's `SplittableRandom` and the xoshiro authors'
//! seeding routine use.
//!
//! It is eight lines of arithmetic on one `u64`, passes BigCrush, and any
//! seed (including 0) is valid. It is not cryptographic, and does not need
//! to be: its only job is to make a playlist varied.
//!
//! The algorithm is pinned by golden tests below (the first outputs for
//! seed 0 are the published reference values). Changing a constant or the
//! order of operations changes every saved seed's meaning, so the golden
//! tests exist to make that a loud, deliberate act.

/// The Weyl-sequence increment: 2^64 divided by the golden ratio, odd.
const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// First multiplier of the output mix.
const MIX_1: u64 = 0xBF58_476D_1CE4_E5B9;

/// Second multiplier of the output mix.
const MIX_2: u64 = 0x94D0_49BB_1331_11EB;

/// A SplitMix64 stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A stream starting from `seed`. Every seed is valid.
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// An independent stream derived from a seed and a salt.
    ///
    /// The salt (a cue's position, say) is mixed through one SplitMix64
    /// round with the seed, so neighbouring salts give unrelated streams.
    /// The timeline uses this so a cue's random duration depends on its
    /// position and the seed only, never on how many other cues drew
    /// numbers before it.
    pub fn derive(seed: u64, salt: u64) -> Self {
        let mut mixer = Self::new(seed ^ salt.wrapping_mul(GAMMA));
        Self::new(mixer.next_u64())
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(MIX_1);
        z = (z ^ (z >> 27)).wrapping_mul(MIX_2);
        z ^ (z >> 31)
    }

    /// A uniform integer in `0..bound` (0 when `bound` is 0).
    ///
    /// Lemire's multiply-and-reject method, so there is no modulo bias.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        let mut product = u128::from(self.next_u64()) * u128::from(bound);
        let mut low = product as u64;
        if low < bound {
            let threshold = bound.wrapping_neg() % bound;
            while low < threshold {
                product = u128::from(self.next_u64()) * u128::from(bound);
                low = product as u64;
            }
        }
        (product >> 64) as u64
    }

    /// A uniform integer in `lo..=hi` (`lo` when the range is empty).
    pub fn between(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        match (hi - lo).checked_add(1) {
            Some(span) => lo + self.below(span),
            // The full u64 range: every value is equally likely already.
            None => self.next_u64(),
        }
    }

    /// A uniform float in `[0, 1)` with 53 bits of resolution.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// A uniform float in `[lo, hi)`; `lo` when the range is empty.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        if hi <= lo {
            return lo;
        }
        lo + self.unit() * (hi - lo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn seed_zero_matches_the_published_reference_stream() {
        let mut rng = SplitMix64::new(0);
        let got = [
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
        ];
        assert_eq!(
            got,
            [
                0xE220_A839_7B1D_CDAF,
                0x6E78_9E6A_A1B9_65F4,
                0x06C4_5D18_8009_454F,
                0xF88B_B8A8_724C_81EC,
            ],
            "SplitMix64 no longer matches its reference output; every saved seed changes meaning"
        );
    }

    #[test]
    fn a_seed_always_gives_the_same_stream() {
        let mut a = SplitMix64::new(1337);
        let mut b = SplitMix64::new(1337);
        for step in 0..1000 {
            assert_eq!(
                a.next_u64(),
                b.next_u64(),
                "streams diverged at step {step}"
            );
        }
    }

    #[test]
    fn derived_streams_differ_by_salt_and_repeat_by_inputs() {
        let first = SplitMix64::derive(7, 0).next_u64();
        let second = SplitMix64::derive(7, 1).next_u64();
        assert_ne!(first, second, "neighbouring salts gave the same stream");
        assert_eq!(
            first,
            SplitMix64::derive(7, 0).next_u64(),
            "derive is not a pure function of its inputs"
        );
    }

    #[test]
    fn below_stays_in_range_and_covers_it() -> Result<()> {
        let mut rng = SplitMix64::new(42);
        let mut seen = [false; 7];
        for _ in 0..500 {
            let value = rng.below(7);
            assert!(value < 7, "below(7) returned {value}");
            if let Some(slot) = seen.get_mut(value as usize) {
                *slot = true;
            }
        }
        assert!(
            seen.iter().all(|hit| *hit),
            "some value in 0..7 never appeared"
        );
        assert_eq!(rng.below(0), 0, "below(0) should be 0");
        assert_eq!(rng.below(1), 0, "below(1) should be 0");
        Ok(())
    }

    #[test]
    fn between_is_inclusive_at_both_ends() {
        let mut rng = SplitMix64::new(9);
        let mut lo = false;
        let mut hi = false;
        for _ in 0..200 {
            let value = rng.between(3, 5);
            assert!((3..=5).contains(&value), "between(3, 5) returned {value}");
            lo |= value == 3;
            hi |= value == 5;
        }
        assert!(lo && hi, "both ends of 3..=5 should appear");
        assert_eq!(rng.between(5, 3), 5, "an empty range returns its low end");
    }

    #[test]
    fn unit_floats_stay_below_one() {
        let mut rng = SplitMix64::new(5);
        for _ in 0..1000 {
            let value = rng.unit();
            assert!((0.0..1.0).contains(&value), "unit() returned {value}");
        }
        assert_eq!(
            rng.range(2.0, 2.0),
            2.0,
            "an empty float range returns its low end"
        );
    }
}
