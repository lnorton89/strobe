# Strobe implementation backlog

**Purpose:** one evidence-based register of every known unfinished or not-yet-verified project area. This is the implementation companion to ROADMAP.md, STATUS_AUDIT.md, PLAN.md, UI_IMPLEMENTATION_PLAN.md, GFX_API_RESEARCH.md, and REMOTE_CONTROL_PWA_RESEARCH.md. It consolidates their open work; it does not re-label untested work as complete.

**Last reconciled:** 2026-10-12 against the current roadmap and status audit.

## How to use this register

- Status means evidence, not optimism. **Open** means not implemented or not verified; **Blocked** means an external dependency, hardware, decision, or fixture is needed; **In progress** means code exists but the acceptance gate is not met; **Done** means code and stated evidence exist.
- A CI pass proves only what CI exercises. It does not prove Windows window-manager behaviour, a physical MIDI device, a mobile browser, an external service, or a long-running real-GPU show.
- Complete work in dependency order. Do not start a new backend, media decoder, or network surface before its compatibility, security, and performance gates are met.
- Each task must update ROADMAP.md and the relevant evidence file when completed. Record machine, OS, GPU/driver, backend, commit, command, logs, and failures for hardware results.
- This backlog is intentionally comprehensive. It is not a promise that every feature should ship in one release or that all independent platform-dependent work can be verified by a cloud agent.

## Release gates and dependency order

1. **G0 — Reliability baseline:** resolve remaining T480 audio lifecycle and soak anomaly; run a clean packaged release; verify crash reports and release artifacts.
2. **G1 — Finish current desktop workflows:** complete Perform stage 2, generated-controls review, playlist authoring, import workflow, show-control editors and media selection.
3. **G2 — Compatibility:** close real-world SSF/Shadertoy gaps with fixtures and documented degraded behaviour; complete the remaining NVIDIA corpus re-run.
4. **G3 — Live control surfaces:** test physical MIDI, complete output feedback and blackout commands, then implement the LAN remote service and controller UI.
5. **G4 — Media and show quality:** playlist media, downbeat/audio-event advance, media resource limits, song-change transitions; evaluate video/webcam and NDI separately.
6. **G5 — Modern renderer research:** build a compatibility report first; only then add a narrow renderer seam and compare wgpu/Naga with direct GLSL-to-SPIR-V. GL/ANGLE remain supported.
7. **G6 — Release:** inspect and attest a real tagged release on supported Windows hardware; do not call the product release-ready before this gate.

## Epic R — Reliability, Windows, release

### R-01 Audio device removal and reconnection
- **Status:** Open; live capture works, device lifecycle does not have complete real-hardware evidence.
- **Work:** unplug/replug the selected input during playback; exercise default-device changes, device disappearance during capture, retry timing, switching to synthetic fallback and returning to live input. Keep the UI status truthful and avoid blocking the render thread.
- **Acceptance:** record logs and observed UI transitions in hardware/T480.md; output keeps rendering throughout; no panic, deadlock, stale device handle or repeated unbounded retry; a device returning can be selected/reopened without restarting Strobe.
- **Dependencies:** physical Windows audio device and access to the T480.

### R-02 Investigate the minute-six performance/resource step
- **Status:** Explained, pending confirmation (T480 Task 20): not reproduced in two 10-minute runs. The before/after values match fullscreen versus windowed output exactly, so the output most likely left fullscreen. The output window now logs resizes, fullscreen state and occlusion, so a repeat names itself.
- **Work:** repeat the same scene-switching soak with process memory, handle counts, GPU time, scale, scene and backend sampled at a fixed interval; compare a no-switch control run and native versus ANGLE. Identify whether the step is a one-time allocation/cache or a leak/regression.
- **Acceptance:** explain the step with evidence or file a narrowly scoped follow-up bug; no unexplained monotonic growth over the full soak.

