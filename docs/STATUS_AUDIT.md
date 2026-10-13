# Strobe status and gap audit

_Last reconciled: 2026-10-12 against the current roadmap and the comprehensive
[implementation backlog](IMPLEMENTATION_BACKLOG.md). The evidence entries below retain their original dates and machine context; do not interpret this reconciliation date as a fresh rerun of every test._

This is an implementation handoff, not a claim that every platform path
is verified. "Implemented" means the code is merged and tested in CI.
Hardware-dependent behaviour stays gated on the evidence listed here.
Ticked items cite where the evidence is. [ROADMAP.md](ROADMAP.md) carries
the short priority order; [IMPLEMENTATION_BACKLOG.md](IMPLEMENTATION_BACKLOG.md)
tracks every known unfinished implementation and validation task across the project.

## Current baseline

The Windows desktop app wires together:

- the control and output windows on one GL context;
- scene loading;
- audio capture with a synthetic fallback;
- QuickJS scene scripts;
- diagnostics and crash monitoring;
- GPU-timed adaptive render scale;
- context-loss recovery with ANGLE fail-over.

The benchmark uses a window context where EGL is unavailable. Every crate
is now wired into the app: the Shadertoy importer (stage 2) and the playlist
core (stage 2, D150-D158). `strobe-autocontrol` is not.

For the deeper contracts, read [ARCHITECTURE.md](ARCHITECTURE.md),
[DECISIONS.md](DECISIONS.md) and [BENCHMARKS.md](BENCHMARKS.md).

## P0: real Windows hardware verification

Linux and llvmpipe cannot prove any of these:

- WGL or ANGLE behaviour;
- WASAPI lifecycle;
- Windows crash-monitor IPC;
- integrated-GPU timings.

- [x] Launch on the T480 with native GL, and record the adapter, driver,
  backend and startup log ([hardware/T480.md](hardware/T480.md)).
- [x] Force ANGLE and verify the same scene corpus loads (T480, all
  scenes).
- [ ] Exercise audio absent, present, removed and reconnected, and verify
  the UI and log state transitions. Live capture is verified so far;
  removal and reconnection are not.
- [x] Benchmark scenes at fixed resolution, recording GPU frame time,
  render scale, backend and driver, with CPU time kept separate
  ([BENCHMARKS.md](BENCHMARKS.md), 2026-10-09).
- [x] Run the crash demo from an ordinary terminal and verify a minidump
  is created (owner, 2026-10-09; bug 2 in T480.md). An agent inside the
  Claude desktop app cannot do this at the default location (MSIX
  virtualisation).
- [x] Symbolicate that dump with the matching PDB. Verified on the T480
  (R5): function names and `file:line`, via `cargo xtask symbolicate`.
- [ ] Test the packaged release executable separately (Q12).
- [ ] Run for at least 30 minutes with repeated scene switches, and
  inspect for crashes, log growth and resource stability.
- [x] Keep commands, environment, logs, screenshots and failed outcomes in
  [hardware/T480.md](hardware/T480.md).

## P0: context-loss recovery

Diagnostics are not recovery. The bounded state machine is built (D66):

- it detects make-current, swap and reset failures;
- it stops GL calls against a lost context;
- it recreates the native context, and tries ANGLE after repeated native
  failures;
- it rebuilds both windows and reloads the active scene;
- it emits a structured `CONTEXT` record with the backend, error and
  reset status;
- its retries are bounded, and terminal failure is clear.

- [x] Deterministic fault injection (`STROBE_FAULT`) covers the state
  transitions, and the retries are bounded (22 policy tests, plus Xvfb
  runs).
- [x] At least one real Windows attempt documented. On the T480 (Task 8),
  an injected loss recovered on native in 181 ms and failed over to ANGLE
  in 171 ms, and the give-up path ended cleanly
  ([hardware/T480.md](hardware/T480.md), bug 1). No real driver reset has
  been observed yet: bug 1 no longer reproduces.

## P1: performer-facing UI

The egui UI was an engineering shell and is now at stage 1 of Contextual Perform (evidence below). The research conclusion was
**Contextual Perform**: a persistent preview and output status, transport
and cues, performance macros and health. The scene browser and inspector
are contextual surfaces.

