// ============================================================================
// Module:       strobe_app::recovery
// Description:  The context-loss policy as a pure state machine: from
//               failures, reset statuses and rebuild results to "carry on",
//               "retry", "rebuild here", "fail over to ANGLE" or "give up".
//
// Dependencies: strobe-gfx (Backend, GlChoice, ResetStatus)
// ============================================================================

//! Surviving a lost or unusable GL context: the decisions.
//!
//! A projector must never go dark because one driver call failed. On the
//! owner's T480 a native `make_current` fails mid-show with `The handle is
//! invalid` while ANGLE runs the same scene fine. This module holds the
//! *policy* for that, with no GL and no window in it, so every branch is a
//! unit test and none needs a broken driver. [`crate::app`] feeds it
//! [`Event`]s and carries out the [`Decision`]s.
//!
//! ## The policy
//!
//! | Situation | Decision |
//! |---|---|
//! | a frame presented | forget that window's failures |
//! | one or two failed frames in a row on a window | [`Decision::RetryNextFrame`] |
//! | **three** failed frames in a row on one window | rebuild |
//! | the driver reports a reset (guilty, innocent, unknown) | rebuild **now** |
//! | a rebuild, with no other native rebuild in the last 60 s | rebuild on the same backend |
//! | a second native rebuild within 60 s, `--gl auto` | **fail over** to ANGLE |
//! | a failed native rebuild, `--gl auto` | **fail over** to ANGLE |
//! | the same, `--gl native` | retry native (a forced backend is never abandoned) |
//! | ANGLE rebuilt twice within 60 s, or a failed ANGLE rebuild, `--gl auto` | [`Decision::GiveUp`] |
//! | any backend, more than [`MAX_REBUILDS`] rebuilds in 60 s | [`Decision::GiveUp`] |
//!
//! ### Why these numbers
//!
//! - **Three failed frames, not one.** A single `make_current` failure can
//!   be a window being minimised or moved between monitors, which recovers
//!   by itself. Three in a row on one window is about 50 ms of output at
//!   60 Hz: far too fast for a person to see as a stall, slow enough to
//!   ignore a hiccup. A driver *reset report* is unambiguous and skips the
//!   wait.
//! - **60 seconds, and a second native rebuild means ANGLE.** One rebuild is
//!   cheap insurance against a one-off (a GPU reset by another process, a
//!   sleep/wake). Two within a minute say the native driver cannot hold this
//!   scene at this load, which is the T480 failure, and ANGLE is the
//!   backend known to hold it. Staying on a native context that has died
//!   twice in a minute risks a show-long loop of black seconds.
//! - **`--gl native` never fails over.** A person who forced a backend
//!   chose it (to reproduce a bug, usually); the app keeps trying that
//!   backend, but stops at [`MAX_REBUILDS`] in a minute rather than
//!   thrashing the GPU for the rest of the night.
//! - **Giving up ends in the existing failure path.** The caller returns
//!   the reason as the app's failure, which `main` logs as a structured
//!   failure and shows in the startup-failure dialog: a clear message beats
//!   an endless black window, and the crash is not a native one.
//!
//! The policy is adjustable: the constants above are the whole of it.
//!
//! ## Time
//!
//! Every method takes `now` so tests control the clock. Nothing here reads
//! the system time.

use std::fmt;
use std::time::{Duration, Instant};
use strobe_gfx::gl::ResetStatus;
use strobe_gfx::{Backend, GlChoice};

/// Consecutive failed frames on one window that mean the context is gone.
pub const FAILED_FRAMES_BEFORE_REBUILD: u32 = 3;

/// How long a rebuild stays "recent": a second one on the same backend
/// inside this window is a repeat.
pub const REPEAT_WINDOW: Duration = Duration::from_secs(60);

/// Most rebuilds allowed inside [`REPEAT_WINDOW`], on any backend, before
/// the app gives up instead of thrashing.
pub const MAX_REBUILDS: usize = 4;

/// The longest the reset status is polled in a healthy run: once a second.
/// It is a driver round trip, so it must not run every frame.
pub const RESET_POLL_PERIOD: Duration = Duration::from_secs(1);

/// Which of the app's two windows a failure was on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The show.
    Output,
    /// The operator's window.
    Control,
}

impl Surface {
    fn index(self) -> usize {
        match self {
            Self::Output => 0,
            Self::Control => 1,
        }
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Output => "output",
            Self::Control => "control",
        })
    }
}