### R-03 Packaged release smoke test
- **Status:** Run on the T480 (Tasks 21 and 22). The one bug it found, the library path depending on the working directory, is fixed and verified on the packaged build. Left: an archive built by `release.yml` itself from a tag (Q12, R-05).
- **Work:** build the release archive from a clean checkout; install/run outside the development tree; test first-run data directories, audio, import listener, scripts, diagnostics, native GL and ANGLE; confirm clean shutdown and upgrade/relaunch behaviour.
- **Acceptance:** attach exact commands, archive hash, OS/GPU/driver, logs and screenshots to hardware/T480.md; test does not depend on Cargo or developer-only DLLs.

### R-04 Crash diagnostics and symbolication in packaged build
- **Status:** Done on the T480 (Task 22): `STROBE_FAULT=crash@120` on the packaged `strobe.exe` wrote a minidump and report, and `cargo xtask symbolicate` with no `--symbols` found the packaged PDB and named `strobe_app::fault::recurse() [fault.rs : 170]`. Left: the same on an archive downloaded from a release (R-05), and an audit of the report text for secrets.
- **Work:** reproduce the crash demo from the packaged release, check diagnostic directory permissions and dump retention, verify matching PDB archive and symbolicate using the shipped build ID.
- **Acceptance:** a packaged-build dump symbolicates to useful function and source locations; secrets and tokens are absent from diagnostic output.

### R-05 Tagged release, supply chain and provenance
- **Status:** Open.
- **Work:** create a real release tag; inspect ZIP contents and layout; verify ANGLE DLL source and license/provenance; include matching PDBs, SBOM, checksums and build attestations; test the published artifact rather than only workflow outputs.
- **Acceptance:** reproducible release checklist completed, archive contents reviewed, checksums verified after download, attestations validate, and the release workflow is proven on an actual tag.

### R-06 Brand completion
- **Status:** In progress.
- **Work:** finish wordmark and lockups; embed the executable resource icon; check title bar, taskbar, installer/archive and high-DPI rendering on Windows.
- **Acceptance:** the same approved mark is legible in the executable, title bar, taskbar, docs and release artifacts; CI validates generated assets and no icon is missing from the packaged binary.

## Epic U — Performer UI and app workflows

### U-01 Command palette and command discovery
- **Status:** Built (D200). Left: MIDI/OSC and the remote converging is by construction (one command table), not yet exercised from a controller; the palette lists commands and panes, not scenes or presets.
- **Work:** implement Ctrl+K over the existing command table; search by command name/category; show shortcut and enabled/disabled reason; preserve focus rules and avoid firing transport shortcuts while editing text.
- **Acceptance:** keyboard, buttons, MIDI/OSC and remote transport converge on the same command semantics; palette supports navigation, execution, escape and empty results; tests cover text focus, widget focus and modal focus.

### U-02 Editor workspace
- **Status:** Built (D204). Left: column numbers (the editor shows lines only), validating `scene.json` before a reload, and a check on the T480.
- **Work:** add source tabs, dirty state, compile/validation feedback, line/column diagnostics, jump-to-error from the failure panel, safe save/backup/restore, and a clear distinction between editing a source file and changing the active runtime scene.
- **Acceptance:** a fixture can be edited, validated, saved, reloaded and restored; compile failure keeps the last good scene live; no shader compiles are triggered from UI paint or on an unintended live frame.

### U-03 Generated-controls review in Import and Inspector
- **Status:** In progress (D202, D219): the Inspector and the palette start a worker, and the result is reviewed (controls, ranges, scores, fidelity, audio ideas), each control kept or dropped and a slider re-ranged, then applied and opened, or discarded. Left: the Import pane button.
- **Work:** invoke generation using the app's renderer/context; display grouped proposed controls, effect scores, ranges and suggested audio bindings; allow accept/reject/edit; preserve original source and licence; do not silently rewrite the source of the user's only copy.
- **Acceptance:** imported and existing scenes can be analysed; accepted controls visibly change the scene; defaults reproduce the original image within the engine's documented fidelity threshold; rejected suggestions do not mutate the source.

### U-04 Performance Macro editor and pinning
- **Status:** Mostly built (D210): range (reach) and colour edited in MAP, the colour on the card, pin and unpin from a control's name in the Inspector; reorder and the unresolved states were already there (D175). Left: a shortcut and smoothing per macro.
- **Work:** edit range and colour, per-macro shortcut and smoothing, pin/unpin from Controls, reorder macros, inspect target resolution, and retain unavailable/broken/conflict states across launches.
- **Acceptance:** macro configuration survives restart; targets resolve only through documented stable identity rules; no silent fuzzy matches; all states have UI and tests.

