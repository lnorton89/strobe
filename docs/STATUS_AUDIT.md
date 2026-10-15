# Strobe status and gap audit

_Last reconciled: 2026-10-13 against the current roadmap and the comprehensive
[implementation backlog](IMPLEMENTATION_BACKLOG.md). The evidence entries below retain their original dates and machine context; do not interpret this reconciliation date as a fresh rerun of every test._

This is an implementation handoff, not a claim that every platform path
is verified. "Implemented" means the code is merged and tested in CI.
Hardware-dependent behaviour stays gated on the evidence listed here.
Ticked items cite where the evidence is. [ROADMAP.md](ROADMAP.md) carries
the short priority order; [IMPLEMENTATION_BACKLOG.md](IMPLEMENTATION_BACKLOG.md)
tracks every known unfinished implementation and validation task across the project.

## Current baseline

- [x] Full desktop remote implementation (2026-10-13, D275) is wired into
  Settings > Remote and the ordinary host egui pass. Evidence:
  `desktop::tests::native_controller_pairs_decodes_edits_host_files_and_reconnects_in_memory`
  exercises native pairing, JPEG decoding, host text-widget editing, the
  existing editor's save/backup path and credential rotation on reconnect.
  `strobe-remote/tests/server.rs` verifies opt-in, separately approved desktop
  scope, refusal of old/rotated phone tokens, exclusive control, clipboard,
  revocation and listener shutdown over real loopback sockets. Desktop bridge
  tests verify finite/bounded input and overflow releases; viewer tests verify
  Tab forwarding, local keyboard exit, Unicode paste and input reset.
  Native RX 580 GL renders of the connection form (900x700, 427x533) and
  network-fed viewer (1440x900, 640x480, 427x533) are in
  `target/desktop-remote-qa/`; inspected with no layout overflow. This is a
  loopback host fixture, not a claim of two physical machines tested.
  Final `cargo xtask ci` passed fmt, workspace all-feature clippy/tests,
  cargo-deny, changelog and brand (`target/desktop-remote-ci.log`); explicit
  full-workspace MSVC-target clippy passed (`target/desktop-remote-windows.log`).
  108 GPU tests passed by skipping unavailable EGL/headless contexts; the
  native GL fixtures provide separate render evidence.
  A separate optimized `target/release/strobe-desktop-control.exe` was built
  from the same app entry point because the existing `strobe.exe` is running
  and locked. Its Windows startup/screenshot smoke test exited successfully
  and wrote `target/desktop-remote-smoke-control.png` and
  `target/desktop-remote-smoke-output.png`. This local test
  build is not a packaged release; the original running processes were left
  untouched.
  PR integration was repeated against upstream `5507a13` on 2026-10-14:
  `cargo xtask ci` and the explicit all-workspace Windows-target clippy
  passed (`target/desktop-remote-pr-ci.log`,
  `target/desktop-remote-pr-windows.log`). New rustls advisories required a
  compatible lockfile patch from 0.23.45 to 0.23.46; no advisory was ignored.
  Fresh before/after thirteen-scene GPU timings are in BENCHMARKS.md.
  Committed visual fixtures: `docs/images/perform/desktop-remote-host-900x700.png`
  and `desktop-remote-viewer-1440x900.png` (native loopback fixture captures).
- [x] Full desktop remote review hardening (2026-10-14, D275). Evidence, all
  on loopback in the Linux container: `clients::tests::a_desktop_grant_is_never_saved_and_ends_with_the_switch`;
  `tests/server.rs` `desktop_input_reaches_the_app_only_from_the_lease_holder`,
  `desktop_input_is_bounded_and_validated_before_the_app_sees_it` and
  `switching_desktop_access_off_ends_desktop_pairings_and_spares_phones`;
  `lib::tests::desktop_messages_read_and_write_byte_for_byte`;
  `desktop::tests::a_remote_pass_cannot_widen_who_reaches_this_computer` and
  `leaving_a_desktop_never_blocks_the_frame_loop`. The asynchronous capture
  readback is not exercised by any test: the headless context has no default
  framebuffer.
