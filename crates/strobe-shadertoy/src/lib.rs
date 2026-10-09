// ============================================================================
// Module:       strobe_shadertoy (library crate root)
// Description:  Shadertoy import: id parsing, the shader JSON model, dialect
//               normalisation, and the converter to a Strobe scene folder.
//
// Dependencies: serde, serde_json, thiserror; strobe-scene (validation)
// ============================================================================

//! Shadertoy to Strobe.
//!
//! Four layers, kept apart so each can change without the others
//! (AGENT_RESEARCH section 9):
//!
//! 1. [`id`]: a pasted URL or bare id becomes a [`ShaderId`].
//! 2. Acquisition: getting the JSON. Not built yet (see below).
//! 3. [`model`]: the JSON as typed Rust, in either of its two shapes.
//! 4. [`convert`]: the deterministic converter from that model to a Strobe
//!    scene folder, with [`dialect`] rewriting the shader text and
//!    [`glsl`] the tokenizer both lean on.
//!
//! ## Acquisition is a separate, later layer
//!
//! Shadertoy is behind Cloudflare, so a server-side fetch of a page does
//! not work; the planned routes (PLAN section 6.1) are the REST API with a
//! key for shaders published "Public + API" (path B), and a userscript in
//! the person's own browser POSTing to a localhost listener (path A).
//! Neither is built. Both end in the same place, a JSON document, which
//! [`model::Shader::from_json`] reads, and everything after that point
//! works offline from a file. The `strobe-import` binary says so when
//! asked for an id.
//!
//! ## What the converter promises
//!
//! - **Determinism.** The same JSON and options give byte-identical files.
//!   Nothing reads a clock, a random number, or the order of a hash map.
//! - **The original code is not edited.** It is normalised by [`dialect`]
//!   (a few renames, some deletions that keep line numbers) and surrounded
//!   by a generated shim; no statement of it is rewritten.
//! - **Honesty about gaps.** What cannot be converted is an error naming
//!   the pass and channel; what is converted with a loss (a video input
//!   bound to black) is a [`convert::Warning`].

pub mod convert;
pub mod dialect;
pub mod glsl;
pub mod id;
pub mod model;

pub use convert::{Converted, Options, convert};
pub use id::{IdError, ShaderId};
pub use model::{ModelError, Shader};