### U-05 Hotkey editor
- **Status:** Built (D208): capture in Settings > Keyboard, clashes named and moved only on a second click, clear to none, reset to default, applied live and saved. Left: a check on the T480 with real keys (the agent's posted keys carry no Ctrl).
- **Work:** capture a shortcut, detect conflicts, show defaults, reset a binding and save it. Use the same focus and command table as the palette.
- **Acceptance:** duplicate/conflicting shortcuts are surfaced and resolved explicitly; text entry cannot accidentally trigger a show command.

### U-06 Named layouts, density, favourites and scene thumbnails
- **Status:** Done, pending a person's use on the T480: favourites (D211), thumbnails from a scene's own `IMAGE_PATH` (D212), named layouts in Settings and the palette (D215), density, Comfortable or Compact (D220), and a picture for any scene taken from its live frame (D222).
- **Work:** save/rename/delete named pane layouts; density options; scene favourites and tags; generate/cache thumbnails off the frame thread; show loading/failed states and invalidation when a scene changes.
- **Acceptance:** layout and favourites persist; thumbnails never compile/render during paint; cache is bounded; browser remains usable at 1280x720 without horizontal scrolling.

### U-07 Playlist workspace authoring
- **Status:** In progress: the selected cue's card edits length, transition and preset with their origins shown, moves, duplicates, deletes and adds the browser's scene after it, and a file changed on disk never overwrites unsaved edits (D213). Dragging a scene from the browser onto a cue inserts it after that cue, and a cue can be disabled from its card and enabled again from the Disabled cues list (D224). Left: multi-select, pasted Shadertoy URLs, comment-preserving save.
- **Work:** selected cue inheritance inspector; inherited versus overridden fields; multi-select editing; drag scenes from library; paste Shadertoy URLs; reorder and duplicate cues; per-cue duration/transition/preset/media; unsaved-change indication and safe reload when file watcher sees an external edit.
- **Acceptance:** author an 80-cue four-hour show in under two minutes using tags and multi-edit; inheritance is visible and deterministic; invalid edits preserve the last valid show.

### U-08 Real scene tags and generation
- **Status:** Built (D205): Synesthesia's `TAGS` is read leniently and normalised, written in place from the Inspector, searched by the browser, and used by Generate, whose preview names each scene's reason. Left: tagging from the Import pane and for a scene that is not playing, exclude fields in the Generate form.
- **Work:** define a backwards-compatible scene tag format; read/write tags; filter and generate playlists by tags rather than only filename words; define missing/unknown tag behaviour.
- **Acceptance:** tag fixtures round-trip; old scenes remain valid; generator selection is deterministic with a seed and reports why a scene was selected/skipped.

### U-09 Responsive desktop behaviour and UI performance
- **Status:** Blocked on real Windows/window-manager validation.
- **Work:** test custom title-bar drag, Aero Snap, resize, DPI changes, maximised undecorated window/taskbar interactions, multi-monitor changes, output monitor removal, and GNOME Wayland. Measure UI frame cost and separate Preview GPU cost on T480.
- **Acceptance:** documented results on T480 and TORNADO; no focus theft or lost fullscreen state; output assignment changes do not destroy the active scene; UI work does not cause visible render-time regression.

### U-10 UI accessibility and polish
- **Status:** In progress.
- **Work:** finish typography/spacing, reduced-motion support, keyboard-only paths, focus visibility, semantic names, high-contrast checks and narrow-window behaviour; test light and dark themes.
- **Acceptance:** all core performance actions can be used without hover; blackout and active state are unmistakable; keyboard focus remains visible; existing WCAG theme tests remain green.

## Epic C — SSF, scripting and shader compatibility

### C-01 Finish meta controls
- **Status:** Mostly done (D207): Limit Colors built; meta values carry across scenes under a cue's or preset's own values. Left: Alpha Channel, with an output that carries alpha (NDI/Spout); the order and conventions against Synesthesia (Q21); the stage's cost on an HD 4000.
- **Work:** implement Limit Colors and Alpha Channel; decide and implement whether meta values persist across scene changes; document reset/persistence semantics; validate order and mirror conventions against Synesthesia references where available.
- **Acceptance:** fixtures test each value and neutral no-op behaviour on GL 3.3 and GLES/ANGLE; neutral settings incur no post pass where technically possible; T480 and TORNADO GPU cost is recorded.

### C-02 GLES/SSF translation gaps
- **Status:** Built (D199); waiting on an ANGLE/D3D11 re-run. Left: function-like macros that mix int and float, a `sampler2D` in `?:`.
- **Work:** expand parser-backed rewrites for implicit int-to-float conversions and non-macro conditional compilation; do not add regex-only source rewriting that can change comments or identifiers.
- **Acceptance:** each discovered real-world failure has a minimized fixture, a stable diagnostic and native/GLES outcome; unsupported cases fail visibly rather than generating corrupt shader text.

### C-03 Synesthesia corpus compatibility
- **Status:** In progress.
- **Work:** rerun the 65 owner-supplied scenes after D193/D194; resolve or document remaining failures for user-media scenes, implicit int-to-float, and non-macro #if on ANGLE; distinguish source incompatibility from unavailable local assets.
- **Acceptance:** publish a matrix with native/ANGLE status and failure category for every scene; no compatibility claim based only on a small synthetic corpus.

### C-04 textureFinalPass validation
- **Status:** In progress.
- **Work:** run latency, resize, context-loss and busy-GPU tests using window-owned contexts on T480 native and ANGLE; compare GPU cost and freshness.
- **Acceptance:** latency remains bounded and asynchronous; no synchronous GPU stall; documented behaviour across context loss and resize.

### C-05 Benchmark scripting limitation
- **Status:** Open documentation/decision task.
- **Work:** keep benchmark reports explicit that scripts are not executed; decide whether script-aware benchmarking is needed and, if so, define deterministic script time/audio inputs before implementation.
- **Acceptance:** output cannot be mistaken for script-complete performance results; any future script mode has reproducibility tests.

### C-06 Meta/control/preset compatibility matrix
- **Status:** In progress.
- **Work:** verify presets and meta controls across scene changes, reset, script writes, audio gains, fav slots and playlist cue changes; clarify whether scene/meta/media channels are independently recallable and persistent.
- **Acceptance:** each documented Synesthesia behaviour is classified supported, intentionally degraded or unsupported with a fixture.

## Epic I — Import and auto-controls

### I-01 Live Shadertoy browser import
- **Status:** Blocked on external-site/browser validation.
- **Work:** test the Send to Strobe userscript against the live site in a real browser; test route changes, JSON extraction, asset URLs, missing channels and attribution; test REST-by-ID separately where the API permits it.
- **Acceptance:** record one successful import through the browser userscript and one API-by-ID attempt on Windows; if the external API is blocked, record the exact response and preserve the browser route as supported.

### I-02 File drop and import UX
- **Status:** Partly done (D218). Built: a drop on either window goes where it belongs (scene folder to the library and played, `.toml` to the playlist loader, a picture or folder of pictures to `syn_Media`, `.json` to the Shadertoy import), and a banner says what the hovering file would do. Left: a real OS drag on Windows and X11 by a person, the Needs-work bin, batch import, a watched drop folder, and attribution metadata.
- **Work:** test actual OS drag/drop on Windows and X11; add thumbnails, a Needs-work bin, richer conversion diagnostics, batch import and an optional watched drop folder.
- **Acceptance:** file-drop and typed-path paths share the same import pipeline; broken imports remain visible with actionable reasons; original source, attribution and license metadata survive.

### I-03 Auto-control engine integration and heuristic tuning
- **Status:** In progress; connected to the app (D202). Left: heuristic tuning on real scenes and the `st_speed` check.
- **Work:** connect the engine to Import/Inspector; confirm the speed control does not incorrectly follow st_speed; tune candidate naming/grouping/ranges on real community scenes; preserve deterministic output; allow user review before audio bindings are enabled.
- **Acceptance:** all library scenes with controls stripped and every tunable fixture produce reviewable controls; structural fixtures can legitimately report no useful controls; accepted defaults preserve the original render; no generated loop control can hang the GPU.

### I-04 Import pipeline reliability
- **Status:** In progress.
- **Work:** verify texture cache eviction and corrupt downloads, duplicate imports, missing assets, cancellation, concurrent import attempts, listener token handling, and import destination collisions.
- **Acceptance:** import errors do not replace the currently playing scene unless explicitly confirmed; credentials are not logged; repeated imports have deterministic replace/cancel behaviour.

## Epic P — Playlist, audio timing and show control

### P-01 Audio-event advance
- **Status:** Built (D226), pending real music: `strobe_audio::events` hears breakdowns and drops in the analyser's raw energies and the show hands them to the clock; a cue that can end only on audio says so when the audio is synthetic. Left: tuning the thresholds on real sets (a person, or recorded tracks we may use).
- **Work:** connect Phase 7 audio hit/event detection to playlist clock advancement with hysteresis and minimum intervals; allow per-show selection of event types and safe fallback when audio is absent.
- **Acceptance:** synthetic fixtures deterministically exercise events; no double advance on one transient; no accidental advance during device reconnect; show continues with a clear status when audio is unavailable.

### P-02 Downbeat and bar detector
- **Status:** Open.
- **Work:** research and implement a confidence-aware downbeat/bar estimate; define behaviour when BPM confidence is low, tempo changes, or beat phase drifts. Do not pretend raw tap tempo is a downbeat detector.
- **Acceptance:** recorded/synthetic fixtures with known tempo and phase quantify error; bar-locked transitions remain bounded and fall back to time-based transitions when confidence is insufficient.

### P-03 Four-hour real show and HD 4000 constraints
- **Status:** Blocked on suitable hardware/time.
- **Work:** measure 80-scene precompile time, parked-scene VRAM/RAM, transition cost with two live scenes, long image sequences, and a four-hour show on the T480; use an HD 4000 system if available.
- **Acceptance:** no runtime shader compiles after preparation; bounded memory; frame-time and transition data published; skips and failed precompiles remain visible.

### P-04 MIDI hardware validation
- **Status:** Blocked on a physical controller.
- **Work:** test device enumeration/port naming, hot-plug timing, a device held open by another Windows application, message rate/flood handling, Learn, soft takeover and conflict resolution on T480.
- **Acceptance:** record exact controller, connection method, OS and results; unplug/replug does not require restart; UI reports unavailable devices without losing saved mappings.

### P-05 MIDI/OSC feedback outputs
- **Status:** OSC built (D209): changes only, 30 Hz cap, a sender thread with a bounded queue, off by default, the loop into the listener refused; tested over loopback and in the app. Left: MIDI output (needs a device, P-04), and a check against a real TouchOSC layout.
- **Work:** add optional MIDI output and OSC output for state feedback to motorised faders/TouchOSC; define subscription and rate limiting; avoid echo loops when output returns to input.
- **Acceptance:** loopback and virtual-device tests verify values, rate limits and no feedback loops; outputs disabled by default and have explicit target/interface settings.

### P-06 Explicit blackout commands
- **Status:** Built (D201): `blackout_on` and `blackout_off` beside the toggle, in the command table, the palette, OSC, MIDI mappings and the remote protocol. Left: the remote server itself (L-02 on).
- **Work:** add set-blackout-on and set-blackout-off command targets alongside toggle, expose them to OSC/MIDI mappings and the command table, and preserve safe semantics for remote clients.
- **Acceptance:** repeated on is idempotent; repeated off is idempotent; mappings do not accidentally unblack an already blacked output; UI/remote state converges after reconnect.

### P-07 Macro and preset completion
- **Status:** In progress: range and colour editing and pinning (D210), preset rename and confirmation before replace or delete (D214) are built. Left: a shortcut and smoothing per macro, the media channel.
- **Work:** complete range/colour editing, macro shortcut/smoothing/pinning; add preset rename; add media preset channel after media-per-cue exists; validate save/recall/delete and missing-control resolution.
- **Acceptance:** no data loss on unresolved targets; presets remain backwards-compatible; user sees what will be changed before destructive replacement.

### P-08 Synesthesia OSC compatibility decision
- **Status:** Open decision.
- **Work:** compare the built-in /strobe namespace to Synesthesia's documented OSC address layout and decide whether compatibility is a requirement; if not, document the intentional divergence.
- **Acceptance:** decision recorded before adding aliases; tests cover every supported address and invalid payloads.

## Epic M — Media and transitions

### M-01 SSF media/cubemap specification audit
- **Status:** Open.
- **Work:** compare MEDIA, CUBEMAPS and syn_MediaType handling against authoritative SSF docs and representative scenes; verify cubemap orientation and image slot semantics.
- **Acceptance:** fixtures cover each documented shape; supported, degraded and unsupported forms are written down; malformed assets fail visibly without crashing the show.

### M-02 Media picker and playlist media
- **Status:** Done (D217, D223): the Inspector's Media row (what shows, where it comes from, Choose / Scene's own), a cue's `media` key edited from the LIST cue card, a preset's media channel, the precedence cue > operator > preset > scene, and choosing again for what already shows starts nothing. Verified on the T480 (Task 26).
- **Work:** add media selection to the UI; support scene media and cue overrides; include media in preset channel semantics; handle missing paths and replacement safely.
- **Acceptance:** media selection is asynchronous; the active scene remains live during decode; cue changes load the expected media without unbounded GPU memory; preset scene/meta/media channels behave independently.