- [x] Desktop viewer polish after the first two-machine test (2026-10-15,
  D286). Evidence, Linux container: `desktop::tests::a_dropped_viewer_keeps_showing_the_last_image_dimmed`,
  `a_waiting_viewer_shows_elapsed_time_a_hint_and_a_way_out`,
  `an_unapproved_connection_gives_up_and_says_so` (real socket; the host
  withdraws the request because the worker closes its socket),
  `input_that_cannot_reach_the_host_ends_the_connection`,
  `a_blank_code_after_the_host_forgot_us_asks_for_a_new_one` (real socket),
  `a_failed_connection_offers_reconnect_only_when_a_pairing_is_held`;
  `lan::tests::only_authenticated_clients_count_as_connected` and the refused
  socket assertions in `tests/server.rs`; shell tests
  `every_viewer_state_fits_every_size_without_text_over_text`,
  `the_palette_opens_the_connect_form_from_its_own_entry` and
  `the_connect_form_is_its_own_section_with_a_hint_not_a_value`; viewer
  screenshots from `STROBE_UI_SHOTS` on llvmpipe. Not re-run on the T480 and
  RX 580 pair.

- [x] Venue mode, host and protocol (2026-10-15, D288). Evidence, all on
  loopback in the Linux container: `trust::tests` (RFC 4231 vectors, an
  independent protocol vector, wrong secret/host/controller/nonce, single use
  and expiry, file parser, no overwrite and no write through a symlink, mode
  0600, loose-permission refusal, limiter backoff, global trip and bound);
  `clients::tests::a_trusted_pairing_is_in_memory_only_listed_replaced_and_revocable`;
  `lib::tests::trust_messages_read_and_write_byte_for_byte_and_are_validated`;
  `tests/server.rs` `a_controller_that_proves_the_trust_file_pairs_with_no_approval`,
  `neither_the_secret_nor_its_proof_is_on_the_wire_or_in_the_log`,
  `a_wrong_secret_a_wrong_controller_or_a_wrong_host_is_refused`,
  `a_replayed_response_is_refused_because_each_nonce_answers_once`,
  `trusted_pairing_is_refused_without_a_trust_file_or_with_desktop_control_off`,
  `the_kill_switch_ends_a_trusted_session_and_stops_new_ones`,
  `the_operator_can_revoke_a_trusted_controller_for_the_run`,
  `a_controller_that_reconnects_takes_back_its_own_lease`,
  `failed_proofs_back_off_per_address_and_then_recover`,
  `phone_pairing_is_unaffected_by_a_trust_file`,
  `version_one_traffic_is_byte_for_byte_what_it_was`; `venue::tests` and
  `cli::tests::venue_mode_flags_need_the_remote_and_are_off_by_default`.
- [x] Venue mode, controller side (2026-10-15, D294). Evidence, all on
  loopback in the Linux container: `trusted::tests`
  (`the_proof_is_the_protocols_published_vector`,
  `the_host_accepts_what_the_handshake_makes_and_only_for_the_host_dialled`,
  `a_challenge_nobody_asked_for_is_refused_and_so_is_a_second`,
  `the_retry_schedule_doubles_to_fifteen_seconds_and_stays_there`,
  `a_controller_id_is_valid_and_survives_odd_computer_names`);
  `trust::tests::a_generated_controller_id_is_always_valid_and_keeps_the_name_readable`;
  `desktop::tests` with a real host:
  `a_trusted_controller_connects_with_no_approval_and_keeps_the_secret_to_itself`,
  `the_wrong_trust_file_is_refused_once_and_says_why_without_retrying`,
  `a_trusted_controller_finds_a_restarted_host_by_itself_and_takes_its_lease_back`,
  `disconnect_stops_the_retrying`,
  `a_trusted_controller_shut_out_of_the_lease_backs_off_rather_than_hammering`,
  `the_kill_switch_is_retried_once_and_then_refused`,
  `a_challenge_nobody_asked_for_is_not_signed_and_ends_the_connection`,
  `the_viewer_says_reconnecting_with_the_attempt_and_keeps_the_last_image`,
  `the_form_remembers_a_path_and_a_launch_with_control_decides_the_first_action`;
  `cli::tests::control_names_a_host_and_takes_the_trust_file_for_the_laptop`;
  `shell_tests::the_connect_form_offers_a_trust_file_in_place_of_the_code`.
- [x] Venue mode on loopback on a real Windows machine (2026-10-15, JaJa
  mini PC, MINIPC.md M6): trusted pairing with no approval, the viewer shows
  the host UI, and after a host kill the controller shows "Reconnecting..."
  and re-pairs by itself 13 s later.
