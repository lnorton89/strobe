# Strobe implementation backlog

**Purpose:** one evidence-based register of every known unfinished or not-yet-verified project area. This is the implementation companion to ROADMAP.md, STATUS_AUDIT.md, PLAN.md, UI_IMPLEMENTATION_PLAN.md, GFX_API_RESEARCH.md, and REMOTE_CONTROL_PWA_RESEARCH.md. It consolidates their open work; it does not re-label untested work as complete.

**Last reconciled:** 2026-10-26 for G-05 and G-06 (D485), and C-01, C-07 and I-05 against Synesthesia 1.25.4 (TORNADO T25, D490); the UI items reflect D464-D484 in ROADMAP.md and UI_REVIEW.md.

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
- **Status:** Open; live capture works, device lifecycle does not have complete real-hardware evidence. The AMD desktop (Task A1) found a reopen loop: any stream error, a WASAPI buffer glitch included, reopened the device at once, 70 times a second. Fixed by D266: glitches are counted and the stream kept, and repeated quick failures back off from 100 ms to 2 s. Re-checked there (Task A2): 188 s with no reopen and no glitch.
- **Work:** unplug/replug the selected input during playback; exercise default-device changes, device disappearance during capture, retry timing, switching to synthetic fallback and returning to live input. Keep the UI status truthful and avoid blocking the render thread.
- **Acceptance:** record logs and observed UI transitions in hardware/T480.md; output keeps rendering throughout; no panic, deadlock, stale device handle or repeated unbounded retry; a device returning can be selected/reopened without restarting Strobe.
- **Dependencies:** physical Windows audio device and access to the T480.

### R-02 Investigate the minute-six performance/resource step
- **Status:** Done, retired (T480 Tasks 20, 34 and 36). Task 17's step was not reproduced in four runs:
  - two 10-minute runs (Task 20);
  - 20 minutes fullscreen plus 20 windowed of a single scene (Task 34);
  - 20 minutes fullscreen on the same transition-demo playlist (Task 36). There, GPU time was 3.21-3.27 ms from minute 2 on (3.22 ms at minutes 5, 6 and 7), private bytes were within 0.4 MB after a 45 MB start-up release, handles held at 417, and there were 0 late frames.

  The before/after values of Task 17 match fullscreen against windowed output exactly, so the output had left fullscreen. The output window now logs resizes, fullscreen state and occlusion, so if it ever happens again the log says which.
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
- **Status:** Done (v0.1.0, 2026-10-14). `v0.1.0` was tagged on the release
  PR's merge commit (`578ea34`) and published as the latest release by
  `release.yml` (run 38495718734). The workflow had first been run end to end
  on a throwaway public mirror (run 38494313118), whose release and tag were
  then deleted; Q12 is resolved by it. All six assets were downloaded from
  the release page and every `.sha256` verified; `gh attestation verify`
  returned 0 for build provenance and for the SBOM predicate; the zip
  contents, the matching `strobe.pdb` and the 279-component CycloneDX SBOM
  were inspected. Two housekeeping items remain: the throwaway mirror repo
  `lnorton89/strobe-release-dryrun` could not be deleted from here (the
  token lacks `delete_repo`), and steps 4-5 of
  [RELEASE_PLAN.md](RELEASE_PLAN.md) (clean-machine smoke test, symbolication
  from the downloaded archive) still need the T480.
- **Work:** create a real release tag; inspect ZIP contents and layout; verify ANGLE DLL source and license/provenance; include matching PDBs, SBOM, checksums and build attestations; test the published artifact rather than only workflow outputs.
- **Acceptance:** reproducible release checklist completed, archive contents reviewed, checksums verified after download, attestations validate, and the release workflow is proven on an actual tag.

### R-06 Brand completion
- **Status:** In progress. The executable icon is embedded (D241): `strobe-app`'s build script compiles `assets/strobe.rc` with the brand `icon.ico` on Windows targets. On the T480 (Task 32) both exes carry the full six-size icon group, and Windows extracts the Strobe mark for the exe and for a shortcut to it. Each exe also carries a version block (D244): product Strobe, its own file name and description, and the workspace version as both file and product version. The T480 reads it back from both exes (Task 33). The wordmark, the horizontal and stacked lockups, a one-colour mark, the README banner and the social preview are made (D256). The type is Space Grotesk outlines set by `assets/brand/tools/typeset.py`, and `cargo xtask brand` renders and checks every raster. Left: a look at Explorer and the taskbar by a person.
- **Work:** finish wordmark and lockups; embed the executable resource icon; check title bar, taskbar, installer/archive and high-DPI rendering on Windows.
- **Acceptance:** the same approved mark is legible in the executable, title bar, taskbar, docs and release artifacts; CI validates generated assets and no icon is missing from the packaged binary.

## Epic U — Performer UI and app workflows

### U-01 Command palette and command discovery
- **Status:** Built (D200). Scenes (Load, Preview) and the playing scene's presets (Apply) are entries too, ranked after exact command matches and built only when the library or presets change (D262). Left: MIDI/OSC and the remote converging is by construction (one command table), not yet exercised from a controller.
- **Work:** implement Ctrl+K over the existing command table; search by command name/category; show shortcut and enabled/disabled reason; preserve focus rules and avoid firing transport shortcuts while editing text.
- **Acceptance:** keyboard, buttons, MIDI/OSC and remote transport converge on the same command semantics; palette supports navigation, execution, escape and empty results; tests cover text focus, widget focus and modal focus.

### U-02 Editor workspace
- **Status:** Built (D204). Errors go to their column where the driver gives one (Mesa), put back on the user's line, and Save and reload checks `scene.json` with the loader's parser first, refusing with the error and its line and column in the Editor (D263). Left: a check on the T480, whose drivers give lines only.
- **Work:** add source tabs, dirty state, compile/validation feedback, line/column diagnostics, jump-to-error from the failure panel, safe save/backup/restore, and a clear distinction between editing a source file and changing the active runtime scene.
- **Acceptance:** a fixture can be edited, validated, saved, reloaded and restored; compile failure keeps the last good scene live; no shader compiles are triggered from UI paint or on an unintended live frame.

