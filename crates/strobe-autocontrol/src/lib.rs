// ============================================================================
// Module:       strobe_autocontrol (library crate root)
// Description:  Auto-generated controls: finds the literals that shape a
//               scene, rewrites them into uniforms and keeps the ones that
//               visibly change the image.
//
// Dependencies: strobe-scene, strobe-shadertoy (tokenizer), strobe-gfx (scoring)
// ============================================================================

//! Auto-generated controls.
//!
//! Most Shadertoy shaders, and many community SSF scenes, have no controls:
//! their character is baked into literal numbers. This crate finds those
//! numbers and turns the ones that matter into a control set, so an import
//! is performable the moment it lands.
//!
//! ```text
//! GLSL files ──▶ analyze ──▶ candidates ──▶ rewrite (all, as uniforms)
//!                                              │ compile once
//!                                              ▼
//!      CONTROLS + GLSL  ◀── keep top K ◀── score (render, nudge, diff)
//! ```
//!
//! - [`analyze`] is pure static analysis over the lossless tokenizer of
//!   `strobe-shadertoy`: `#define`s and `const`s, `TIME` multipliers, loop
//!   bounds, `vec3` colours, coordinate scales and frequencies. It is
//!   conservative on purpose.
//! - [`rewrite`] turns chosen candidates into uniform references **without
//!   moving a line**, and writes the matching `CONTROLS` entries.
//! - [`score`] is the only part that needs a GL context. It compiles the
//!   rewritten scene once and measures each control by rendering fixed
//!   frames with its value nudged.
//! - [`generate`] runs all of it and returns a [`report::Report`] plus the
//!   rewritten scene; [`output`] writes that as a new scene folder.
//! - [`suggest`] proposes audio mappings for the strongest controls.
//!
//! Everything is deterministic: fixed frames, fixed times, the seeded
//! synthetic audio track. The same scene gives the same controls, which is
//! what lets golden tests pin the result.
//!
//! Scoring runs at import time, never per frame.

pub mod analyze;
pub mod candidate;
pub mod generate;
pub mod naming;
pub mod output;
pub mod report;
pub mod rewrite;
pub mod score;
pub mod source;
pub mod suggest;

pub use analyze::{Analysis, analyze};
pub use candidate::{Candidate, Group, Kind, Site, ValueType};
pub use generate::{GenerateError, Generated, Options, authored_controls, generate, is_authored};
pub use output::{OutputError, write_scene};
pub use report::Report;
pub use score::ScoreOptions;