### M-03 Image-sequence resource management
- **Status:** Partly done (D225): a sequence is bounded at 192 MiB of decoded frames and a long one is shrunk as a whole to fit (down to 256 px on its long side) instead of being cut; decoding was already off the frame thread with at most one upload a frame (D186). Left: measuring upload and resident memory on the T480.
- **Work:** measure sequence upload and resident memory on T480/shared memory; implement bounded frame cache or streaming if full sequence residency is unsafe; test resize, loop and decode errors.
- **Acceptance:** long sequence memory is bounded/documented; decoding stays off the frame thread; missed frames degrade gracefully.

### M-04 Video and webcam input
- **Status:** Deferred research; not a v1 blocker.
- **Work:** compare lightweight maintained decoder/capture options, license implications, hardware decode availability, Windows device lifecycle and upload path. Avoid introducing a heavy dependency without measurements.
- **Acceptance:** proposal includes binary size, supported formats, device removal/reconnect, latency, GPU upload cost and license review; implementation starts only after a decision.

### M-05 Song-change transitions
- **Status:** Open.
- **Work:** define song-change detection source and confidence, transition interaction with playlist anchors and audio-event advance, and manual override behaviour.
- **Acceptance:** deterministic tests for no audio, low confidence, track boundary and manual override; no double transition with a cue transition already in progress.

