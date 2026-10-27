// ============================================================================
// Module:       strobe_scene (library crate root)
// Description:  The Synesthesia Shader Format, modelled: scene.json, controls
//               and their per-frame runtime, and the shader assembler.
//
// Dependencies: serde, serde_json, thiserror; strobe-ssf-lib (GLSL library)
// ============================================================================

//! The Synesthesia Shader Format (SSF), as data.
//!
//! A scene is a folder ending in `.synScene` holding `main.glsl` (a
//! fragment shader with `vec4 renderMain(void)` as its entry point),
//! `scene.json` (title, controls, passes, images), and optionally
//! `script.js`. This crate turns that folder into checked Rust values and
//! knows everything about it that does not need a GPU:
//!
//! - [`json`] reads `scene.json` as written — tolerant of what real scenes
//!   contain (a number where an array is allowed, keys Strobe does not use).
//! - [`scene`] validates it into a [`SceneDef`]: every control typed, every
//!   name resolvable, every pass sized. A scene that fails here never
//!   reaches the renderer, and the error names the control or pass at fault.
//! - [`control`] is the control runtime — smoothing, speed integration,
//!   bangs, counters — run once per frame on the render thread.
//! - [`assemble`] builds the GLSL the driver actually compiles: version
//!   header for the context's dialect, the standard and audio uniforms, the
//!   SSF function library, the user's code, and the `main` that calls
//!   `renderMain`.
//! - [`fixups`] are the GLSL ES rewrites for desktop-only SSF idioms, and
//!   [`promote`] is the type-aware one: every implicit int→float
//!   conversion, written out, through function-like macros on ES, and a
//!   sampler chosen with `?:` moved outside its call.
//! - [`directives`] reads an `#if` on a name that is not a macro as `0`,
//!   on every driver, as NVIDIA's compiler does.
//! - [`include`] expands `#include` against the scene folder and the GLSL
//!   library folders when a scene loads.
//! - [`meta`] is the output-wide meta control bank (master, brightness,
//!   hue, mirror...), an ordinary [`control::ControlBank`].
//! - [`pin`] decides whether a scene's final pass is pinned to its frame
//!   size, because its shader reads a root-sized pass texel by texel (D280).
//! - [`names`] is the name matching the JavaScript API relies on:
//!   case-insensitive, ignoring `_`, `-` and spaces.
//! - [`tags`] reads a scene's `TAGS` leniently, normalises them, and
//!   writes them back without disturbing the rest of `scene.json`.
//! - [`transition`] holds the live values of a scene's `HARD_TRANSITIONS`
//!   and `SMOOTH_TRANSITIONS`, which change on a song change (D489).
//! - [`synesthesia`] finds the person's own Synesthesia install and the
//!   scene folders in its settings, for the `#include` path and the app's
//!   library (D271).
//!
//! Nothing here touches GL, the filesystem beyond [`load`], the
//! [`include`] it runs, and the folder checks and settings files
//! [`synesthesia`] reads, or a clock:
//! `dt` is passed in. That is what lets every rule above be tested on any
//! machine without a window.

pub mod assemble;
pub mod audio;
pub mod control;
pub mod date;
pub mod directives;
pub mod error;
pub mod fixups;
pub mod include;
pub mod json;
pub mod json_edit;
pub mod load;
pub mod meta;
pub mod names;
pub mod pin;
pub mod preset;
pub mod promote;
pub mod scene;
pub mod synesthesia;
pub mod synthetic;
pub mod tags;
pub mod transition;

pub use error::SceneError;
pub use load::SceneFolder;
pub use scene::SceneDef;
