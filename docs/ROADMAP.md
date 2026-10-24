# Roadmap and status

**Update this file whenever you finish something.** It is the first file
the next agent reads. Design detail lives in [PLAN.md](PLAN.md), decisions
in [DECISIONS.md](DECISIONS.md), the evidence-based gap list in
[STATUS_AUDIT.md](STATUS_AUDIT.md), and real-hardware results in
[hardware/T480.md](hardware/T480.md).

_Last updated: 2026-10-23 (phones ask under a name and are approved on the host, no code anywhere, D469; Strobes find each other on the LAN and a desktop asks the host with no code, approved or denied there; a streamlined Remote page, D468; Mappings rebuilt on the form grammar, mappings and macros first, D467; the UI foundations and a rebuilt Settings: system Segoe UI, D465; the form grammar, a type scale, toggle switches, outlined fields, and Settings as seven pages with a category list and search, D466; an unchanged release build no longer relinks the app: the build scripts stop watching the git index and a CARGO_MANIFEST_DIR path, D464; a screenshot-based UI review with the owner's answers, docs/UI_REVIEW.md); 2026-10-17 (remote and venue-mode bug sweep before the two-machine test, 2026-10-21: a command answered busy is withdrawn, never run later, D460; a remote on a system-chosen address follows it when DHCP moves it, D461; the control window no longer waits for a paired-list save, D462; a relative trust-file path is kept whole, D463; GL renderer bug sweep, 2026-10-21: a render-scale change keeps a playlist's warming and outgoing scenes, D440; start-up and shutdown bug sweep for the venue launcher: a fullscreen launch puts no crash or stop box over its show, D450; an `--output-monitor` not yet connected is waited for, D451; a playlist that does not load at launch leaves a scene playing, D452; the log says when the fullscreen output loses the focus, D453; a refused command line exits 2, D454; playlist and show-run bug sweep: an anchored show is placed in the run nearest now across midnight, D430, and its wait follows a corrected system clock, D437; a held cue ignores audio events, keeps its hold through an edit, and Hold needs a playing cue, D431, D433, D436; an edit above the live cue keeps the show on it, D432; a warm-up the show no longer needs is parked, D434; an unchanged save is not a reload, D435; audio, MIDI and GL-log follow-ups: a capture stream that stops calling back without an error is noticed and reopened on a backoff, D420; a MIDI fader flood keeps its latest value per source and can no longer crowd out notes, D421; a GL driver message repeated every frame is logged five times and then summarised once a minute, D422; app and remote follow-ups: the output's monitor is followed by name, position and size across display changes, D400; macro keys let go on focus loss and never dropped in a crowded frame, D401-D402; the palette's highlight stays on its entry, D403; the desktop remote sends no NaN from a zero-size viewer, lets a remote click's modifiers go, sends an oversized capture smaller and lets a code-paired desktop take back its own dead lease, D404-D407; clocks keep their precision four hours into a show: speed controls, `TIME` and a playlist's show time sum in `f64`, D390-D393, with wrapping, the Shadertoy clock and mic-switch continuity left as Q29-Q31; the synthetic audio track's clocks are integrals, smooth four hours into a show, D330; adaptive resolution follows missed refreshes, verified on the venue Vega 3, D314-D320; every GL context primed for AMD WGL, D311; bug sweeps of LAN/desktop remote, scene loading and controls, and JS scripts, D302, D312-D313, D321-D326; follow-up bug sweep of scene loading, the player, scripts and the Shadertoy importer: passes past the texture limit refused by name or clamped, sampler units for active samplers only, two control shapes that failed a scene now load, unhandled promise rejections reported, `let`/`const` and function-value event callbacks, Shadertoy channel arrays in global initialisers, UTF-16 exports, texture file names that cannot collide, Replace only over an import of the same shader, D410-D418, with the control shapes left as Q32; clocks keep their precision four hours into a show: speed controls, `TIME` and a playlist's show time sum in `f64`, D390-D393, with wrapping, the Shadertoy clock and mic-switch continuity left as Q29-Q31; the synthetic audio track's clocks are integrals, smooth four hours into a show, D330; adaptive resolution follows missed refreshes, verified on the venue Vega 3, D314-D320; every GL context primed for AMD WGL, D311; bug sweeps of LAN/desktop remote, scene loading and controls, and JS scripts, D302, D312-D313, D321-D326); 2026-10-15 (six heavy scenes cheaper for the Vega 3, D310; `pow` of a negative base on ANGLE, D301; Linux support documented, with a Nix flake, D295; one click on a scene card makes it live, and LIB has Scenes and Controls pages, D291); 2026-10-14 (the control window follows Synesthesia's layout, D281-D285; v0.1.0 tagged and published; earlier claims reconciled with D199-D238 and the T480 and TORNADO records)._

## Now

**Owner goal:** a working, testable, benchmarkable VJ engine for older
hardware. Prefer measured improvements and explicit acceptance tests over
speculative tuning. "Compiled" is not "verified": say which machine and
which backend proved a claim.

### Implemented and integrated