### M-06 NDI output
- **Status:** Deferred research; not a v1 blocker.
- **Work:** evaluate SDK/runtime licensing, Windows packaging, pixel format, frame pacing, GPU readback versus shared textures, latency and failure handling.
- **Acceptance:** measured prototype with frame-time impact and a clear optional-dependency story before committing to a full implementation.

## Epic L — LAN remote and PWA

### L-01 Finish the transport-independent protocol
- **Status:** In progress in this PR.
- **Work:** keep message types versioned, strict, documented and allowlisted; align command semantics with app command IDs; add compatibility tests for older/newer protocol versions.
- **Acceptance:** unknown fields/commands fail closed; request IDs and message sizes are bounded; protocol contract is documented and tests pass in CI.

### L-02 Listener lifecycle and HTTP/WebSocket transport
- **Status:** Open.
- **Work:** select and document the minimal maintained server stack; run it on a dedicated thread/runtime; opt-in bind to a chosen LAN interface; serve embedded same-origin assets; start/stop idempotently; clean shutdown; bounded inbound and outbound queues.
- **Acceptance:** no sockets open by default; the render loop never blocks on network; bind failure is actionable; disabled listener rejects old clients immediately.

### L-03 Pairing, tokens and revocation
- **Status:** Open; message shapes only.
- **Work:** cryptographic pairing code, short expiry, attempt limits, explicit desktop approval, high-entropy per-client token, secure persistence, individual/all-client revoke, token rotation and secret redaction.
- **Acceptance:** tests for expired/replayed pairing, brute-force rate limit, invalid/revoked token, restart semantics and logs; no long-lived token in QR URL or query string.