### U-03 Generated-controls review in Import and Inspector
- **Status:** In progress (D202, D219): the Inspector and the palette start a worker, and the result is reviewed (controls, ranges, scores, fidelity, audio ideas), each control kept or dropped and a slider re-ranged, then applied and opened, or discarded. Built since (D246): a control can be renamed in the review, and the name is checked as the engine checks its own; the Import pane offers "Generate controls for it" once an imported scene is playing, and opens the Inspector, where the review appears. Since D488 a scene with no controls offers *Generate controls* on the Controls view and tab, where the review appears too, and the review can **save the kept controls into the scene itself** (with backups of what they replace, refused for a scene changed since it was measured, open with unsaved edits, or Synesthesia's), besides keeping a copy or discarding. Left: a person using it on a real import.
- **Work:** invoke generation using the app's renderer/context; display grouped proposed controls, effect scores, ranges and suggested audio bindings; allow accept/reject/edit; preserve original source and licence; do not silently rewrite the source of the user's only copy.
- **Acceptance:** imported and existing scenes can be analysed; accepted controls visibly change the scene; defaults reproduce the original image within the engine's documented fidelity threshold; rejected suggestions do not mutate the source.

### U-04 Performance Macro editor and pinning
- **Status:** Mostly built (D210): range (reach) and colour edited in MAP, the colour on the card, pin and unpin from a control's name in the Inspector; reorder and the unresolved states were already there (D175). A glide (D227) and a key (D228) per macro are built too.
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
- **Status:** In progress: the selected cue's card edits length, transition and preset with their origins shown, moves, duplicates, deletes and adds the browser's scene after it, and a file changed on disk never overwrites unsaved edits (D213). Dragging a scene from the browser onto a cue inserts it after that cue, and a cue can be disabled from its card and enabled again from the Disabled cues list (D224). A Shadertoy URL or id typed on the card is imported and added after the cue without loading it by hand (D229). Ctrl-click selects several cues and the card's setting edits go to all of them (D230), and a save keeps the file's comments (D231). Left: authoring an 80-cue show by a person against the two-minute acceptance.
- **Work:** selected cue inheritance inspector; inherited versus overridden fields; multi-select editing; drag scenes from library; paste Shadertoy URLs; reorder and duplicate cues; per-cue duration/transition/preset/media; unsaved-change indication and safe reload when file watcher sees an external edit.
- **Acceptance:** author an 80-cue four-hour show in under two minutes using tags and multi-edit; inheritance is visible and deterministic; invalid edits preserve the last valid show.

### U-08 Real scene tags and generation
- **Status:** Built (D205): Synesthesia's `TAGS` is read leniently and normalised, written in place from the Inspector, searched by the browser, and used by Generate, whose preview names each scene's reason. Since D264: any library scene is tagged by name, from the Inspector, the Import pane (the scene just imported) or the browser's selected scene, and Generate has an Exclude field whose preview says which scenes it skipped and why. Left: a person using it on a real library.
- **Work:** define a backwards-compatible scene tag format; read/write tags; filter and generate playlists by tags rather than only filename words; define missing/unknown tag behaviour.
- **Acceptance:** tag fixtures round-trip; old scenes remain valid; generator selection is deterministic with a seed and reports why a scene was selected/skipped.

### U-09 Responsive desktop behaviour and UI performance
- **Status:** In progress. Narrow windows fixed (D260), and re-checked on the T480 (Task 38): BLACKOUT is visible at every size, no text overlaps, and the one leftover (the rail's SET entry half under the transport at 1024x640) is fixed. The T480's resize run (Task 35) had found the Controls pane over BLACKOUT, the toolbar and title bar overlapping, and the portrait transport drawn over itself. All of it reproduces and is fixed headless at the same logical sizes. Two tests now hold at every size in every state: `nothing_is_painted_over_blackout_at_any_size` and `no_text_is_drawn_over_other_text_at_any_size`. `STROBE_UI_SHOTS` paints every state as PNGs without a window. Left: on real Windows, snap, DPI changes, the maximised undecorated window and the taskbar, a second monitor and its removal; GNOME Wayland; UI frame cost on the T480.
- **Work:** test custom title-bar drag, Aero Snap, resize, DPI changes, maximised undecorated window/taskbar interactions, multi-monitor changes, output monitor removal, and GNOME Wayland. Measure UI frame cost and separate Preview GPU cost on T480.
- **Acceptance:** documented results on T480 and TORNADO; no focus theft or lost fullscreen state; output assignment changes do not destroy the active scene; UI work does not cause visible render-time regression.

### U-10 UI accessibility and polish
- **Status:** In progress. Built so far:
  - reduced motion (the setting, or the OS);
  - density (D220);
  - a keyboard path to every performance command, through keys and the palette;
  - a visible keyboard-focus ring (D243): 2 px of the accent, tested at 3:1 or more against every surface in every theme.

  - narrow-window behaviour (D260, U-09).

  Left:
  - a keyboard-only pass by a person on Windows;
  - a high-contrast check.
- **Work:** finish typography/spacing, reduced-motion support, keyboard-only paths, focus visibility, semantic names, high-contrast checks and narrow-window behaviour; test light and dark themes.
- **Acceptance:** all core performance actions can be used without hover; blackout and active state are unmistakable; keyboard focus remains visible; existing WCAG theme tests remain green.

## Epic C — SSF, scripting and shader compatibility

### C-01 Finish meta controls
- **Status:** Reopened (TORNADO T25, 2026-10-26). Built before (D180, D207): the post stage, Limit Colors, meta values carrying across scenes; Alpha Channel deferred to an output that carries alpha (D248, with M-06). Synesthesia 1.25.4's own meta stage has now been read from its engine (R16 in OPEN_QUESTIONS), and Strobe's differs on almost every point:
  - **Order:** Synesthesia samples through the mirrors, then Limit Colors, invert, hue and saturation, contrast, gamma, brightness, clamp, the media overlay, master. Strobe: mirrors, brightness, contrast, gamma, hue/saturation, Limit Colors, invert, master.
  - **Methods:** brightness multiplies (Strobe adds); contrast `k` is squared above 1; gamma above 1 becomes `1 + 2(g - 1)`; hue (turns, added) and saturation (S times `1 + 2.5 s abs(s)`, neutral 0) work in HSV on the clamped colour (Strobe: a grey-axis rotation and a luma mix); Limit Colors runs black, Low, High, white over the square root of luminance (Strobe: Low to High over luma).
  - **Names and mirrors:** Synesthesia's `Vert_Mirror` shows the left half mirrored on the right and `Hor_Mirror` the top half on the bottom; Strobe calls the first horizontal and its vertical reflects bottom onto top. Its colours are `Low_Color` / `High_Color` (Strobe: `limit_low` / `limit_high`), which matters to OSC (`/controls/meta/high_color` is the manual's own example) and to scripts' `meta/` names.
  - **Missing:** Transition (a button that fires the scene's transitions; with D489), Alpha Channel, and the media group (Media Contrast, Gamma, Hue, Saturation, Invert Media, Vert and Hor Flip, Fit or Fill, Media Scale, playback and overlay).
  - **Ranges:** stored normalised with 0.5 neutral; the raw ranges need Synesthesia running (S5).

  Not changed in this pass because it changes the meaning of every saved Strobe preset and cue that holds a meta value (a stored brightness of 0 would turn black under a multiplying brightness), so it needs a migration and a decision.
  Left: that decision and the change; the stage's cost on an HD 4000.
- **Work:** follow R16: reorder the post stage and change each method to Synesthesia's; rename the meta controls to Synesthesia's names (`vert_mirror`, `hor_mirror`, `low_color`, `high_color`) keeping Strobe's old names as aliases for scripts, OSC and saved presets; migrate stored values (an additive brightness `b` becomes a multiplier near `1 + b`); add Transition (with D489's transitions) and Fit or Fill (the `_ratioFix` choice, R18); then S5 and S9 in OPEN_QUESTIONS with Synesthesia running.
- **Acceptance:** fixtures test each value and neutral no-op behaviour on GL 3.3 and GLES/ANGLE; neutral settings incur no post pass where technically possible; T480 and TORNADO GPU cost is recorded.

### C-02 GLES/SSF translation gaps
- **Status:** Built (D199, D261); the ANGLE/D3D11 re-run passed (TORNADO T18: 63 of 65 real scenes in both dialects, the two failures the scenes' own bugs). The last two gaps are closed (D261), with original fixtures in `render.rs` that failed on ES before and pass in both dialects on llvmpipe:
  - function-like macros that mix int and float: on ES each call is typed through its expansion, and the conversion goes into the argument, or the one call is inlined on its own line (`function_like_macros_mixing_int_and_float_compile_in_both_dialects`);
  - a sampler in `?:`: `texture(c ? a : b, uv)` becomes `(c ? texture(a, uv) : texture(b, uv))` on every dialect (`a_sampler_chosen_with_a_ternary_compiles_in_both_dialects`). Any other sampler `?:` fails on ES with Strobe's message at the user's line (`a_sampler_choice_that_cannot_be_rewritten_fails_with_strobes_message_on_es`).
  - Not yet run: ANGLE on the T480 or TORNADO, and the owner's corpus.
- **Work:** expand parser-backed rewrites for implicit int-to-float conversions and non-macro conditional compilation; do not add regex-only source rewriting that can change comments or identifiers.
- **Acceptance:** each discovered real-world failure has a minimized fixture, a stable diagnostic and native/GLES outcome; unsupported cases fail visibly rather than generating corrupt shader text.

### C-03 Synesthesia corpus compatibility
- **Status:** Done (TORNADO T22, T24): COMPATIBILITY.md section 3 lists all 65 owner scenes with their native and ANGLE status, failure category and the constructs each uses.
  - 63 of 65 in both dialects.
  - The two failures are source bugs in the scenes themselves.
  - Of the rest, 3 are driver differences from undefined behaviour, 1 needs a user image, 1 names a picture that is missing locally, and none is a Strobe gap.
  - Int-to-float (23 scenes) and non-macro `#if` (2) pass on ANGLE since D199.
- **Work:** rerun the 65 owner-supplied scenes after D193/D194; resolve or document remaining failures for user-media scenes, implicit int-to-float, and non-macro #if on ANGLE; distinguish source incompatibility from unavailable local assets.
- **Acceptance:** publish a matrix with native/ANGLE status and failure category for every scene; no compatibility claim based only on a small synthetic corpus.

### C-04 textureFinalPass validation
- **Status:** Done. `strobe-bench --readback` (D245) runs the latency, cost, busy-GPU, resize and lost-context checks on any context, the window one included. It passes on llvmpipe in both dialects and on the T480's window context on both backends (Task 33):
  - native and ANGLE both have exactly 2 frames of latency, and cost +0.24 and +0.23 ms a frame;
  - with the GPU busy, ANGLE's data averages 2.3 frames old and `render()` takes 8.4 ms, which is queueing, still with no stall.
- **Work:** run latency, resize, context-loss and busy-GPU tests using window-owned contexts on T480 native and ANGLE; compare GPU cost and freshness.
- **Acceptance:** latency remains bounded and asynchronous; no synchronous GPU stall; documented behaviour across context loss and resize.

### C-05 Benchmark scripting limitation
- **Status:** Decided (D239): the bench stays script-free, and `results.md` now says so above the table and marks each scene with a `script.js` (`*`), so its numbers cannot be read as script-complete. Script cost is measured by the corpus harness, which runs scripts with a fixed `dt` and synthetic audio and reports `frame_ms`.
- **Work:** keep benchmark reports explicit that scripts are not executed; decide whether script-aware benchmarking is needed and, if so, define deterministic script time/audio inputs before implementation.
- **Acceptance:** output cannot be mistaken for script-complete performance results; any future script mode has reproducibility tests.

### C-06 Meta/control/preset compatibility matrix
- **Status:** Done (D247): COMPATIBILITY.md's "Synesthesia's preset, meta and control behaviours" classifies 16 documented behaviours with a test each. Built for it: default and random on a control, a group or a bank, over OSC and as a mapping target. Left as Unsupported, with reasons: lock, bank undo, the `/preset` suffix, `/presets/new`, and recalling one part of a preset.
- **Work:** verify presets and meta controls across scene changes, reset, script writes, audio gains, fav slots and playlist cue changes; clarify whether scene/meta/media channels are independently recallable and persistent.
- **Acceptance:** each documented Synesthesia behaviour is classified supported, intentionally degraded or unsupported with a fixture.

### C-07 The tilt-shift blur as Synesthesia's engine draws it
- **Status:** Open (TORNADO T25). Synesthesia's `_hBlurTiltShift` / `_vBlurTiltShift` (and `syn_pass_*BlurTiltShift`) loop `1 + 20 * blurAmt` taps each side, mirror out-of-range coordinates, weight with a Gaussian whose width grows with `abs(uv.y - blurLoc)` to the power 2.3, and the vertical one steps by `1 / RENDERSIZE.x` and measures from `uv.y`. Strobe's is its own (a capped, constant-bound Gaussian). None of the 87 shipped scenes calls it, so nothing visible depends on it yet.
- **Work:** match the engine within Strobe's constant-loop-bound rule (cap the taps, break early), with a render test of one value per form.
- **Acceptance:** a fixture's centre pixel equals the engine's formula in both dialects; the cap is documented.

## Epic I — Import and auto-controls

### I-01 Live Shadertoy browser import
- **Status:** Blocked on external-site/browser validation.
- **Work:** test the Send to Strobe userscript against the live site in a real browser; test route changes, JSON extraction, asset URLs, missing channels and attribution; test REST-by-ID separately where the API permits it.
- **Acceptance:** record one successful import through the browser userscript and one API-by-ID attempt on Windows; if the external API is blocked, record the exact response and preserve the browser route as supported.

### I-02 File drop and import UX
- **Status:** Partly done (D218). Built: a drop on either window goes where it belongs (scene folder to the library and played, `.toml` to the playlist loader, a picture or folder of pictures to `syn_Media`, `.json` to the Shadertoy import), and a banner says what the hovering file would do. Batch import (D235): a dropped folder of scene folders adds them all to the library, one a frame, none played, with a summary banner and each refusal in the log and DIAG. The Needs-work bin exists as the LIB "Needs work" filter and DIAG's list of this run's failures with reasons. Imports already record author, source and licence in the scene. A watched drop folder (D240): `--watch-folder <dir>` adds scenes that settle in it to the library through the same queue, playing none. Settings has the watched folder too (D265): a path, Apply and a switch, applied live, saved in `ui.toml`, and checked as at launch (a folder, and not the library itself). Left: a real OS drag on Windows and X11 by a person.
- **Work:** test actual OS drag/drop on Windows and X11; add thumbnails, a Needs-work bin, richer conversion diagnostics, batch import and an optional watched drop folder.
- **Acceptance:** file-drop and typed-path paths share the same import pipeline; broken imports remain visible with actionable reasons; original source, attribution and license metadata survive.

### I-03 Auto-control engine integration and heuristic tuning
- **Status:** In progress; connected to the app (D202). The `st_speed` check is done (D246): a generated speed control used to ignore `st_speed`, and now ticks at the import clock's rate, so it follows it as `iTime` does. Heuristic tuning on real community scenes: TORNADO T26 ran the engine over the owner's 65 scenes (266 controls kept in 49). It found six problems, all now addressed with original fixtures (D259):
  - scripted scenes refused for undeclared `setUniform` uniforms, now declared from a static scan of `script.js`; a scene whose own GLSL is broken is now an error, not "0 candidates";
  - fade-polynomial and hash constants offered as knobs, now traps;
  - 116 of 266 names were a stem and a number; names now come from the assigned variable or field, the function, the helper or the loop's body, with roles (`low`/`high`, `x`/`y`) before numbers, and `p_oNNNN_` prefixes are dropped;
  - position and direction `vec3`s taken for colours, now left alone;
  - 28 loop bounds and 6 `FAR`s kept, now one Quality control a scene, and tiny controls next to a strong one are cut; two scenes' rewrites changed the default picture (0.020, 0.0076), now a fidelity gate drops the offending controls;
  - misses: speeds in `#define` bodies, `TIME / k`, and colours in `#define`s and `const vec3`s, now found.

  Left: a re-run of T26 on TORNADO to confirm on the real scenes and a real GPU.
- **Work:** connect the engine to Import/Inspector; confirm the speed control does not incorrectly follow st_speed; tune candidate naming/grouping/ranges on real community scenes; preserve deterministic output; allow user review before audio bindings are enabled.
- **Acceptance:** all library scenes with controls stripped and every tunable fixture produce reviewable controls; structural fixtures can legitimately report no useful controls; accepted defaults preserve the original render; no generated loop control can hang the GPU.

### I-04 Import pipeline reliability
- **Status:** Audited (D242). Already covered, with tests:
  - imports run one at a time on a bounded queue, and a full queue refuses;
  - a re-import asks Replace or Cancel, and the listener answers 409;
  - a failed import loads nothing;
  - the listener token and the REST key print as `***`, and the key is redacted from every URL and error;
  - downloads land under a temporary name and are renamed into place, and HTML, empty and oversized bodies are refused;
  - one failed texture reads black without failing the import, and an unreachable host stops the rest.

  Decided not to build: cache eviction, since the cache only ever holds Shadertoy's finite texture set, and cancelling a running job, since each one is bounded by its timeouts.
- **Work:** verify texture cache eviction and corrupt downloads, duplicate imports, missing assets, cancellation, concurrent import attempts, listener token handling, and import destination collisions.
- **Acceptance:** import errors do not replace the currently playing scene unless explicitly confirmed; credentials are not logged; repeated imports have deterministic replace/cancel behaviour.

### I-05 Synesthesia's installed scenes in the library
- **Status:** In progress; built (D271). **TORNADO T25 (2026-10-26):** the real 1.25.4 install holds 87 scenes in `release\` and the welcome scene in `default\`, 88 in all, as expected; `cargo xtask corpus` read all 88 in place and every one loads, compiles and renders in both dialects (native NVIDIA and ANGLE/D3D11), so the engine side of "one plays" holds. The app side (the 88 marked Synesthesia in the library, playing one from the browser, the read-only refusal, Copy to my library, Settings hiding them, nothing under `Program Files` changing) still needs a person at the keyboard (S13 in OPEN_QUESTIONS). Earlier on TORNADO the library listed 150 scenes from the installed Synesthesia beside its own (the shipped scenes and the scene folders in its settings). Where Synesthesia is installed (`Program Files (x86)` or `Program Files`, or `STROBE_SYNESTHESIA_DIR`), the library lists, after the library folder's own, the scenes it ships (`release`, `default`; read-only) and the scene folders in its own settings (`vs.json` `sceneFolders` and `primarySceneFolder`, `preferences.json` `PRIMARY_SCENE_FOLDER`, searched recursively as its preferences say; writable). One detection (`strobe_scene::synesthesia`) serves the library and `#include`. On by default; Settings > "Show Synesthesia's scenes" (ui.toml) and `--no-synesthesia` turn it off. Writes into the install (tags, a picture, Editor saves, presets) are refused with a notice offering "Copy to my library", which copies the scene in under a free name and moves a playing scene or an open Editor onto the copy. Search, the palette, playlists, presets, fav slots and `--scene` find these scenes, the library folder's own first on a name clash.
- **Work:** run it on a real install: the 88 scenes appear (87 `release` + the welcome scene), one plays, a tag on one is refused and Copy to my library works, and a Custom Scene Folder from Synesthesia's settings is listed and taggable. Decide with Q7 whether the default stays on.
- **Acceptance:** on a Windows machine with Synesthesia 1.25.4 installed, the library shows 88 scenes marked Synesthesia beside the library's, Settings hides them, and nothing under `Program Files` changes after tagging, saving and copying.

## Epic P — Playlist, audio timing and show control

### P-01 Audio-event advance
- **Status:** Built (D226), pending real music: `strobe_audio::events` hears breakdowns and drops in the analyser's raw energies and the show hands them to the clock; a cue that can end only on audio says so when the audio is synthetic. Left: tuning the thresholds on real sets (a person, or recorded tracks we may use).
- **Work:** connect Phase 7 audio hit/event detection to playlist clock advancement with hysteresis and minimum intervals; allow per-show selection of event types and safe fallback when audio is absent.
- **Acceptance:** synthetic fixtures deterministically exercise events; no double advance on one transient; no accidental advance during device reconnect; show continues with a clear status when audio is unavailable.

### P-02 Downbeat and bar detector
- **Status:** Built (D233), pending real music: `strobe_audio::downbeat` scores each beat by its bass energy and band-profile rise, folds the scores into four slots, and reports the leading slot with a confidence (its lead past a noise allowance), after eight beats of warm-up and four bars of evidence, with two bars of hysteresis; it resets when the grid loses its lock. The app moves the clock's bar lines to it at confidence 0.5 or more and counts from the grid's origin otherwise, so a low-confidence estimate falls back to the old behaviour. Synthetic fixtures: an accented "one" found at all four phases with confidence 1; even kicks at 97-174 BPM give 0; a one-bar fill does not move the bar; a new track's bars are followed. Left: recorded music we may use, and measuring the error there.
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
- **Status:** OSC built (D209): changes only, 30 Hz cap, a sender thread with a bounded queue, off by default, the loop into the listener refused; tested over loopback and in the app. MIDI built (D257):
  - each MIDI mapping's control goes back to its named device as a CC, a 14-bit pitch bend or a pad light;
  - the mapping's range, curve and invert are undone, so the fader shows where moving it would put the control;
  - changes only, 30 Hz, a sender thread with a bounded queue;
  - off by default, with "Send feedback" in MAP's MIDI block.

  Left: a real controller (P-04) and a real TouchOSC layout.
- **Work:** add optional MIDI output and OSC output for state feedback to motorised faders/TouchOSC; define subscription and rate limiting; avoid echo loops when output returns to input.
- **Acceptance:** loopback and virtual-device tests verify values, rate limits and no feedback loops; outputs disabled by default and have explicit target/interface settings.

### P-06 Explicit blackout commands
- **Status:** Built (D201): `blackout_on` and `blackout_off` beside the toggle, in the command table, the palette, OSC, MIDI mappings and the remote protocol. Left: the remote server itself (L-02 on).
- **Work:** add set-blackout-on and set-blackout-off command targets alongside toggle, expose them to OSC/MIDI mappings and the command table, and preserve safe semantics for remote clients.
- **Acceptance:** repeated on is idempotent; repeated off is idempotent; mappings do not accidentally unblack an already blacked output; UI/remote state converges after reconnect.

### P-07 Macro and preset completion
- **Status:** In progress: range and colour editing and pinning (D210), preset rename and confirmation before replace or delete (D214), the preset media channel (D223), a glide per macro (D227) and a key per macro (D228) are built. Left: a person using them with a real controller.
- **Work:** complete range/colour editing, macro shortcut/smoothing/pinning; add preset rename; add media preset channel after media-per-cue exists; validate save/recall/delete and missing-control resolution.
- **Acceptance:** no data loss on unresolved targets; presets remain backwards-compatible; user sees what will be changed before destructive replacement.

### P-08 Synesthesia OSC compatibility decision
- **Status:** Decided and built (D234), from Synesthesia's OSC manual as read on TORNADO (T20): its input addresses that mean the same in Strobe are aliases of the built-in space (`/controls/scene|meta/<name>[/x|y|r|g|b]`, `/controls/global/<type>/<n>`, `/favslots/<n>`, `/playlist/next|previous|toggleplay`), and two or three numbers to a whole pad or colour set its components. Ignored, with tests: the `/raw` `/default` `/random` `/preset` `/lock` suffixes, scene-specific addresses, string-argument messages. Feedback stays on `/strobe/...`. Left: a real TouchOSC or Synesthesia layout against Strobe, and whether control names keep underscores in Synesthesia's addresses (the manual contradicts itself; Strobe ignores them either way).
- **Work:** compare the built-in /strobe namespace to Synesthesia's documented OSC address layout and decide whether compatibility is a requirement; if not, document the intentional divergence.
- **Acceptance:** decision recorded before adding aliases; tests cover every supported address and invalid payloads.

## Epic M — Media and transitions

### M-01 SSF media/cubemap specification audit
- **Status:** Audited against SSF's documentation (TORNADO T21, Q22; D238). Matches: `IMAGES {NAME, PATH}`, `CUBEMAPS {NAME, PATHS}` in GL face order, `syn_MediaType` 0 none / 1 image, `MEDIA` as a slot with `WRAP`/`FILTER`. Fixed: media the operator chooses is now sampled with the slot's `WRAP`/`FILTER`, defaulting to repeat/linear as SSF documents (it was always clamp). Divergences kept: Strobe's own `MEDIA` path form (a scene's own picture or sequence); video (2), webcam (3), NDI and Spout are not inputs (M-04). Left: cube-face orientation against a real Synesthesia render (undocumented); the undocumented user-media helpers (`_loadUserImage` and family) that eight real scenes use.
- **Work:** compare MEDIA, CUBEMAPS and syn_MediaType handling against authoritative SSF docs and representative scenes; verify cubemap orientation and image slot semantics.
- **Acceptance:** fixtures cover each documented shape; supported, degraded and unsupported forms are written down; malformed assets fail visibly without crashing the show.

### M-02 Media picker and playlist media
- **Status:** Done (D217, D223): the Inspector's Media row (what shows, where it comes from, Choose / Scene's own), a cue's `media` key edited from the LIST cue card, a preset's media channel, the precedence cue > operator > preset > scene, and choosing again for what already shows starts nothing. Verified on the T480 (Task 26).
- **Work:** add media selection to the UI; support scene media and cue overrides; include media in preset channel semantics; handle missing paths and replacement safely.
- **Acceptance:** media selection is asynchronous; the active scene remains live during decode; cue changes load the expected media without unbounded GPU memory; preset scene/meta/media channels behave independently.