| Area | What exists | Verified on |
|---|---|---|
| Workspace | Conventions as tests, pinned toolchain and actions, `cargo xtask ci \| changelog \| bench \| symbolicate \| brand`, release workflow (D50-D59) | CI, and the v0.1.0 tag (2026-10-14, D270) |
| `strobe-scene`, `strobe-ssf-lib` | SSF parsing and validation, controls runtime, shader assembly, `PASS_SOURCES` (D90), the SSF GLSL library | Headless GL 3.3 core and GLES 3.0 (llvmpipe) |
| `strobe-gfx` | Native GL and ANGLE context providers, headless EGL, multipass and ping-pong, implicit final pass, images, GPU timer queries, robust contexts and reset status (D66) | llvmpipe; T480 native and ANGLE |
| `strobe-bench` | Timings, frames, contact sheet; headless, window and auto contexts (D70-D72), now `strobe_gfx::offscreen` (D140) | llvmpipe; T480 ([BENCHMARKS.md](BENCHMARKS.md)) |
| `strobe-app` | Control window (maximized) and output window on one context; the output starts hidden and the scene frame runs on its own schedule (D96); **Contextual Perform stage 1** (D130-D138): rail, custom title bar with health readouts, preview, transport (prev, next, blackout; hold shown as not wired), first-controls macros, and contextual panes for the scene browser (search, health filter, Preview apart from Load), inspector, diagnostics, import and settings, with saved layout, a focus-aware keyboard model and themes; `--diagnose`, adaptive resolution (D64), context-loss recovery with ANGLE fail-over (D66) | Xvfb (screenshots at 1280x720 and 1440x900 for every state, xdotool for keys); T480: smoke tests, recovery on real WGL (Task 8), hidden output, F11 and the close button (Task 9), the Perform UI at 150% scaling (Task 12) |
| `strobe-app` layout | **Synesthesia-style control window** (D281-D285): a 60-point rail that switches the main view (PLAY, LIB, GET, LIST, EDIT, MAP, DIAG, SET), a persistent side panel at 31% of the window (preview, status strip with output, fullscreen, fps, audio level and snapshot, the fav slots in reach from every tab, and Playlist, Controls, Presets and Edit icon tabs), the library as a grid with a toolbar, a filter row (count, search, sort and direction, favourites, health) and caption cards (title, author, "..." menu, Synesthesia marker, D271), and a help bar under the transport and BLACKOUT, which stay put. **D291:** one click on a card makes it live (a double-click is one load; Enter on a focused card too; Preview moved to the card menu and the palette), and LIB switches between **Scenes** and a large, touch-sized **Controls** page (the same widgets as the panel's tab) with a segmented switch, Ctrl+L or the palette; the colour swatch, zero-wide before, can be clicked again | Headless shell tests (overflow, nothing over BLACKOUT, no text over text) on every state at ten sizes, and llvmpipe screenshots; **not yet seen on a real Windows display or the T480** (U-09 at 150%) |
| `strobe-audio` | Capture with retry and synthetic fallback, analysis, spectrum and level-trail textures, tap tempo (D20-D23, D61) | T480 live mic at 48 kHz |
| `strobe-script` | QuickJS host: events, `update`, controls, `setUniform` compiled in at load, budget and faults (D62) | Xvfb; T480 (`11-script-orbits`) |
| `strobe-diag` | Session logs, structured failures, panic reports, out-of-process minidumps, GL debug output (D40-D45) | Linux; T480 (`overflow` gives a `.dmp`, bug 2) |
| `strobe-shadertoy` | Stage 1: id parser, both JSON shapes, dialect rewrite, converter to a scene folder, `strobe-import` (D90-D95). Stage 2: REST client, texture downloader and cache, shared import pipeline, loopback listener, `strobe-import --id` (D110, D111, D113, D114, D117) | Nine original fixtures on llvmpipe, both dialects; stage 2 against fakes and a loopback mock, never the live site |
| `strobe-autocontrol` | Stage 1: finds candidate literals, rewrites them to uniforms, keeps the ones whose measured effect matters, suggests audio bindings, CLI with `--context auto\|headless\|window` (D123-D126, D140) | llvmpipe, both dialects: 11 of 11 stripped library scenes and 8 of 14 Shadertoy fixtures get controls. In the app through a worker process (D202); T480 Tasks 13 and 23
| `strobe-playlist` | Stage 1, pure core: model, inheritance, TOML, generator, timeline, playback clock, resume (D73-D77). **Stage 2, in the app (D150-D158):** `--playlist` and the LIST pane, precompile with progress, scenes parked at 1x1, warm-up at a quarter of the pixels, the compositor (crossfade, fade-through-black, custom GLSL), real Hold, extend, restart, skip-section and Next/Prev, NOW/NEXT and remaining time, Generate, Save, a file watcher, resume every 5 s, the wall-clock anchor | Unit tests and 4-hour soaks; stage 2 on headless GL in both dialects (`crates/strobe-app/tests/show.rs`) and under Xvfb with llvmpipe. T480 Task 14: the transition demo closed with 0 runtime compiles, transitions at 60 fps costing about 0.5 ms of GPU |
| Brand | Parametric Flow guidelines and concept mark (lnorton89/strobe#6); the default theme in the brand palette, the window and taskbar icon, the title-bar mark, and icon PNGs and `.ico` rendered from SVG with a CI check (D190). Open: wordmark, lockups, the `.exe` resource icon | Xvfb screenshots; not yet seen on Windows |
| `strobe-io` | Show control (D170-D179): MIDI input and OSC on two worker threads feeding one bounded queue, drained once per scene frame; the mapping model (source to target, range, curve, invert, button mode, soft takeover), learn with conflict detection, the six target states, the built-in `/strobe/...` OSC space, the Performance Macro set, `show-control.json`. Presets and fav slots in `strobe-scene`. In the app: the MAP pane, the macro strip, the preset row, cue `preset`, configurable hotkeys | Unit tests with no devices, real UDP over loopback, `tests/show.rs` on headless GL in both dialects, and the app under Xvfb with hand-built OSC datagrams and a real click on Learn. **No MIDI hardware has touched it** (the parser and the mapper are tested from recorded bytes). Bug sweep 2026-10-17: nested OSC arrays refused (D370), presses with no release fire every time (D371), a lost device lets go of its held pads (D372), a narrowed fader picks up at the end of its reach (D373), twin controllers and fav slot 0 fixed. Follow-ups: the build with MIDI off is clippy-checked in CI and `cargo xtask ci`, the OSC listener binds again after a socket error with a bounded backoff (D374), a double beyond the f32 range is clamped (D375), a hand-edited macro counter at `u32::MAX` recovers (D376) |
| Scenes | Twelve original scenes over five GPU-load tiers, one scripted, one circuit-board animation (lnorton89/strobe#11) | llvmpipe and T480 |

### In flight

- **Adaptive resolution watches missed refreshes, not only the mean GPU time
  (2026-10-16, D314-D320), verified on the projector (M18-M26; D318's
  lock and its expiry seen in the M26 soak):**
  the venue mini PC showed scene 13 at 42-45 fps with a 13.6 ms mean under
  the 16.7 ms budget, about 9% of frames a refresh late, and the controller
  never moved (M16). Each presented output frame's interval now feeds the
  controller; four seconds with about 4% or more late, and the GPU at 60% of
  the cadence or more, lower the scale one step (1.00 -> 0.90 -> 0.80), and it
  climbs back only when the scene gets clearly cheaper. Off without vsync,
  with an unknown refresh rate or a hidden output; the cadence is never
  faster than 60 Hz, so 144 Hz monitors are unaffected. A single hitch no
  longer moves the mean far enough to drop. The perf line gives the scale. A
  minimised control window now draws nothing (the undecorated window kept a
  client area, so the old 0x0 check missed it), but still runs deferred scene
  loads, actions and the playlist compile. M18 on the projector: clean on the
  new scenes (16 at 59.4 fps), but on the old 13 the misses left at 0.80 were
  not the GPU's and the scale hunted 0.65-0.80. D315: a late drop that does
  not halve the misses is undone and the late rule stops at that scale until
  the scene changes; climbing back after a late drop waits for 30 s without
  misses; misses are counted from each window's span (the CPU timestamps are
  not quantised to the refresh); a covered, paint-starved control window runs
  only its chores between 5 s full draws. M19: the covered window works, but
  the halving test undid steps that paid and locked 13 and 16 at 1.00 (16
  fell from 59.4 to 49.2 fps). D316: a step pays when it cuts the missed
  share by a fifth and by 5 points (the old 13 ends at 0.80, 16 at 0.80),
  and the perf line leads with the controller's own missed-refresh count.
  M20 passed: the old 13 from 42.5 to 58.5 fps at 0.70, 16 at 59.8 fps at
  0.80, no hunting, no climb back. Its third step waited three to four
  minutes behind the load line; D317 lets a step that paid lift that gate
  for the next one; M21 confirmed it (0.70 at 18.7 s, 59.9 fps). M21 also
  locked at 1.00 for the night on a first launch whose verdict windows a
  previous process's teardown disturbed; D318 makes the late lock expire (60,
  120, 240 s) and puts off verdicts on disturbed windows (a frame over 100 ms,
  or the GPU time up after a step down). M22 ran clean; M23 showed a
  competitor's GPU load waited out (no step under its stalls, normal steps
  after) but a held 0.70 creeping to 0.60 on noise. D319: a settled scale (a
  paid verdict, or 30 s at it) steps again only when the recent windows (up
  to 10 s) average 1.5 times the drop line. M24 verified D318's put-off
  verdicts (bench loads of 4 s and 31 s inside the first verdict, no lock) and
  D319 (ten minutes at 0.70 with no step on noise). Its one stray step came
  from a 915 ms Windows snapshot (VSS) stall, and D320 keeps a stall's
  windows out of a settled scale's evidence. D320 also logs every put-off
  verdict and every lock that runs out. Open: `hardware/MINIPC.md` Task M25,
  and the snapshots' schedule on the venue machine (about three a day,
  18:30-19:10).