/// Something that happened to the GL context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<'a> {
    /// `make_current` or the swap failed on `window`. `reset` is the
    /// driver's reset status when the caller could ask for it safely (the
    /// context was current), else `None`.
    Failed {
        /// Which window.
        window: Surface,
        /// The driver or glutin error text.
        error: &'a str,
        /// The reset status, if it was queried.
        reset: Option<ResetStatus>,
    },
    /// A health-check poll of the reset status, made while the context was
    /// current for `window`.
    Reset {
        /// Which window the poll ran for.
        window: Surface,
        /// The driver's answer.
        status: ResetStatus,
    },
    /// `window` drew and swapped without error.
    Presented(Surface),
    /// A rebuild finished. `error` is `None` when `backend` is up.
    Rebuilt {
        /// The backend that was attempted.
        backend: Backend,
        /// Why it failed, if it did.
        error: Option<&'a str>,
    },
}

/// Why a rebuild was decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// The driver reported a reset.
    Reset {
        /// Which window it was noticed on.
        window: Surface,
        /// What the driver said.
        status: ResetStatus,
        /// The failed call that prompted the query, if there was one.
        error: Option<String>,
    },
    /// Consecutive failed frames on a window.
    FailedFrames {
        /// Which window.
        window: Surface,
        /// How many in a row.
        count: u32,
        /// The last error text.
        error: String,
    },
    /// The previous rebuild failed.
    FailedRebuild {
        /// The backend that would not start.
        backend: Backend,
        /// What it reported.
        error: String,
    },
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reset {
                window,
                status,
                error: None,
            } => write!(
                f,
                "the driver reported a reset on the {window} window ({status})"
            ),
            Self::Reset {
                window,
                status,
                error: Some(error),
            } => write!(
                f,
                "the driver reported a reset on the {window} window ({status}) after: {error}"
            ),
            Self::FailedFrames {
                window,
                count,
                error,
            } => write!(f, "{count} failed {window} frames in a row: {error}"),
            Self::FailedRebuild { backend, error } => {
                write!(f, "rebuilding on {backend} failed: {error}")
            }
        }
    }
}

/// A rebuild to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebuild {
    /// The backend to build on.
    pub backend: Backend,
    /// Whether this switches backends (native to ANGLE).
    pub failover: bool,
    /// What led to it.
    pub cause: Cause,
}

/// What the app should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to do.
    Continue,
    /// Skip this frame and try again on the next; not yet worth a rebuild.
    RetryNextFrame,
    /// Tear the context down and build another.
    Rebuild(Rebuild),
    /// Nothing more can be tried; the text says why.
    GiveUp(String),
}

/// A rebuild decided at some time, for the repeat rules.
#[derive(Debug, Clone, Copy)]
struct Decided {
    at: Instant,
    backend: Backend,
}

/// The policy's state.
#[derive(Debug, Clone)]
pub struct Recovery {
    /// Whether the person's `--gl` choice allows ANGLE (it is `auto`).
    allow_angle: bool,
    /// The backend in use.
    backend: Backend,
    /// Consecutive failed frames per window.
    failures: [u32; 2],
    /// Rebuilds decided inside [`REPEAT_WINDOW`].
    recent: Vec<Decided>,
    last_poll: Option<Instant>,
}

impl Recovery {
    /// A policy for a run that started on `backend` under `choice`.
    pub fn new(choice: GlChoice, backend: Backend) -> Self {
        Self {
            allow_angle: choice == GlChoice::Auto,
            backend,
            failures: [0; 2],
            recent: Vec::new(),
            last_poll: None,
        }
    }

    /// The backend in use.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// Whether it is time for a health-check poll of the reset status:
    /// true at most once per [`RESET_POLL_PERIOD`].
    pub fn poll_due(&mut self, now: Instant) -> bool {
        let due = self
            .last_poll
            .is_none_or(|last| now.saturating_duration_since(last) >= RESET_POLL_PERIOD);
        if due {
            self.last_poll = Some(now);
        }
        due
    }