- [ ] Venue mode, on two physical machines: a laptop controlling a show
  machine over Wi-Fi, a Wi-Fi drop while connected (do the retries land
  within the 15 s cap on a real network, and is the lease taken back?),
  Disconnect while reconnecting (untested), the controller's file
  permissions on Windows, and a four-hour soak. A Windows host reading a
  trust file from the user profile is covered by M6.
- [ ] Full desktop remote: two physical Strobe desktops over Wi-Fi, reconnect
  after loss/sleep, minimized host, a host control window covered by the
  fullscreen output (does it still redraw and capture?), the fenced capture
  readback on a real window (native GL and ANGLE), cross-DPI/zoom behavior on
  real monitors, a remote click on Settings' Approve refused with the banner,
  and T480 CPU/frame-time/memory soak remain unverified.
- [x] The control frame runs while Windows sends the control window no
  paints (2026-10-15, D290). Verified on the JaJa mini PC (MINIPC.md M5):
  `control window: no paint from Windows for 1217 ms (takeover 1)` at 3.5 s,
  about 119 driven redraws a minute all unpainted, playlist 14/14 in 5.4 s,
  59.9 fps, and `WM_CLOSE` to the control window closes cleanly. The laptop
  remote showing and operating the host on that machine is the two-machine
  item above. Built; the scheduling decision is unit-tested
  in the Linux container (`stats::tests::a_control_window_with_no_paints_is_run_by_the_loop_at_its_own_period`,
  `a_painted_control_window_is_never_driven_by_the_loop`,
  `a_blocked_loop_is_not_a_window_without_paints`,
  `the_loop_hands_back_only_to_a_prompt_paint`). No container reproduces
  Windows' paint order. Acceptance test, on the JaJa mini PC with the show
  launch: the log says `control window: no paint from Windows ... (D290)`,
  every later `perf:` line has a nonzero `control N redraws ... (... M unpainted)`
  with the output still at 60 fps, the playlist gets past 1 of 14, and the
  laptop's desktop remote shows and operates the host (MINIPC.md).

- [x] Chromatic Rise scene loads, compiles and renders on Windows RX 580
  native GL 3.3 and ANGLE GLES 3.0 at 1280x720 (2026-10-13).
  Evidence: `target/bench/chromatic-rise-final/results.md` and
  `target/bench/chromatic-rise-angle-final/results.md`, corresponding PNGs
  visually inspected; timings in BENCHMARKS.md. Exact reference fidelity,
  live audio tuning and older integrated-GPU performance remain unverified.
  `cargo xtask ci` passed the available gates on this Windows MSVC host:
  fmt, workspace all-feature clippy/tests, deny, changelog and brand;
  separate Windows cross-checks are redundant on this host. 108 headless
  GPU tests skipped for unavailable EGL contexts; actual shader rendering
  is evidenced by the native/ANGLE window-context benchmarks above.
  The scene corpus check also passed both dialects at 640x360 for ten
  frames (`target/corpus/chromatic-rise/report.md`); a separate frame at
  TIME=30 was visually inspected to confirm evolving shapes.
  Superseded 2026-10-14 by the D276 rebuild (feedback loop, two passes):
  verified only headless on llvmpipe in both dialects so far; the Windows
  native/ANGLE and HD 4000 re-run is open.

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
core (stage 2, D150-D158), and `strobe-autocontrol` through a worker
process (D202).

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
- [x] Test the packaged release executable separately (Q12): built with
  the workflow's flags, run from `%TEMP%` and from another working
  directory, crashed on purpose and symbolicated with no `--symbols`
  (T480 Tasks 21 and 22). An archive from a real tagged release is R-05.
- [x] Run for at least 30 minutes with repeated scene switches, and
  inspect for crashes, log growth and resource stability. TORNADO T19
  (commit 9215858): 31 minutes, a 10 s crossfade playlist over all 12
  scenes, with 186 cues and 185 transitions, all finished.
  - No crash, warning or error; a clean close (exit 0).
  - Working set 145.1 to 145.6 MB over the last 20 minutes (0.2 MB per
    10 minutes), with handles flat at 421.
  - The log grew 2.9 KB a minute.
  - 60 fps, with 0 to 4 late frames a minute.

  Not yet: the same on the T480's iGPU, whose earlier 30-minute run (Task
  17) held one scene.
