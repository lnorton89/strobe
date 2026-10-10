// ============================================================================
// Module:       strobe_app::show
// Description:  Playing a playlist: the glue between strobe-playlist's clock
//               and the app's scenes, compositor, transport and UI.
//
// Dependencies: strobe-playlist; submodules
// ============================================================================

//! The show: a playlist being played.
//!
//! `strobe-playlist` is pure (D73-D77): it knows which cue should be playing
//! and says so in events. This module is the other half, everything that
//! touches the app.
//!
//! | Module | What |
//! |---|---|
//! | [`beat`] | `BeatInfo` from an audio frame's scalars |
//! | [`cache`] | one slot per distinct scene; precompile progress |
//! | [`resolve`] | scene references to folders; control overrides to bank writes |
//! | [`start`] | wall clock, then resume point, then the first cue |
//! | [`resume`] | the resume file, written off the frame loop |
//! | [`run`] | [`Show`]: the precompile, the frame, warm-up and transitions |
//! | [`view`] | timeline rows for the LIST pane |
//! | [`watch`] | noticing that the playlist file was saved |
//! | [`generate`] | the Generate dialog's form and the library snapshot |

pub mod beat;
pub mod cache;
pub mod generate;
pub mod resolve;
pub mod resume;
pub mod run;
pub mod start;
pub mod view;
pub mod watch;

pub use run::{Env, FrameReport, Notice, PARKED, Phase, Show, StepReport, warm_size};