### L-04 Network security and abuse controls
- **Status:** Open.
- **Work:** Host/Origin validation, same-origin WebSocket checks, message-size/client limits, rate limiting, idle timeout, public-network refusal/warning, firewall guidance, CSWSH/DNS-rebinding protection and clear residual risk for plaintext LAN transport.
- **Acceptance:** adversarial tests reject unexpected origins/hosts, oversized JSON, floods, malformed payloads and slow clients without panic, queue growth or frame stalls.

### L-05 App-thread command/state bridge
- **Status:** Open.
- **Work:** bounded channel drained by app event-loop; revalidate each command on the owning thread; use existing command path; return accepted/rejected result; publish revisioned snapshots and coalesce stale state updates.
- **Acceptance:** every command runs on app thread; playlist and no-playlist semantics are correct; full queue returns Busy; reconnect gets a complete fresh snapshot; tests show no frame-time blocking.

### L-06 Touch-first controller UI
- **Status:** Open.
- **Work:** responsive local HTML/CSS/JS for current/next scene, cue/time, hold, output/blackout, audio status, macros, connection state and reconnect; QR/pairing flow; no external CDN or telemetry.
- **Acceptance:** portrait/landscape, safe areas, keyboard accessibility, high contrast, reduced motion and stale/disconnected states tested; no live preview streaming in v1.