- [ ] Offscreen frames render on AMD native WGL at any size (D296, fixed
  pending verification by D311). The mini PC's Vega 3 dropped draws into
  targets over 16384 pixels until the context had drawn into a small one
  (MINIPC.md M11-M14); every context is now primed with one 16x16 draw
  (`gl::prime`). Headless evidence covers only the prime's clean state, not
  the fix: `the_context_prime_leaves_the_state_clean`
  (`strobe-gfx/tests/render.rs`, both dialects). Acceptance test, MINIPC.md
  Task M15: `strobe-bench --context window --gl native` at 640x360 and
  1920x1080 with no `STROBE_D296_*` variables, no all-black warnings, the
  `GL prime: ... landed` log line, `--gl angle` matching, and black again
  with `STROBE_GL_NO_PRIME=1`.
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

## P1: Linux

Linux is documented and builds (D295); what a person on a real Linux machine
would meet is not yet seen. Evidence so far is the cloud container, with no
GPU, on Mesa's llvmpipe.

- [x] A clean Ubuntu 24.04 builds the app and the bench with
  `build-essential pkg-config libasound2-dev` and nothing else, and the
  bench, `--diagnose` and a control-plus-output screenshot run on X11 (Xvfb)
  and Wayland (headless Weston) once the run-time libraries in the README are
  installed (2026-10-15, D295).
- [x] The Nix flake: `nix develop -c cargo build`, `nix build` and `nix run`
  on x86_64-linux; the package's `--help`, `--diagnose`, bench, and X11 and
  Wayland screenshots (2026-10-15, D295). aarch64-linux evaluates only.
- [ ] A real Linux GPU (Intel, AMD, NVIDIA): native GL 3.3 core, the
  scenes, timings. Acceptance: `strobe --diagnose` and `strobe-bench` output
  with the driver named, in `docs/hardware/`. Only WSL2's D3D12 driver on the
  RX 580 has run (the WSL2 item below, AMD.md A14); that is Mesa over
  Windows' GPU driver, not a Linux GPU driver.
- [ ] Audio capture on ALSA, PipeWire and PulseAudio machines, and
  removal and reconnection of the device. Acceptance: a live input moves the
  level meter; the log shows the transitions. Under WSLg (A14) the source was
  synthetic until ALSA's default was pointed at PulseAudio; the `pulse` and
  `pipewire` fallback is unit-tested but not yet seen live.
- [ ] MIDI input from a real controller through ALSA's sequencer.
- [ ] GNOME and KDE on Wayland: window decorations, the custom title bar,
  fullscreen output on a second monitor.
- [ ] NixOS: `nix run` opens a window with `hardware.graphics.enable`, and
  PipeWire's ALSA plugin finds the input.
- [x] WSL2 with WSLg on the AMD desktop's Debian (2026-10-15, AMD.md A14):
  toolchain, build (289 s), `--help`, the headless bench (llvmpipe over EGL),
  the window bench and the three screenshots (default, X11, Wayland) pass;
  apt was pre-done and MIDI is skipped. Mesa uses llvmpipe unless
  `GALLIUM_DRIVER=d3d12`, which names "D3D12 (Radeon RX 580 Series) | 4.2
  Core" and ran about 3x faster. Transcript: `target/linux-check.txt` from
  `scripts/check-linux.sh`.

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
- [x] Narrow windows at the T480's 150% sizes (Task 35 found the drawer
  over BLACKOUT, overlapping toolbar and title bar, and a garbled portrait
  transport; D260 fixes them). Evidence: in every scenario at every size,
  including 683x427, 533x400, 427x320 and 320x533 points,
  `nothing_is_painted_over_blackout_at_any_size` and
  `no_text_is_drawn_over_other_text_at_any_size` pass, and so does the
  overflow test. Headless PNGs (`STROBE_UI_SHOTS`) were read by eye. Not
  yet re-checked on the T480 itself.
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
  the Xvfb screenshot `docs/images/perform/palette-1440x900.png`. It also
  lists library scenes (Load, Preview) and the playing scene's presets
  (Apply), after exact command matches (D262). Evidence: `command_palette`
  tests and `ui::shell_tests::the_palette_loads_previews_and_applies_presets_by_name`.
  **Not yet:** used by a person on Windows.
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
- [x] Named layouts (D215), saved in Settings and applied from Settings or
  the palette. Evidence: `ui::state` and `command_palette` tests.
