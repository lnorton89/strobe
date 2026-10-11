# Roadmap and status

**Update this file whenever you finish something.** It is the first file
the next agent reads. Design detail lives in [PLAN.md](PLAN.md), decisions
in [DECISIONS.md](DECISIONS.md), the evidence-based gap list in
[STATUS_AUDIT.md](STATUS_AUDIT.md), and real-hardware results in
[hardware/T480.md](hardware/T480.md).

_Last updated: 2026-10-11 (output post stage, cube maps, syn_Media stills and sequences)._

## Now

**Owner goal:** a working, testable, benchmarkable VJ engine for older
hardware. Prefer measured improvements and explicit acceptance tests over
speculative tuning. "Compiled" is not "verified": say which machine and
which backend proved a claim.

### Implemented and integrated

| Area | What exists | Verified on |
|---|---|---|
| Workspace | Conventions as tests, pinned toolchain and actions, `cargo xtask ci \| changelog \| bench \| symbolicate \| brand`, release workflow (D50-D59) | CI. The release workflow has never run on a tag |
| `strobe-scene`, `strobe-ssf-lib` | SSF parsing and validation, controls runtime, shader assembly, `PASS_SOURCES` (D90), the SSF GLSL library | Headless GL 3.3 core and GLES 3.0 (llvmpipe) |
| `strobe-gfx` | Native GL and ANGLE context providers, headless EGL, multipass and ping-pong, implicit final pass, images, GPU timer queries, robust contexts and reset status (D66) | llvmpipe; T480 native and ANGLE |
| `strobe-bench` | Timings, frames, contact sheet; headless, window and auto contexts (D70-D72), now `strobe_gfx::offscreen` (D140) | llvmpipe; T480 ([BENCHMARKS.md](BENCHMARKS.md)) |
| `strobe-app` | Control window (maximized) and output window on one context; the output starts hidden and the scene frame runs on its own schedule (D96); **Contextual Perform stage 1** (D130-D138): rail, custom title bar with health readouts, preview, transport (prev, next, blackout; hold shown as not wired), first-controls macros, and contextual panes for the scene browser (search, health filter, Preview apart from Load), inspector, diagnostics, import and settings, with saved layout, a focus-aware keyboard model and themes; `--diagnose`, adaptive resolution (D64), context-loss recovery with ANGLE fail-over (D66) | Xvfb (screenshots at 1280x720 and 1440x900 for every state, xdotool for keys); T480: smoke tests, recovery on real WGL (Task 8), hidden output, F11 and the close button (Task 9). The new UI has not run on the T480 or on real Windows |
| `strobe-audio` | Capture with retry and synthetic fallback, analysis, spectrum and level-trail textures, tap tempo (D20-D23, D61) | T480 live mic at 48 kHz |
| `strobe-script` | QuickJS host: events, `update`, controls, `setUniform` compiled in at load, budget and faults (D62) | Xvfb; T480 (`11-script-orbits`) |
| `strobe-diag` | Session logs, structured failures, panic reports, out-of-process minidumps, GL debug output (D40-D45) | Linux; T480 (`overflow` gives a `.dmp`, bug 2) |
| `strobe-shadertoy` | Stage 1: id parser, both JSON shapes, dialect rewrite, converter to a scene folder, `strobe-import` (D90-D95). Stage 2: REST client, texture downloader and cache, shared import pipeline, loopback listener, `strobe-import --id` (D110, D111, D113, D114, D117) | Nine original fixtures on llvmpipe, both dialects; stage 2 against fakes and a loopback mock, never the live site |
| `strobe-autocontrol` | Stage 1: finds candidate literals, rewrites them to uniforms, keeps the ones whose measured effect matters, suggests audio bindings, CLI with `--context auto\|headless\|window` (D123-D126, D140) | llvmpipe, both dialects: 11 of 11 stripped library scenes and 8 of 14 Shadertoy fixtures get controls. Not in the app yet
| `strobe-playlist` | Stage 1, pure core: model, inheritance, TOML, generator, timeline, playback clock, resume (D73-D77). **Stage 2, in the app (D150-D158):** `--playlist` and the LIST pane, precompile with progress, scenes parked at 1x1, warm-up at a quarter of the pixels, the compositor (crossfade, fade-through-black, custom GLSL), real Hold, extend, restart, skip-section and Next/Prev, NOW/NEXT and remaining time, Generate, Save, a file watcher, resume every 5 s, the wall-clock anchor | Unit tests and 4-hour soaks; stage 2 on headless GL in both dialects (`crates/strobe-app/tests/show.rs`) and under Xvfb with llvmpipe. T480 Task 14: the transition demo closed with 0 runtime compiles, transitions at 60 fps costing about 0.5 ms of GPU |
| Brand | Parametric Flow guidelines and concept mark (lnorton89/strobe#6); the default theme in the brand palette, the window and taskbar icon, the title-bar mark, and icon PNGs and `.ico` rendered from SVG with a CI check (D190). Open: wordmark, lockups, the `.exe` resource icon | Xvfb screenshots; not yet seen on Windows |
| Scenes | Eleven original scenes over five GPU-load tiers, one scripted | llvmpipe and T480 |

### In flight

- **Perform UI stage 2** (see item 2): the panes built in stage 1 will also
  hold the review panel for generated controls and the Playlist workspace.

## Next, in priority order

Each item names its acceptance test. Details and evidence are in
[STATUS_AUDIT.md](STATUS_AUDIT.md).

1. **Close out the T480 validation.** Recovery on real WGL, bug 1 at full
   scale, an audio device removed and reconnected, a 30-minute soak with
   scene switches. *Accept:* each result
   recorded in [hardware/T480.md](hardware/T480.md), failures included.
2. **Perform-first UI.** *Stage 1 built* (D130-D138, rustaman's UI system
   ported, D5): Contextual Perform with the shell, the panes above,
   persistence, the keyboard model and every state in the audit
   (screenshots in [images/perform/](images/perform/)). *Left (stage 2):*
   - a curated, persistent Performance Macro set with target resolution
     and the unavailable, broken, learning and conflict states (D137);
   - the Playlist workspace (LIST) with real cues, NEXT and remaining
     time, and Hold wired to it; the Editor (EDIT) and Mappings (MAP)
     workspaces; MIDI/OSC Learn;
   - the generated-controls review panel (item 4) and "open the shader
     error in the Editor";
   - a command palette (Ctrl+K) over the command table, saved named
     layouts, density, scene thumbnails and favourites;
   - on the T480 and real Windows: the custom title bar's drag, snap and
     resize and DPI behaviour, a maximised undecorated window against the
     taskbar, the UI's frame cost, and a Preview's cost (D132, D136).

   *Accept (stage 1, met on Xvfb):* screenshots at 1280x720 and 1440x900 of
   the empty, loading, failure, missing-audio and script-fault states.
3. **Shadertoy import, stage 2.** Built: the REST client (path B), the
   localhost listener and the "Send to Strobe" userscript (path A), the
   asset downloader and cache, the app's Import panel and file drop (path
   C), `strobe_Date` as local time, the browser-matched audio texture, and
   shader failures that name the pass file (D110-D117). Open:
   - the live site: the REST call, texture URLs and the userscript's route
     to the page's JSON have only been run against fakes (Q19), so one
     real import from each of path A and path B on the T480 is the
     acceptance test;
   - a real file drop on Windows and X11 (the typed path runs the same
     job);
   - thumbnails and the Needs-work bin;
   - a batch route and the watched drop folder.

   *Accept:* representative single-pass and multipass imports play on
   native and ANGLE, a missing asset fails visibly, and attribution
   survives.