### M-03 Image-sequence resource management
- **Status:** Done (D225, T480 Task 28): a sequence is bounded at 192 MiB of decoded frames and a long one is shrunk as a whole to fit (down to 256 px on its long side) instead of being cut; decoding stays off the frame thread with at most one upload a frame (D186). On the T480 a 475 MiB loop loads in about 4 s, costs about 280 MB, stays flat and plays at 60 fps.
- **Work:** measure sequence upload and resident memory on T480/shared memory; implement bounded frame cache or streaming if full sequence residency is unsafe; test resize, loop and decode errors.
- **Acceptance:** long sequence memory is bounded/documented; decoding stays off the frame thread; missed frames degrade gracefully.

### M-04 Video and webcam input
- **Status:** Deferred research; not a v1 blocker.
- **Work:** compare lightweight maintained decoder/capture options, license implications, hardware decode availability, Windows device lifecycle and upload path. Avoid introducing a heavy dependency without measurements.
- **Acceptance:** proposal includes binary size, supported formats, device removal/reconnect, latency, GPU upload cost and license review; implementation starts only after a decision.

### M-05 Song-change transitions
- **Status:** Built (D237), pending real music: `strobe_audio::song` hears a new track as a gap (0.8-10 s of silence after 30 s of music, then 1.5 s of sound) or a new tempo held 8 s at confidence 0.6 (double and half time are the same tempo), at most one per 30 s. A cue ends on it with `advance = "audio-song"` (alone or in a list with the timer). Only live audio counts, and any change of audio source resets the detector, so a reconnect is never a song. Any audio event during a transition is ignored, so a crossfade is never cut short by a second. Manual next works as always. Tests: gap, breath, stop, octave flip, unsure tempo, too-short tempo, spacing, reset, nonsense input, the mid-transition guard and the file round trip. Left: real sets; a tempo-matched DJ mix with no gap is not heard (a cue's timer or a person covers it).
- **Work:** define song-change detection source and confidence, transition interaction with playlist anchors and audio-event advance, and manual override behaviour.
- **Acceptance:** deterministic tests for no audio, low confidence, track boundary and manual override; no double transition with a cue transition already in progress.

### M-06 NDI output
- **Status:** Deferred research; not a v1 blocker.
- **Work:** evaluate SDK/runtime licensing, Windows packaging, pixel format, frame pacing, GPU readback versus shared textures, latency and failure handling.
- **Acceptance:** measured prototype with frame-time impact and a clear optional-dependency story before committing to a full implementation.

## Epic L — LAN remote and PWA

### L-01 Finish the transport-independent protocol
- **Status:** Done (D232, D236): command semantics aligned with the app's command ids; `negotiate` and `decode_client` bound and validate every client message (4 KiB, request ids, 128-byte secrets) and tell a client of another version `version_mismatch` even when its `hello` has fields this version does not know; every version-1 message is a golden test, read and written byte for byte.
- **Work:** keep message types versioned, strict, documented and allowlisted; align command semantics with app command IDs; add compatibility tests for older/newer protocol versions.
- **Acceptance:** unknown fields/commands fail closed; request IDs and message sizes are bounded; protocol contract is documented and tests pass in CI.

### L-02 Listener lifecycle and HTTP/WebSocket transport
- **Status:** Built (D250, D252, D254); real devices still to test. `strobe_remote::server` is an opt-in listener on its own threads (one accept thread, one per connection, at most eight), bound to an address the operator gives or this computer's LAN address, never by default (`--remote`, `--remote-bind`, `--remote-port`, or Settings' Start, never saved). It serves the embedded page at `/` and a WebSocket at `/ws` carrying protocol v1, with our own small HTTP and RFC 6455 subset and `sha1` 0.10 the only new crate (cargo-deny clean). Inbound frames are bounded before they are read; there is no outbound queue (the latest snapshot, coalesced, and each reply, with a 2 s write timeout). Start and stop are idempotent; stop shuts every socket at once and frees the port; bind failures say what to do (port taken, not this computer's address, permission, public). Tests: `crates/strobe-remote/tests/server.rs` (`stopping_drops_every_client_at_once_and_frees_the_port`, `a_public_or_taken_address_is_refused_with_advice`, `the_page_is_served_same_origin_with_a_strict_policy`), `strobe_app::lan` (`nothing_listens_until_the_remote_is_started`, `the_remote_starts_publishes_and_stops`), `cli::tests::the_lan_remote_is_off_unless_asked_for`. **Not yet:** a phone over Wi-Fi, the Windows firewall prompt, the T480.
- **Work:** select and document the minimal maintained server stack; run it on a dedicated thread/runtime; opt-in bind to a chosen LAN interface; serve embedded same-origin assets; start/stop idempotently; clean shutdown; bounded inbound and outbound queues.
- **Acceptance:** no sockets open by default; the render loop never blocks on network; bind failure is actionable; disabled listener rejects old clients immediately.

### L-03 Pairing, tokens and revocation
- **Status:** Built (D251, D468, D469); real devices still to test. Pairing is now an approval at the host with no code: a phone sends `request_pair` with a name, a desktop `request_desktop` (`a_desktop_asks_without_a_code_and_the_host_approves_or_denies`), and either queues a request the operator approves or denies (90 s, at most four waiting, one per address, 30 s after a Deny). The D251 six-digit code is still parsed for the frozen version-1 `pair` message (120 s, one use, five wrong guesses in all burn it), but the app never opens it, so an old page is told pairing is not open. Each client gets a 256-bit token, stored only as its SHA-256 in `remote-clients.json` (owner-only on Unix, atomic write), rotated on every authentication (the old one dies when the new one is first used), revocable one by one or all at once with the listener on or off (an open connection is closed). Secrets print `***` and never reach logs or URLs. Tests: `strobe_remote::pairing` (expiry, replay, brute force, approval, denial), `strobe_remote::clients` (rotation, revocation, restart, damaged file, digests on disk), `strobe_remote::secret`, and `tests/server.rs` (`pairing_needs_the_code_and_the_desktop_and_then_commands_run`, `a_denied_request_pairs_nothing_and_a_replayed_code_fails`, `guessing_codes_burns_the_code_and_drops_the_guesser`, `tokens_rotate_and_a_revoked_client_is_dropped_and_refused`, `paired_clients_survive_a_restart_and_revoke_all_clears_them`, `no_secret_ever_reaches_the_log`). Since D469 also `a_phone_asks_with_a_name_and_the_host_approves_it_with_no_code` and the desktop request tests. **Not yet:** pairing a real phone.
- **Work:** cryptographic pairing code, short expiry, attempt limits, explicit desktop approval, high-entropy per-client token, secure persistence, individual/all-client revoke, token rotation and secret redaction.
- **Acceptance:** tests for expired/replayed pairing, brute-force rate limit, invalid/revoked token, restart semantics and logs; no long-lived token in QR URL or query string.

### L-04 Network security and abuse controls
- **Status:** Built (D252); real devices still to test. Host must be this listener's IP literal and port (names refused: DNS rebinding), the upgrade's Origin exactly the page's own (cross-site WebSocket hijacking), a public bind refused, every-interface and loopback binds warned, peers from public addresses closed unread; eight connections, 4 KiB messages, 10 messages/s with bursts of 20, 30 s idle, 120 s to pair or authenticate (the code's lifetime), 5 s for the request head, slow readers dropped after a 2 s write; no CORS; a hash-based CSP on the page. Residual risk of plain HTTP stated in D252 and the user guide. Adversarial tests in `tests/server.rs`: `an_unexpected_host_is_refused_to_stop_dns_rebinding`, `another_origin_cannot_open_the_socket`, `oversized_and_malformed_messages_are_refused_without_harm`, `a_flood_is_cut_off`, `a_full_command_queue_is_busy_not_a_backlog`, `a_client_that_stops_reading_is_dropped_not_buffered`, `connections_beyond_the_limit_are_turned_away`, `idle_unpaired_and_trickling_connections_time_out`; frame-level cases in `server::ws` and `server::http`. "Without frame stalls" rests on the design (the app only `try`s) until the T480 soak (L-08) measures it.
- **Work:** Host/Origin validation, same-origin WebSocket checks, message-size/client limits, rate limiting, idle timeout, public-network refusal/warning, firewall guidance, CSWSH/DNS-rebinding protection and clear residual risk for plaintext LAN transport.
- **Acceptance:** adversarial tests reject unexpected origins/hosts, oversized JSON, floods, malformed payloads and slow clients without panic, queue growth or frame stalls.