- [x] Saving over a hand-written playlist keeps its comments and layout
  where the content still matches, a cue's comment travels with the cue,
  and a merge that would not read back exactly falls back to the canonical
  text (D231). Evidence: `merge` and `file` tests.
- [x] Several cues edited at once (D230): a length, transition, preset,
  media or disable goes to every Ctrl-clicked cue; structure stays one cue
  at a time. Evidence: `show::edit` tests. Not yet: clicked by a person.
- [x] A key per macro (D228): held, the macro goes to the top of its reach,
  let go to the bottom; a key a command holds is refused; one key drives
  one macro. Evidence: `strobe-io` engine test, the shell's key test, and
  on the T480 (Task 29) with posted key messages: 0.8 on press and 0.2 on
  release within 18 ms.
- [x] A glide per macro (D227): a mapped fader eases even a plain slider,
  and any other write ends the glide. Evidence: `strobe-io` engine and
  `strobe-scene` runtime tests.
- [x] Breakdowns and drops heard and handed to the playlist clock (D226):
  once each, none on a short dip, on silence or a reconnect, the same at
  30 and 144 fps, and through the real analyser on a kick/hiss/kick
  signal. Evidence: `strobe-audio` `events` tests. Not yet: real music.
- [x] A long image sequence shrinks as a whole to fit 192 MiB and plays
  complete; only below 256 px is it cut, with a message (D225). Evidence:
  `assets` tests with a small budget; on the T480 (Task 28) a 475 MiB loop
  shrank to 192 MiB, loaded in about 4 s, held flat at about 280 MB extra
  and played at 60 fps.
- [x] Cue disable and re-enable from the LIST pane, and a browser scene
  dragged onto a cue inserted after it (D224). Evidence: `show::edit`
  tests (the edits themselves) and the shell scenarios drawing the pane.
  Not yet: a real pointer drag, by a person or a scripted test.
- [x] A preset's media channel (D223): saved when `syn_Media` shows
  something other than the scene's own, applied on recall and at a cue
  start under the cue's own media and the operator's. Evidence:
  `strobe-scene` `preset` tests (relative inside the scene, off in old
  files), `tests/show.rs::a_cues_media_replaces_...` (cue 4 the preset's,
  cue 5 the cue's over the preset's).
- [x] A scene's picture taken from its live frame (D222): `thumbnail.jpg`
  at 320x180 and `IMAGE_PATH` set in place, the browser's cached picture
  dropped. Evidence: `snapshot` and `json_edit` tests. Not yet: clicked by
  a person.
- [x] Density, Comfortable or Compact (D220), saved and chosen in Settings
  or with `--density`; compact fits all 12 library scenes at 1280x720.
  Evidence: shell scenarios at compact density (no overflow at any size),
  `ui::state` round trip, `docs/images/perform/compact-browser-1280x720.png`.
  It also fixed the macro cards, whose sliders were clipped off at both
  densities.
- [x] The generated-controls review keeps, drops and re-ranges each
  control, rewriting the copy without measuring again (D219); dropping all
  deletes the copy. Evidence: `strobe-autocontrol/tests/engine.rs::a_review_...`
  (the reviewed copy compiles and draws the original at its defaults) and
  `generate::tests::a_review_...`.
- [x] Dropped files routed by kind (D218): scene folder, playlist,
  picture or picture folder, Shadertoy export; a scene from outside the
  library is copied in, never over one already there. Evidence:
  `intake` tests. Not yet: a real OS drag on Windows or X11 by a person.
- [x] Choosing `syn_Media` in the Inspector and per cue (D217): a cue's
  media replaces the scene's own and the next cue gets it back, with no
  compile; choosing nothing returns a scene to its own picture or to black.
  Evidence: `strobe-gfx/tests/assets.rs::choosing_none_goes_back_...`,
  `strobe-app/tests/show.rs::a_cues_media_replaces_...`, `strobe-playlist`
  parse and write round trip, `show::edit` tests; both GL dialects on
  llvmpipe; on the T480 (Task 26) the cue, operator and scene precedence
  held with no black flash. Not yet: the Media row clicked by a person.
- [x] Preset rename, Replace named before it happens, Delete in two clicks
  (D214). Evidence: `strobe-scene` `preset` tests.
- [x] Editing the selected cue in the LIST pane, with each setting's origin
  shown, and unsaved edits kept when the file changes on disk (D213).
  Evidence: `show::edit` tests and the shell scenario. Not yet: run by a
  person on the T480.
