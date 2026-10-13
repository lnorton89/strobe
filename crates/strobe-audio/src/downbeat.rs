// ============================================================================
// Module:       strobe_audio::downbeat
// Description:  Which beat of the grid starts a bar: per-beat accent and
//               spectral change folded into four slots, with a confidence
//               and hysteresis, and no answer at all when unsure.
//
// Dependencies: std only
// ============================================================================

//! The downbeat estimate (backlog P-02, D233).
//!
//! The beat grid (`tempo::BeatGrid`) knows where beats fall but not which
//! of them is "one", so a bar used to be counted from wherever the grid
//! began (D75). This module listens to each beat of the grid and gives
//! every beat a score from two cues that tend to mark the start of a bar
//! in four-on-the-floor and most popular music:
//!
//! - **accent:** the mean low-end energy over the beat (the half beat
//!   either side of it), so a harder kick or a sub drop on "one" counts.
//!   Not the peak, and not a narrow window around the beat: a kick's peak
//!   depends on where the analysis hops fall against it, and the grid
//!   wobbles by a fraction of a beat, so both vary from beat to beat and
//!   read as accents on even kicks. The mean over the beat moves with
//!   neither;
//! - **change:** how far the beat's band profile (the mean level of each
//!   band over the beat) *rose* from the previous beat's, in log terms, so
//!   a chord or bass-note change counts however loud it is. Only rises
//!   count, as in spectral flux: an accented beat is followed by a fall,
//!   and counting falls would credit the beat after the downbeat equally.
//!   A band's rise counts only past [`CHANGE_DEAD_ZONE`], and the change's
//!   running mean is floored at [`CHANGE_FLOOR`]: on steady music the
//!   true change is nearly zero, and dividing the wobble of where beat
//!   spans fall by a mean of nearly zero made it the loudest thing heard.
//!
//! Each is divided by its own running mean, so both average about one and
//! neither drowns the other, and their sum is added to the slot
//! `beat index mod 4`. Slots decay once a bar with a time constant of
//! [`TAU_BEATS`], so the estimate follows a track change within a few
//! bars, and the slots are compared only when all four have had the same
//! bar: decaying per beat, or comparing mid-bar, favours whichever slot
//! was scored last, enough to call a one-and-three tie for one of them.
//!
//! **Confidence is the lead.** The slot with the most evidence is the
//! downbeat; confidence is how far it leads the runner-up, past
//! [`NOISE_LEAD`] and as a fraction of [`FULL_LEAD`], and it is held at zero until [`WARMUP_BEATS`] have
//! taught the means and [`MIN_BEATS`] more have been scored. A kick on every beat or on one and three ties two or four
//! slots and gives a confidence near zero, which is the honest answer: the
//! caller then counts bars as before. Nothing here is a time signature;
//! bars are four beats, as everywhere else (D75).
//!
//! **Hysteresis.** Once reported, the downbeat moves only when another
//! slot has led for [`SWITCH_BARS`] bars in a row, so one loud fill
//! cannot shift every bar line. [`Downbeat::reset`] forgets everything; the
//! analyser calls it when the grid loses its lock, since the grid's beat
//! numbers no longer mean the same beats after it re-aligns.

/// Beats in a bar.
pub const BEATS_PER_BAR: usize = 4;

/// Bands in the profile (the analyser's four, bass first).
pub const BANDS: usize = 4;

/// Time constant of the slots' memory, in beats (eight bars).
pub const TAU_BEATS: f32 = 32.0;

/// Beats that only teach the features' means, before any slot is scored:
/// a score against a mean of one or two beats is noise.
pub const WARMUP_BEATS: u32 = 8;

/// Beats scored before any estimate (four bars), after the warm-up.
pub const MIN_BEATS: u32 = 16;

/// The lead over the runner-up, as a fraction of the leader, that counts
/// as full confidence.
pub const FULL_LEAD: f32 = 0.4;

/// A lead this small is noise, not a downbeat: even kicks at a tempo whose
/// beat does not divide the analysis hop wobble by up to about 7 %.
pub const NOISE_LEAD: f32 = 0.12;

/// Bars a challenger must lead in a row before the downbeat moves.
pub const SWITCH_BARS: u32 = 2;

/// Time constant of the features' running means, in beats.
const MEAN_TAU_BEATS: f32 = 16.0;