4. **Auto-generated controls.** *Stage 1 built* (`strobe-autocontrol`,
   D123-D126). *Left:* the Import and Inspector review panel and the app
   calling the engine, confirming the CLI's window context on the T480 (built,
   D140; `--context window` matches headless on llvmpipe under Xvfb, not yet
   run on Windows), the speed control not following `st_speed`, and a
   second look at the heuristics on real community scenes. *Accept:* every
   fixture, and every library scene with its `CONTROLS` stripped, gets a
   useful, named, grouped control set that visibly changes the image. Met
   on llvmpipe for all 11 library scenes and for the fixtures with anything
   tunable in them; 6 of the 14 fixtures are structural and get none, on
   purpose (see `NO_CONTROLS` in `crates/strobe-autocontrol/tests/engine.rs`).
5. **Playlists, stage 2.** *Built* (D150-D158): load-time precompile with
   progress, a second player for warm-up, the transition compositor, the
   Playlist pane and generate card, the file watcher, resume every 5 s, the
   wall-clock anchor and live commands. *Left:*
   - audio-event advance (`clock.audio_event()` needs Phase 7 detection);
   - the Playlist **workspace** proper: the selected cue's inheritance
     inspector, multi-select edits, drag from the library, pasted Shadertoy
     URLs (D157);
   - real tags in the scene format, so Generate can pick by tag;
   - MIDI and OSC mapping of the cue commands (the command table is ready);
   - a downbeat detector, so "bars" line up with the music (D75);
   - the T480: the load time of an 80-scene show, the cost of a transition
     (two scenes at once) on an HD 4000, and VRAM with many scenes parked.

   *Accept:* a 4-hour, 80-cue show authored from tags in under two minutes,
   then a 4-hour soak with zero runtime compiles. Met in part: authoring
   from name words is a card away, and zero compiles after load is asserted
   by tests and logged by every run; the 4-hour soak has only run as the pure
   clock (strobe-playlist), not with real scenes on real hardware.
