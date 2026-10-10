// ============================================================================
// Module:       strobe_shadertoy::acquire
// Description:  Layers (a) and (c): getting a shader's JSON (the REST API)
//               and its textures (the downloader and cache).
//
// Dependencies: sha2; ureq (only with the `net` feature)
// ============================================================================

//! Acquisition.
//!
//! Getting bytes is kept apart from converting them (AGENT_RESEARCH
//! section 9): nothing here knows what a pass is, and the converter never
//! touches a socket. The pieces:
//!
//! | Piece | What | Where |
//! |---|---|---|
//! | [`Fetch`], [`Endpoints`] | the HTTP seam and Shadertoy's addresses | [`fetch`] |
//! | [`UreqFetch`] | the real client (feature `net`) | [`http`] |
//! | [`ApiKey`] | the API key, from the environment or a key file | [`key`] |
//! | [`fetch_shader_json`] | path B: the REST API | [`rest`] |
//! | [`download`], [`AssetCache`] | textures, with a SHA-256-keyed cache | [`assets`] |
//!
//! The browser route, path A, is the localhost [`crate::listener`]; the
//! dropped-file route, path C, needs no acquisition at all. Every route ends
//! in the same JSON text, which [`crate::import`] converts and writes.
//!
//! The `net` feature gates only the client. With it off (the Windows
//! cross-check from Linux, which cannot build `ring`) everything else still
//! compiles and is tested against fakes.

pub mod assets;
pub mod fetch;
#[cfg(feature = "net")]
pub mod http;
pub mod key;
pub mod rest;

pub use assets::{AssetCache, CACHE_DIR, DownloadFailure, Downloaded, download, sha256_hex};
pub use fetch::{
    API_TIMEOUT, ASSET_TIMEOUT, BASE_ENV, DEFAULT_BASE, Endpoints, Fetch, FetchError, Limits,
    MAX_ASSET_BYTES, MAX_SHADER_BYTES, redact_key,
};
#[cfg(feature = "net")]
pub use http::UreqFetch;
pub use key::{ApiKey, KEY_ENV, KEY_FILE, KEY_PAGE, KeyError};
pub use rest::{RestError, fetch_shader_json};
