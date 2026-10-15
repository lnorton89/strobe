// ============================================================================
// Module:       strobe_gfx::gl::trace
// Description:  An opt-in record of where the renderer binds, draws, clears,
//               blits and reads: which framebuffer, at what viewport. For
//               hardware that renders black, and for a test of the invariant
//               that offscreen work never touches the default framebuffer.
//
// Dependencies: log; the parent module's wrappers call into it
// ============================================================================

//! A trace of the renderer's framebuffer traffic (D296).
//!
//! The question it answers is "what did the renderer ask the driver to do,
//! and where": for every bind, viewport, clear, draw, blit, read-back and
//! texture allocation, the framebuffer it targeted (0 is the window's
//! default framebuffer) and the viewport in force. It makes no GL calls of
//! its own; it only remembers what the safe wrappers in [`super`] were
//! asked, so it costs one one-byte `Cell` read per wrapper when off. When on,
//! the one driver round trip is [`super::Gl::viewport`] reading back
//! `GL_VIEWPORT` the first time each size is set (every time while a test
//! captures).
//!
//! Two uses:
//!
//! - **On a machine that renders black.** `STROBE_GL_TRACE=1` logs each
//!   *distinct* event once (target `gl_trace`, level `info`), then
//!   [`Gl::trace_summary`] prints each with how many times it happened. A
//!   frame-by-frame log would be megabytes; the distinct set is a page and
//!   shows whether the renderer ever targets framebuffer 0, which viewport
//!   each pass draws at, and what it reads back. A number after the `1`
//!   (`STROBE_GL_TRACE=500`) caps how many distinct events are kept.
//! - **In a test.** [`Gl::trace_capture`] records into memory whatever the
//!   environment says, and [`Gl::trace_take`] returns the events, so a test
//!   can assert that an offscreen render drew only into its own targets.
//!
//! The state it follows (draw and read framebuffer, viewport) is the state
//! the wrappers set, not a query of the driver, so it is exact for code that
//! goes through [`super::Gl`] and blind to anything that does not. Nothing
//! else in the workspace calls GL.

use std::cell::{Cell, RefCell};

/// The environment variable that turns the log on.
pub const ENV: &str = "STROBE_GL_TRACE";

/// The most distinct events kept when the variable gives no number.
const DEFAULT_LIMIT: usize = 512;

/// Which framebuffer binding point a bind targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// `GL_FRAMEBUFFER`: both draw and read.
    Both,
    /// `GL_DRAW_FRAMEBUFFER`.
    Draw,
    /// `GL_READ_FRAMEBUFFER`.
    Read,
}

/// One thing the renderer asked the driver to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A framebuffer was bound (`fbo` 0 is the default framebuffer).
    Bind {
        /// Which binding point.
        binding: Binding,
        /// The framebuffer, 0 for the default.
        fbo: u32,
    },
    /// The viewport was set to `0, 0, size`.
    Viewport {
        /// Width and height.
        size: [i32; 2],
    },
    /// The colour buffer was cleared.
    Clear {
        /// The draw framebuffer at the time.
        draw_fbo: u32,
        /// The viewport at the time.
        viewport: [i32; 2],
    },
    /// A draw call.
    Draw {
        /// The draw framebuffer at the time.
        draw_fbo: u32,
        /// The viewport at the time.
        viewport: [i32; 2],
    },
    /// A framebuffer-to-framebuffer copy.
    Blit {
        /// The read framebuffer at the time.
        read_fbo: u32,
        /// The draw framebuffer at the time.
        draw_fbo: u32,
        /// The source rectangle's size.
        src: [i32; 2],
        /// The destination rectangle, `[x0, y0, x1, y1]`.
        dst: [i32; 4],
    },
    /// `glReadPixels` from the read framebuffer.
    ReadPixels {
        /// The read framebuffer at the time.
        read_fbo: u32,
        /// Width and height read.
        size: [i32; 2],
    },
    /// A 2D texture was allocated.
    Texture {
        /// Width and height.
        size: [i32; 2],
        /// The format's name.
        format: &'static str,
    },
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bind { binding, fbo } => write!(f, "bind {binding:?} fbo {fbo}"),
            Self::Viewport { size } => write!(f, "viewport {}x{}", size[0], size[1]),
            Self::Clear { draw_fbo, viewport } => write!(
                f,
                "clear into fbo {draw_fbo} at viewport {}x{}",
                viewport[0], viewport[1]
            ),
            Self::Draw { draw_fbo, viewport } => write!(
                f,
                "draw into fbo {draw_fbo} at viewport {}x{}",
                viewport[0], viewport[1]
            ),
            Self::Blit {
                read_fbo,
                draw_fbo,
                src,
                dst,
            } => write!(
                f,
                "blit fbo {read_fbo} ({}x{}) to fbo {draw_fbo} ({},{} to {},{})",
                src[0], src[1], dst[0], dst[1], dst[2], dst[3]
            ),
            Self::ReadPixels { read_fbo, size } => {
                write!(f, "read {}x{} from fbo {read_fbo}", size[0], size[1])
            }
            Self::Texture { size, format } => {
                write!(f, "texture {}x{} {format}", size[0], size[1])
            }
        }
    }
}