6. **SSF and script gaps.** *Partly done (D160-D163), 2026-10-10.*
   - [x] Script `meta/` control writes land in a meta `ControlBank` held by
     the session (D160). **No output post stage reads it yet**, so they
     change nothing on screen; that stage is the next piece.
   - [x] **The output post stage reads it** (D180-D184, 2026-10-11): master,
     invert, brightness, gamma, contrast, hue, saturation and the mirrors
     in one program after the compositor, **no pass at all at neutral**; the
     inspector's "Output (meta)" section; meta reactivity and audio speed
     multiply the Audio panel's gains. Verified on llvmpipe, both dialects
     (`crates/strobe-gfx/tests/post.rs`) and under Xvfb with a script
     (`docs/images/post/`). **Its cost on an HD 4000 / the T480 is
     unmeasured.** `Limit Colors` and `Alpha Channel` are not built, and
     meta values reset on a scene change (D182).
   - [x] `textureFinalPass` through an asynchronous PBO readback, two frames
     late, never a stall (D162, Q14). Checked on llvmpipe only; the T480 and
     ANGLE/D3D11 are open.
   - [x] No shader compiles after load: a late `setUniform` is refused
     (D161).
   - [~] GLES rewrite for `PASSINDEX == 0.0` (D163). Other implicit
     `int` to `float` conversions need a parser (Q8).
   
   *Accept:* a fixture per behaviour, on both dialects where it applies.
7. **Presets, MIDI, OSC and hotkeys** on the shared `ControlBank`, without
   duplicating its toggle, bang or smoothing semantics.
