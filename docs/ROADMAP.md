# Roadmap and status

**Update this file whenever you finish something.** It is the first file
the next agent reads. Design detail lives in [PLAN.md](PLAN.md), decisions
in [DECISIONS.md](DECISIONS.md), the evidence-based gap list in
[STATUS_AUDIT.md](STATUS_AUDIT.md), and real-hardware results in
[hardware/T480.md](hardware/T480.md).

_Last updated: 2026-10-09._

## Now

**Owner goal:** a working, testable, benchmarkable VJ engine for older
hardware. Prefer measured improvements and explicit acceptance tests over
speculative tuning. "Compiled" is not "verified": say which machine and
which backend proved a claim.

### Implemented and integrated

| Area | What exists | Verified on |
|---|---|---|
| Workspace | Conventions as tests, pinned toolchain and actions, `cargo xtask ci \| changelog \| bench`, release workflow (D50-D59) | CI. The release workflow has never run on a tag |
| `strobe-scene`, `strobe-ssf-lib` | SSF parsing and validation, controls runtime, shader assembly, `PASS_SOURCES` (D90), the SSF GLSL library | Headless GL 3.3 core and GLES 3.0 (llvmpipe) |
| `strobe-gfx` | Native GL and ANGLE context providers, headless EGL, multipass and ping-pong, implicit final pass, images, GPU timer queries, robust contexts and reset status (D66) | llvmpipe; T480 native and ANGLE |
| `strobe-bench` | Timings, frames, contact sheet; headless, window and auto contexts (D70-D72) | llvmpipe; T480 ([BENCHMARKS.md](BENCHMARKS.md)) |
| `strobe-app` | Control window (maximized) and output window on one context, scene browser, controls, preview, console, `--diagnose`, adaptive resolution (D64), context-loss recovery with ANGLE fail-over (D66) | Xvfb; T480 smoke tests. Recovery on real WGL is in progress |
| `strobe-audio` | Capture with retry and synthetic fallback, analysis, spectrum and level-trail textures, tap tempo (D20-D23, D61) | T480 live mic at 48 kHz |
| `strobe-script` | QuickJS host: events, `update`, controls, `setUniform` compiled in at load, budget and faults (D62) | Xvfb; T480 (`11-script-orbits`) |
| `strobe-diag` | Session logs, structured failures, panic reports, out-of-process minidumps, GL debug output (D40-D45) | Linux; T480 (`overflow` gives a `.dmp`, bug 2) |
| `strobe-shadertoy` | Stage 1: id parser, both JSON shapes, dialect rewrite, converter to a scene folder, `strobe-import` (D90-D95) | Nine original fixtures on llvmpipe, both dialects |
| `strobe-playlist` | Stage 1, pure core: model, inheritance, TOML, generator, timeline, playback clock, resume (D73-D77) | Unit tests and 4-hour soaks; not in the app yet |
| Scenes | Eleven original scenes over five GPU-load tiers, one scripted | llvmpipe and T480 |

### In flight

- **Output window hidden at launch.** Only the control window opens. The
  operator shows the output, or sends it fullscreen to a monitor, when
  ready, and the preview keeps rendering while the output is hidden.
- **T480 Task 8:** context-loss recovery on real WGL (injected loss,
  ANGLE fail-over, give-up dialog), and bug 1 at a forced render scale
  of 1.0.

## Next, in priority order

Each item names its acceptance test. Details and evidence are in
[STATUS_AUDIT.md](STATUS_AUDIT.md).

1. **Close out the T480 validation.** Recovery on real WGL, bug 1 at full
   scale, an audio device removed and reconnected, a 30-minute soak with
   scene switches, and a symbolicated `.dmp` (Q11). *Accept:* each result
   recorded in [hardware/T480.md](hardware/T480.md), failures included.
