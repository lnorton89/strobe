# Strobe: working notes for agents

Strobe is a Synesthesia-style VJ engine in Rust. It plays audio-reactive
GLSL scenes (the `.synScene` format, JavaScript API included), imports
Shadertoy shaders, and runs long playlists. It targets **Windows first,
on old laptops** (OpenGL 3.3 floor, roughly a 2012 Intel HD 4000), with
macOS and Linux later. Performance is a feature, not a polish step.

**Read these before changing anything:**

| Doc | When |
|---|---|
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Always. It shows what's done, what's next, and the current state of each phase. **Update it when you finish something.** |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Before adding a crate, moving code between crates, or touching the frame loop. |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Before reversing a choice. Every non-obvious decision and its reason is logged there. Add one when you make one. |
| [`docs/OPEN_QUESTIONS.md`](docs/OPEN_QUESTIONS.md) | Things not yet verified against real Synesthesia behaviour. Check here before assuming. |
| [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) | Before and after any change on a per-frame path. |
| [`docs/PLAN.md`](docs/PLAN.md) | The original design plan and its research sources. It's historical. Where it disagrees with DECISIONS.md, DECISIONS.md wins. |
| [`docs/AGENT_RESEARCH.md`](docs/AGENT_RESEARCH.md) | Before starting app, script, audio, Shadertoy, playlist, diagnostics, or performance work. It records verified upstream behaviour and integration contracts. |
| [`docs/UI_DESIGN.md`](docs/UI_DESIGN.md) | Before touching strobe-app UI. It defines the Perform-first information architecture, control UX, workspaces, theme/motion rules, and UI acceptance criteria. |

## Commands

```bash
cargo test --workspace                                    # everything
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --target x86_64-pc-windows-msvc -- -D warnings
cargo run --release -p strobe-bench -- --size 1280x720     # benchmark + frames
```

All of these are clean on the branch, and CI enforces them. A warning is
a build failure.

### Seeing what you changed without a window

`strobe-bench` renders every scene in `scenes/` on a **headless** context
and writes a PNG per scene, `contact-sheet.png`, and `results.md` to
`target/bench/`. Open the PNGs to check visuals. Frames are a pure
function of time (fixed `dt`, synthetic 120 BPM audio), so two runs give
identical images.

In a cloud container with no GPU, the context is Mesa's llvmpipe. Install
it with `apt-get install -y libegl1 libegl-mesa0 mesa-utils`. Absolute
timings are CPU rasterisation, but the ranking between scenes and the
before/after delta of a change are meaningful. `--api gles` compiles
every scene in the GLSL ES dialect that the ANGLE path uses.

The renderer tests (`crates/strobe-gfx/tests/render.rs`) compile and
render against that same headless context in both dialects. Without
EGL they print a skip and pass. Set `STROBE_REQUIRE_HEADLESS_GL=1`
(CI's Linux job does) to make a missing driver fail instead.

## Commit only what you changed

More than one agent may work here at a time. Run `git status` before
staging. Stage files by name. Never `git add -A`, `git add .`, or
`git commit -a`. Commit subjects use `feat:` / `fix:` / `refactor:` /
`docs:` / `test:` / `build:` / `perf:` / `ci:`.

Line endings are `.gitattributes`' job: LF everywhere, Windows
included.

---

## Rules the build enforces

These are inherited from the owner's `rustaman` project and enforced by
lints or by tests in `crates/strobe-conventions`.

### Every source file opens with the header banner

A ruled block naming `Module:`, `Description:` and `Dependencies:`,
then a blank line, then the `//!` docs. `Module:` must match the file's
path: `crates/strobe-scene/src/control/kind.rs` is
`strobe_scene::control::kind`, a `lib.rs` is
`strobe_scene (library crate root)`, a `main.rs` is
`strobe_bench (binary crate root)`, and `tests/x.rs` is
`x (integration test)`. Copy an existing header and fix all three
fields. The banner is plain `//`; the reasoning belongs in the `//!`
block below it.

### No `unwrap`, `expect`, `panic!`, `todo!`, `unreachable!`, not even in tests

These are denied workspace-wide. Use `?`, `let … else`, or an explicit
fallback. Tests return `anyhow::Result<()>`, and every `assert!` carries
a message. Release builds use `panic = "abort"`, and the output window
is on a projector.

### `unsafe` lives only in `crates/strobe-gfx/src/gl/`

- One named, private, **safe** function per unsafe operation. The
  `unsafe` block holds that one call and nothing else.
- Every block gets a `// SAFETY:` comment stating the actual reasoning.
  For GL calls that is the current-context invariant (`Gl` is built only
  by `gl::context` right after making a context current, and `Gl` is
  `!Send`) plus, for pointer-taking calls, the checked slice length.
- Every GL object has an owning type with a `Drop` (`Program`, `Texture`,
  `Framebuffer`, `VertexArray`, `Shader`).
- No `pub unsafe fn`, and no unsafe in tests.
  `strobe_conventions::unsafe_boundary` fails the build otherwise.

Need a new GL call? Add a leaf wrapper in `gl/mod.rs` next to its
siblings. Don't reach for `glow` from anywhere else.

### `allow` is denied; use `#[expect(lint, reason = "…")]`

### `wildcard_enum_match_arm` is denied

Name every variant. That's what catches a new control type silently
rendering nothing.

### Constants live with what they govern

There is no `constants.rs`. Use a function-local `const` first, then a
module-private one, then `pub` in the module that owns the concept.

### Per-frame paths do not allocate or query the driver

Uniform locations and sampler units are resolved at link time
(`player::build_program`). Capabilities are decided once (`caps`). If a
draw path needs a fact, cache it when the scene loads. Never call
`glGet*` in the frame loop.

---

## Things that are deliberate, not oversights

Read [`docs/DECISIONS.md`](docs/DECISIONS.md) for the full reasoning.
The short list:

- **Native OpenGL via glow, not wgpu.** naga's GLSL frontend rejects too
  much real Shadertoy/SSF code. ANGLE (GLES 3.0 on D3D11) is the
  fallback for machines with a broken WGL driver. It goes through the
  same glutin EGL code path, pointed at ANGLE's `libEGL.dll`.
- **`#version 330 core` on every desktop context.** SSF code relies on
  desktop GLSL's implicit int→float conversion (`PASSINDEX == 0.0`
  appears in real scenes), which GLSL ES rejects.
- **One program per pass, with `PASSINDEX` a compile-time constant.**
  Register allocation follows the heaviest path in a program. `--shared`
  in the bench measures the Synesthesia-faithful alternative.
- **The final pass is implicit.** `PASSES` lists buffer passes, and
  `renderMain` then runs once more with `PASSINDEX == PASSES.len()` to
  produce the output. This is confirmed against real scenes.
- **`color` controls are `vec3`.** The SSF docs say so, even though ISF
  uses `vec4`.
- **Smoothing is frame-rate independent:**
  `alpha = 1 - (1 - PARAMS)^(dt*60)`. That equals SSF's per-frame
  behaviour at 60 fps and covers the same wall-clock time at any rate.
- **Pass sizes are stored as fractions of the design resolution**, so a
  scene designed at 1080p and played at 720p keeps its passes in
  proportion.
- **The scene library in `scenes/` is original work.** Shadertoy is
  unreachable from cloud containers (Cloudflare 403), and the community
  Synesthesia scenes found so far carry no license. Don't commit
  third-party scenes without a license that allows it.