- **`pow` of a negative base on ANGLE (2026-10-15, D301), built, and it does
  NOT fix the bug on hardware (AMD.md Task A15, RX 580): ANGLE gives the same
  pixels with and without it, because Direct3D's `log2` also takes `|x|`.
  The rewrite is harmless and stays; the redo (an explicit sign test giving
  a real NaN) is parked until the owner asks for it:** the ES dialect writes `pow(x, y)` as `exp2(log2(x) * (y))`,
  the native drivers' sequence, because Direct3D's compiler answers a negative
  base with `|x|^y` (sprinkles stopped growing, a centre ring after 600 frames)
  and folds `pow(x, 1.0)` (PLUS black). Integer powers of 2 or more and literal
  bases keep `pow`; desktop is unchanged. Open: AMD.md Task A15 (the probe,
  the corpus black lists, sprinkles and PLUS on native and ANGLE) and Q28.

- **Six heavy scenes made cheaper for the venue's Vega 3 (2026-10-15, D310),
  measured on the Vega 3 at 1080p (MINIPC.md Task M17, 2026-10-16): 04
  21.0 → 10.7 ms, 05 25.6 → 10.0, 08 17.5 → 7.7, 13 16.0 → 8.5, 14 22.2 →
  18.7, 16 15.5 → 14.9; 04 and 16 pixel-identical, the others visually the
  same. 14 and 16 are still at or over the 16.7 ms budget there.** Exact culling or arithmetic in 04, 13, 14 and
  16, and 05's warp fields, 08's lights and 13's broad noise drawn at reduced
  size. The review put 14's layers and 16's noise back at full size, which
  had changed their look. The llvmpipe ratios are in BENCHMARKS.md;
  the Vega 3 numbers above are the real ones.

- **Linux support (2026-10-15, D295), documented and built, not yet seen on
  real Linux hardware:** the README has a Linux section (Debian and Ubuntu
  packages verified on a clean Ubuntu 24.04, Fedora and Arch names from the
  package databases, Wayland and X11, graphics, audio, WSL2), `flake.nix` gives
  a dev shell, a software-GL shell and a wrapped package for x86_64 and
  aarch64 Linux (built and run on x86_64 under Xvfb and Weston), and
  COMPATIBILITY has the rows. Open, in STATUS_AUDIT's "P1: Linux": a real
  Linux GPU, ALSA, PipeWire and PulseAudio capture, MIDI hardware, NixOS, and
  WSL2 with WSLg on the AMD desktop.