- [x] Scene thumbnails from `IMAGE_PATH` in the browser (D212), decoded on
  a worker and bounded. Evidence: `ui::thumbs` tests, the library scan
  test, and [images/perform/browser-thumbnails-1440x900.png](images/perform/browser-thumbnails-1440x900.png).
- [x] Favourite scenes (D211): a star per browser row and a Favourites
  filter, saved in `ui.toml`. Evidence: `library` and `ui::state` tests.
- [x] The macro range and colour editor, and pinning from the Inspector
  (D210). Evidence: `strobe-io` `macros` tests, the shell scenarios, and a
  strip screenshot under Xvfb. Not yet: real input on the T480.
- [x] OSC feedback out (D209). Evidence: `strobe-io` `feedback` tests,
  and the app under Xvfb sending its state to a Python listener and
  following a control set over OSC. Not yet: a real TouchOSC layout, and
  MIDI output.
- [x] Synesthesia's OSC input addresses where they mean the same (D234):
  `/controls/scene|meta/<name>`, `/controls/global/<type>/<n>`, two or three
  numbers to a whole pad or colour, `/favslots/<n>`, `/playlist/...`.
  Evidence: `strobe-io` `osc` and `engine` tests, and T480 Task 31 over
  UDP, each address with the feedback it produced.
- [x] Audio-structure inputs to the playlist (D226, D233, D237): breakdowns
  and drops, a downbeat estimate that moves the bar lines when sure, and
  song changes from a gap or a new tempo. Evidence: `strobe-audio`
  `events`, `downbeat`, `song` and `behaviour` tests (synthetic kicks at
  several tempos and phases), and the playlist clock's tests. Not yet: real
  music.
- [x] Shortcuts edited in Settings (D208): capture, a named clash moved only
  on purpose, clear, reset, live and saved. Evidence: `commands.rs` and
  `ui::shell_tests` (capture fires nothing, Esc cancels, the clash row at six
  sizes). Not yet: real keys on the T480.
- [ ] Performance Macros as persistent bindings with the Unresolved states;
  Editor and Mappings (stage 2). (Hold and the Playlist pane are built, D156
  and D157, and the command palette, D200.)
- [x] Scene tags (D205): `TAGS` read leniently and normalised, written in
  place from the Inspector, searched by the browser, and used by Generate
  with a reason per pooled scene. Evidence: `strobe_scene::tags` tests (a
  write keeps every other byte), the library scan test, the generator's
  `Why` tests, a corpus round trip (91 files, other keys unchanged, local
  only), and [images/perform/tags-1440x900.png](images/perform/tags-1440x900.png).
- [x] Tagging any scene, and exclusions in Generate (D264): the Import pane
  tags the scene it imported and the browser its selected scene, playing
  or not, with the Inspector's row and one action naming the scene; the
  Generate form's Exclude field leaves out scenes by tag or name, and the
  preview lists them with the reason. Evidence: `ui::tags` tests (a
  headless click on Save), `show::generate`
  `excluded_tags_and_names_are_never_picked_and_say_why`, and four
  `ui::shell_tests` scenarios at every size.
- [x] The Editor (D204): tabs per source file, unsaved state, Save vs Save
  and reload, atomic saves with backups in the data directory, and "Open in
  Editor" / "Go to line" from a compile error. Evidence: `editor.rs` unit
  tests, the `ui::shell_tests` scenario "editor open with an unsaved edit",
  and [images/perform/editor-1440x900.png](images/perform/editor-1440x900.png)
  (Xvfb). Not yet: typing through a real window manager, the T480.