    /// Feeds one event and returns what to do about it.
    pub fn handle(&mut self, now: Instant, event: Event<'_>) -> Decision {
        match event {
            Event::Presented(window) => {
                self.failures[window.index()] = 0;
                Decision::Continue
            }
            Event::Failed {
                window,
                error,
                reset,
            } => {
                let count = &mut self.failures[window.index()];
                *count += 1;
                let count = *count;
                if let Some(status) = reset.filter(|status| status.is_reset()) {
                    let error = Some(error.to_string());
                    return self.rebuild(
                        now,
                        Cause::Reset {
                            window,
                            status,
                            error,
                        },
                    );
                }
                if count >= FAILED_FRAMES_BEFORE_REBUILD {
                    return self.rebuild(
                        now,
                        Cause::FailedFrames {
                            window,
                            count,
                            error: error.to_string(),
                        },
                    );
                }
                Decision::RetryNextFrame
            }
            Event::Reset { window, status } if status.is_reset() => self.rebuild(
                now,
                Cause::Reset {
                    window,
                    status,
                    error: None,
                },
            ),
            Event::Reset { .. } => Decision::Continue,
            Event::Rebuilt {
                backend,
                error: None,
            } => {
                self.backend = backend;
                self.failures = [0; 2];
                Decision::Continue
            }
            Event::Rebuilt {
                backend,
                error: Some(error),
            } => self.rebuild_failed(now, backend, error),
        }
    }

    /// Decides a rebuild after a loss was detected.
    fn rebuild(&mut self, now: Instant, cause: Cause) -> Decision {
        self.prune(now);
        let repeated = self.recent.iter().any(|d| d.backend == self.backend);
        if repeated && self.allow_angle && self.backend == Backend::Angle {
            return Decision::GiveUp(format!(
                "ANGLE lost the GL context twice within {} s; {cause}",
                REPEAT_WINDOW.as_secs()
            ));
        }
        let failover = repeated && self.allow_angle && self.backend == Backend::Native;
        let backend = if failover {
            Backend::Angle
        } else {
            self.backend
        };
        self.begin(now, backend, failover, cause)
    }

    /// Decides what follows a rebuild that did not come up.
    fn rebuild_failed(&mut self, now: Instant, backend: Backend, error: &str) -> Decision {
        self.prune(now);
        let cause = Cause::FailedRebuild {
            backend,
            error: error.to_string(),
        };
        if self.allow_angle {
            return match backend {
                Backend::Native => self.begin(now, Backend::Angle, true, cause),
                Backend::Angle => {
                    Decision::GiveUp(format!("neither backend could be started again; {cause}"))
                }
            };
        }
        // A forced backend is retried, within the limit.
        self.begin(now, backend, false, cause)
    }

    /// Records and returns a rebuild, or gives up at the limit.
    fn begin(&mut self, now: Instant, backend: Backend, failover: bool, cause: Cause) -> Decision {
        if self.recent.len() >= MAX_REBUILDS {
            return Decision::GiveUp(format!(
                "{} rebuilds of the GL context within {} s; {cause}",
                self.recent.len(),
                REPEAT_WINDOW.as_secs()
            ));
        }
        self.recent.push(Decided { at: now, backend });
        Decision::Rebuild(Rebuild {
            backend,
            failover,
            cause,
        })
    }