- **Venue mode (2026-10-15, D288):** a headless show machine is controlled
  from a laptop with no click at it. Built: `--allow-desktop-control`,
  `--remote-trust FILE` and `--make-remote-trust FILE` on the host; a trust
  file of 32 OS-random bytes; trusted pairing by HMAC-SHA256 challenge and
  response (the secret is never on the wire, nonce single-use, constant-time
  compare); per-address and global backoff on failed proofs; a trusted
  pairing is per connection, listed as *trusted*, revocable for the run, and
  ends with the kill switch; a launch-time `--remote` retries until the
  network is up. **Controller side built (D294):** `strobe --control
  HOST:PORT --remote-trust FILE` opens straight into the viewer; Settings >
  Control another Strobe has "Use a trust file" (the path is remembered, never
  the secret); the proof is made only for the controller's own `trust_start`;
  a stable controller id; and a trusted connection that drops retries by
  itself (1 s, 2 s, 4 s ... 15 s) as a `Stage::Reconnecting` on D286's viewer
  until Disconnect, stopping with the reason on a refusal. Next: a two-machine
  soak on the T480 and the JaJa mini PC.
- **Full desktop remote control (2026-10-13, D275):** native Strobe instances
  can mirror and operate another instance's complete control window,
  including the editor and host file operations. Built on `strobe-remote`'s
  pairing/listener, with a separate desktop grant and host switch, one
  controller lease, bounded input/capture workers, text clipboard and
  in-memory reconnect credentials. Loopback tests cover host editing/saving,
  revocation and unchanged phone permissions; Windows RX 580 native GL
  captures verify the connection form and viewer layouts. Next: two physical
  desktops over Wi-Fi, loss/sleep/minimized-window behavior and a T480
  streaming soak. The raster UI stream is capped at 5 Hz; see USER_GUIDE.md.
  Review hardening (2026-10-14): desktop grants end with the switch and are
  never saved, a remote cannot pair devices or open listeners, capture is a
  fenced asynchronous readback, and clipboard text flows only when asked.
  Viewer polish (2026-10-15, D286), from the first T480 to RX 580 test: the
  viewer keeps the last image dimmed when the host drops it, waiting for
  approval shows its seconds and ends at 60 s, a refusal says a new pairing
  code is needed and where it appears, the connect form is its own Settings
  section with a Ctrl+K entry, and the host counts only authenticated
  clients as connected. The T480 to RX 580 re-test of these states is open.

- **Control frames without paints (2026-10-15, D290), verified on the mini
  PC (Task M5: takeover at 3.5 s, about 119 driven redraws a minute, the
  playlist 14/14 at 60 fps); since D315 a covered control window that is not
  streamed draws only every 5 s and runs its chores in between:** at the JaJa show the fullscreen output took every
  `WM_PAINT`, so the control window ran 0 frames a minute and the playlist,
  UI actions and desktop remote stopped with it (fca44a5 restarted only the
  playlist). When a control redraw request goes unanswered for 250 ms and
  4 loop passes, `about_to_wait` now runs the whole control frame itself, at
  2 Hz (5 Hz with a desktop remote), until Windows paints it promptly again.


- **AMD native WGL renders black above about 160x90 offscreen (D296), fixed
  by D311 and verified on the mini PC (Task M17, 2026-10-16):** root-caused there (MINIPC.md
  M11-M14): the Vega 3's driver drops every draw into a target over 16384
  pixels (64 KiB of RGBA8) until the context has drawn into a small target
  once; after that, everything lands. The app escaped it because egui draws
  first; the bench, corpus and autocontrol did not. Every GL context now does
  one 16x16 draw when it is created (`gl::prime`, from `Gl::from_loader`),
  with the state restored; headless-tested for clean state in both dialects,
  which cannot reproduce the bug. On the Vega 3 the prime costs 47-129 ms
  once per context (the driver's first-draw cost, paid there instead of by
  the first scene).

- **Render scale and pinned scenes (D280), built, not yet re-run on the
  RX 580:** a scene with a root `WIDTH` x `HEIGHT`, a literal buffer pass of
  that size, and a shader that reads it with `texelFetch` draws its final
  pass at that size (2 of Synesthesia's 90: `neon_city`, which now shows the
  whole city at every scale, and `entombed_silicon_dreams`). The nine that
  sample such a pass by uv keep the render-scale lever. Adaptive resolution
  leaves a pinned scene at the ceiling, and for every scene undoes a drop
  that cut under 15% of GPU time per halving; two such verdicts in a row
  lock lowering for 30 s (doubling on each re-lock) or until the next scene.
  Open: the RX 580 re-run (AMD.md).

- **Chromatic Rise scene (2026-10-13):** added
  `scenes/13-chromatic-rise.synScene`, an original procedural interpretation
  of the owner's TouchDesigner RISE video, with eleven controls and bass
  response. Rendered and visually inspected at 1280x720 on the RX 580,
  native GL 3.3 and ANGLE GLES 3.0. Next: owner comparison against the
  original network and older integrated-GPU timing; see BENCHMARKS.md.
  **Rebuilt 2026-10-14 (D276)** as a feedback-advected, posterised marbling
  (fibrous striations, ringed bubbles, hard neon hue steps) after the owner
  found the first draft too smooth; twelve controls. Checked headless on
  llvmpipe in both dialects; the Windows and HD 4000 re-run is still open.

- **Scenes 14-16 rebuilt (2026-10-15, D277-D279):** `14-electric-bloom`,
  `15-neon-ribbon-flow` and `16-acid-mirror` replace Codex's first drafts.
  Electric Bloom is a three-layer garden with unfurling petals and drifting
  pollen; Neon Ribbon Flow is five translucent, glowing neon ribbons with
  parallax; Acid Mirror is a double-buffered advected paint buffer folded
  into bilateral symmetry. Each is within 2x of `05-domain-warp` at 1080p on
  llvmpipe, renders identically in desktop GLSL 3.30 and GLSL ES 3.0, and
  keeps a stable palette at 2, 10, 30 s and 5 min. Open: real-GPU timing
  (Vega 3, HD 4000), owner comparison against the reference videos and the
  floral image, which was never available.
  **Reviewed before merge (2026-10-15, the "Revised" notes of D276-D279):**
  13 lost its one-pixel row artifact (an unstable cross-flow sharpen) and
  gained long shear drags, an acid-green palette, bubble clusters and a
  clock (frame-rate independent drag, no jump when a speed slider moves);
  14 lost the black dots at its petal tips and 9% of its cost; 15 is more
  saturated; 16 gained flow-following hairline streaks, crisper edges, more
  black, and re-seeds at once after a resize. All four: 1.1x to 1.5x
  `05-domain-warp` on llvmpipe, identical in both dialects, stable to 5 min.