impl Event {
    /// Whether this event writes to, or reads from, the default
    /// framebuffer: a draw, clear or blit that lands in it, or a read-back
    /// that comes from it. Binding it is not one of these; leaving the
    /// default bound between passes is harmless.
    pub fn touches_default_framebuffer(&self) -> bool {
        match self {
            Self::Clear { draw_fbo, .. } | Self::Draw { draw_fbo, .. } => *draw_fbo == 0,
            Self::Blit {
                read_fbo, draw_fbo, ..
            } => *read_fbo == 0 || *draw_fbo == 0,
            Self::ReadPixels { read_fbo, .. } => *read_fbo == 0,
            Self::Bind { .. } | Self::Viewport { .. } | Self::Texture { .. } => false,
        }
    }
}

/// How the trace is currently used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    /// Log each distinct event once, and count.
    Log,
    /// Keep every event, in order, for [`Trace::take`].
    Capture,
}

/// The framebuffer state the wrappers have set.
#[derive(Debug, Clone, Copy)]
struct State {
    draw_fbo: u32,
    read_fbo: u32,
    viewport: [i32; 2],
}

/// The recorder one context shares among its clones.
pub(super) struct Trace {
    mode: Cell<Mode>,
    state: Cell<State>,
    limit: Cell<usize>,
    /// Distinct events with their counts (log mode), or every event with a
    /// count of 1 (capture mode).
    events: RefCell<Vec<(Event, u64)>>,
    /// Events dropped because `limit` distinct ones were already kept.
    dropped: Cell<u64>,
}

impl Trace {
    /// A recorder configured from `STROBE_GL_TRACE`.
    pub(super) fn from_env() -> Self {
        let setting = std::env::var(ENV).ok();
        let (mode, limit) = parse(setting.as_deref());
        Self {
            mode: Cell::new(mode),
            state: Cell::new(State {
                draw_fbo: 0,
                read_fbo: 0,
                viewport: [0, 0],
            }),
            limit: Cell::new(limit),
            events: RefCell::new(Vec::new()),
            dropped: Cell::new(0),
        }
    }

    /// Whether anything is recorded. The wrappers check this first.
    pub(super) fn on(&self) -> bool {
        self.mode.get() != Mode::Off
    }

    /// Records every event in order from now on.
    pub(super) fn capture(&self) {
        self.mode.set(Mode::Capture);
        self.limit.set(usize::MAX);
        self.events.borrow_mut().clear();
    }

    /// The events recorded so far, in order, and forgets them. In log mode
    /// these are the distinct events, not a sequence.
    pub(super) fn take(&self) -> Vec<Event> {
        self.events
            .take()
            .into_iter()
            .map(|(event, _)| event)
            .collect()
    }

    pub(super) fn bind(&self, binding: Binding, fbo: u32) {
        let mut state = self.state.get();
        match binding {
            Binding::Both => {
                state.draw_fbo = fbo;
                state.read_fbo = fbo;
            }
            Binding::Draw => state.draw_fbo = fbo,
            Binding::Read => state.read_fbo = fbo,
        }
        self.state.set(state);
        self.record(Event::Bind { binding, fbo });
    }

    /// Records a viewport change; true when this viewport is new (log mode)
    /// or always (capture mode), so the caller may check it against the
    /// driver once.
    pub(super) fn viewport(&self, size: [i32; 2]) -> bool {
        let mut state = self.state.get();
        state.viewport = size;
        self.state.set(state);
        self.record(Event::Viewport { size })
    }

    pub(super) fn clear(&self) {
        let state = self.state.get();
        self.record(Event::Clear {
            draw_fbo: state.draw_fbo,
            viewport: state.viewport,
        });
    }

    pub(super) fn draw(&self) {
        let state = self.state.get();
        self.record(Event::Draw {
            draw_fbo: state.draw_fbo,
            viewport: state.viewport,
        });
    }