### L-05 App-thread command/state bridge
- **Status:** Bridge built (D232): `strobe_remote::bridge` gives a bounded queue (full → `Busy` at once, gone app → `Unavailable`, unanswered → `Busy` after the adapter's timeout), a reply per request, and a snapshot whose revision moves only on change. Remote command ids are tested to be mappable app `CommandId`s. The app now drains it (D254): step 1c of the frame takes up to eight commands with `try`, checks each against the app's state (`lan::refusal`, tested in `playlist_commands_need_a_ready_playlist`), runs it through the command's own `Action`, answers it, and publishes the state at 10 Hz. Left: the T480 soak showing no frame-time cost (L-08).
- **Work:** bounded channel drained by app event-loop; revalidate each command on the owning thread; use existing command path; return accepted/rejected result; publish revisioned snapshots and coalesce stale state updates.
- **Acceptance:** every command runs on app thread; playlist and no-playlist semantics are correct; full queue returns Busy; reconnect gets a complete fresh snapshot; tests show no frame-time blocking.

### L-06 Touch-first controller UI
- **Status:** Built (D253); no real browser has rendered it yet. One embedded file (`crates/strobe-remote/src/server/controller.html`) with current/next scene, cue time left, Hold, explicit Blackout on/off, Next/Previous, Restart, +30 s, Skip section, Tap tempo, output and audio status, a connection pill with reconnect and stale states, and the pairing flow; portrait and landscape layouts, safe areas, keyboard focus, `aria-live`/`aria-pressed`, high contrast, forced colours and reduced motion; no CDN, telemetry or live preview, served under a CSP that allows only its own hashed script and style. Tests in `server::page` check each property statically; the script passes `node --check`, and Node 22's WebSocket client interoperated with the listener. **Not built:** Performance Macros (protocol v1 has no macro command; that is v2, D236) and a QR code. **Not yet:** Android Chrome and iOS Safari on real devices.
- **Work:** responsive local HTML/CSS/JS for current/next scene, cue/time, hold, output/blackout, audio status, macros, connection state and reconnect; QR/pairing flow; no external CDN or telemetry.
- **Acceptance:** portrait/landscape, safe areas, keyboard accessibility, high contrast, reduced motion and stale/disconnected states tested; no live preview streaming in v1.

### L-07 PWA HTTPS and real-device validation
- **Status:** The TLS decision is made (D258): HTTPS is the operator's own reverse proxy with a real certificate for a domain they own, and `--remote-proxy NAME` lets that proxy in, from this computer only, with an `https://NAME` origin and a `wss:` policy (tested). No self-signed certificate, no warning to click past. Left: Android Chrome and iOS Safari through such a proxy over Wi-Fi; only then a manifest and service worker.
- **Work:** compare user-managed local HTTPS/reverse proxy and future certificate-enrolment options; do not tell users to bypass warnings; add manifest/service worker only once installability is genuinely supportable.
- **Acceptance:** Android Chrome and iOS Safari tested on real devices over Wi-Fi; sleep/resume, rotation, roam, browser restart, updates and revocation tested; only then advertise installable PWA support.

### L-08 Remote soak and operational docs
- **Status:** Docs written, soak open. `docs/USER_GUIDE.md` has "Remote control from a phone": enabling it, pairing, the trusted-network-only limit of plain HTTP, the Windows firewall prompt (allow private networks only), and turning it off; it claims no HTTPS or PWA. The listener is always visible while on (title bar "Remote on" with the connection count) and has a one-click Stop beside it (D254). **Not done:** the T480 soak (multiple phones, stop/start, Wi-Fi loss, command floods, idle CPU and memory, frame time) and seeing the Windows firewall prompt on a real machine.
- **Work:** test multiple clients, stop/start, Wi-Fi loss, Windows firewall, idle CPU/memory, command floods and pairing lifecycle on T480; document trusted-network-only limits if TLS is absent.
- **Acceptance:** no render regression or unbounded resource growth; listener always visibly enabled and has a one-action desktop kill switch.

## Epic G — Graphics backend research

### G-01 Machine-readable shader compatibility report
- **Status:** Met by the corpus harness (D195): `cargo xtask corpus` writes `report.json` with the driver each dialect ran on, one record per scene and dialect (status, categorised cause, first error line, compile, first-frame and per-frame times, black/white flags, an FNV-1a checksum of the final frame) and a derived `report.md`. `tests/corpus.rs` runs a working, a broken, a scripted, a bad-JSON, a throwing and a blank scene through it on CI's headless context in both dialects, so a deliberately broken shader is caught and categorised in CI. Run on TORNADO over the real corpus (T6, T18) and on the T480 over the library (Task 30: 12 of 12 in both dialects). Left: the Shadertoy fixtures through the same report (they are covered by the autocontrol engine tests today).
- **Work:** compile/render the current scene library, Shadertoy fixtures, Synesthesia corpus and owner-supplied shaders; record backend/compiler, compile result, image checksum, frame time and categorized failure; run on llvmpipe CI, T480 and TORNADO.
- **Acceptance:** deterministic report format, a deliberately broken shader is detected in CI, and unsupported shaders are categorized rather than omitted silently.

### G-02 Renderer seam design
- **Status:** Done as a contract (D255): [RENDERER_SEAM.md](RENDERER_SEAM.md) lists what a backend must provide at each stage and where GL leaks into the app today. No trait or code moved: the GL path and its benchmarks are unchanged, and the trait waits for a second implementation (G-04).
- **Work:** define only the scene/pass-boundary contract required for device/context creation, scene preparation, frame render, resize, diagnostics, timing and device-loss recovery; keep GL implementation and GL 3.3 floor intact.
- **Acceptance:** current GL path remains behaviourally unchanged and benchmark baselines are comparable; no per-frame allocation or synchronous readback introduced.

### G-03 wgpu/Naga versus direct GLSL-to-SPIR-V spike
- **Status:** Done, with a written go/no-go (D255, `docs/research/g03-frontends/README.md`), on 26 scenes and 51 pass sources:
  - **naga: no-go.** Its GLSL front end has no combined samplers, and its SPIR-V front end fails on loading them, so 0 of 51 pass the GLSL route and only the 11 passes that sample no texture pass the SPIR-V one.
  - **glslang -> SPIR-V -> SPIRV-Cross: viable as a front end.** 51 of 51 to GL SPIR-V, 50 of 51 to Vulkan SPIR-V, and HLSL and MSL from all 50.

  Not measured, because no second renderer exists: image fidelity and frame time on another API.
  - **Full corpus done (D292, 2026-10-15):** 120 scenes and 288 passes, the owner's 90 Synesthesia scenes included. naga's GLSL input takes 0 of 288. glslang takes all 227 corpus passes, and SPIRV-Cross writes HLSL and MSL for every one. naga 30.0.1 is still the newest. Splitting samplers in SPIR-V first lets naga take 60 of 60 library passes but only 56 of 227 corpus passes.
- **Work:** run both candidates over the exact same corpus; report GLSL construct failures, image fidelity, load/compile time, frame time, binary size, dependency and maintenance cost; include Vulkan and platform path considerations for DX12/Metal/MoltenVK.
- **Acceptance:** written go/no-go based on corpus results; no production backend enabled by default until the full compatibility and hardware matrix passes.

### G-04 Backend vertical slice and device-loss handling
- **Status:** Deferred, and narrowed by D292. Vulkan and Metal come
  through ANGLE (G-05 to G-07) with no new renderer. This slice is for a
  platform or feature ANGLE cannot serve, such as D3D12-only features,
  compute, or a target with no GLES.
  - If it is ever built, its front end is glslang -> SPIR-V -> SPIRV-Cross
    (227 of 227 corpus passes, `docs/research/g03-frontends/README.md`'s
    follow-up). It is not naga.
- **Work:** behind a feature flag, implement one representative multipass feedback scene, audio texture, resize, async timing/readback and loss/recreate path; keep GL as fallback.
- **Acceptance:** matching visual tests and load-time shader preparation on T480 and TORNADO; no regressions to GL/ANGLE; release claims distinguish Vulkan, DX12, Metal and MoltenVK support.

### G-05 ANGLE's Vulkan backend on real GPUs
- **Status:** Run on a GPU (D485): on TORNADO (GTX 1080 Ti, driver 582.78) `strobe-bench --context window --gl angle-vulkan` rendered the 17 library scenes with images matching ANGLE-D3D11 and frame times within run-to-run noise of it (BENCHMARKS.md, 2026-10-26). It starts only with Electron's `vulkan-1.dll` beside ANGLE, which the release now ships. Before that: CPU Vulkan in a container (D292, [research/angle-backends.md](research/angle-backends.md)). Left: the T480, the corpus over the owner's scenes, a 30-minute soak and the verdict on Vulkan as a fallback.
- **Work:** on the T480 and TORNADO, with the DLLs the release ships plus Electron's `vulkan-1.dll` (or the driver's own in `System32`), set `ANGLE_DEFAULT_PLATFORM=vulkan` and run `strobe-bench --context window --gl angle` and `strobe-corpus --context window --dialect gles`. Compare each with ANGLE-D3D11 and native GL: images, frame times, compile times, and a 30-minute soak.
- **Acceptance:** the renderer string names Vulkan, and the corpus matches ANGLE-D3D11's pass list. The frame times are recorded in BENCHMARKS.md, and a written verdict says whether Vulkan should ever be the fallback before D3D11.

