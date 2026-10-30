# strobe-web

The engine behind the website's shader sandbox, `/play` (D524): Strobe's own
scene player drawing into a browser's WebGL2 canvas, and its audio analyser,
compiled to WebAssembly.

The page (`site/src/components/play/`) is an editor beside a canvas. It sends
a scene's files here, and this crate builds them with the same assembler and
player the app runs, in the GLSL ES 3.00 dialect the ANGLE path already
speaks, which is exactly WebGL2's. What a scene does in the sandbox is what
it does in the app, line numbers in errors included.

## Layout

- `sandbox` — the engine, plain Rust over a `strobe_gfx::gl::Gl`: load a
  scene from its files' text (a failed load keeps the last good scene),
  controls by name, the script's uniforms, three audio sources (the
  synthetic 120 BPM track, pushed live samples through `strobe-audio`'s
  `Analyzer`, silence), and a frame.
- `web` (wasm32 only) — the wasm-bindgen class the page calls, `Engine`, which
  opens the canvas's WebGL2 context and wraps it with `Gl::from_webgl2`.
- `tests/sandbox.rs` — the sandbox on the headless GLES 3.0 context, so what
  the page depends on is tested without a browser.

## Building

`npm run engine` in `site/` (also run by `npm run dev` and `npm run build`):

```text
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <Cargo.lock's wasm-bindgen> --locked
cargo build -p strobe-web --target wasm32-unknown-unknown --profile release-web
wasm-bindgen --target web --no-typescript --out-dir site/public/generated/engine target/wasm32-unknown-unknown/release-web/strobe_web.wasm
```

## Design notes

- **Scripts run in the page, not here.** QuickJS is C and does not build for
  `wasm32-unknown-unknown`, so `script.js` runs in a Web Worker behind a shim
  of the script API (`site/src/lib/play/script-worker.ts`), one frame behind
  the picture. This crate declares the uniforms the script made at load and
  takes their values each frame, as the app does with `strobe-script`.
- **No clock.** `std::time` panics on this target, so nothing on the
  sandbox's path reads one: time, `dt` and the wall clock come from the page.

## Status and known gaps

Open work for this crate is tracked in
[`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md) under the
IDs given here; a gap with no ID is by design, with its decision named.

- Built and run in Chromium (the desktop app's browser pane): the library's
  scenes compile and play, an edit's error marks its line, a scripted scene
  runs in the worker, share links round-trip. Other browsers and phones are
  not yet tried (backlog W-02).
- Textures and media (`IMAGE_PATH`, image inputs), `#include`, and
  `textureFinalPass` are not yet supported in the browser (backlog W-01).
- The native tests skip where there is no headless EGL, as on Windows; CI's
  Linux job runs them, but CI does not yet build the wasm engine or the site
  (backlog W-03).
- Saved scenes, accounts and a gallery are left out by design: the sandbox
  has no server, and a scene lives in the browser, a link or a zip (D524).
