# strobe-ssf-lib

The Synesthesia Shader Format (SSF) GLSL function library: `_scale`,
`_noise`, `_fbm`, `_rotate`, `_hsv2rgb`, `_palette`, `_loadMedia`,
`_edgeDetect` and the rest of the documented helpers, re-implemented from
their documented signatures and behaviour. The crate is one `.glsl` file
and one constant that embeds it as text.

## Role in the workspace

- **Depends on:** nothing, not even another Strobe crate.
- **Depended on by:** `strobe-scene`, whose shader assembler
  (`strobe_scene::assemble`) places the library ahead of every scene's
  `main.glsl`. The renderer reaches it only through that assembled text.
- **Does not:** parse scenes, touch GL, or compile anything itself. It is
  text. It relies on declarations that the assembler emits before it
  (`RENDERSIZE`, `_xy`, `_uv`, `syn_Media`, `syn_MediaType`), so the file
  is not a standalone shader.

## Main types / entry points

| Item | What |
|---|---|
| `strobe_ssf_lib::LIBRARY` | `&str`, the whole of `src/ssf.glsl` via `include_str!`. The crate's only public item. |
| `src/ssf.glsl` | The functions, grouped as math, hashes, noise, coordinates, colour, media, edges and tilt shift. Private helpers are prefixed `strobe_` so they cannot collide with a scene's names. |

## Usage

The only consumer is the assembler, which appends the constant to the
shader text it builds (`crates/strobe-scene/src/assemble.rs`):

```rust
let glsl: &str = strobe_ssf_lib::LIBRARY;
```

To see the library in a finished shader, read the tests in
`crates/strobe-scene/src/assemble.rs`, or the `the_ssf_library_compiles_in_both_dialects` test in
`crates/strobe-gfx/tests/render.rs`, which calls a function from every
family.

## Testing

```bash
cargo test -p strobe-ssf-lib
```

The two unit tests in `src/lib.rs` are cheap guards, not compiler runs:

- `every_documented_function_is_defined` checks that each of the 63
  documented names listed in the test appears as a function definition in
  the source text.
- `no_float_literal_lacks_a_decimal_point_in_a_float_context` greps for a
  few integer-literal patterns (`* 2)`, `+ 1;`, ...) that would break the
  GLSL ES dialect.

Compilation under a real GLSL compiler happens in `strobe-gfx`:

```bash
apt-get install -y libegl1 libegl-mesa0 mesa-utils      # headless Mesa llvmpipe
STROBE_REQUIRE_HEADLESS_GL=1 cargo test -p strobe-gfx --test render
```

Without a headless EGL device those tests print a skip and pass; the
variable turns a missing driver into a failure (CI's Linux job sets it).

## Design notes

- **One source, two dialects.** The same text compiles as desktop
  `#version 330 core` and as GLSL ES 3.00 (the ANGLE path), so every float
  literal has a decimal point, no built-in is redeclared, and loop bounds
  are constant. See D2 and D6 in
  [`DECISIONS.md`](../../docs/DECISIONS.md).
- **Behaviour, not bit-exactness.** The contract is ranges, shapes and
  argument order. Exact numerical output differs from Synesthesia's own
  implementation, and the renderer's tests lock Strobe's output instead.
- **Re-implemented, not copied.** `src/lib.rs` says the functions are
  re-implemented from their documented signatures and behaviour.
- **Compiled into every program.** The assembler appends the library to
  each pass's program, and D7 makes that one program per pass by default,
  so the text is compiled once per pass.

## Status and known gaps

- Every documented function name is defined and compiles on GL 3.3 core
  and GLES 3.0 (headless Mesa), per [`ROADMAP.md`](../../docs/ROADMAP.md).
- That is llvmpipe and CI only. The library has not been compiled by every
  real vendor driver. The T480 ran the scene library through the app on
  an Intel UHD 620 ([`T480.md`](../../docs/hardware/T480.md)), which is the
  only real-GPU evidence.
- Open question Q1 and Q10 in
  [`OPEN_QUESTIONS.md`](../../docs/OPEN_QUESTIONS.md) concern the layout of
  `syn_Spectrum` / `syn_LevelTrail`, which the assembler declares, not this
  file.
- ROADMAP's "Known gaps" notes that `_loadMedia` and friends read
  `syn_Media`, which is a black 1x1 texture today (media input is phase 7).
- Q8: SSF code that relies on desktop-only implicit conversions is not
  rewritten for the GLES dialect; that is a limit of the ANGLE path, not of
  this library, which is written ES-clean.