2. **Perform-first UI.** Choose the topology from
   [mockups/strobe-perform-directions.html](mockups/strobe-perform-directions.html),
   then build it per [UI_DESIGN.md](UI_DESIGN.md) and
   [UI_IMPLEMENTATION_PLAN.md](UI_IMPLEMENTATION_PLAN.md), porting
   rustaman's UI system (D5). Runtime state stays out of egui memory, and
   widgets emit typed actions. *Accept:* screenshots at 1280x720 and
   1440x900 of the empty, loading, failure, missing-audio and script-fault
   states.
3. **Shadertoy import, stage 2.**
   - Build the REST client (path B), the localhost listener and userscript
     (path A), the asset downloader, the app's Import action, thumbnails
     and the Needs-work bin.
   - In the app, wire through `strobe_Date` and the browser-matched audio
     texture, and name the pass file in shader failures.
   
   *Accept:* representative single-pass and multipass imports play on
   native and ANGLE, a missing asset fails visibly, and attribution
   survives.
4. **Auto-generated controls** (new, described below). *Accept:* every
   fixture, and every library scene with its `CONTROLS` stripped, gets a
   useful, named, grouped control set that visibly changes the image.
5. **Playlists, stage 2.** Load-time precompile with progress, a second
   player for warm-up, the transition compositor, the Playlist workspace
   and generate dialog, the file watcher, resume every 5 s, and live
   commands. *Accept:* a 4-hour, 80-cue show authored from tags in under
   two minutes, then a 4-hour soak with zero runtime compiles.
6. **SSF and script gaps.**
   - Script `meta/` control writes.
   - `textureFinalPass` through an asynchronous (PBO) readback, never a
     synchronous one (Q14).
   - GLES rewrites for desktop-only idioms (Q8).
   
   *Accept:* a fixture per behaviour, on both dialects where it applies.
7. **Presets, MIDI, OSC and hotkeys** on the shared `ControlBank`, without
   duplicating its toggle, bang or smoothing semantics.
8. **Media and cubemaps**, then song-change transitions, then NDI out.
9. **A real tagged release.** Inspect the zip, the ANGLE DLL provenance,
   the matching PDB archive, the SBOM, the checksums and the attestations
   (Q12).

### Auto-generated controls (planned)

Most Shadertoy shaders, and many community SSF scenes, have no controls:
their character is baked into literal numbers. Strobe should find those
numbers and generate a control set automatically, so an import is
performable the moment it lands. This generalises PLAN.md §6.3's manual
"promote-to-control" into an engine that also runs on SSF scenes whose
`scene.json` has no `CONTROLS`.

- **Find candidates by static analysis** of the GLSL. The token stream
  from `strobe-shadertoy`'s dialect stage already exists. Candidates
  include:
  - top-level `#define NAME <number>` and `const float NAME = <literal>;`;
  - literals that scale time (`iTime * 0.3`, `TIME * k`), which become
    speed;
  - loop bounds and step counts, which become quality, with a cap so a
    slider cannot hang the GPU;
  - colour literals (`vec3(0.9, 0.4, 0.1)`), which become colour
    controls;
  - zoom, scale and frequency factors applied to the coordinates.
- **Rank candidates by measured effect.**
  - Render a few frames headless at low resolution with each candidate
    nudged up and down, and score the image difference. The bench's
    headless renderer is already the tool.
  - Keep the ones that visibly matter, drop the dead ones, and order the
    rest by effect.
  - Reject any candidate whose nudge produces NaN or black.
- **Generate the controls.**
  - Rewrite each kept candidate into a uniform, in the local copy only,
    keeping line numbers.
  - Derive a range from the default: `[0, 2x]`, symmetric for negatives,
    integer for loop counts.
  - Name it readably from its identifier or context, and put it in a
    group (Motion, Colour, Shape, Quality).
  - Write the result as ordinary SSF `CONTROLS`, so it survives export
    and runs anywhere.
- **Suggest audio bindings.** Propose a default reactive mapping for the
  strongest few controls, for example bass to scale or zoom and highs to
  brightness or colour. Use the same `syn_*` values as the "Strobe"
  control group (D92). Bindings stay off until the operator accepts them.
- **Review before keeping.** An Import or Inspector panel lists the
  generated controls with live previews, and the operator can rename,
  re-range, drop or keep each one. Nothing is applied to a scene that
  already has authored controls unless the operator asks.