- **Scene 17, Blacklight Carpet (2026-10-19, D439):** an original
  confetti carpet under a UV tube, after the owner's arcade-carpet photo.
  Fluorescent confetti, sprinkles and coiled helix streamers on black pile,
  in five parallax layers that drift along a slowly turning path while the
  pieces spin and the coils roll. Rendered and inspected at 1920x1080 and
  640x360 on TORNADO, native GL 3.3 and ANGLE GLES 3.0 identical (max
  difference 1/255), stable at 6 s, 5 min and 1 h; autocontrol keeps the
  same 8 controls in both dialects with its controls stripped. 2.2x
  `05-domain-warp` natively, 2.5x on ANGLE. Open: llvmpipe and old-GPU
  (HD 4000, Vega 3) timing, and the owner's look at it in motion.

- **Perform UI stage 2** (see item 2): the panes built in stage 1 hold the
  generated-controls review (D202, D219); the full Playlist workspace is
  still to come.
- **LAN remote v1 (D250-D254), built, not yet on a phone:** an opt-in
  listener in `strobe-remote` on its own threads, serving a one-file
  controller page and a WebSocket with protocol v1; pairing with a one-use
  code and desktop approval; rotating per-client tokens kept as digests,
  revocable; Host, Origin, size, rate, idle and slow-client limits with
  adversarial tests over real sockets; drained by the app once a frame, with
  "Remote on" and a Stop button in the title bar. Next: real Android and iOS
  devices over Wi-Fi, the Windows firewall prompt and the T480 soak (L-08).
  Installable PWA still waits on HTTPS (L-07).

## Next, in priority order

Each item names its acceptance test. Evidence is in [STATUS_AUDIT.md](STATUS_AUDIT.md); the complete cross-epic register of open implementation and validation work is [IMPLEMENTATION_BACKLOG.md](IMPLEMENTATION_BACKLOG.md). Keep the roadmap short and update both documents as work lands.

1. **Close out the T480 validation.** Recovery on real WGL, bug 1 at full
   scale, an audio device removed and reconnected, a 30-minute soak with
   scene switches. *Accept:* each result
   recorded in [hardware/T480.md](hardware/T480.md), failures included.
   *Done except the audio reconnect:* recovery (Task 8), bug 1 (Tasks 8 and
   14), and the soak (Task 17: a 30-minute fullscreen playlist, 271
   transitions, 0 runtime compiles, flat memory and handles, 60 fps, exit in
   0.11 s). The minute-6 step is retired (R-02): it matches the output
   leaving fullscreen, not a leak. Four later runs showed no step,
   including 20 minutes fullscreen on the same playlist (Tasks 20, 34 and
   36). Open: unplugging and replugging the audio device.
2. **Perform-first UI.** *Stage 1 built* (D130-D138, rustaman's UI system
   ported, D5): Contextual Perform with the shell, the panes above,
   persistence, the keyboard model and every state in the audit
   (screenshots in [images/perform/](images/perform/)). *Left (stage 2):*
   - ~~a curated, persistent Performance Macro set with target resolution
     and the unavailable, broken, learning and conflict states~~ (built,
     D175; the range and colour editor and pinning from the Inspector are
     built, D210; a shortcut and smoothing per macro are built, D227 and
     D228);
   - the Playlist workspace (LIST) with real cues, NEXT and remaining
     time, and Hold wired to it (the LIST pane is built, D157); ~~the Editor
     (EDIT) workspace~~ (built, D204) and a full Mappings workspace (the MAP pane with MIDI
     and OSC Learn is built, D177);
   - ~~the generated-controls review panel~~ (built: D202, and keep, drop
     and re-range per control, D219);
     ~~"open the shader error in the Editor"~~ (built, D204);
   - ~~density~~ (built, D220; saved named layouts, D215; scene thumbnails
     from `IMAGE_PATH` or the live frame, D212 and D222; favourites, D211;
     the command palette, Ctrl+K, D200, with scenes and presets, D262);
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
   - ~~thumbnails and the Needs-work bin~~ (built: D212, D222; the LIB
     "Needs work" filter and DIAG's list of this run's failures);
   - ~~a batch route and the watched drop folder~~ (built, D235, D240).

   *Accept:* representative single-pass and multipass imports play on
   native and ANGLE, a missing asset fails visibly, and attribution
   survives.
4. **Auto-generated controls.** *Stage 1 built* (`strobe-autocontrol`,
   D123-D126). *Stage 2 built (D202):* the app runs the engine in a worker
   process from the Inspector or the palette, and shows the result for
   review (open the copy or discard it), with keep, drop and re-range per
   control (D219). The window context scores on the T480, native and ANGLE
   (Tasks 13 and 23, D140). ~~the same button on the Import pane after an import~~,
   ~~the speed control not following `st_speed`~~ (both D246), ~~a
   second look at the heuristics on real community scenes~~ (T26's
   findings fixed in D259), and a re-run of T26 on TORNADO. *Accept:* every
   fixture, and every library scene with its `CONTROLS` stripped, gets a
   useful, named, grouped control set that visibly changes the image. Met
   on llvmpipe for all 11 library scenes and for the fixtures with anything
   tunable in them; 6 of the 14 fixtures are structural and get none, on
   purpose (see `NO_CONTROLS` in `crates/strobe-autocontrol/tests/engine.rs`).