/// A band's log rise from one beat to the next that is still noise
/// (about 22 %): beat spans differ by a hop and catch more or less of a
/// kick's tail.
pub const CHANGE_DEAD_ZONE: f32 = 0.2;

/// The least the change's running mean is taken to be, so a steady track's
/// near-zero change is not inflated into a cue.
pub const CHANGE_FLOOR: f32 = 0.3;

/// Keeps logs and divisions finite in silence.
const FLOOR: f32 = 1e-4;

/// A downbeat estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// The grid beat, modulo four, that starts a bar: bar `n` starts at
    /// grid beat `4n + offset`.
    pub offset: u8,
    /// 0 to 1: how far that beat leads the next best.
    pub confidence: f32,
}

/// The estimator. Feed it once per analysis step with [`Downbeat::observe`].
#[derive(Clone, Debug, Default)]
pub struct Downbeat {
    /// The grid beat being collected (nearest whole beat).
    current: Option<i64>,
    /// The accent summed over the current beat.
    accent: f32,
    /// Band levels summed over the current beat, and how many steps.
    sum: [f32; BANDS],
    steps: u32,
    /// The previous beat's band profile.
    previous: Option<[f32; BANDS]>,
    /// Running means of the two features.
    accent_mean: f32,
    change_mean: f32,
    /// Evidence per slot.
    slots: [f32; BEATS_PER_BAR],
    /// Whole beats heard since the last reset.
    seen: u32,
    /// The reported downbeat slot, and its confidence when last compared.
    reported: Option<u8>,
    confidence: f32,
    /// A slot leading the reported one, and for how many bars.
    challenger: Option<(u8, u32)>,
}

impl Downbeat {
    /// A fresh estimator.
    pub fn new() -> Self {
        Self::default()
    }

    /// One analysis step: the grid position in beats, the low-end accent
    /// energy heard in this step, and the four band levels.
    pub fn observe(&mut self, beats: f64, accent: f32, bands: [f32; BANDS]) {
        if !beats.is_finite() || !accent.is_finite() || bands.iter().any(|b| !b.is_finite()) {
            return;
        }
        let nearest = beats.round();
        let index = nearest as i64;
        match self.current {
            Some(current) if current == index => {}
            Some(current) => {
                if index == current + 1 {
                    self.close(current);
                } else {
                    // The grid jumped (a snap, or a long stall): the beat
                    // being collected is not a whole beat; drop it.
                    self.previous = None;
                }
                self.start(index);
            }
            None => self.start(index),
        }
        self.accent += accent.max(0.0);
        for (sum, band) in self.sum.iter_mut().zip(bands) {
            *sum += band.max(0.0);
        }
        self.steps += 1;
    }

    /// The downbeat, once there is one.
    pub fn estimate(&self) -> Option<Bar> {
        let offset = self.reported?;
        Some(Bar {
            offset,
            confidence: self.confidence,
        })
    }