7b. **LAN remote from a phone or tablet** (planned, after item 7). The
   proposal and threat model are in
   [REMOTE_CONTROL_PWA_RESEARCH.md](REMOTE_CONTROL_PWA_RESEARCH.md)
   (lnorton89/strobe#7). It is a same-origin web remote, served by Strobe on
   an opt-in LAN listener, with pairing, per-client tokens, Host and Origin
   checks and rate limits. Commands go over a typed WebSocket protocol into
   a bounded channel that the app thread drains, which is the same path
   OSC takes in item 7. So it reuses that remote command bridge and the
   command table, and opens no second route to the show. *v1* is a LAN web
   remote only. *Installable PWA* waits on a tested HTTPS and certificate
   story on Android and iOS, because a LAN IP over plain HTTP is not a
   secure context. *Accept:* Phase 0 to 2 of the research doc, including
   real-device tests and a T480 soak with no frame-time regression.
8. **Media and cubemaps**, then song-change transitions, then NDI out.
   *Built (D185, D186), 2026-10-11:* cube maps from six faces or a
   strip/cross picture, with a missing one reading black and reported through
   the failure path; `syn_Media` as a still or an image sequence (a `MEDIA`
   key or `--media`), decoded off the frame thread and uploaded once, with
   `syn_MediaType` 1 while it shows. Verified on llvmpipe in both dialects
   (`crates/strobe-gfx/tests/assets.rs`) and under Xvfb. *Left:*
   - **video and webcam input**, deliberately later: they need a decoder or
     capture dependency the engine does not carry (nothing heavy should be
     added for them);
   - checking the `MEDIA` / `CUBEMAPS` / `syn_MediaType` shapes against SSF's
     docs (Q22), and a media picker in the UI (only `--media` exists);
   - media per playlist cue (a cue plays the scene's own `MEDIA`);
   - the T480: the cost of uploading a long sequence, and VRAM with 192 MiB
     of frames on shared memory.

   *Accept:* a cube-map fixture where each direction returns its face, a
   missing one failing visibly while the scene runs, and an image read
   through `syn_Media`. Met on llvmpipe.
9. **A real tagged release.** Inspect the zip, the ANGLE DLL provenance,
   the matching PDB archive, the SBOM, the checksums and the attestations
   (Q12).

### Auto-generated controls (stage 1 built)

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

It lives in the `strobe-autocontrol` crate (D123), so SSF scenes need not
pull in the importer. Scoring needs a GL context, so it runs at import time,
never per frame.

**Built (stage 1):**
- analysis, the line-preserving rewrite, scoring by measured effect, naming,
  audio suggestions (opt-in bindings that default to off), the report and a
  scene writer that never edits in place (D124-D126);
- the `strobe-autocontrol <scene-dir> [--out DIR] [--keep N] [--report]`
  binary, with `--force-merge`, `--analyze-only`, `--audio-bindings`,
  `--api gles` and `--context auto|headless|window` / `--gl` (a hidden-window
  context where there is no EGL device, shared with the bench, D140);
- headless tests in both dialects over every library scene (controls
  stripped) and 14 Shadertoy fixtures.

**Left:**
- the Import and Inspector review panel (rename, re-range, drop, keep, live
  previews) and the app calling `generate` with its own context;
- accepting a suggested audio binding in the app, and binding a speed
  control (it is an integrator, so it is data only in GLSL);
- verifying on a real GPU and the T480, and trying community scenes (the
  heuristics have only seen Strobe's own library and original fixtures);
- speed controls that also follow `st_speed` and `st_audio_time` (D125);
- vector and `xy` candidates (an offset, a centre) and dropdown candidates
  (a mode switch) are not generated.

## Phases

| Phase | Scope | State |
|---|---|---|
| 0 | Workspace, conventions, CI, GL contexts | Done. A tagged release has not been run. |
| 1 | Renderer, SSF, controls, output window | Done, with the Perform-first UI at stage 1 (D130-D138). Images, cube maps and `syn_Media` stills and sequences work (D185, D186); video and webcam do not. |
| 2 | JavaScript API | Integrated (D62). `meta/` writes land in a meta bank (D160) that the output post stage and audio gains read (D180, D184); `textureFinalPass` is an asynchronous readback (D162); late uniforms are refused (D161). |
| 3 | Audio | Integrated (D61). The bench still uses the synthetic stand-in. |
| 4 | Shadertoy import | The converter (D90-D95), acquisition (REST client, texture downloader, localhost listener), the Import panel, file drop and the userscript (D110-D117) are built, and stage 1 of auto-generated controls (D123-D126). Open: live-site verification (Q19), thumbnails, the Needs-work bin, batch import, and the app's review panel for generated controls. |
| 5 | Playlists | The pure core (D73-D77) and the app integration (D150-D158) are built. Verified on the T480 (Task 14). Open: audio-event advance, the Playlist workspace's inspector, MIDI/OSC. |
| 6 | Presets, MIDI, OSC, hotkeys | Not started. |
| 7 | NDI, media input, song-change transitions | Not started, except media stills and image sequences (D186); video, webcam and NDI are open. `HARD`/`SMOOTH_TRANSITIONS` are declared but never change. |

## Known gaps in what exists

- The performer-facing UI is at stage 1 plus the Playlist pane (D157): the Editor and Mappings workspaces, persistent macros and thumbnails are not built, and none of it has run on the T480.
- Context-loss recovery is verified under Xvfb with injected faults;
  real-driver behaviour on the T480 is pending (bug 1, D66).
- The bench's `syn_Spectrum` and `syn_LevelTrail` are silent placeholders,
  and the bench does not run scripts. `11-script-orbits` drives controls
  instead (D62).
- `syn_Media` shows a still or an image sequence only (D186); **video and
  webcam are later** and read as no media. Cube maps load (D185), but the
  `MEDIA` and `CUBEMAPS` shapes and `syn_MediaType`'s values are unverified
  against SSF (Q22). The bench has no media or cube-map scene.
- The meta controls change the picture (the post stage, D180) but the stage's
  cost on an old Intel GPU is unmeasured, meta values reset on a scene change
  (D182), and `Limit Colors` and `Alpha Channel` are not built (Q21).
  `textureFinalPass` is two frames late and unverified on the T480 (D162,
  Q14).
- The GLES dialect rewrites `PASSINDEX == 0.0` only (D163); the rest of the
  implicit int-to-float family is open (Q8). The base library is written
  ES-clean.
- Adaptive-resolution thresholds are validated on one machine, the T480.
- Native crash capture cannot see a bare `abort()` on Windows, which is a
  fast-fail ([DEBUGGING.md](DEBUGGING.md)).

### Shadertoy import (stages 1 and 2)

- Stage 2 is built but has never touched shadertoy.com (Cloudflare blocks
  the cloud containers). Path B needs an API key (`STROBE_SHADERTOY_KEY`
  or `shadertoy.key` in the data directory, D117). The userscript's route
  to the page's JSON is a best guess and says what it tried when it fails
  (D115, Q19).
- The listener is off by default (`--listen`, or the Import panel's
  switch) and its token is shown in the panel (D114).
- A real file drop was not exercised under Xvfb; the typed-path route runs
  the same job and was (D116).
- Still open: thumbnails, the Needs-work bin, batch import, the watched
  drop folder.
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

### Playlists (stages 1 and 2)

The app plays playlists (D150-D158). Known gaps:

- **Audio-event advance** (`drop`, `breakdown`) is parsed and shown but never
  fires: nothing calls `clock.audio_event()` until Phase 7 detection exists.
- **A downbeat detector** (D75): a "bar" is counted from the beat grid's
  origin, so a beat-locked cue ends on a bar line of *that* grid.
- **The Playlist pane is not the workspace UI_DESIGN section 13 describes**:
  no per-cue inheritance inspector, no multi-select edit, no drag from the
  library, no pasted-URL import. Save rewrites the file from the model, so
  comments are lost.
- **No scene tags**, so Generate matches name words (D157).
- **Cue `preset` is ignored** (presets are Phase 6) and **`meta` overrides
  are written to the session's meta bank** (D160), which nothing renders
  from yet.
- **Two scenes render during a transition**, doubling the cost for its
  length. Unmeasured on the T480.
- **Loading a scene by hand unloads the playlist** (D150).
- **Context loss** reloads the playlist and compiles every scene again
  before it resumes; on a big show that is a pause the length of the load.
- Everything here was verified on llvmpipe under Xvfb; none of it has run on
  real Windows or the T480.
