// ============================================================================
// Module:       strobe_audio::capture::watchdog
// Description:  Notices an input stream that has stopped calling back
//               without reporting an error, and paces the reopens that
//               follow. Pure: fed with timestamps, so it is unit-tested.
//
// Dependencies: std (time)
// ============================================================================

//! The silent-stall watchdog (D420).
//!
//! A cpal stream reports a lost device through its error callback, which the
//! worker acts on. Some losses never reach that callback: on Windows WASAPI
//! an unplugged USB microphone, or one that went to sleep, can leave the
//! stream "playing" with no more data callbacks and no error. The stream
//! then looked healthy for the rest of the show while the analyser decayed to
//! silence.
//!
//! The worker tells [`Watchdog::observe`] on every pass whether samples
//! arrived since the last one. **Silence is not a stall:** a working device
//! in a quiet room still delivers buffers, of zeros, so its ring fills and
//! it counts as heard. Only a stream that delivers nothing at all for
//! [`STALL_AFTER`] is stalled. A loopback stream on Windows is never
//! watched: WASAPI delivers no packets to a loopback client while nothing is
//! playing, so there, no callbacks *is* silence.
//!
//! On a stall the worker drops the stream and opens the device again after
//! [`stall_delay`]: at once the first time, then after a pause that doubles
//! up to [`STALL_RETRY_MAX`], so a device that keeps opening and then
//! staying mute costs one open every few seconds at most. The count of
//! stalls in a row is cleared by the first samples that arrive.
//!
//! Everything here is a few `Instant` comparisons: no allocation, no lock.

use std::time::{Duration, Instant};

/// How long a stream may deliver nothing before it is treated as stalled.
/// A device delivers a buffer every 10 to 20 ms (WASAPI's shared-mode
/// period is 10 ms), and a freshly opened one within a few hundred ms, so
/// three seconds is far beyond any healthy gap and still well inside "a few
/// seconds" for an operator looking at a dead meter.
pub(crate) const STALL_AFTER: Duration = Duration::from_secs(3);

/// The pause before the second reopen of a device that stalled twice in a
/// row; it doubles for each further stall.
const STALL_RETRY_FIRST: Duration = Duration::from_secs(2);

/// The longest pause between reopens of a device that keeps stalling.
pub(crate) const STALL_RETRY_MAX: Duration = Duration::from_secs(16);

/// How long to wait before reopening a device after `stalls` stalls in a
/// row: none after the first (the reopen usually cures it), then
/// [`STALL_RETRY_FIRST`] doubling up to [`STALL_RETRY_MAX`].
pub(crate) fn stall_delay(stalls: u32) -> Duration {
    if stalls <= 1 {
        return Duration::ZERO;
    }
    let doublings = (stalls - 2).min(8);
    STALL_RETRY_FIRST
        .saturating_mul(1_u32 << doublings)
        .min(STALL_RETRY_MAX)
}

/// What one observation means for the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Watch {
    /// Samples are arriving, or the stream has not been quiet for long.
    Fine,
    /// The first samples after one or more stalls: the input is back.
    Recovered,
    /// Nothing has arrived for [`STALL_AFTER`]: drop the stream and reopen
    /// the device after `delay`. `stalls` counts this one.
    Stalled {
        /// Stalls in a row, with no samples between, including this one.
        stalls: u32,
        /// How long to wait before the reopen.
        delay: Duration,
    },
}

/// The watchdog's state across streams: when samples last arrived on the
/// stream being watched, and how many streams in a row stalled.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Watchdog {
    /// When samples last arrived, or when the stream opened.
    heard: Instant,
    /// Stalls in a row with no samples between.
    stalls: u32,
    /// Whether the stream being watched has not been reported yet: a stall
    /// is reported once per stream, and [`Self::opened`] arms the next one.
    armed: bool,
}