- **Deterministic.** The same shader gives the same controls. Analysis and
  scoring use fixed frames and seeded audio, so golden tests can pin the
  output.

The likely home is a module in `strobe-shadertoy`, or a small
`strobe-autocontrol` crate if SSF scenes need it without the importer.
Scoring needs a GL context, so it runs at import time, never per frame.

## Phases

| Phase | Scope | State |
|---|---|---|
| 0 | Workspace, conventions, CI, GL contexts | Done. A tagged release has not been run. |
| 1 | Renderer, SSF, controls, output window | A functional engineering shell. The Perform-first UI is not built. Images work; media and cubemaps are placeholders. |
| 2 | JavaScript API | Integrated (D62). `meta/` writes and `textureFinalPass` are open. |
| 3 | Audio | Integrated (D61). The bench still uses the synthetic stand-in. |
| 4 | Shadertoy import | The stage 1 converter is done (D90-D95). Acquisition, the Import UI and auto-generated controls are open. |
| 5 | Playlists | The stage 1 pure core is done (D73-D77). App integration is open. |
| 6 | Presets, MIDI, OSC, hotkeys | Not started. |
| 7 | NDI, media input, song-change transitions | Not started. `HARD`/`SMOOTH_TRANSITIONS` are declared but never change. |

## Known gaps in what exists

- The app is an engineering shell, not the performer-facing UI.
- Context-loss recovery is verified under Xvfb with injected faults;
  real-driver behaviour on the T480 is pending (bug 1, D66).
- The bench's `syn_Spectrum` and `syn_LevelTrail` are silent placeholders,
  and the bench does not run scripts. `11-script-orbits` drives controls
  instead (D62).
- `syn_Media` is black and `syn_MediaType` is 0. Cubemaps are reserved but
  never loaded.
- `textureFinalPass` returns black in scripts (Q14), and `meta/` control
  writes are dropped.
- The GLES dialect does not rewrite every desktop-only SSF idiom (Q8). The
  base library is written ES-clean.
- Adaptive-resolution thresholds are validated on one machine, the T480.
- Native crash capture cannot see a bare `abort()` on Windows, which is a
  fast-fail ([DEBUGGING.md](DEBUGGING.md)).

### Shadertoy import (stage 1)

- `strobe-import --id` is not built. Path B (the REST API with a key),
  path A (the userscript and localhost listener), the asset downloader and
  the app's Import UI are stage 2.
- The app does not call the importer, and it does not feed `strobe_Date`,
  so the player falls back to the UTC wall clock (Q15).
- The app's shader-failure detail still says `main.glsl`, so a compile
  error in a buffer (`passes/buffer_a.glsl:14`) shows without a line
  number.
- The audio input is resampled from the 256-texel spectrum, not taken from
  the browser-matched `AudioFrame::shadertoy` (D93, Q18).
- Not supported:
  - `cubemap` and `volume` inputs, and `sound` and `cubemap` passes: these
    are errors;
  - `keyboard`, `video` and `webcam` inputs: they read black, with a
    warning;
  - `srgb` textures: not decoded (Q16);
  - packing built-ins on a 3.30 context, and implicit int-to-float: neither
    is rewritten (Q8).
- A resize restarts `iTime` at `TIME` and `iFrame` at 0 (D91), which is a
  visible jump at a non-default Speed.
- Every buffer is double-buffered, even one that never reads itself (D95).

### Playlists (stage 1)

The app does not use `strobe-playlist` yet. Stage 2 needs:

- precompile at load, reporting failures through `mark_scene_failed`;
- a second `ScenePlayer` for the warm-up;
- the transition compositor;
- the Playlist workspace (UI_DESIGN §13) and the generate dialog;
- a file watcher (`restore_lenient` keeps the position);
- `BeatInfo` from the audio frame, including a real downbeat detector
  (D75);
- `sync_to_wall_clock` at start-up;
- live commands from the UI, hotkeys, MIDI and OSC.