### L-07 PWA HTTPS and real-device validation
- **Status:** Blocked on TLS/trust decision.
- **Work:** compare user-managed local HTTPS/reverse proxy and future certificate-enrolment options; do not tell users to bypass warnings; add manifest/service worker only once installability is genuinely supportable.
- **Acceptance:** Android Chrome and iOS Safari tested on real devices over Wi-Fi; sleep/resume, rotation, roam, browser restart, updates and revocation tested; only then advertise installable PWA support.

### L-08 Remote soak and operational docs
- **Status:** Open.
- **Work:** test multiple clients, stop/start, Wi-Fi loss, Windows firewall, idle CPU/memory, command floods and pairing lifecycle on T480; document trusted-network-only limits if TLS is absent.
- **Acceptance:** no render regression or unbounded resource growth; listener always visibly enabled and has a one-action desktop kill switch.

## Epic G — Graphics backend research

### G-01 Machine-readable shader compatibility report
- **Status:** Open; required before backend abstraction.
- **Work:** compile/render the current scene library, Shadertoy fixtures, Synesthesia corpus and owner-supplied shaders; record backend/compiler, compile result, image checksum, frame time and categorized failure; run on llvmpipe CI, T480 and TORNADO.
- **Acceptance:** deterministic report format, a deliberately broken shader is detected in CI, and unsupported shaders are categorized rather than omitted silently.

### G-02 Renderer seam design
- **Status:** Open; do not over-abstract before G-01.
- **Work:** define only the scene/pass-boundary contract required for device/context creation, scene preparation, frame render, resize, diagnostics, timing and device-loss recovery; keep GL implementation and GL 3.3 floor intact.
- **Acceptance:** current GL path remains behaviourally unchanged and benchmark baselines are comparable; no per-frame allocation or synchronous readback introduced.

### G-03 wgpu/Naga versus direct GLSL-to-SPIR-V spike
- **Status:** Open.
- **Work:** run both candidates over the exact same corpus; report GLSL construct failures, image fidelity, load/compile time, frame time, binary size, dependency and maintenance cost; include Vulkan and platform path considerations for DX12/Metal/MoltenVK.
- **Acceptance:** written go/no-go based on corpus results; no production backend enabled by default until the full compatibility and hardware matrix passes.

### G-04 Backend vertical slice and device-loss handling
- **Status:** Deferred until G-01–G-03 justify implementation.
- **Work:** behind a feature flag, implement one representative multipass feedback scene, audio texture, resize, async timing/readback and loss/recreate path; keep GL as fallback.
- **Acceptance:** matching visual tests and load-time shader preparation on T480 and TORNADO; no regressions to GL/ANGLE; release claims distinguish Vulkan, DX12, Metal and MoltenVK support.