### G-06 Choose ANGLE's backend from `--gl`
- **Status:** Done (D485): `--gl angle-d3d11 | angle-vulkan | angle-gl | angle-metal` in the app, bench, autocontrol and corpus, through `ANGLE_DEFAULT_PLATFORM` set before any thread, with the renderer string checked afterwards so a backend ANGLE lacks fails with a message instead of a quiet fall-back. Checked on TORNADO: D3D11, Vulkan and GL start, Metal is refused. The extension-string check of the work list is replaced by that renderer check, which also catches a backend that is advertised but does not start.
- **Work:**
  - `--gl angle-d3d11 | angle-vulkan | angle-gl | angle-metal` (with `angle` keeping today's default), shown in the diagnostics line.
  - Each choice is offered only when ANGLE's client-extension string advertises it (`EGL_ANGLE_platform_angle_vulkan`, `_metal`, `_d3d`).
  - The type is passed as `EGL_PLATFORM_ANGLE_TYPE_ANGLE`, either through a glutin patch (preferred, upstreamable: display attributes for the ANGLE platform) or through the environment variable, set by a leaf wrapper in `gl/` before any thread starts.
  - D66's fail-over order stays native, then ANGLE's default.
- **Acceptance:** each choice starts or fails with a message naming what is missing. The bench and corpus accept the same flag. Nothing changes when the flag is absent.

### G-07 macOS through ANGLE-Metal
- **Status:** Not started (D292). Waits for a Mac to test on.
- **Work:**
  - An EGL provider in `strobe-gfx/src/gl/` for Apple targets, since glutin compiles EGL out there. It dlopens `libEGL.dylib` and calls `eglGetPlatformDisplayEXT(EGL_PLATFORM_ANGLE_ANGLE, …, TYPE_METAL_ANGLE)`; the type must be explicit, because ANGLE's own default on macOS is its GL backend.
  - It creates a GLES 3.0 context, with a window surface from the winit `NSView`'s `CALayer`.
  - It provides vsync, `get_proc_address` and robustness, under the unsafe-boundary rules.
  - A headless variant for the bench and tests, which ends D55's two-tier macOS CI.
  - Fetch `libEGL.dylib` and `libGLESv2.dylib` from the pinned Electron `darwin-arm64` and `darwin-x64` zips with hashes, as `fetch-angle.ps1` does (minimum macOS 12), and re-sign them inside the `.app`.
- **Acceptance:** the library and the corpus in the GLES dialect on Apple silicon, with images compared against ANGLE-D3D11 and native GL, and frame times recorded. CGL stays available as `--gl native` for comparison.

## Epic D — Documentation, evidence and maintainability

### D-01 Reconcile status documents
- **Status:** Ongoing, reconciled 2026-10-13. This round:
  - STATUS_AUDIT gained the 30-minute soak (TORNADO T19), the Synesthesia OSC addresses (T480 Task 31), the audio-structure inputs and the bench's script note, each with its evidence;
  - L-01, L-05, M-01, M-05, P-02, P-08, C-05 and I-02 here match their decisions (D232-D239);
  - stale "no downbeat detector" claims were removed from the crate READMEs and ROADMAP.
- **Work:** keep ROADMAP.md the short priority view, STATUS_AUDIT.md the evidence log, and this file the cross-epic task register; remove contradictory stale claims as tasks change.
- **Acceptance:** no task is simultaneously described as both unimplemented and verified; every checked item names its evidence location.

### D-02 User-facing install and troubleshooting guide
- **Status:** Written (`docs/USER_GUIDE.md`): install from the release zip, first run, keys, scenes, audio and Windows microphone privacy, output, media, playlists, controllers, Shadertoy licences, graphics backends, and where logs and crash reports are. Left: a person without a toolchain following it on a clean machine (the acceptance).
- **Work:** document clean Windows installation, ANGLE DLL provenance, audio permissions/devices, shader imports and license responsibility, diagnostics location, recovery and common GPU-driver issues.
- **Acceptance:** a person without a Rust toolchain can install, launch, import a scene, choose audio, fullscreen output and collect diagnostics using only the guide.

### D-03 Feature/compatibility matrix
- **Status:** Written: [COMPATIBILITY.md](COMPATIBILITY.md), ten sections, every row Supported, Degraded, Unsupported or Unverified with its evidence (127 rows on 2026-10-13). Keep it in step with STATUS_AUDIT.md, which wins on conflict.
- **Work:** publish supported/degraded/unsupported matrix for SSF, Shadertoy, scripts, media, GL/ANGLE, Windows release, MIDI/OSC and remote; include evidence and platform limitations.
- **Acceptance:** no feature is described as supported solely because a type or UI control exists; evidence links to fixtures or hardware logs.

### D-04 CI coverage and non-flaky hardware gates
- **Status:** Skips are counted (D249): every GPU test that skips records it through `strobe_gfx::skip`, `cargo xtask ci` prints how many passed by skipping and why, and CI's Windows job writes the same to its summary. Coverage of the listed areas:
  - protocol: `strobe-remote` golden messages;
  - command semantics: `strobe-io` and `commands.rs`;
  - parser rewrites: `promote` and the converter;
  - importer security: path, size and host limits;
  - generated controls;
  - asset errors;
  - no runtime compiles: the show and playlist tests.

  The hardware gates stay manual, with their artifacts in `hardware/T480.md` and `TORNADO.md`. Left: nothing in code; a GPU runner would let the Windows job require a context.
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