    /// Forgets everything (the grid lost its lock).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn start(&mut self, index: i64) {
        self.current = Some(index);
        self.accent = 0.0;
        self.sum = [0.0; BANDS];
        self.steps = 0;
    }

    /// Scores the finished beat `index` and updates the estimate.
    fn close(&mut self, index: i64) {
        if self.steps == 0 {
            return;
        }
        let steps = self.steps as f32;
        let accent = self.accent / steps;
        let profile = self.sum.map(|sum| sum / steps);
        let change = self.previous.map_or(0.0, |previous| {
            previous
                .iter()
                .zip(profile)
                .map(|(before, now)| {
                    (((now + FLOOR) / (before + FLOOR)).ln() - CHANGE_DEAD_ZONE).max(0.0)
                })
                .sum::<f32>()
        });
        self.previous = Some(profile);

        // A plain average while warming up, then a running one.
        let blend = if self.seen < WARMUP_BEATS {
            1.0 / (self.seen as f32 + 1.0)
        } else {
            1.0 - (-1.0 / MEAN_TAU_BEATS).exp()
        };
        self.accent_mean += (accent - self.accent_mean) * blend;
        self.change_mean += (change - self.change_mean) * blend;
        self.seen = self.seen.saturating_add(1);
        if self.seen <= WARMUP_BEATS {
            return;
        }
        let score =
            accent / self.accent_mean.max(FLOOR) + change / self.change_mean.max(CHANGE_FLOOR);

        let slot = index.rem_euclid(BEATS_PER_BAR as i64) as usize;
        if slot == 0 {
            let keep = (-(BEATS_PER_BAR as f32) / TAU_BEATS).exp();
            for evidence in &mut self.slots {
                *evidence *= keep;
            }
        }
        if let Some(evidence) = self.slots.get_mut(slot) {
            *evidence += score;
        }
        if slot == BEATS_PER_BAR - 1 {
            self.decide();
        }
    }

    /// Moves the reported downbeat, with hysteresis.
    fn decide(&mut self) {
        if self.seen < WARMUP_BEATS + MIN_BEATS {
            return;
        }
        let Some(best) = self.leader() else {
            return;
        };
        match self.reported {
            None => {
                if self.confidence_of(best) > 0.0 {
                    self.reported = Some(best);
                }
            }
            Some(reported) if reported == best => self.challenger = None,
            Some(_) => {
                let count = match self.challenger {
                    Some((slot, count)) if slot == best => count + 1,
                    Some(_) | None => 1,
                };
                if count >= SWITCH_BARS {
                    self.reported = Some(best);
                    self.challenger = None;
                } else {
                    self.challenger = Some((best, count));
                }
            }
        }
        self.confidence = self.reported.map_or(0.0, |slot| self.confidence_of(slot));
    }

    /// The slot with the most evidence.
    fn leader(&self) -> Option<u8> {
        let mut best: Option<(usize, f32)> = None;
        for (slot, &evidence) in self.slots.iter().enumerate() {
            if best.is_none_or(|(_, most)| evidence > most) {
                best = Some((slot, evidence));
            }
        }
        best.and_then(|(slot, _)| u8::try_from(slot).ok())
    }

    /// How far `slot` leads every other slot, 0 to 1.
    fn confidence_of(&self, slot: u8) -> f32 {
        if self.seen < WARMUP_BEATS + MIN_BEATS {
            return 0.0;
        }
        let slot = usize::from(slot);
        let Some(&own) = self.slots.get(slot) else {
            return 0.0;
        };
        let rival = self
            .slots
            .iter()
            .enumerate()
            .filter(|&(other, _)| other != slot)
            .map(|(_, &evidence)| evidence)
            .fold(0.0_f32, f32::max);
        if own <= FLOOR {
            return 0.0;
        }
        let lead = (own - rival) / own;
        ((lead - NOISE_LEAD) / (FULL_LEAD - NOISE_LEAD)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps per beat: about 94 Hz analysis at 120 BPM.
    const STEPS: usize = 47;

    /// Feeds `beats` whole beats starting at grid beat `first`, with
    /// `accent(beat)` for the quarter beat after the beat and `bands(beat)`
    /// held over it.
    fn feed(
        downbeat: &mut Downbeat,
        first: i64,
        beats: i64,
        accent: impl Fn(i64) -> f32,
        bands: impl Fn(i64) -> [f32; BANDS],
    ) {
        for beat in first..first + beats {
            for step in 0..STEPS {
                // Steps from half a beat before the beat to half after, as
                // the nearest-beat rule collects them.
                let position = beat as f64 - 0.5 + (step as f64 + 0.5) / STEPS as f64;
                // A kick's energy lasts about a quarter of a beat.
                let on = (0.0..0.25).contains(&(position - beat as f64));
                downbeat.observe(position, if on { accent(beat) } else { 0.02 }, bands(beat));
            }
        }
    }

    fn flat(_: i64) -> [f32; BANDS] {
        [0.5, 0.4, 0.3, 0.2]
    }

    #[test]
    fn a_heavier_hit_on_one_is_found_at_any_grid_offset() {
        for offset in 0..4_i64 {
            let mut downbeat = Downbeat::new();
            // The bar starts on grid beats 4n + offset.
            let accent = |beat: i64| {
                if (beat - offset).rem_euclid(4) == 0 {
                    1.0
                } else {
                    0.6
                }
            };
            feed(&mut downbeat, 3, 64, accent, flat);
            let bar = downbeat.estimate();
            assert_eq!(
                bar.map(|bar| bar.offset),
                Some(offset as u8),
                "offset {offset}: {bar:?}"
            );
            assert!(
                bar.is_some_and(|bar| bar.confidence > 0.5),
                "and sure of it: {bar:?}"
            );
        }
    }

    #[test]
    fn a_change_of_chord_each_bar_marks_the_downbeat_without_any_accent() {
        let mut downbeat = Downbeat::new();
        // Even kicks; the harmony moves every four beats starting at beat 2.
        let bands = |beat: i64| {
            if (beat - 2).div_euclid(4) % 2 == 0 {
                [0.5, 0.6, 0.2, 0.2]
            } else {
                [0.5, 0.2, 0.6, 0.2]
            }
        };
        feed(&mut downbeat, 0, 64, |_| 0.8, bands);
        assert_eq!(
            downbeat.estimate().map(|bar| bar.offset),
            Some(2),
            "{:?}",
            downbeat.estimate()
        );
    }

    #[test]
    fn even_kicks_give_no_confident_downbeat() {
        let mut downbeat = Downbeat::new();
        feed(&mut downbeat, 0, 64, |_| 0.8, flat);
        assert!(
            downbeat.estimate().is_none_or(|bar| bar.confidence < 0.1),
            "nothing to tell the beats apart: {:?}",
            downbeat.estimate()
        );
        let mut one_and_three = Downbeat::new();
        feed(
            &mut one_and_three,
            0,
            64,
            |beat| if beat % 2 == 0 { 1.0 } else { 0.4 },
            flat,
        );
        assert!(
            one_and_three
                .estimate()
                .is_none_or(|bar| bar.confidence < 0.1),
            "one and three tie: {:?}",
            one_and_three.estimate()
        );
    }

    #[test]
    fn nothing_is_said_before_four_bars_and_a_reset_forgets() {
        let mut downbeat = Downbeat::new();
        let accent = |beat: i64| if beat % 4 == 1 { 1.0 } else { 0.5 };
        let early = i64::from(WARMUP_BEATS + MIN_BEATS) - 2;
        feed(&mut downbeat, 0, early, accent, flat);
        assert_eq!(downbeat.estimate(), None, "too early");
        feed(&mut downbeat, early, 16, accent, flat);
        assert_eq!(
            downbeat.estimate().map(|bar| bar.offset),
            Some(1),
            "then found"
        );
        downbeat.reset();
        assert_eq!(downbeat.estimate(), None, "forgotten");
    }

    #[test]
    fn one_loud_fill_does_not_move_the_bar_but_a_new_track_does() {
        let mut downbeat = Downbeat::new();
        let on = |offset: i64| {
            move |beat: i64| {
                if (beat - offset).rem_euclid(4) == 0 {
                    1.0
                } else {
                    0.5
                }
            }
        };
        feed(&mut downbeat, 0, 64, on(0), flat);
        assert_eq!(downbeat.estimate().map(|bar| bar.offset), Some(0), "found");
        // A two-beat fill hammering beat 3.
        feed(
            &mut downbeat,
            64,
            4,
            |beat| if beat % 4 == 3 { 3.0 } else { 0.5 },
            flat,
        );
        feed(&mut downbeat, 68, 4, on(0), flat);
        assert_eq!(
            downbeat.estimate().map(|bar| bar.offset),
            Some(0),
            "a fill is not a new bar line"
        );
        // A new track whose bars start a beat later, for sixteen bars.
        feed(&mut downbeat, 72, 64, on(1), flat);
        assert_eq!(
            downbeat.estimate().map(|bar| bar.offset),
            Some(1),
            "followed: {:?}",
            downbeat.estimate()
        );
    }

    #[test]
    fn a_jump_in_the_grid_and_bad_input_are_survived() {
        let mut downbeat = Downbeat::new();
        let accent = |beat: i64| if beat % 4 == 0 { 1.0 } else { 0.5 };
        feed(&mut downbeat, 0, 32, accent, flat);
        downbeat.observe(f64::NAN, 1.0, flat(0));
        downbeat.observe(40.0, f32::INFINITY, flat(0));
        // A snap of several beats: the partial beat is dropped.
        feed(&mut downbeat, 44, 32, accent, flat);
        assert_eq!(
            downbeat.estimate().map(|bar| bar.offset),
            Some(0),
            "{:?}",
            downbeat.estimate()
        );
    }
}