## Epic D — Documentation, evidence and maintainability

### D-01 Reconcile status documents
- **Status:** Open.
- **Work:** keep ROADMAP.md the short priority view, STATUS_AUDIT.md the evidence log, and this file the cross-epic task register; remove contradictory stale claims as tasks change.
- **Acceptance:** no task is simultaneously described as both unimplemented and verified; every checked item names its evidence location.

### D-02 User-facing install and troubleshooting guide
- **Status:** Open.
- **Work:** document clean Windows installation, ANGLE DLL provenance, audio permissions/devices, shader imports and license responsibility, diagnostics location, recovery and common GPU-driver issues.
- **Acceptance:** a person without a Rust toolchain can install, launch, import a scene, choose audio, fullscreen output and collect diagnostics using only the guide.

### D-03 Feature/compatibility matrix
- **Status:** In progress.
- **Work:** publish supported/degraded/unsupported matrix for SSF, Shadertoy, scripts, media, GL/ANGLE, Windows release, MIDI/OSC and remote; include evidence and platform limitations.
- **Acceptance:** no feature is described as supported solely because a type or UI control exists; evidence links to fixtures or hardware logs.

### D-04 CI coverage and non-flaky hardware gates
- **Status:** In progress.
- **Work:** ensure deterministic unit/integration tests cover protocol, command semantics, parser rewrites, importer security, generated controls, asset errors and no-runtime-compiles; keep hardware tests as explicit manual gates with artifacts.
- **Acceptance:** CI is reproducible; platform-only tests clearly report skip reasons; skipped Windows/GPU tests are not counted as passed.

## Cross-cutting engineering rules

- Preserve the existing GL 3.3 and ANGLE paths as the reference while evaluating new APIs.
- Keep shader compilation, filesystem scanning, thumbnail generation, asset decode, device enumeration and network I/O off the frame/paint path.
- Bound every queue, cache, log and externally controlled input. Treat a slow consumer as a normal condition.
- Never silently discard attribution or license metadata during import, conversion, auto-control generation, preset save or export.
- Keep current scene/output alive on failed load, import, edit, media replacement or backend switch wherever possible.
- Use typed commands and stable IDs. Avoid raw key strings, fuzzy control resolution and a second source of truth for playlist or control state.
- Every user-facing feature needs empty, loading, success, failure, unavailable and reconnect/recovery states where applicable.
- Separate implementation completion from external validation. If a hardware or third-party service cannot be accessed, record it as blocked with the exact reason.

## Suggested execution queue

1. R-01 audio reconnection, R-02 soak anomaly, R-03/R-04 packaged smoke and crash verification.
2. U-01 command palette, U-02 editor foundation, U-03 generated-controls review, U-07/U-08 playlist authoring/tags.
3. C-01/C-02/C-03 compatibility and media specification fixtures.
4. I-01/I-02 live import and OS file drop; I-03 auto-controls integration.
5. P-04 physical MIDI, P-05 feedback outputs, P-06 explicit blackout and P-07 macro/preset completion.
6. L-02 through L-06 LAN remote MVP; L-07 remains gated on a defensible HTTPS solution.
7. M-01 through M-03 media selection and resource management; P-01/P-02 audio-event and downbeat work.
8. G-01 compatibility harness; only then G-02/G-03 and a justified G-04 spike.
9. R-05/R-06 final release and brand checks; deferred video/webcam and NDI only after their research gates.

## Definition of project completion

Strobe is not finished merely because it compiles or because the current demo works. Completion means:

- core supported workflows have explicit acceptance criteria and passing tests;
- Windows packaged builds have been exercised on real hardware;
- the supported SSF/Shadertoy compatibility surface is measured and documented;
- live performance paths have no shader compilation or blocking I/O after preparation;
- playlist and show-control paths have bounded resource use and real-device evidence;
- import preserves attribution and handles missing/corrupt assets safely;
- the LAN remote, if shipped, is opt-in, authenticated, bounded, revocable and tested on real mobile devices;
- release artifacts, symbols, provenance, checksums and attestations are verified;
- deferred features are explicitly labeled deferred rather than implied complete.