- [x] Error columns and the `scene.json` check (D263): on Mesa a compile
  error names `main.glsl:<line>:<column>` on the user's text, through the
  assembler's rewrites and Mesa's whitespace folding, and the Editor's "Go
  to" puts the cursor there; Save and reload refuses a `scene.json` the
  loader would refuse, writing and reloading nothing. Evidence: the render
  test `a_compile_error_names_the_users_column_when_the_driver_gives_one`
  (llvmpipe), `assemble` and `editor.rs` tests, the shell scenario "editor
  refused a reload, scene.json does not parse". Not yet: the T480, whose
  drivers give no column.

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
- [x] Limit Colors, and meta settings carrying across scenes (D207).
  Evidence, llvmpipe, both dialects: `post.rs`
  `limit_colors_maps_luma_onto_the_two_colours`; `meta.rs`
  `a_new_bank_carries_what_it_has_not_set_itself`; `tests/show.rs`
  `the_operators_output_settings_carry_across_cues_under_a_cues_own_meta`
  (master 0 and a hue survive two cues, and a cue's own hue wins). Not
  built: Alpha Channel (no output carries alpha yet).
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
- [x] The benchmark does not execute scripts, and says so: `results.md`
  states it above the table and marks each scene with a `script.js` `*`
  (D239). Script cost is the corpus harness's `frame_ms`.
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
  **Video and webcam are not built (later, ROADMAP).** The `MEDIA` key and
  `syn_MediaType` values are checked against SSF's documentation (TORNADO
  T21, Q22); D238 samples chosen media as the slot says. On the T480 a
  475 MiB loop shrinks to fit 192 MiB, loads in about 4 s and holds about
  280 MB, flat, at 60 fps (Task 28, D225).
- [x] Load cubemaps, and report missing or invalid assets without
  crashing the show (D185). Evidence, llvmpipe, both dialects: a fixture
  samples +X, -X, +Y, -Y, +Z and -Z and gets each face's colour from six
  files and from a 4:3 cross picture; a missing cube map and a mismatched
  one each return an `AssetProblem` naming the sampler and file, read black,
  and raise no GL error (`assets.rs`); `failures::asset_problem` has a test
  for the failure block. **Not verified:** cube orientation against a real
  scene (Q22). A missing or broken `IMAGES` file no longer fails the scene
  load: it reads black, is reported, and the scene plays (D201; `assets.rs`
  `a_missing_image_reports_a_problem_and_reads_black`).
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
- [x] Generate controls from the app (D202): the Inspector button or the
  palette starts `strobe --generate-controls` as a worker. The show keeps
  playing, and the result is reviewed and then opened or discarded. The
  source scene is byte for byte unchanged, and the licence travels with
  the copy. Evidence: `crates/strobe-app/tests/generate.rs` runs the real
  executable as the app does, and the Xvfb run in
  `docs/images/perform/generate-controls-1440x900.png` (Ctrl+K,
  "generate", Enter: 4 controls, fidelity 0.0, about 0.2 s on llvmpipe).
  On the T480 the worker ran on a hidden window context and wrote the copy
  (Task 23), and the review keeps, drops and re-ranges each control (D219).
  **Not yet:** the button pressed by a person on the T480.
- [ ] Thumbnails, conversion diagnostics beyond the warnings list, and the
  Needs-work bin. **Built:** thumbnails from `IMAGE_PATH` and from a live
  frame (D212, D222); "Needs work", the LIB filter and the DIAG section
  that lists each scene that failed to load this run with its reason;
  batch import by dropping a folder of scene folders (D235, unit-tested,
  not yet dropped by a person); a watched folder (D240) set in Settings,
  applied live and saved, checked the same way at launch (D265; `intake`,
  `app`, `ui::state` and `ui::settings` tests). **Not yet:** conversion
  diagnostics beyond the warnings list.