impl Watchdog {
    /// A watchdog that has seen no stall.
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            heard: now,
            stalls: 0,
            armed: true,
        }
    }

    /// A stream has just opened: it has [`STALL_AFTER`] from `now` to
    /// deliver its first samples. The stall count is kept, so a device that
    /// opens and stays mute again backs off further.
    pub(crate) fn opened(&mut self, now: Instant) {
        self.heard = now;
        self.armed = true;
    }

    /// Whether the last stream stalled and nothing has been heard since.
    /// The worker keeps the state at "no input" through a reopen while this
    /// holds, so the UI does not flash "live" for a stream that may stay
    /// mute.
    pub(crate) fn stalled(&self) -> bool {
        self.stalls > 0
    }

    /// One pass of the worker at `now`: `samples` says whether any arrived
    /// since the last pass.
    pub(crate) fn observe(&mut self, now: Instant, samples: bool) -> Watch {
        if samples {
            self.heard = now;
            if self.stalls > 0 {
                self.stalls = 0;
                return Watch::Recovered;
            }
            return Watch::Fine;
        }
        if !self.armed || now.saturating_duration_since(self.heard) < STALL_AFTER {
            return Watch::Fine;
        }
        self.stalls = self.stalls.saturating_add(1);
        self.armed = false;
        Watch::Stalled {
            stalls: self.stalls,
            delay: stall_delay(self.stalls),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// Ten-millisecond passes from `start` to `start + span`, the worker's
    /// rate, each reporting `samples`, collecting everything but `Fine`.
    fn run(
        dog: &mut Watchdog,
        start: Instant,
        span: Duration,
        samples: impl Fn(Duration) -> bool,
    ) -> Vec<(Duration, Watch)> {
        let step = Duration::from_millis(10);
        let mut seen = Vec::new();
        let mut at = Duration::ZERO;
        while at <= span {
            let watch = dog.observe(start + at, samples(at));
            if watch != Watch::Fine {
                seen.push((at, watch));
            }
            at += step;
        }
        seen
    }

    #[test]
    fn silence_that_still_calls_back_is_never_a_stall() -> Result<()> {
        // A quiet room: every buffer is zeros, but buffers keep coming, so
        // the ring has samples on every pass. Ten minutes of it.
        let start = Instant::now();
        let mut dog = Watchdog::new(start);
        dog.opened(start);
        let seen = run(&mut dog, start, Duration::from_secs(600), |_| true);
        assert!(seen.is_empty(), "silence was taken for a stall: {seen:?}");
        assert!(!dog.stalled(), "no stall recorded");
        Ok(())
    }

    #[test]
    fn a_stream_that_stops_calling_back_is_a_stall_within_the_limit() -> Result<()> {
        // Delivering for five seconds, then nothing: the stall is reported
        // once, STALL_AFTER after the last samples, and not again for the
        // same stream.
        let start = Instant::now();
        let mut dog = Watchdog::new(start);
        dog.opened(start);
        let last = Duration::from_secs(5);
        let seen = run(&mut dog, start, Duration::from_secs(20), |at| at <= last);
        assert_eq!(seen.len(), 1, "reported once per stream: {seen:?}");
        let Some((at, watch)) = seen.first().copied() else {
            return Ok(());
        };
        let late = at.saturating_sub(last);
        assert!(
            late >= STALL_AFTER && late < STALL_AFTER + Duration::from_millis(20),
            "noticed {late:?} after the last samples"
        );
        assert_eq!(
            watch,
            Watch::Stalled {
                stalls: 1,
                delay: Duration::ZERO
            },
            "the first stall reopens at once"
        );
        assert!(dog.stalled(), "the stall is remembered for the reopen");
        Ok(())
    }

    #[test]
    fn short_gaps_and_a_slow_first_buffer_are_not_stalls() -> Result<()> {
        // A device that takes two seconds to send its first buffer after
        // opening, then has a scheduling hiccup of a second and a half.
        let start = Instant::now();
        let mut dog = Watchdog::new(start);
        dog.opened(start);
        let seen = run(&mut dog, start, Duration::from_secs(30), |at| {
            let ms = at.as_millis();
            ms >= 2_000 && !(10_000..11_500).contains(&ms)
        });
        assert!(seen.is_empty(), "a gap under the limit is fine: {seen:?}");
        Ok(())
    }

    #[test]
    fn a_device_that_keeps_stalling_is_reopened_on_a_backoff_and_recovers() -> Result<()> {
        let start = Instant::now();
        let mut dog = Watchdog::new(start);
        let mut now = start;
        let mut delays = Vec::new();
        for _ in 0..8 {
            dog.opened(now);
            now += STALL_AFTER;
            let Watch::Stalled { delay, .. } = dog.observe(now, false) else {
                return Err(anyhow::anyhow!("a mute stream was not reported"));
            };
            delays.push(delay);
            now += delay;
        }
        assert_eq!(
            delays,
            [0, 2, 4, 8, 16, 16, 16, 16].map(Duration::from_secs),
            "at once, then doubling to the cap"
        );
        assert!(dog.stalled(), "still stalled after eight mute streams");
        // The next stream delivers: recovered, and the count starts over.
        dog.opened(now);
        assert_eq!(
            dog.observe(now + Duration::from_millis(30), true),
            Watch::Recovered,
            "the first samples after a stall"
        );
        assert!(!dog.stalled(), "recovered");
        assert_eq!(
            dog.observe(now + Duration::from_millis(40), true),
            Watch::Fine,
            "recovered is reported once"
        );
        now += Duration::from_secs(60);
        dog.observe(now, true);
        assert_eq!(
            dog.observe(now + STALL_AFTER, false),
            Watch::Stalled {
                stalls: 1,
                delay: Duration::ZERO
            },
            "a later stall is a first one again"
        );
        Ok(())
    }

    #[test]
    fn the_stall_backoff_never_overflows() -> Result<()> {
        assert_eq!(stall_delay(0), Duration::ZERO, "no stall, no wait");
        assert_eq!(stall_delay(u32::MAX), STALL_RETRY_MAX, "capped");
        // At the cap a mute device is opened about once every
        // STALL_AFTER + STALL_RETRY_MAX, not once per pass.
        let per_minute = 60.0 / (STALL_AFTER + stall_delay(100)).as_secs_f32();
        assert!(per_minute <= 4.0, "{per_minute} reopens a minute");
        Ok(())
    }
}