5. **Playlists, stage 2.** *Built* (D150-D158): load-time precompile with
   progress, a second player for warm-up, the transition compositor, the
   Playlist pane and generate card, the file watcher, resume every 5 s, the
   wall-clock anchor and live commands. *Left:*
   - ~~audio-event advance~~ (built, D226: breakdowns and drops heard in
     the raw energy; D237: song changes, from a gap or a new tempo;
     thresholds still to tune on real sets);
   - the Playlist **workspace** proper: ~~the selected cue's inheritance
     inspector and its edits~~ (built, D213); ~~multi-select edits, drag
     from the library, pasted Shadertoy URLs, comment-keeping save~~
     (built, D224, D229-D231);
   - ~~real tags in the scene format, so Generate can pick by tag~~ (built:
     Synesthesia's `TAGS`, D205);
   - ~~MIDI and OSC mapping of the cue commands~~ (built: commands, `Cue` jumps and fav slots are mapping targets, and `/strobe/cmd/<id>` and `/strobe/cue/<n>` need no mapping, D173);
   - ~~a downbeat detector, so "bars" line up with the music~~ (built,
     D233; not yet heard on real music);
   - the T480: the load time of an 80-scene show, the cost of a transition
     (two scenes at once) on an HD 4000, and VRAM with many scenes parked.

   *Accept:* a 4-hour, 80-cue show authored from tags in under two minutes,
   then a 4-hour soak with zero runtime compiles. Met in part: authoring
   from tags (D205) is a card away, and zero compiles after load is asserted
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
     unmeasured.** `Limit Colors` is built and meta values carry across
     scenes (D207); `Alpha Channel` waits for an output that carries alpha
     (NDI, Phase 7).
   - [x] `textureFinalPass` through an asynchronous PBO readback, two frames
     late, never a stall (D162, Q14). Checked on llvmpipe only; the T480 and
     ANGLE/D3D11 are open.
   - [x] No shader compiles after load: a late `setUniform` is refused
     (D161).
   - [x] Implicit `int` to `float` written out by a typed rewrite on every
     dialect, `#if` on a non-macro read as 0, and initialised uniforms on
     ES (D163, D199). The owner's 90-scene corpus: 87/90 on desktop and
     87/90 on GLES, up from 85 and 17 (D195).
   
   *Accept:* a fixture per behaviour, on both dialects where it applies.
7. **Presets, MIDI, OSC and hotkeys** on the shared `ControlBank`, without
   duplicating its toggle, bang or smoothing semantics. *Built (D170-D179),
   2026-10-11:*
   - [x] `strobe-io`: MIDI (`midir`) and OSC (`rosc`) workers, a bounded queue
     drained once per frame, hot-plug tolerated, a mapping model with range,
     curve, invert, button mode and soft takeover, learn with conflict
     detection, six target states (unavailable is not an error), the built-in
     `/strobe/...` OSC space. OSC is off until asked for and loopback-only
     unless chosen (D172).
   - [x] Presets (`presets/*.json` in the scene folder) and eight fav slots;
     a cue's `preset` now applies on cue start (D176).
   - [x] The MAP pane (devices, OSC port, mappings, learn, activity), the
     curated Performance Macro strip (D175, D177), configurable hotkeys
     (D178), `show-control.json` (D179).
   - Verified: unit tests with no devices, loopback UDP, headless GL in both
     dialects, and the app under Xvfb driven by real datagrams
     ([STATUS_AUDIT.md](STATUS_AUDIT.md)).

   *Left:*
   - **MIDI on real hardware** (a controller on the T480): port names,
     hot-plug timing, a device another program holds open on WinMM, message
     rates. Nothing here has seen a controller (D171, Q23);
   - ~~an OSC **output** for feedback~~ (built, D209); a MIDI output (needs a
     device to test against);
   - ~~Synesthesia's OSC address layout~~ (built: its addresses are
     aliases where they mean the same, D234);
   - ~~macro range and colour editors, pinning from the Controls pane~~
     (built, D210); ~~a shortcut and smoothing per macro~~ (built, D227 and
     D228);
   - ~~an in-app hotkey editor~~ (built, D208);
   - ~~presets for **media**~~ (built, D223: PLAN's third channel, beside
     media chosen in the Inspector and per cue, D217); ~~renaming a
     preset~~ (built, D214);
   - a MIDI-learn flow exercised by a person on the T480 (the Learn click is
     verified under Xvfb with OSC only).

   *Accept (PLAN Phase 6):* every playlist action and control reachable from
   MIDI and OSC. Met for OSC (built-in addresses, tested) and for MIDI by
   mapping (the mapper does not care where a message came from, and is tested
   from recorded bytes); **not met on hardware**.
7b. **LAN remote from a phone or tablet** (v1 built, D250-D254; real
   devices, the firewall prompt and the T480 soak open). The
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
   - ~~checking the `MEDIA` / `CUBEMAPS` / `syn_MediaType` shapes against
     SSF's docs~~ (done in TORNADO T21, Q22; chosen media follows the slot's
     settings, D238); ~~a media picker in the UI~~ and ~~media per playlist
     cue~~ (built, D217: the Inspector's Media row and a cue's `media` key);
   - ~~the T480: the cost of uploading a long sequence, and memory with
     192 MiB of frames on shared memory~~ (measured, T480 Task 28, D225:
     about 4 s to load, about 280 MB, flat, 60 fps).

   *Accept:* a cube-map fixture where each direction returns its face, a
   missing one failing visibly while the scene runs, and an image read
   through `syn_Media`. Met on llvmpipe.
9. ~~**A real tagged release.**~~ **Done (v0.1.0, 2026-10-14).** The zip, the
   ANGLE DLL provenance, the matching PDB archive, the SBOM, the checksums
   and both attestations were inspected on the published release page, after
   the workflow was proven end to end on a throwaway mirror (Q12 resolved,
   D270). The ordered checklist is [RELEASE_PLAN.md](RELEASE_PLAN.md); what
   remains from it is the clean-machine smoke test and symbolication from the
   downloaded archive (steps 4 and 5), which need the T480.
