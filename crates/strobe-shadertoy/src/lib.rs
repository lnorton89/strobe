// ============================================================================
// Module:       strobe_shadertoy (library crate root)
// Description:  Shadertoy import: id parsing, the shader JSON model, dialect
//               normalisation, the converter to a Strobe scene folder, and
//               acquisition (REST client, texture downloader, listener).
//
// Dependencies: serde, serde_json, thiserror, sha2, getrandom; strobe-scene
//               (validation); ureq (feature `net`)
// ============================================================================

//! Shadertoy to Strobe.
//!
//! Layers, kept apart so each can change without the others
//! (AGENT_RESEARCH section 9):
//!
//! 1. [`id`]: a pasted URL or bare id becomes a [`ShaderId`].
//! 2. [`acquire`] and [`listener`]: getting the JSON and the textures. The
//!    REST API with a key (path B), the texture downloader and its cache,
//!    and the localhost listener the browser userscript posts to (path A).
//! 3. [`model`]: the JSON as typed Rust, in either of its two shapes.
//! 4. [`convert`]: the deterministic converter from that model to a Strobe
//!    scene folder, with [`dialect`] rewriting the shader text and
//!    [`glsl`] the tokenizer both lean on.
//!
//! [`import`] is the pipeline that joins them: JSON text in, scene folder
//! in the library out. Every route (a dropped file, an id, the userscript,
//! `strobe-import`) ends in [`import::import_json`], so there is one place
//! where a re-import is detected, textures are fetched and the folder is
//! written.
//!
//! ## Acquisition is separate from conversion
//!
//! Shadertoy is behind Cloudflare, so a server-side fetch of a page does
//! not work; the routes are the REST API with a key for shaders published
//! "Public + API" (path B), a userscript in the person's own browser POSTing
//! to a loopback listener (path A), and an exported file (path C). All end
//! in a JSON document, which [`model::Shader::from_json`] reads, and
//! everything after that point works offline from a file.
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

pub mod acquire;
pub mod convert;
pub mod dialect;
pub mod glsl;
pub mod id;
pub mod import;
pub mod listener;
pub mod model;

pub use convert::{Converted, Options, convert};
pub use id::{IdError, ShaderId};
pub use model::{ModelError, Shader};