- Keep authoritative runtime state out of egui widget memory.
- Route actions through typed app commands; widgets should not own
  runtime side effects.
- Keep Preview separate from Load.
- Persist the browser's query, filters, sort, width and visibility as
  workflow state.
- Represent each macro target's state explicitly: resolved, unavailable,
  broken, disabled, learning and conflict.
- Keep expensive work out of paint; use caches, workers and messages.
- Treat 1280x720 as a first-class target. Test these states:
  - empty and loading;
  - failure, missing audio and script fault;
  - a narrow window.
- Keep the output visually independent from the control UI. The output
  window starts hidden; the operator shows it.

**Stage 1 (D130-D138) evidence.** Machine: the cloud container, Xvfb,
Mesa llvmpipe, native GL 3.3 core, a debug build. Screenshots are the app's
own `--screenshot` (the control window's back buffer), read by eye; the six
committed ones are in [images/perform/](images/perform/), the rest were
checked and not committed. Interaction was driven with xdotool.

- [x] Contextual Perform topology at 1280x720, 1440x900 and 1920x1080: rail,
  preview, title-bar readouts, transport, macros, optional panes
  (`perform-1440x900.png`, `perform-1280x720.png`).
- [x] 1280x720: the browser and the inspector do not both open; the preview
  stays about 880 x 495 with the strip and macros visible
  (`browser-preview-1280x720.png`).
- [x] Empty library (`empty-library-1280x720.png`), loading (a real load,
  "Preparing X..." over the live picture), scene failure with the old scene
  still playing (`scene-failure-1440x900.png`), a scene that fails at start,
  missing audio (`No audio input`, warning), synthetic audio, script fault and
  disabled (in the title bar and the inspector), output hidden, blackout (the
  output window's capture is black), a context rebuild notice
  (`STROBE_FAULT=lose-context@20`), a 900x600 window (rail labels and
  readouts shrink) and a 720x480 window (the pane becomes a drawer).
- [x] Preview separate from Load: a previewed scene shows in the browser and
  the live scene, controls and output are unchanged; a failed preview marks the
  row and shows a banner while the old scene plays.
- [x] Pane visibility, widths, order and the browser query persist: a
  splitter drag and a typed query were written to `ui.toml` and restored by a
  second launch.
- [x] Keyboard: Right, Left, B, F11, Ctrl+F, Esc; none fires while the search
  field has focus (typing "kal", then Right, changed nothing).
- [x] Command palette (Ctrl+K, D200): runs a command by name, opens panes,
  keeps unavailable commands listed with the reason, Esc and a second Ctrl+K
  close it, and typing into it fires no shortcut. Evidence: headless egui
  tests in `ui::shell_tests` that drive the real shell with key events, and
  the Xvfb screenshot `docs/images/perform/palette-1440x900.png`. **Not
  yet:** used by a person on Windows.
- [x] Themes: every built-in theme passes the WCAG tests; the high-contrast
  and light themes were looked at.
- [x] Every state at six window sizes fits its window, as headless egui tests
  (`ui::shell_tests`).
- [ ] The custom title bar's drag, Aero Snap, resize and DPI behaviour, a
  maximised undecorated window against the taskbar, and GNOME Wayland: need a
  window manager (T480 / real Windows; D132).
- [ ] The UI's frame cost and a Preview's GPU cost on the T480 (the layout
  pass was about 1 ms in a debug build on llvmpipe; the paint there is
  software rasterisation and says nothing).
- [ ] Performance Macros as persistent bindings with the Unresolved states;
  Editor and Mappings; a command palette (stage 2). (Hold and the Playlist
  pane are built, D156 and D157.)

Read [UI_DESIGN.md](UI_DESIGN.md),
[UI_IMPLEMENTATION_PLAN.md](UI_IMPLEMENTATION_PLAN.md),
[UI_MOCKUP_WORKFLOW.md](UI_MOCKUP_WORKFLOW.md) and
[the four-direction mockup](mockups/strobe-perform-directions.html). Choose
the topology before any broad redesign.

## P1: SSF and script compatibility

- [x] Script writes to `meta/` controls reach a meta `ControlBank` (D160).
  Evidence, llvmpipe: `strobe-script` conformance tests `meta_*` (routing,
  clamping, toggles, default, randomize, groups, unknown names) and
  `strobe-app/tests/script_meta.rs` (a session, both dialects).
  (The effect on screen is the next item.)
- [x] An output post stage applies the meta controls (D180-D184). Evidence,
  llvmpipe, both dialects, `crates/strobe-gfx/tests/post.rs`: a neutral pass
  through the program is the input bit for bit; `apply()` draws nothing at
  neutral; invert, hue (red to green to blue at 1/3 and 2/3 of a turn),
  saturation 0 (luma grey), brightness, contrast, gamma, master 0 (black,
  even with invert) and master 0.5 on known colours; horizontal and vertical
  mirrors exact on odd and even sizes and a true reflection of a gradient;
  no shader compiles after the stage is built. `MetaValues` (neutral
  detection, slots) in `strobe-scene/src/meta.rs` tests; the audio gain
  product in `strobe-app` `audio::tests`. **In the app, Xvfb + llvmpipe,
  native GL 3.3:** a script writing `meta/hue`, `saturation`,
  `horizontal_mirror` and `brightness` turned the orange aurora green,
  mirrored it left onto right and brightened it; another writing
  `meta/master` 0.5 and `meta/invert` gave the inverted picture at half
  level; the preview and `--screenshot` output both show it, and the
  inspector shows the "Output (meta)" controls
  (`docs/images/post/`). On the T480 (Task 15, 2560x1440 fullscreen, UHD
  620): a scene with hue, brightness and a mirror ran at 60 fps and about
  2.9 ms of GPU, against 1.8 ms for the same scene untouched, so the stage
  costs about **1.1 ms** native. On ANGLE it was 3.1 ms, and the picture
  was visibly hue-shifted and mirrored. **Not verified:** an HD 4000, where
  1.1 ms at 1440p may double (D180's fallback is to fold the stage into the
  present blit when the output is shown), and the guessed order and mirror
  conventions against Synesthesia (Q21).
- [x] `textureFinalPass` through an asynchronous PBO readback (D162, Q14).
  Evidence, llvmpipe, both dialects: `render.rs`
  `the_final_pass_readback_arrives_two_frames_late_and_is_black_before`,
  `the_downsample_keeps_the_pictures_orientation_and_average`,
  `a_busy_gpu_costs_freshness_never_a_stall_and_the_ring_recovers`,
  `resizing_restarts_the_readback_from_black`,
  `a_lost_context_makes_the_readback_inert_and_its_drops_harmless`; and
  `script_uniforms.rs` `texture_final_pass_reads_the_picture_a_few_frames_late`.
  **Not yet:** the latency assertion on the T480. `script_uniforms.rs`
  skips there because Windows has no headless EGL device (Task 14). The
  scripted scene runs clean on Intel native, but the fence behaviour and
  cost on Intel and ANGLE/D3D11 are unmeasured. Running these tests on the
  window context (D140) would close this.
- [x] A script-created uniform that appears after initial preparation is
  refused, and no shader compiles after load (D161). Evidence:
  `script_uniforms.rs`
  `a_uniform_that_appears_later_is_refused_and_nothing_compiles_after_load`
  counts compiles through `Gl::shader_compiles()` in both dialects.
- [ ] The benchmark does not execute scripts. Keep this limitation
  explicit.
- [x] GLES translation of desktop-only SSF idioms (R8, D199): implicit
  int-to-float, `#if` on a non-macro, initialised uniforms. Evidence:
  `render.rs` `implicit_int_to_float_and_non_macro_if_compile_in_both_dialects`
  checks values on both dialects; the owner's 90-scene corpus compiles 87
  on GLES (17 before) and every deterministic desktop frame is
  bit-identical with the rewrite on (llvmpipe). **Not yet:** re-run on
  ANGLE/D3D11 on TORNADO or the T480.

*Acceptance:* a fixture per behaviour, tests on both shader dialects where
they apply, and explicit supported, degraded or unsupported claims.

## P1: scene assets, Shadertoy and auto-generated controls

- [x] `syn_Media` and `syn_MediaType`: a documented media path for stills
  and image sequences (D186). Evidence, llvmpipe, both dialects,
  `crates/strobe-gfx/tests/assets.rs`: a still read upright through
  `syn_Media` with `syn_MediaType` 1; no media is black and type 0; a
  3-picture sequence at 1 fps shows frame 1, 2, 3 and loops; a user-chosen
  file replaces the scene's and a bad one reports and leaves the old picture;
  an empty folder reports. `assets::tests` cover ordering, flipping, shrinking
  and the decoder thread. Under Xvfb, `--media <folder>` played a two-picture
  sequence and `--media <missing>` printed the IO failure block and ran on.
  **Video and webcam are not built (later, ROADMAP).** **Not verified:** the
  `MEDIA` key and `syn_MediaType` values against SSF (Q22); upload cost and
  memory on the T480.
- [x] Load cubemaps, and report missing or invalid assets without
  crashing the show (D185). Evidence, llvmpipe, both dialects: a fixture
  samples +X, -X, +Y, -Y, +Z and -Z and gets each face's colour from six
  files and from a 4:3 cross picture; a missing cube map and a mismatched
  one each return an `AssetProblem` naming the sampler and file, read black,
  and raise no GL error (`assets.rs`); `failures::asset_problem` has a test
  for the failure block. **Not verified:** cube orientation against a real
  scene (Q22); a missing or broken `IMAGES` file still fails the whole scene
  load, as before.
- [x] Shadertoy conversion (stage 1, D90-D95). It covers:
  - multipass dependencies in Shadertoy order, and buffer feedback;
  - texture and buffer channels;
  - a live audio texture, `iDate` and the "Strobe" control group;
  - token-level dialect rewriting;
  - attribution and licence metadata in `SOURCE`.
  
  Nine original fixtures render on both dialects.
- [x] Shadertoy acquisition (stage 2, D110-D117), against fakes. *Evidence:*
  `cargo test -p strobe-shadertoy` (REST, downloader, cache, pipeline and
  listener, with a loopback mock Shadertoy and no live site); under Xvfb
  (llvmpipe, native GL, 2026-10-10) a shader POSTed to the real listener by
  `tools/userscript/test/run.mjs` was imported, selected and rendered, a
  fixture path typed into the Import panel imported and loaded a four-buffer
  scene, a second import showed Replace / Cancel and Replace swapped it, and
  an id with no key showed the "no Shadertoy API key" failure.
- [x] The import listener on Windows (T480, Task 11). It binds loopback
  only; the responses are 200, then 409 for a re-import, and 401 for a bad
  token; the imported scene loads.
- [ ] Shadertoy against the live site. The REST API answered the T480's
  plain HTTP client with a Cloudflare 403 (Q19), so the browser userscript
  is the route to prove. *Accept:* one real import from the button in the
  owner's browser, and one by id if the API can be reached at all, each
  recorded in [hardware/T480.md](hardware/T480.md).
- [x] Auto-generated controls on Windows (T480, Task 13, D140).
  `--context window` scores on Intel UHD 620 in 0.55 s (8 candidates, 7
  kept), and two runs give byte-identical reports. `auto` falls back to the
  window, and says so. Native and ANGLE keep the same 7 controls in the
  same order, with a largest score difference of 0.00008.
- [ ] A real file drop (X11 and Windows). The typed path runs the same job.
- [ ] Thumbnails, conversion diagnostics beyond the warnings list, and the
  Needs-work bin.
- [x] Auto-generated controls engine, stage 1 (D123-D126;
  [ROADMAP.md](ROADMAP.md), "Auto-generated controls"). Evidence:
  `crates/strobe-autocontrol/tests/engine.rs` on headless llvmpipe, both
  GL 3.3 core and GLES 3.0. Every library scene with `CONTROLS` stripped
  (11 of 11) and 8 of 14 Shadertoy fixtures get controls; the other 6 are
  structural and documented in `NO_CONTROLS`. Each kept control changes the
  image (scored, then re-checked on the finished scene), the rewrite
  reproduces the original at its defaults (fidelity 0 in all 25), a second
  run is identical. **Not verified:** a real GPU or the T480 (llvmpipe only),
  and the Import/Inspector review panel does not exist, so nothing in the app
  calls the engine yet.
- Preserve source and licence metadata, and make it inspectable. Never
  infer commercial rights.

*Acceptance:* representative single-pass and multipass imports work on
native GL and on ANGLE, missing assets fail visibly, and attribution
survives export.

## NVIDIA (TORNADO)

- [x] Every library scene and the nine Shadertoy fixtures compile and
  render on NVIDIA's own GLSL compiler and on ANGLE/D3D11 on NVIDIA, with
  matching contact sheets (GTX 1080 Ti, driver 582.78, Task T1,
  [hardware/TORNADO.md](hardware/TORNADO.md)). No NVIDIA-only compile
  differences showed up on this set.
- [x] The driver's KHR_debug output has no recompile or state warnings,
  only buffer-usage notices (131185) and the expected screenshot readback
  sync (131154).
- [x] A simulation's look no longer depends on the output size (D191);
  verified on llvmpipe at 720p and 1440p. **Not yet:** re-checked on
  TORNADO at 2160p.
- [ ] Owner-supplied real Shadertoy shaders through NVIDIA's compiler.
- [~] **Real Synesthesia scenes** (65 of the owner's, T2): 47 compile on
  native and 35 on ANGLE at commit 963b535, with failures grouped by cause in
  [hardware/TORNADO.md](hardware/TORNADO.md). D193 and D194 fix the shape and
  size causes (CRLF, missing `TYPE`, 0-sized passes, `MEDIA` slots,
  multi-name globals, absolute data passes). Open: Synesthesia's user-media
  library (8 scenes), and implicit int to float and non-macro `#if` on ANGLE
  (12, Q8). Re-run at 40ee040 (T4): 55 native, 38 ANGLE. D199 addresses the
  ANGLE causes and the library gaps T4 found; **not yet re-run** after it.

## P2: playlists, show control and release

- [x] Playlist inheritance (defaults, then section, then cue), the
  generator, the timeline, the playback clock with beat-locked ends, and
  resume. This is the pure core (D73-D77), with 4-hour soaks.
- [x] Playlist app integration (D150-D158), on llvmpipe under Xvfb and on
  headless GL in both dialects. Evidence:
  - precompile before cue activation, one scene per control frame, with
    progress and per-cue skipped state: the log of
    `--playlist playlists/transition-demo.toml` (`compiled ... (3/5)`,
    `cue 3 (ghost-scene) skipped`) and
    [`docs/images/perform/playlist-1280x720.png`](images/perform/playlist-1280x720.png);
  - zero shader compiles after the load: `closed with 0 runtime shader
    compile(s)` in every run, and `crates/strobe-app/tests/show.rs` compares
    `Gl::shader_compiles()` before and after a whole show;
  - warm-up and transitions: the same log (`warming up cue 2 ... at
    480x270`, `transition 0 started ... FadeThroughBlack`), the mixed frames
    in screenshots at `--frames 330`, `660` (a custom wipe,
    [`playlist-wipe-output.png`](images/perform/playlist-wipe-output.png)) and
    `1260` (a crossfade), and `tests/compositor.rs` (crossfade at 0, 0.5 and 1
    gives A, the mix and B; feedback survives `resize_keeping`);
  - Hold, Next, Extend, Restart, Previous: `tests/show.rs`; the strip in the
    screenshots (`-00:03 FADE 58%`);
  - resume: stop mid-show and restart, `playlist: resumed the saved
    position`; `--no-resume`: `starting at the first cue`;
  - the wall-clock anchor: an anchor a minute in the past starts on the
    scheduled cue (`anchored: ...`);
  - the file watcher: saving a longer file logs `playlist file changed:
    reloading` and compiles only the new scene; saving a broken one keeps
    the playing playlist and shows `line 3, column 13: "3mn" is not a
    duration ...`.
  - on the T480 (Task 14, release, native GL): the same demo closed with 0
    runtime compiles, loaded in about 0.27 s, and ran its transitions at
    60 fps, with GPU rising from 0.4-0.8 ms to about 1.2-1.3 ms during each
    ([hardware/T480.md](hardware/T480.md)).
  **Not verified:** an HD 4000 (the cost of two scenes at once, VRAM), ANGLE
  on the T480, a 4-hour show with real scenes, and audio-event advance (not
  built).
- [~] Wire presets, MIDI, OSC and hotkeys into the shared `ControlBank`.
  Don't duplicate its toggle, bang or smoothing semantics. (D170-D179,
  2026-10-11.) **Verified, with evidence:**
  - one control model: the mapper writes only `ControlBank::set_normalized`,
    `set` and `press`; `strobe-io` tests pin that a mapped smoothed slider's
    target moves at once and its output eases, that a pad flips a toggle or
    follows it, that a bang counts one press per press, that a dropdown wraps;
  - **OSC end to end**, real UDP into the running app under Xvfb
    (`--osc --osc-port 9477`, datagrams hand-built in `python3`): the log has
    `OSC: listening on 127.0.0.1:9477`, then `/strobe/control/hue_shift 0.800:
    applied`, `/strobe/meta/hue 0.500: applied`, `/strobe/cmd/toggle_blackout
    1.000: applied` with `blackout on` and later `blackout off`,
    `/strobe/cmd/next_scene 1.000: applied` with `Loaded 02-plasma-pulse`,
    `/strobe/control/nope 0.500: unavailable: "nope" is not a control of the
    scene that is playing`, `show control: there is no command
    "not_a_command"`, and `/1/fader1 0.500: unmapped`. The macro strip in the
    screenshot moves to `0.80` and `0.50` from the same messages;
  - **Learn in the real UI**: an `xdotool` click on a macro's Learn button
    (the card says "move a control to bind it", the banner says Learning), then
    `/9/fader 0.300` logged `learned /9/fader -> macro #1`, the mapping was
    written to `show-control.json`, and `/9/fader 0.900` then moved the control
    ([`map-learning-1440x900.png`](images/perform/map-learning-1440x900.png) is
    the armed state, with Learning, Conflict and Resolved rows; Unavailable, Disabled and Conflict rows were read in a taller capture, and Broken is drawn in the headless layout tests but was not read in a screenshot);
  - **MIDI parsing** from recorded byte sequences (a fader sweep, a pad, a
    wheel, a patch change, clock ticks, sysex, malformed and truncated), and
    the worker starting cleanly where there is no MIDI service (this container
    has no `/dev/snd/seq`: `snd-virmidi` cannot be loaded here, so a virtual
    port was not possible);
  - **presets**: capture and apply through the bank, bad names refused, a
    cue's `preset` applied on cue start with an override beating it and a
    missing preset falling back to `default.json`, with zero shader compiles
    (`crates/strobe-app/tests/show.rs`, headless GL, both dialects);
  - hotkeys: parsing, clash and unknown-command rejection, and the D134 focus
    rule over rebound keys (unit tests);
  - `cargo deny` clean; the Windows cross-check builds `strobe-io` with
    `midir` (no C compiler needed, D170).
  **Not verified:** any MIDI hardware (port names, hot-plug timing, a device
  held by another program on WinMM, message rates), Windows or the T480 for any
  of it, OSC from another machine (`--osc-all-interfaces` was only bound, never
  reached from a LAN), and a person's use of Learn with a controller.
- [ ] Run a tag-triggered release, then inspect:
  - the zip and the ANGLE DLL provenance;
  - the matching PDB archive;
  - the SBOM, checksums and attestations.
- [ ] Document a clean-machine install, a smoke test, and the
  symbolication steps. Portable-crate CI is not a working macOS app; that
  still needs a suitable GL context provider.

## Regression traps

- `FrameInputs` and `strobe_scene::audio::SCALARS` are authoritative
  contracts; avoid parallel representations.
- Audio, scripts, playlists and input crates must not depend on
  `strobe-gfx`.
- Never put any of these in the frame or paint path: blocking device
  enumeration, shader compilation, heavy I/O, or synchronous GPU readback.
- "Works under llvmpipe" does not prove Windows driver behaviour.
- Keep benchmark frames and timing metadata together. llvmpipe CPU
  timings are not integrated-GPU bandwidth measurements.
- Crash symbols must match the exact executable build.
- On the T480, an agent's crash drill must set `STROBE_DIAG_DIR` inside the
  repo, because the Claude desktop app's MSIX package context breaks the
  AF_UNIX socket at the default location.

## Handoff sequence

1. Read [ROADMAP.md](ROADMAP.md) and
   [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).
2. Reproduce the baseline before editing.
3. Make one bounded change, with a regression test.
4. Run `cargo xtask ci` and the relevant backend or hardware test.
5. Update the roadmap and the decision records in the same change.
6. Record evidence and uncertainty. Do not turn "compiled" into
   "verified".