10. **Modern graphics APIs (Vulkan, wgpu, DX12, Metal): research only.** The
   plan is in [GFX_API_RESEARCH.md](GFX_API_RESEARCH.md)
   (lnorton89/strobe#8). OpenGL 3.3 and ANGLE stay the reference and the
   fallback. The gating risk is shader compatibility, not API coverage: Naga
   rejects real SSF and Shadertoy GLSL (see DECISIONS on why glow and not
   wgpu). So stage 0 comes first, and it is useful on its own: a
   machine-readable compatibility report (compile, image checksum, frame
   time, failure category) over the library, the Shadertoy fixtures and
   owner-supplied local shaders, on llvmpipe, the T480 and TORNADO. Then
   come a narrow renderer seam with GL unchanged (stage 1), and a spike
   comparing wgpu/Naga with GLSL to SPIR-V into Vulkan on that same corpus
   (stage 2). A backend ships only behind a flag, until the research doc's
   acceptance matrix passes on the T480 and TORNADO. *Accept (stage 0):* the
   report runs in CI on llvmpipe and catches a deliberately broken shader.
   *Stage 0 built (D195), 2026-10-13:* `cargo xtask corpus` (the
   `strobe-corpus` binary) loads every scene the way the app does, script
   included, renders it in both dialects in a child process per scene, and
   writes `target/corpus/report.{json,md}` with a cause per failure. Its
   integration test runs synthetic scenes on llvmpipe. Open: the CI job that
   runs it on `scenes/`. Windows has no headless EGL, and the harness falls
   back to a window context there: the T480 ran it on `scenes/` in both
   dialects (Task 30) and TORNADO on the owner's corpus (T18).
   *Stages 1 and 2 done (D255), 2026-10-13:*
   - **The seam** is a contract, [RENDERER_SEAM.md](RENDERER_SEAM.md), with
     no code moved.
   - **The spike** (`docs/research/g03-frontends/`) rules naga out: it has no
     combined samplers in either front end, so 0 of 51 passes go through
     its GLSL input. glslang -> SPIR-V -> SPIRV-Cross is viable as a front
     end: 51 of 51 to SPIR-V, HLSL and MSL for 50.
   - **No backend is started:** that waits for a platform that needs one.

   *Follow-up (D292), 2026-10-15:*
   - **The full corpus agrees with the spike.** The owner's 90 Synesthesia
     scenes plus the library, 120 scenes and 288 passes: naga's GLSL input
     takes 0. glslang takes all 227 corpus passes, and SPIRV-Cross writes
     HLSL and MSL for every one.
   - **naga 30.0.1 is still the newest**, with no combined-sampler work.
     Splitting the samplers in SPIR-V first gets naga 60 of 60 library
     passes but only 56 of 227 corpus passes.
   - **The second backend is ANGLE's.** ANGLE's Vulkan backend, already
     inside the shipped Electron build, rendered the library and the
     corpus with no code change (`ANGLE_DEFAULT_PLATFORM=vulkan`, on
     lavapipe). ANGLE's Metal backend is the macOS answer, and needs an EGL
     provider of our own because glutin has no EGL on Apple.
   - See [research/angle-backends.md](research/angle-backends.md).
   - **Next:** backlog G-05 to G-07 (ANGLE-Vulkan on the T480 and TORNADO,
     a backend-type choice in `--gl`, ANGLE-Metal on a Mac). A new
     renderer (G-04) stays deferred.

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
- ~~the Inspector review panel and the app calling `generate` with its own
  context~~ (built: D202, D219); ~~the same review from the Import pane
  and renaming a control~~ (D246), and live previews;
- accepting a suggested audio binding in the app, and binding a speed
  control (it is an integrator, so it is data only in GLSL);
- ~~verifying on a real GPU and the T480~~ (Tasks 13 and 23); ~~trying
  community scenes~~ (TORNADO T26 ran the owner's 65; its six findings are
  fixed in D259); a re-run of T26 to confirm;
- ~~speed controls that also follow `st_speed` and `st_audio_time`
  (D125)~~ (D246: they tick at the import clock's rate);
- vector and `xy` candidates (an offset, a centre) and dropdown candidates
  (a mode switch) are not generated.

## Phases

| Phase | Scope | State |
|---|---|---|
| 0 | Workspace, conventions, CI, GL contexts | Done. Tagged and published as v0.1.0 on 2026-10-14 (D270). |
| 1 | Renderer, SSF, controls, output window | Done, with the Perform-first UI at stage 1 (D130-D138). Images, cube maps and `syn_Media` stills and sequences work (D185, D186); video and webcam do not. |
| 2 | JavaScript API | Integrated (D62). `meta/` writes land in a meta bank (D160) that the output post stage and audio gains read (D180, D184); `textureFinalPass` is an asynchronous readback (D162); late uniforms are refused (D161). |
| 3 | Audio | Integrated (D61). The bench still uses the synthetic stand-in. |
| 4 | Shadertoy import | The converter (D90-D95), acquisition (REST client, texture downloader, localhost listener), the Import panel, file drop and the userscript (D110-D117) are built, auto-generated controls with their review in the app (D123-D126, D202, D219), drops routed by kind (D218) and a URL on a playlist cue (D229). batch import and a watched folder (D235, D240). Open: live-site verification (Q19), a real OS drop by a person. |
| 5 | Playlists | The pure core (D73-D77), the app integration (D150-D158), cue editing with inheritance shown (D213), media per cue (D217), drag, disable, multi-select, URL import and comment-keeping save (D224, D229-D231), and audio-event advance (D226, D237) are built. Verified on the T480 (Tasks 14, 26). Open: hearing the downbeat estimate (D233) and the audio events on real music. |
| 6 | Presets, MIDI, OSC, hotkeys | Built (D170-D179): `strobe-io`, presets and fav slots, the MAP pane, macros stage 2, hotkeys, a hotkey editor (D208), OSC feedback (D209), macro reach and colour (D210), the preset media channel (D223), and a glide and a key per macro (D227, D228). Open: a controller on the T480, MIDI output. |
| 7b | LAN remote / PWA | LAN web remote v1 built (D232, D236, D250-D254): listener, pairing, tokens, revocation, abuse limits, the controller page, the app integration with a kill switch, and the user guide's section. Tested over loopback sockets only. Next: real phones over Wi-Fi, the Windows firewall prompt, the T480 soak; then solve HTTPS before claiming installable PWA support. See [REMOTE_CONTROL_PWA_RESEARCH.md](REMOTE_CONTROL_PWA_RESEARCH.md). |
| 7 | NDI, media input, song-change transitions | Media stills and image sequences (D186), chosen in the Inspector, per cue and per preset (D217, D223), bounded by shrinking (D225); breakdown and drop detection (D226) and song changes (D237) feed playlists. Video, webcam and NDI are open. `HARD`/`SMOOTH_TRANSITIONS` are declared but never change. |

## Known gaps in what exists

- The performer-facing UI is at stage 1 plus the Playlist pane (D157), the MAP pane (D177) and the Editor (D204), with named layouts (D215) and density (D220); little of it has been used on the T480 by a person with real input.
- Context-loss recovery is verified under Xvfb and on real WGL on the T480
  with injected losses: native rebuild, ANGLE fail-over and give-up (Task 8,
  D66). A real driver reset has not been seen there yet (bug 1).
- The bench's `syn_Spectrum` and `syn_LevelTrail` are silent placeholders,
  and the bench does not run scripts. `11-script-orbits` drives controls
  instead (D62).
- `syn_Media` shows a still or an image sequence only (D186); **video and
  webcam are later** and read as no media. Cube maps load (D185). The
  `MEDIA` and `CUBEMAPS` shapes and `syn_MediaType`'s values are checked
  against SSF's documentation (TORNADO T21, Q22, D238); cube orientation is
  not documented there. The bench has no media or cube-map scene.
- The meta controls change the picture (the post stage, D180) but the stage's
  cost on an old Intel GPU is unmeasured, and `Alpha Channel` waits for an
  output that carries alpha (D207, Q21).
  `textureFinalPass` is two frames late and unverified on the T480 (D162,
  Q14).
- Implicit int-to-float is written out by a typed, parser-backed rewrite on
  every dialect (D199, R8), not only `PASSINDEX == 0.0` (D163). On ES it
  also types function-like macro calls through their expansions, and on
  every dialect a sampler chosen with `?:` inside a call is moved outside
  it (D261); both are checked on llvmpipe only, not yet on ANGLE hardware
  or the owner's corpus. The base library is written ES-clean.
- Adaptive-resolution thresholds are validated on one machine, the T480.
  The missed-refresh rule, its verdict and the fast third step (D314-D317)
  are verified on the mini PC (M20-M24), as are D318's put-off verdicts
  and D319's settled line. D318's lock expiry has not yet happened on the
  hardware, and D320's stall exclusion and log lines are checked on
  synthetic sequences only; Task M25 is their hardware test.
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
- Built since: thumbnails (D212, D222), the "Needs work" filter and list,
  batch import of a dropped folder of scenes (D235) and a watched folder
  (D240), set in Settings and saved (D265). Still open: a real OS drop by
  a person.
- Built since: where Synesthesia is installed, its scenes are in the
  library by default (D271, backlog I-05): the ones it ships read-only
  after the library folder's, then the scene folders in its own settings.
  A write into the install is refused with "Copy to my library". Settings
  and `--no-synesthesia` turn it off. Checked with fake installs and a
  fake install over the private corpus on Xvfb; open: a real Windows
  install by a person.
- Not supported:
  - `cubemap` and `volume` inputs, and `sound` and `cubemap` passes: these
    are errors;
  - `keyboard`, `video` and `webcam` inputs: they read black, with a
    warning;
  - `srgb` textures: not decoded (Q16);
  - packing built-ins on a 3.30 context: not rewritten. Implicit
    int-to-float is written out by D199's typed rewrite (R8).
- A resize restarts `iTime` at `TIME` and `iFrame` at 0 only when it wipes
  a buffer that reads them (`resize` on an import with buffers, D91), which a
  buffer initialising on `iFrame == 0` needs. An import whose Image pass
  alone reads time, and any `resize_keeping`, keeps the clock (D287).
- Every buffer is double-buffered, even one that never reads itself (D95).

### Playlists (stages 1 and 2)

The app plays playlists (D150-D158). Known gaps:

- **Audio-event advance** (`drop`, `breakdown`, `song`) fires from
  `strobe_audio::events` (D226) and `strobe_audio::song` (D237), tested on
  synthetic structure, not yet on real music. A tempo-matched DJ mix with
  no gap gives no song change.
- **The downbeat estimate** (D233) moves the bar lines a beat-locked cue
  ends on to the beat it hears as "one", once it is sure; while unsure
  (even kicks, the first eight bars) bars count from the grid's origin
  (D75). Tested on synthetic kicks, not yet on real music.
- **The Playlist pane** edits one cue or several (D213, D230), takes scenes
  dragged from the library and Shadertoy URLs (D224, D229), and its save
  keeps a hand-written file's comments (D231). Not yet timed against the
  80-cue, two-minute acceptance by a person.
- **Tags are edited for the playing scene only** (the Inspector), and the
  Generate form has no exclude field yet (D205).
- **A cue's `preset` applies** (D176); `meta` overrides and presets reach
  the session's meta bank, which the output post stage reads (D180).
- **Two scenes render during a transition**, doubling the cost for its
  length. Unmeasured on the T480.
- **Loading a scene by hand unloads the playlist** (D150).
- **Context loss** reloads the playlist and compiles every scene again
  before it resumes; on a big show that is a pause the length of the load.
- Everything here was verified on llvmpipe under Xvfb. On the T480: the
  transition demo (Task 14), a 30-minute show (Task 17) and media per cue
  (Task 26).
