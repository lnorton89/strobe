// ============================================================================
// Module:       strobe_playlist (library crate root)
// Description:  Playlists for Strobe: the model, inheritance, TOML I/O, a
//               deterministic generator, the timeline and the pure playback
//               state machine. No GL, no threads, no I/O but load and save.
//
// Dependencies: serde, toml, thiserror
// ============================================================================

//! Playlists.
//!
//! A playlist is a list of scenes to play, in order, with durations,
//! transitions and a way to end each cue. This crate is everything about
//! that which does not touch a GPU, a thread or a clock of its own, split
//! the way `docs/AGENT_RESEARCH.md` section 11 asks:
//!
//! | Part | Module | What it is |
//! |---|---|---|
//! | data model | [`model`], [`duration`] | `Playlist`, `Section`, `Cue`, settings, the duration grammar |
//! | inheritance | [`resolve`] | playlist, section, cue and built-ins flattened to `ResolvedCue`; resume identity; [`distinct_scenes`] |
//! | persistence | [`parse`], [`write`], [`file`] | the TOML format (PLAN section 7.2), with line/column errors |
//! | generator | [`generate`], [`rng`] | a section from a library snapshot, pure in its seed |
//! | timeline | [`timeline`], [`schedule`] | every cue's start and length; wall-clock placement |
//! | playback | [`clock`], [`tempo`] | [`PlaylistClock`]: the state machine the frame loop ticks |
//! | resume | [`resume`] | where playback was, as a few lines of TOML |
//!
//! # How the app uses it
//!
//! ```text
//! load      file::load            -> Playlist        (keep the old one on Err)
//! compile   distinct_scenes       -> [SceneRef]      (precompile all; zero compiles later)
//! plan      Timeline::from_playlist -> Timeline      (preview, totals, wall times)
//! run       PlaylistClock::new(timeline, ..)
//!           per frame:  clock.tick(dt, &beat); for e in clock.drain_events() { .. }
//!           on failure: clock.mark_scene_failed(&scene)
//!           on input:   clock.apply(Command::Next) ...
//!           every 5 s:  clock.resume_point() -> ResumePoint::save
//! ```
//!
//! Time is always an argument: `dt` for the scene clock, a time of day for
//! the wall clock. Nothing in here reads a system clock, so every behaviour
//! is reproducible from its inputs and the 4-hour soak test runs in
//! milliseconds.

pub mod duration;
pub mod error;
pub mod file;
pub mod generate;
pub mod model;
pub mod parse;
pub mod resolve;
pub mod rng;
pub mod schedule;
pub mod tempo;
pub mod timeline;
pub mod write;

pub use duration::{BEATS_PER_BAR, DurationError, DurationSpec, Length};
pub use error::{ParseError, PlaylistError};
pub use generate::{
    GenerateError, GenerateSpec, Generated, LibraryScene, LibrarySnapshot, PoolFilter, generate,
};
pub use model::{
    AdvanceMode, AudioEventKind, ControlValue, Cue, CueSettings, LoopMode, MetaOverrides, Playlist,
    SceneRef, Section, ShuffleSpec, TransitionKind, TransitionSpec, WallClockAnchor,
};
pub use resolve::{ResolvedCue, ResolvedPlaylist, distinct_scenes, identity, resolve};
pub use schedule::{Placement, WallTime};
pub use tempo::BeatInfo;
pub use timeline::{Timeline, TimelineConfig, TimelineCue};
