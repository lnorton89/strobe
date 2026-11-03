// ============================================================================
// Module:       strobe_web (library crate root)
// Description:  The browser sandbox's engine: Strobe's scene player on a
//               WebGL2 canvas, compiled to wasm for the website's /play page.
//
// Dependencies: strobe-gfx (without `native`), strobe-scene, strobe-audio
//               (without `capture`); wasm-bindgen, web-sys (wasm32 only)
// ============================================================================

//! The engine behind the website's shader sandbox (D524).
//!
//! The page is an editor beside a canvas. It sends the scene's files here,
//! this crate builds them with the same assembler and player the app runs
//! (in the GLSL ES 3.00 dialect the ANGLE path already speaks, which is
//! exactly WebGL2's), and draws each frame into the canvas.
//!
//! [`sandbox`] is the engine, plain Rust over a `Gl`, tested natively on the
//! headless context. `web` (wasm32 only) is the wasm-bindgen surface the page
//! calls. Build it with:
//!
//! ```text
//! cargo build -p strobe-web --target wasm32-unknown-unknown --profile release-web
//! wasm-bindgen --target web --out-dir site/public/engine target/wasm32-unknown-unknown/release-web/strobe_web.wasm
//! ```
//!
//! which `npm run engine` in `site/` does.

pub mod sandbox;
#[cfg(target_arch = "wasm32")]
mod web;