    pub(super) fn blit(&self, src: [i32; 2], dst: [i32; 4]) {
        let state = self.state.get();
        self.record(Event::Blit {
            read_fbo: state.read_fbo,
            draw_fbo: state.draw_fbo,
            src,
            dst,
        });
    }

    pub(super) fn read_pixels(&self, size: [i32; 2]) {
        let state = self.state.get();
        self.record(Event::ReadPixels {
            read_fbo: state.read_fbo,
            size,
        });
    }

    pub(super) fn texture(&self, size: [i32; 2], format: &'static str) {
        self.record(Event::Texture { size, format });
    }

    /// Keeps `event`; true when it is new (every event is new in capture
    /// mode).
    fn record(&self, event: Event) -> bool {
        let mut events = self.events.borrow_mut();
        if self.mode.get() == Mode::Capture {
            events.push((event, 1));
            return true;
        }
        if let Some((_, count)) = events.iter_mut().find(|(seen, _)| *seen == event) {
            *count += 1;
            return false;
        }
        if events.len() >= self.limit.get() {
            self.dropped.set(self.dropped.get() + 1);
            return false;
        }
        log::info!(target: "gl_trace", "{event}");
        events.push((event, 1));
        true
    }

    /// Each distinct event with its count, one per line, in first-seen
    /// order. Empty when nothing was recorded.
    pub(super) fn summary(&self) -> String {
        use std::fmt::Write as _;
        let events = self.events.borrow();
        let mut out = String::new();
        for (event, count) in events.iter() {
            let _ = writeln!(out, "{count:>8} x {event}");
        }
        if self.dropped.get() > 0 {
            let _ = writeln!(
                out,
                "{:>8} more events not kept (limit {} distinct)",
                self.dropped.get(),
                self.limit.get()
            );
        }
        out
    }
}

/// The mode and distinct-event limit a `STROBE_GL_TRACE` value means.
fn parse(setting: Option<&str>) -> (Mode, usize) {
    match setting.map(str::trim) {
        None | Some("" | "0" | "off" | "false") => (Mode::Off, DEFAULT_LIMIT),
        Some(text) => match text.parse::<usize>() {
            Ok(limit) if limit > 1 => (Mode::Log, limit),
            Ok(_) | Err(_) => (Mode::Log, DEFAULT_LIMIT),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_variable_turns_the_log_on_and_a_number_caps_it() -> Result<()> {
        assert_eq!(parse(None), (Mode::Off, DEFAULT_LIMIT), "unset is off");
        assert_eq!(parse(Some("0")), (Mode::Off, DEFAULT_LIMIT), "0 is off");
        assert_eq!(parse(Some("1")), (Mode::Log, DEFAULT_LIMIT), "1 is on");
        assert_eq!(parse(Some("40")), (Mode::Log, 40), "a number is a cap");
        assert_eq!(parse(Some("yes")), (Mode::Log, DEFAULT_LIMIT), "any word");
        Ok(())
    }

    #[test]
    fn a_draw_remembers_the_framebuffer_and_viewport_set_before_it() -> Result<()> {
        let trace = Trace::from_env();
        trace.capture();
        trace.bind(Binding::Both, 7);
        trace.viewport([64, 36]);
        trace.draw();
        trace.bind(Binding::Both, 0);
        trace.draw();
        let events = trace.take();
        let draws: Vec<&Event> = events
            .iter()
            .filter(|event| matches!(event, Event::Draw { .. }))
            .collect();
        assert_eq!(
            draws,
            vec![
                &Event::Draw {
                    draw_fbo: 7,
                    viewport: [64, 36]
                },
                &Event::Draw {
                    draw_fbo: 0,
                    viewport: [64, 36]
                },
            ],
            "the first draw is into fbo 7, the second into the default"
        );
        assert!(
            !draws
                .first()
                .is_some_and(|event| event.touches_default_framebuffer()),
            "a draw into fbo 7 leaves the default alone"
        );
        assert!(
            draws
                .get(1)
                .is_some_and(|event| event.touches_default_framebuffer()),
            "a draw with the default bound touches it"
        );
        Ok(())
    }

    #[test]
    fn log_mode_keeps_each_distinct_event_once_with_a_count() -> Result<()> {
        let trace = Trace::from_env();
        trace.mode.set(Mode::Log);
        for _ in 0..5 {
            trace.bind(Binding::Both, 3);
            trace.draw();
        }
        let summary = trace.summary();
        assert!(
            summary.contains("5 x draw into fbo 3 at viewport 0x0"),
            "the five identical draws are one line: {summary}"
        );
        assert_eq!(summary.lines().count(), 2, "one bind, one draw: {summary}");
        Ok(())
    }
}