- [x] Auto-generated controls engine, stage 1 (D123-D126;
  [ROADMAP.md](ROADMAP.md), "Auto-generated controls"). Evidence:
  `crates/strobe-autocontrol/tests/engine.rs` on headless llvmpipe, both
  GL 3.3 core and GLES 3.0. Every library scene with `CONTROLS` stripped
  (11 of 11) and 8 of 14 Shadertoy fixtures get controls; the other 6 are
  structural and documented in `NO_CONTROLS`. Each kept control changes the
  image (scored, then re-checked on the finished scene), the rewrite
  reproduces the original at its defaults (fidelity 0 in all 25), a second
  run is identical. On the T480 the engine scored on Intel through a hidden
  window, native and ANGLE agreeing (Tasks 13 and 23), and the app's worker
  wrote the copy there (Task 23). The app calls the engine from the
  Inspector and the palette (D202), and the review card keeps, drops and
  re-ranges each control (D219).
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
  (12, Q8). Re-run at 40ee040 (T4): 55 native, 38 ANGLE. **At bc4668e
  (T6, after D199): 56 native and 56 ANGLE, with no regressions; with
  scripts running (`strobe-corpus` on Windows), 62 of 65.** The missing
  picture now plays as black (D201), so T8 and T18 give 63 of 65 in both
  dialects. The two left are the scenes' own bugs.
  `magnetismic` drew differently on ANGLE because a pass sampled its own
  target (T7). D203 double-buffers such passes, confirmed in T8 (native and
  ANGLE now agree, 0.00 at 100 frames). Two scenes still differ, from
  undefined behaviour in their own code (`pow(0,0)`, `fract(sin)` hashes;
  T9). Strobe does not fix those.

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
  on the T480, a 4-hour show with real scenes, and audio-event advance on
  real music (built, D226 and D237, and tested only on synthetic audio).
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
- [~] LAN remote v1 (D250-D254, 2026-10-13). **Verified, with evidence:**
  - the listener over real loopback sockets in CI
    (`crates/strobe-remote/tests/server.rs`, 17 tests): the page and its
    CSP; a foreign `Host` (421) and `Origin` (403); pairing only with the
    code and an Approve, denial, a replayed code, five wrong guesses burning
    the code; token rotation, revocation dropping open connections, paired
    clients surviving a restart; oversized, malformed, unmasked, binary and
    non-UTF-8 frames; a flood (1008); a full command queue answered busy
    with nothing queued beyond its capacity; a client that stops reading
    dropped; the connection limit (503); idle, unpaired and half-sent
    requests timed out; stop dropping every client at once and freeing the
    port; a public address refused and a taken port explained; and a log
    capture finding no code or token in any log line;
  - interoperability with a real WebSocket implementation: Node 22's
    built-in client (undici) fetched the page, paired, ran `blackout_on` and
    saw the state change, re-authenticated with a rotated token, and was
    refused without an `Origin` (a scratch server on loopback, 2026-10-13);
  - **the running app** under Xvfb with llvmpipe (`--remote --remote-bind
    127.0.0.1`, a paired client seeded in `remote-clients.json`), driven by
    Node's WebSocket client: the log has `remote: listening on
    127.0.0.1:47820`, `client c0000abcd connected`, `blackout on` with
    `remote: ran blackout_on`, `remote: refused toggle_hold: needs a
    playlist; load one on the desktop`, `remote: ran next_scene` with
    `Loaded 01-aurora-gradient`, and `blackout off`; the phone side received
    `authenticated`, a rotated token, and a state message after each change
    (revisions 1 to 4). The title bar's "Remote on" and Stop remote are in
    [`remote-on-1440x900.png`](images/perform/remote-on-1440x900.png);
  - the app side: `strobe_app::lan` tests (nothing listens until started,
    refusals for playlist commands, start, publish and stop), the
    `--remote` flags, and the shell drawn with the remote on and a phone
    waiting at every size without overflow (`shell_tests`);
  - `cargo deny` clean with `sha1` 0.10 added.
  **Not verified:** any phone or tablet (Android Chrome, iOS Safari) over
  Wi-Fi, the page rendered in a real browser, the Windows firewall prompt,
  Windows at all, multiple phones, Wi-Fi loss and sleep, and the T480 soak
  for frame time, idle CPU and memory (L-08).
- [x] Run a tag-triggered release, then inspect:
  - the zip and the ANGLE DLL provenance;
  - the matching PDB archive;
  - the SBOM, checksums and attestations.
  Evidence: **v0.1.0, 2026-10-14** (run 38495718734; D270, Q12 resolved).
  The workflow's three jobs (draft, Windows build, publish) all succeeded,
  after the same three ran green on a throwaway mirror first (run
  38494313118, since deleted). All six assets were downloaded **from the
  release page**, every `.sha256` verified, and `gh attestation verify`
  returned 0 for build provenance and 0 for the CycloneDX predicate. The zip
  holds exactly `strobe.exe`, `libEGL.dll`, `libGLESv2.dll`,
  `d3dcompiler_47.dll`, `LICENSES.chromium.html`, `README.md`, `LICENSE` and
  twelve `*.synScene` folders; the symbols archive holds `strobe.pdb`; the
  SBOM is CycloneDX 1.3 with 279 components. The ANGLE DLLs are covered by
  `LICENSES.chromium.html` and were hash-checked by
  `scripts/fetch-angle.ps1` during the build, which fails the release on a
  mismatch. **Not yet:** the clean-machine install and smoke test, and
  symbolicating a crash from the *downloaded* archive (steps 4 and 5 of
  [RELEASE_PLAN.md](RELEASE_PLAN.md)).
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