    fn prune(&mut self, now: Instant) {
        self.recent
            .retain(|d| now.saturating_duration_since(d.at) < REPEAT_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    const ERROR: &str = "make current: [6] The handle is invalid. (os error 6)";

    fn auto(backend: Backend) -> (Recovery, Instant) {
        (Recovery::new(GlChoice::Auto, backend), Instant::now())
    }

    fn forced(backend: Backend) -> (Recovery, Instant) {
        (
            Recovery::new(GlChoice::Only(backend), backend),
            Instant::now(),
        )
    }

    fn fail(window: Surface) -> Event<'static> {
        Event::Failed {
            window,
            error: ERROR,
            reset: None,
        }
    }

    /// A health-check poll of the output window's reset status.
    fn poll(status: ResetStatus) -> Event<'static> {
        Event::Reset {
            window: Surface::Output,
            status,
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// Fails the output until the policy decides something other than a
    /// retry, returning that decision.
    fn lose(policy: &mut Recovery, now: Instant) -> Decision {
        for _ in 0..FAILED_FRAMES_BEFORE_REBUILD {
            let decision = policy.handle(now, fail(Surface::Output));
            if decision != Decision::RetryNextFrame {
                return decision;
            }
        }
        Decision::RetryNextFrame
    }

    fn rebuild_of(decision: Decision) -> Result<Rebuild> {
        match decision {
            Decision::Rebuild(rebuild) => Ok(rebuild),
            Decision::Continue | Decision::RetryNextFrame | Decision::GiveUp(_) => {
                Err(anyhow!("expected a rebuild, got {decision:?}"))
            }
        }
    }

    fn done(policy: &mut Recovery, now: Instant, backend: Backend) {
        let decision = policy.handle(
            now,
            Event::Rebuilt {
                backend,
                error: None,
            },
        );
        assert_eq!(decision, Decision::Continue, "a rebuild that came up");
    }

    #[test]
    fn a_presented_frame_costs_nothing() {
        let (mut policy, now) = auto(Backend::Native);
        assert_eq!(
            policy.handle(now, Event::Presented(Surface::Output)),
            Decision::Continue,
            "healthy"
        );
    }

    #[test]
    fn one_and_two_failed_frames_are_retried_and_the_third_rebuilds() -> Result<()> {
        let (mut policy, now) = auto(Backend::Native);
        assert_eq!(
            policy.handle(now, fail(Surface::Output)),
            Decision::RetryNextFrame,
            "first"
        );
        assert_eq!(
            policy.handle(now, fail(Surface::Output)),
            Decision::RetryNextFrame,
            "second"
        );
        let rebuild = rebuild_of(policy.handle(now, fail(Surface::Output)))?;
        assert_eq!(rebuild.backend, Backend::Native, "same backend first");
        assert!(!rebuild.failover, "not a fail-over");
        assert_eq!(
            rebuild.cause,
            Cause::FailedFrames {
                window: Surface::Output,
                count: 3,
                error: ERROR.to_string()
            },
            "the cause names the window, the count and the error"
        );
        Ok(())
    }

    #[test]
    fn a_good_frame_between_failures_starts_the_count_again() {
        let (mut policy, now) = auto(Backend::Native);
        for _ in 0..2 {
            policy.handle(now, fail(Surface::Output));
        }
        policy.handle(now, Event::Presented(Surface::Output));
        for round in 0..2 {
            assert_eq!(
                policy.handle(now, fail(Surface::Output)),
                Decision::RetryNextFrame,
                "failure {round} after the good frame"
            );
        }
    }

    #[test]
    fn the_windows_are_counted_separately() {
        let (mut policy, now) = auto(Backend::Native);
        for _ in 0..2 {
            policy.handle(now, fail(Surface::Output));
            policy.handle(now, fail(Surface::Control));
        }
        // Two each: neither window has three, and a good control frame
        // forgets only the control window's failures.
        policy.handle(now, Event::Presented(Surface::Control));
        assert!(
            matches!(
                policy.handle(now, fail(Surface::Output)),
                Decision::Rebuild(_)
            ),
            "the output's own third failure rebuilds"
        );
    }

    #[test]
    fn a_driver_reset_rebuilds_at_once() -> Result<()> {
        for status in [
            ResetStatus::Guilty,
            ResetStatus::Innocent,
            ResetStatus::Unknown,
        ] {
            let (mut policy, now) = auto(Backend::Native);
            let rebuild = rebuild_of(policy.handle(now, poll(status)))?;
            assert_eq!(
                rebuild.cause,
                Cause::Reset {
                    window: Surface::Output,
                    status,
                    error: None
                },
                "{status}"
            );
        }
        Ok(())
    }

    #[test]
    fn no_error_and_unsupported_are_not_a_loss() {
        let (mut policy, now) = auto(Backend::Native);
        for status in [ResetStatus::NoError, ResetStatus::Unsupported] {
            assert_eq!(
                policy.handle(now, poll(status)),
                Decision::Continue,
                "{status}"
            );
        }
    }

    #[test]
    fn a_failure_with_a_reset_status_does_not_wait_for_three() -> Result<()> {
        let (mut policy, now) = auto(Backend::Native);
        let rebuild = rebuild_of(policy.handle(
            now,
            Event::Failed {
                window: Surface::Output,
                error: "swap buffers: lost",
                reset: Some(ResetStatus::Innocent),
            },
        ))?;
        assert_eq!(
            rebuild.cause,
            Cause::Reset {
                window: Surface::Output,
                status: ResetStatus::Innocent,
                error: Some("swap buffers: lost".to_string())
            },
            "the cause keeps the failed call that prompted the query"
        );
        Ok(())
    }

    #[test]
    fn a_failure_with_a_clean_reset_status_still_counts_frames() {
        let (mut policy, now) = auto(Backend::Native);
        let decision = policy.handle(
            now,
            Event::Failed {
                window: Surface::Output,
                error: "swap buffers: busy",
                reset: Some(ResetStatus::NoError),
            },
        );
        assert_eq!(decision, Decision::RetryNextFrame, "one failure, no reset");
    }

    #[test]
    fn the_second_native_rebuild_within_a_minute_fails_over_to_angle() -> Result<()> {
        let (mut policy, start) = auto(Backend::Native);
        let first = rebuild_of(lose(&mut policy, start))?;
        assert_eq!(first.backend, Backend::Native, "first loss stays native");
        done(&mut policy, start, Backend::Native);

        let second = rebuild_of(lose(&mut policy, start + secs(30)))?;
        assert_eq!(second.backend, Backend::Angle, "second loss goes to ANGLE");
        assert!(second.failover, "flagged as a fail-over");
        Ok(())
    }

    #[test]
    fn a_native_rebuild_after_the_window_is_a_fresh_start() -> Result<()> {
        let (mut policy, start) = auto(Backend::Native);
        rebuild_of(lose(&mut policy, start))?;
        done(&mut policy, start, Backend::Native);
        let later = rebuild_of(lose(&mut policy, start + REPEAT_WINDOW + secs(1)))?;
        assert_eq!(later.backend, Backend::Native, "61 s later is not a repeat");
        assert!(!later.failover, "no fail-over");
        Ok(())
    }

    #[test]
    fn a_native_rebuild_that_fails_falls_over_to_angle() -> Result<()> {
        let (mut policy, now) = auto(Backend::Native);
        rebuild_of(lose(&mut policy, now))?;
        let next = rebuild_of(policy.handle(
            now,
            Event::Rebuilt {
                backend: Backend::Native,
                error: Some("context: out of memory"),
            },
        ))?;
        assert_eq!(next.backend, Backend::Angle, "ANGLE next");
        assert!(next.failover, "a fail-over");
        assert_eq!(
            next.cause,
            Cause::FailedRebuild {
                backend: Backend::Native,
                error: "context: out of memory".to_string()
            },
            "the cause is the failed rebuild"
        );
        done(&mut policy, now, Backend::Angle);
        assert_eq!(policy.backend(), Backend::Angle, "now on ANGLE");
        Ok(())
    }

    #[test]
    fn if_angle_fails_too_the_policy_gives_up() -> Result<()> {
        let (mut policy, now) = auto(Backend::Native);
        rebuild_of(lose(&mut policy, now))?;
        rebuild_of(policy.handle(
            now,
            Event::Rebuilt {
                backend: Backend::Native,
                error: Some("no"),
            },
        ))?;
        let decision = policy.handle(
            now,
            Event::Rebuilt {
                backend: Backend::Angle,
                error: Some("display: no EGL"),
            },
        );
        match decision {
            Decision::GiveUp(reason) => {
                assert!(reason.contains("display: no EGL"), "reason: {reason}");
            }
            Decision::Continue | Decision::RetryNextFrame | Decision::Rebuild(_) => {
                return Err(anyhow!("expected GiveUp, got {decision:?}"));
            }
        }
        Ok(())
    }

    #[test]
    fn angle_losing_the_context_twice_in_a_minute_gives_up() -> Result<()> {
        let (mut policy, start) = auto(Backend::Angle);
        let first = rebuild_of(lose(&mut policy, start))?;
        assert_eq!(first.backend, Backend::Angle, "ANGLE rebuilds on ANGLE");
        done(&mut policy, start, Backend::Angle);
        let second = lose(&mut policy, start + secs(10));
        assert!(
            matches!(second, Decision::GiveUp(_)),
            "twice in a minute: {second:?}"
        );
        Ok(())
    }

    #[test]
    fn angle_losing_the_context_a_minute_apart_is_rebuilt_again() -> Result<()> {
        let (mut policy, start) = auto(Backend::Angle);
        rebuild_of(lose(&mut policy, start))?;
        done(&mut policy, start, Backend::Angle);
        let later = rebuild_of(lose(&mut policy, start + REPEAT_WINDOW + secs(5)))?;
        assert_eq!(later.backend, Backend::Angle, "a fresh loss");
        Ok(())
    }

    #[test]
    fn a_forced_native_backend_is_retried_not_abandoned() -> Result<()> {
        let (mut policy, start) = forced(Backend::Native);
        for step in 0..MAX_REBUILDS {
            let now = start + secs(step as u64 * 5);
            let rebuild = rebuild_of(lose(&mut policy, now))?;
            assert_eq!(rebuild.backend, Backend::Native, "rebuild {step}");
            assert!(!rebuild.failover, "never fails over when forced");
            done(&mut policy, now, Backend::Native);
        }
        let over = lose(&mut policy, start + secs(MAX_REBUILDS as u64 * 5));
        assert!(
            matches!(over, Decision::GiveUp(_)),
            "but not forever: {over:?}"
        );
        Ok(())
    }

    #[test]
    fn a_forced_native_rebuild_that_fails_is_tried_again() -> Result<()> {
        let (mut policy, now) = forced(Backend::Native);
        rebuild_of(lose(&mut policy, now))?;
        let retry = rebuild_of(policy.handle(
            now,
            Event::Rebuilt {
                backend: Backend::Native,
                error: Some("context: busy"),
            },
        ))?;
        assert_eq!(retry.backend, Backend::Native, "still native");
        assert!(!retry.failover, "no fail-over");
        Ok(())
    }

    #[test]
    fn a_forced_angle_backend_is_retried_even_on_a_quick_repeat() -> Result<()> {
        let (mut policy, start) = forced(Backend::Angle);
        rebuild_of(lose(&mut policy, start))?;
        done(&mut policy, start, Backend::Angle);
        let second = rebuild_of(lose(&mut policy, start + secs(5)))?;
        assert_eq!(second.backend, Backend::Angle, "forced ANGLE stays");
        Ok(())
    }

    #[test]
    fn too_many_rebuilds_in_a_minute_give_up_on_any_backend() -> Result<()> {
        // A forced backend never fails over or gives up on a repeat, so
        // only the count limit can stop it: resets a second apart.
        let (mut policy, start) = forced(Backend::Angle);
        for step in 0..MAX_REBUILDS {
            let now = start + secs(step as u64);
            rebuild_of(policy.handle(now, poll(ResetStatus::Guilty)))?;
            done(&mut policy, now, Backend::Angle);
        }
        let decision = policy.handle(start + secs(MAX_REBUILDS as u64), poll(ResetStatus::Guilty));
        match decision {
            Decision::GiveUp(reason) => {
                assert!(reason.contains("rebuilds"), "reason: {reason}");
            }
            Decision::Continue | Decision::RetryNextFrame | Decision::Rebuild(_) => {
                return Err(anyhow!("expected GiveUp, got {decision:?}"));
            }
        }
        // Once the old ones age out, rebuilding is allowed again.
        let later = start + REPEAT_WINDOW + secs(MAX_REBUILDS as u64 + 1);
        rebuild_of(policy.handle(later, poll(ResetStatus::Guilty)))?;
        Ok(())
    }

    #[test]
    fn a_successful_rebuild_clears_the_failure_counts() {
        let (mut policy, now) = auto(Backend::Native);
        lose(&mut policy, now);
        done(&mut policy, now, Backend::Native);
        assert_eq!(
            policy.handle(now, fail(Surface::Output)),
            Decision::RetryNextFrame,
            "the next failure is the first again"
        );
    }

    #[test]
    fn the_reset_poll_is_due_at_most_once_a_second() {
        let (mut policy, start) = auto(Backend::Native);
        assert!(policy.poll_due(start), "the first poll is due");
        assert!(
            !policy.poll_due(start + Duration::from_millis(16)),
            "not the next frame"
        );
        assert!(
            !policy.poll_due(start + Duration::from_millis(999)),
            "not before a second"
        );
        assert!(policy.poll_due(start + secs(1)), "a second later");
        assert!(
            !policy.poll_due(start + secs(1) + Duration::from_millis(500)),
            "and then the clock restarts"
        );
    }

    #[test]
    fn the_cause_reads_as_a_sentence_for_the_failure_record() {
        let text = Cause::FailedFrames {
            window: Surface::Output,
            count: 3,
            error: ERROR.to_string(),
        }
        .to_string();
        assert!(text.contains("3 failed output frames"), "text: {text}");
        assert!(text.contains("The handle is invalid"), "text: {text}");
        let reset = Cause::Reset {
            window: Surface::Output,
            status: ResetStatus::Innocent,
            error: None,
        }
        .to_string();
        assert!(reset.contains("innocent context reset"), "text: {reset}");
        assert!(reset.contains("output window"), "text: {reset}");
    }
}
