# JaJa PDX projector mini-PC

The venue machine that drives the projector. Reached through a Claude Code
bridge session that runs as a Windows **service** (session 0), so anything
it launches directly lands on an invisible desktop; real-display runs go
through a one-shot scheduled task in the signed-in user's session 1,
created and deleted per run with the owner's approval.

| | |
|---|---|
| Machine | AZW SER3 mini-PC |
| CPU | AMD Ryzen 3 3200U, 2 cores / 4 threads |
| RAM | 14 GB usable |
| GPU | AMD Radeon Vega 3 (integrated), driver 31.0.21925.1001 (2026-05-19); GL `3.3.0 Core Profile Context 26.5.2.260413` |
| OS | Windows 11 Pro 26H2, build 26300.9711 (updated from 25H2 26200 during a reboot on 2026-10-14) |
| Projector | Sony (EDID SNY8203), 1920x1200 @ 60 Hz, the only display at the console |
| Power | High performance plan, with display-off, sleep and hibernate at never on AC and DC (owner, 2026-10-14, after it idle-slept once during testing). The step 7 numbers below were measured under Balanced |
| Tools | git, Rust 1.98, Visual Studio 2022; no `gh` |

## The released v0.1.0 on a clean install (2026-10-14, Task M2, R-03)

Unzipped from the published archive (SHA-256 verified) into
`%USERPROFILE%\strobe-test\`, against a fresh `%LOCALAPPDATA%\strobe`.

- **Native GL:** Vega 3, OpenGL 3.3 core. The only WARN is the known one:
  `WGL_ARB_create_context_robustness` is not supported, so a driver reset
  would not be reported. There are no ERRORs.
- **ANGLE:** starts in session 1, ES 3.0 on D3D11, with no robustness
  WARN. In session 0 it fails at surface creation (`EGL 0x300e`, context
  loss): a session with no presented desktop, not the GPU.
- **Scripts:** 11-script-orbits runs cleanly in session 1. One interrupt
  of the 4 ms per-frame script budget in session 0 came from contention
  with a Windows Update worker, and recovered on its own.
- **Shadertoy listener:** `--listen` binds 127.0.0.1:47811 only, and
  writes its token.
- **Relaunch over an existing data folder:** works. Frames are
  byte-identical across runs.
- **First run:** creates `logs\` and `run\`. Screenshot mode saves no
  `ui.toml`, by design.
- **Found and fixed on master after the release:**
  - A screenshot run never ended when the control window could not paint
    (session 0, or RDP). It now gives up after 10 s.
  - A fatal startup error raised a message box on an invisible desktop and
    hung. Unattended runs and service sessions now only log.
  - strobe-bench reported timings for all-black frames in session 0. It
    now fails when every frame is black.
- **Not fixed:** killed runs leave stale `run\strobe-crash-<pid>.sock`
  files, which nothing cleans up.

## The 12 shipped scenes on the projector (2026-10-14, Task M2 step 7)

The runs were in session 1, with `--show-output` and the output window
moved over the whole 1920x1200 desktop (client area 1904x1161, scale 1.0
before adaptive). Each ran 66 s with synthetic audio and a 60 Hz budget.

| Scene | fps | GPU ms | Late frames | Worst gap | Settled scale |
|---|---|---|---|---|---|
| 01-aurora-gradient | 60.0 | 3.90 | 0 / 3603 | 23.6 ms | 1.00 |
| 02-plasma-pulse | 60.0 | 4.92 | 4 / 3599 | 39.8 ms | 1.00 |
| 03-kaleido-tunnel | 59.8 | 6.47 | 11 / 3586 | 61.7 ms | 1.00 |
| 04-voronoi-glass | 59.2 | 12.73 | 45 / 3551 | 36.9 ms | 0.80 |
| 05-domain-warp | 59.1 | 12.24 | 36 / 3547 | 39.7 ms | 0.70 |
| 06-feedback-trails | 60.0 | 5.84 | 1 / 3599 | 25.4 ms | 1.00 |
| 07-reaction-diffusion | 59.9 | 7.15 | 2 / 3597 | 34.6 ms | 1.00 |
| 08-bloom-orbs | 59.5 | 13.12 | 8 / 3568 | 42.2 ms | 0.90 |
| 09-sdf-cathedral | 57.1 | 13.91 | 29 / 3429 | 72.6 ms | 0.50 |
| 10-volume-clouds | **27.7** | 33.36 | 1660 / 1661 | 554.6 ms | **0.25 (minimum)** |
| 11-script-orbits | 60.0 | 7.03 | 2 / 3600 | 25.1 ms | 1.00 |
| 12-silicon-atlas | 56.6 | 14.14 | 24 / 3399 | 119.4 ms | 0.40 |

**ANGLE**, the three heaviest:
- 09: 56.1 fps at 0.50.
- 10: 28.8 fps at 0.25.
- 12: 56.6 fps at 0.40.

These are within about 1 fps of native. 12-silicon-atlas takes
**6.5 s to load on ANGLE**, against 1.4 s native, because of D3D shader
compilation.

**Summary:**
- Six scenes hold 60 fps at full scale.
- 04, 05 and 08 hold 60 fps by scaling down.
- 09 and 12 are marginal.
- 10-volume-clouds cannot hold 30 fps even at the minimum scale. It needs
  a cheaper path on this class of GPU.

## High performance, and the bench in session 1 (2026-10-14, Task M3)

**Power plan:** High performance makes no difference here. Under it, 09
ran at 56.6 fps (57.1 under Balanced) and 10 at 26.7 fps (27.7). The
3200U and Vega 3 are GPU-bound.

**strobe-bench at 1920x1200**, in session 1, from a cdfab8e build:
- **Native:** fails the black-frame check. A *hidden* window on AMD's WGL
  draws nothing even on the real desktop, so this is not only a session-0
  effect. It matters beyond the bench: the control-generation worker
  (D140, `--context window`) uses a hidden window too. Being checked
  (M3b).
- **ANGLE:** valid, and it matches the app at scale 1.0:

| Scene | Compile ms | Mean ms | p95 ms | 60 fps | 30 fps |
|---|---|---|---|---|---|
| 01-aurora-gradient | 57 | 1.13 | 1.32 | yes | yes |
| 02-plasma-pulse | 35 | 1.65 | 1.94 | yes | yes |
| 03-kaleido-tunnel | 50 | 1.63 | 1.84 | yes | yes |
| 04-voronoi-glass | 133 | 18.91 | 20.64 | no | yes |
| 05-domain-warp | 187 | 23.27 | 25.51 | no | yes |
| 06-feedback-trails | 42 | 1.82 | 1.97 | yes | yes |
| 07-reaction-diffusion | 158 | 4.16 | 4.60 | yes | yes |
| 08-bloom-orbs | 89 | 15.14 | 16.35 | barely | yes |
| 09-sdf-cathedral | 153 | 48.17 | 54.52 | no | no |
| 10-volume-clouds | 359 | 423.63 | 464.28 | no | no |
| 11-script-orbits (script not run) | 94 | 3.74 | 4.69 | yes | yes |
| 12-silicon-atlas | **6506** | 92.62 | 100.61 | no | no |

## The show's black output: a control window with no paints (2026-10-15, D290)

**What happened.** The venue setup: two projectors in Duplicate mode as one
1920x1080 desktop, no keyboard or mouse. A scheduled task (interactive,
session 1) ran `cmd /c start "" /min start-show.cmd`, which starts
`strobe.exe --library ... --playlist ... --no-synesthesia --output-monitor 1
--fullscreen --audio synthetic` with `start "" /b`. The output came up
black and stayed there.

**The log.**
- "output placed at launch: monitor 1, fullscreen true", then "output window
  resized to 1920x1080, fullscreen true", then "control window in view:
  redrawing at 30 Hz".
- The perf line: "control 1 redraws (158.8 ms)" at 60 s, then "control 0
  redraws" at 120 s and 180 s. The output ran at 60 fps throughout.
- The playlist stalled at 1 of 14: it compiled one scene per control frame.

**The windows.** "Strobe Output" at 0,0-1920,1080, visible and foreground;
"Strobe" (control) at -8,-8-1928,1040, maximised, behind it.

**What did not make the control window draw:**
- 90 posted `InvalidateRect`/`WM_PAINT` nudges;
- moving the output off-screen for 180 s.

So it was not occlusion. Earlier runs launched with `--scene` from another
task had logged "covered by the fullscreen output: redrawing at 2 Hz", and
the control window did draw now and then.

**First workaround, fca44a5.** The output frame compiles the next playlist
step when the control window has been quiet for 500 ms. That started the
show. Everything else in the control frame still never ran: UI actions, the
remote and LAN UI passes, the desktop remote's capture, `ui_state.tick`, the
remote mapping's persist and preview loads. So the laptop's desktop remote
could not have reached this machine.

**Root cause (D290).** Paint starvation, not occlusion:
- winit on Windows sends `RedrawRequested` only from `WM_PAINT`.
- Windows makes `WM_PAINT` for the topmost dirty window of the thread first.
- winit returns to `about_to_wait` after every `RedrawRequested`.
- On Vega 3 the fullscreen output's vsync wait is inside its own swap, so
  each output frame takes about 16.7 ms. That is longer than the 6.9 ms
  `OUTPUT_PERIOD`, so the output asks for its next frame on every pass.

The output, above the control window, is therefore always the first dirty
window, and the control window's paint never comes. The nudges and the
off-screen move changed neither the z-order nor the output's dirtiness. The
"in view" state did not matter either: focus said the control window was on
top, and z-order said otherwise.

**The fix (D290).** When a control redraw request has gone unanswered for
250 ms and 4 passes of the event loop, `about_to_wait` runs the control frame
itself. It keeps doing so at 2 Hz, or 5 Hz while a desktop remote holds the
lease, until Windows answers a request promptly again. The whole frame runs
(egui, paint, desktop capture, swap, actions, playlist compile). fca44a5
stays as the backstop.

### Hotfix deploy

On the mini PC (it has Rust 1.98 and Visual Studio 2022), in the strobe
checkout, at the commit carrying D290:

```
git fetch origin
git checkout <hotfix commit>
cargo build --release -p strobe-app
```

Stop the show (or wait for it to end) and copy `target\release\strobe.exe`
over the `strobe.exe` that `start-show.cmd` launches. Keep the old one beside
it as `strobe-prev.exe` to roll back. Then relaunch through the same
scheduled task, so the run is in session 1 exactly as at the show.

### Verification after the show

Run the same launch (scheduled task, `start-show.cmd`, the show playlist,
`--output-monitor 1 --fullscreen`) for at least 3 minutes, then read the
newest log in `%LOCALAPPDATA%\strobe\logs\`:

1. Within the first seconds, this line, once:
   `control window: no paint from Windows for N ms (takeover 1); the event loop runs its frames every 500 ms until one comes (D290)`
2. Every `perf:` line after the first minute shows the control window
   running, all or nearly all of it from the loop: about
   `control 120 redraws ... (worst ... ms, 120 unpainted)`. That is
   2 Hz x 60 s. It is about 300 while the laptop's desktop remote is
   connected. **`control 0 redraws` is a failure.**
3. The same `perf:` lines still show 60 fps with no new late frames. If
   they do not, the loop-driven swap costs the show frames (reopen D290).
4. The playlist advances past 1 of 14 and the output is not black.
5. With the laptop connected to the desktop remote, the viewer shows the
   host's control window and updates about 5 times a second, and a click
   (Next, say) takes effect on the projector.

`STROBE_LOG=debug` also logs every later takeover and every hand-back
("Windows paints it again"), if the pattern needs a closer look.

## The first JaJa PDX show (2026-10-15)

The four-hour show kit from the private `jaja-strobe-assets` repository
(`playlists/jaja-4h.toml`, `show/start-show.cmd`) ran on the two
mirrored projectors. The topology was Windows Duplicate at 1920x1080. It
ran on build 4460897, which is c879851 plus the playlist-compile backstop
(fca44a5), with `--audio synthetic` because there was no microphone. The
StrobeShow scheduled task started it at logon in session 1.

- **Timeline:** started 19:38. The output was black until the backstop
  hotfix (see the section above). Run 3 started 20:02:11, prepared 14 scenes
  in 4 s, ran the test card for 2 min, then played all 53 cues on schedule.
  It reached the final cue (`01-aurora-gradient`, `advance = "manual"`) at
  23:56:53, 3 h 54 m 41 s after the test card, and held there until the
  owner stopped it.
- **Stability:** no crash restarts in the run. Private bytes rose from 206
  to 217 MB over the first three hours and then stayed flat; the working
  set stayed at 25-38 MB. No new WARN or ERROR lines after startup.
- **Frame rate:** about 60 fps almost throughout. Exceptions:
  - `14-electric-bloom` with its `hue = 0.85` override ran at 52-54 fps,
    GPU 14-15 ms, with no late frames;
  - one crossfade between two heavy scenes (`15-neon-ribbon-flow` into
    `05-domain-warp`) gave 40 late frames in that minute.
- **Audio:** the synthetic 120 BPM beat looked jarring on the projectors.
  `--audio ambient` (the synthetic track without hits or beat triggers)
  was built during the show but not deployed. Use it, or a USB mic with
  `--audio auto`, next time. Stereo Mix is no use here: no default
  recording device is set, so `auto` falls back to synthetic.
- **Not used:** the laptop remote. Venue mode (D288, D294) landed on master
  after the show started.

## Black frames above about 160x90 (2026-10-15, D296)

`strobe-bench --context window --gl native [--show-window] --filter 05`:
correct at 100x60, 160x90, 161x90, 160x91; all black in about 0.2 ms at
200x90, 160x120, 240x135, 256x256, 300x300, 320x180, 480x270, 640x360 and
1920x1200. The corpus at 160x90 shows `neon_city` black on native only (1.7 ms
against 555.6 ms on ANGLE; its literal passes are far larger than the
window). The app's real 1920x1080 output window renders fine.

The code was audited and the offscreen path never touches the default
framebuffer, so the pixel-ownership explanation does not hold as stated; see
D296. The cause is not yet known (found later: M11-M14 below, fixed by
D311). Run this in session 1 (a one-shot scheduled
task as before) from a build of this commit, and send back the four output
folders and the logs.

```powershell
cd $env:USERPROFILE\strobe
$env:STROBE_LOG = "info,gl_driver=debug,gl_trace=info"
$env:STROBE_LOG_STDERR = "1"
$env:STROBE_GL_DEBUG = "1"
$env:STROBE_GL_TRACE = "1"
$b = "cargo run --release -p strobe-bench --"

# 1. The ladder, default 16x16 stand-in window. Finds the limit and says
#    whether clears, draws or the viewport go wrong.
cmd /c "$b --context window --gl native --probe --size 200x90 --out target\probe-small > target\probe-small.log 2>&1"

# 2. The same ladder with a window as large as the screen allows. Windows
#    caps a new window at about the screen's size; the "Context:" line in
#    probe.md gives the size it actually got.
cmd /c "$b --context window --gl native --probe --size 200x90 --window-size 2048x1200 --out target\probe-big > target\probe-big.log 2>&1"

# 3. The real failure with the trace, small and large window.
cmd /c "$b --context window --gl native --filter 05 --size 200x90 --out target\bench-small > target\bench-small.log 2>&1"
cmd /c "$b --context window --gl native --filter 05 --size 200x90 --window-size 2048x1200 --out target\bench-big > target\bench-big.log 2>&1"
```

Reading it:

- **`probe-small` fails and `probe-big` passes** (and `bench-big` is not
  black): the driver limits work to the window's drawable. The first bad row
  in `probe.md` is the limit; the "Viewport read back" column shows whether it
  is a clamped viewport (smaller than asked) or dropped draws (clear 100 %,
  draw 0 %).
- **Both fail at the same size:** a size limit that is not the window's. A
  pixel count near 16,384 shows as 128x128 passing and 129x128 failing with
  `1000x16` and `16x1000` passing; a per-axis limit shows as those two
  failing. The driver is dropping work for the target; read the `gl_driver`
  lines in the log.
- **Both pass:** the bench scene differs from the probe, so compare the
  `gl_trace` summaries (near the end of `bench-small.log`, before
  `wrote target\bench-small`): each pass's draw should be into a non-zero
  framebuffer at its own size, and any `glViewport(...) reads back as` warning
  names the clamp.
- **Everything passes at 200x90 but `bench-small` is black:** a shader or
  state problem specific to 05 on this driver, not size.

## Black frames root-caused: large draws need a small draw first (2026-10-15, M11-M14, D311)

The bisect on the throwaway `d296-experiment` branch (`strobe-bench --bisect`
and its `STROBE_D296_*` switches), run on this machine with native WGL
(driver 31.0.21925.1001). The branch is not merged; its result is D311.

- **M11:** clears land in every target. Draws into any target over 16384
  pixels (64 KiB of RGBA8) write nothing, with no GL error and no debug
  message. `STROBE_D296_SETTLE=read` (`glReadPixels` on each new target, no
  draw) and `SETTLE=finish` (`glFinish`) leave the bench fully black;
  `SETTLE=red` (clear to red) stays solid red. A read or a finish does not
  unwedge the driver; only a draw does.
- **M12:** every bisect stage lands (draw-time state, one shader change at a
  time, the player's vertex stage, the player's own target), but trivial
  scenes (solid green, `_uv`) through the bench loop are black.
- **M13:** a frame-count sweep at 640x360 from 1 to 150 frames
  (`--warmup 0 --frames 1` included) is black every time. At 128x128 it is
  fine. The frame count is not the trigger.
- **M14 (decisive):** with the player path run first in the process
  (`STROBE_D296_E_FIRST`), `01-d296-solid` at 640x360 wrote 0 %. The next
  draw, the same scene at 128x128, wrote 100 %, and so did 640x360 again and
  every later draw at any size, with new players and new targets. With
  `STROBE_D296_PRIME` (one 1-frame 128x128 scene before the bench loop),
  every 640x360 scene renders; without it, every one is black. So draws into
  targets over 64 KiB are lost until the context has drawn into a small
  target once.

The fix (D311): every GL context draws once into a 16x16 target when it is
created (`gl::prime`) and reads it back. At debug level the log says
`GL prime: one 16x16 draw landed in N us (D311)`; a prime that does not land
is a warning.

### Task M17 part 1: the fix verified (2026-10-16, master b8789bf)

Run as written for Task M15, in session 1 alongside the soak, with no
`STROBE_D296_*` variable set, over the default library (16 scenes).

- **Native 640x360:** no black frame, no warning.
  `GL prime: one 16x16 draw landed in 128667 us (D311)`.
- **Native 1920x1080:** no black frame. Prime landed in 98767 us. Means:
  01 2.76, 04 12.49, 05 11.72, 08 8.45, 09 47.25, 10 381.49, 12 80.31,
  13 10.11, 14 20.36, 15 15.86, 16 17.30 ms.
- **ANGLE 640x360:** no black frame, prime 47149 us, timings within a few
  per cent of native.
- **Native 640x360 with `STROBE_GL_NO_PRIME=1`:** scenes 01-07 black again,
  then 08 and every later scene render. 08's first passes are
  `$WIDTH/4 x $HEIGHT/4` (160x90 = 14400 pixels, under the threshold), so
  its first draw primes the context by accident: an independent
  confirmation of the M14 mechanism. Only 7 of 16 were black, so the
  bench's "every frame black" error did not trip.

The prime costs 47-129 ms once per context on this driver: the first draw a
context makes is slow, and the prime now pays it instead of the first
scene. That is once per run and once per context recovery.

## D290 verified on the projector (2026-10-15, Task M5)

The "Verification after the show" list above, run on the mini PC through the
scheduled task in session 1, with `--audio ambient`.

- **Takeover:** `control window: no paint from Windows for 1217 ms
  (takeover 1)` at 3.5 s, once.
- **Redraws:** about 119 driven redraws a minute, all unpainted (the
  expected 2 Hz x 60 s). Not `control 0 redraws`.
- **Playlist:** 14/14 prepared in 5.4 s, so it is no longer stuck at 1 of 14.
- **Frame rate:** 59.9 fps in the `perf:` lines.
- **Audio:** the ambient source was logged. (The log line said "synthetic
  source (--audio synthetic)" first; fixed since, so it names the source
  chosen.)
- **Close:** `WM_CLOSE` to the control window closes the app cleanly.
- **Not checked here:** the laptop's remote viewer on this run; M6 below
  covers venue mode.

## Venue mode on loopback (2026-10-15, Task M6, D288, D294)

Host and controller both ran on the mini PC, over 127.0.0.1.

- **Pairing:** `--remote --remote-trust FILE` on the host and `--control
  127.0.0.1:47812 --remote-trust FILE` on the controller paired with no
  approval.
- **Viewer:** it showed the host's UI.
- **Host kill:** after the host process was killed the controller showed
  "Reconnecting…" and re-paired by itself 13 s later, when the host was
  back.
- **Not tested:** Disconnect (and that it stops the retries).
- **Cost:** running both ends on one 2-core machine cost the host 210 late
  frames a minute. That is the price of the test setup, not a measurement of
  venue mode; the real test is two machines.
- **Seen on the controller, since fixed:** a pure controller logged `ERROR IO
  "library scenes: cannot read scenes"` and a WARN for no audio device, and
  the card said it retried "about every 15 s" when the wait is 1, 2, 4, 8,
  then 15 s.

## The D310 scene pass measured (2026-10-16, Task M17 part 2)

1920x1080, native WGL, `--show-window --window-size 1920x1080 --size
1920x1080 --filter <scene>`, alternating the scenes before D310 (6923fa5, in
a temporary worktree) and after (b8789bf). No black frames in any of the 12
runs. Means, with p95 in brackets:

| Scene | Before | After | fps before → after |
|---|---|---|---|
| 04-voronoi-glass | 21.00 (28.75) | 10.68 (12.57) | 47.6 → 93.6 |
| 05-domain-warp | 25.62 (33.10) | 9.96 (11.88) | 39.0 → 100.4 |
| 08-bloom-orbs | 17.50 (25.78) | 7.71 (13.22) | 57.1 → 129.6 |
| 13-chromatic-rise | 16.04 (18.23) | 8.51 (12.50) | 62.3 → 117.5 |
| 14-electric-bloom | 22.17 (30.28) | 18.74 (26.88) | 45.1 → 53.4 |
| 16-acid-mirror | 15.51 (24.11) | 14.92 (17.83) | 64.5 → 67.0 |

- **Looks:** 04 and 16 are pixel-identical. 05 and 08 differ by under half a
  level on average. 13 matches to the eye; its largest differences are
  isolated pixels flipping between adjacent posterise bands. 14 is
  indistinguishable.
- **Compile times** fell as well: 04 205 → 25 ms, 08 280 → 98, 13 428 →
  121, 14 729 → 63, 16 479 → 94.
- **Still over budget at 1080p:** 14 (18.7 ms) and 16 (14.9 ms, p95 17.8).
  D310's "Would reopen" names the next step for each.

## 42-45 fps on a 13.6 ms scene: the mean was under budget (2026-10-16, Task M16, D314)

The question: why do the heavy scenes show about 42-47 fps on the projector
when their GPU time is under the 16.7 ms frame? Scene `13-chromatic-rise`
was held for minutes with the hold-13 playlist
(`jaja-show\playlists\m16-hold13.toml`), fullscreen 1920x1080, vsync at
60 Hz, through the scheduled task in session 1. Numbers from the `perf:`
lines.

| Run | Build and setup | fps | GPU ms | Late |
|---|---|---|---|---|
| A | adaptive on, with `--remote` | 44.6 | 12.41 | 7.0% |
| B | adaptive on, without `--remote` | 44.0 | 12.64 | 8.8% |
| C | as B, control window minimised | 42.6 | 13.62 | 10.2% |
| D | `--no-adaptive --render-scale 1` | 42.2 | 13.6 | about 9% |
| E | show-night build 4460897 | 42.5-42.8 | | |
| F | master build, synthetic audio | 42.2-42.3 | | |

- **Adaptive resolution** went 1.00 -> 0.90 -> 0.95 once and then sat; in
  one run it never moved.
- **The same pattern** holds for scenes 04, 05, 08, 14 and 16: GPU 12.6 to
  13.8 ms, about 47 fps over a 33 h soak.
- **Not the CPU** (strobe about 5%) and not thermal throttling.
- **Not `--remote`** (A against B), **not the D290 control frames** (E logged
  0 to 1 control redraws a minute and is no faster; C, minimised, still logged
  about 116 loop-driven redraws a minute), and **not a regression** (E, the
  show-night build, is the same as F, master).

**Conclusion.** Scene 13 costs about 13.6 ms of GPU at full size. That is
under the 16.7 ms budget on average, so the adaptive controller, which
watched only the mean, had nothing to lower. But enough frames (GPU spikes,
plus the present and compositor overhead the timer does not see) straddle
the vsync boundary that the presented rate is a mix of 16.7 and 33.3 ms
intervals, 42-45 fps.

**What changed (D314).** The controller now also counts presented frames
that miss the refresh, and steps the scale down (1.00 -> 0.90 -> 0.80) when
about 4% or more of the frames miss for four seconds while the GPU is busy
enough to be the cause. It does not climb back into the judder unless the
scene gets clearly cheaper. The per-minute perf line now also gives the
scale. A minimised control window now draws nothing: run C's 116 redraws a
minute were the undecorated window keeping a client area when minimised,
which the old 0x0 check missed.

## The missed-refresh rule on the projector (2026-10-16, Task M18, D314 -> D315)

Five runs through the scheduled task in session 1, fullscreen on the
projector, 390 s each; numbers from the `perf:` lines of minutes 2 to 6. The
33 h soak was paused for them, from 13:55:38 to 14:29:13. "OLD 13" is
`13-chromatic-rise` as it was before D310 (13.6 ms at scale 1.0), held by
`jaja-show\playlists\m16-hold13.toml`.

| Run | Build and setup | fps | Late | Scale |
|---|---|---|---|---|
| 1 | before (6ca5248), OLD 13 | 42.5 | 11.9% | 1.00 |
| 2 | after (D314), OLD 13 | 53.2 | 4.2% | hunted 0.65-0.80 |
| 3 | after, OLD 13, control window minimised | 51.5 | 1.2% | 1.00 -> 0.90 (7.7 s) -> 0.80 (129 s), then steady |
| 4 | `--no-adaptive --render-scale 1`, OLD 13 | 42.3 | 8.2% | 1.00 |
| 5 | after, the new scenes, holding `16-acid-mirror` | 59.4 | 0.2% | 1.00 -> 0.90 (10 s) -> 0.80 (42 s), then steady |

**Run 2's adaptive lines:**
- 8.4 s 1.00 -> 0.90: 25/240 missed, GPU 13.6 ms (D314)
- 14.1 s 0.90 -> 0.80: 11/240, GPU 11.3 (D314)
- 242.0 s 0.80 -> 0.70: 21/480, GPU 11.6 (D314)
- 244.7 s 0.70 -> 0.75: mean rule, GPU 7.2 ms against 16.7
- 257.1 s 0.75 -> 0.65: 28/600, GPU 10.9 (D314)
- 259.7 s 0.65 -> 0.70: mean rule, GPU 6.4
- 262.3 s 0.70 -> 0.75: mean rule, GPU 7.3

**What it showed:**
- **D314 works where the GPU is the cause.** Run 5 is clean: two steps and
  59.4 fps. Run 2's first step took 13 from 42.5 to about 52 fps.
- **Below that the misses are not the GPU's.** At 0.80, 13 takes 9.1 ms of
  GPU and still runs about 52 fps with about 4.3% late. Further steps did not
  stop the misses, and the mean rule climbed straight back each time (7.2 ms
  predicted at 0.75 is under 0.85 x 11.6 ms, so D314's climb gate let it).
  That is the hunting.
- **The control window costs refreshes once the GPU is not the bottleneck.**
  Run 2, covered and driven by the loop (about 117 control frames a minute,
  D290), missed 4.2%; run 3, minimised, 1.2%. Run 3 logged `control 0
  redraws`, which confirms the D314 minimised fix.
- **The intervals are not quantised.** 52 fps on a 60 Hz display with only
  1.2% of intervals over 25 ms means many sit around 19 ms: the timestamps are
  taken on the CPU at the start of each output frame, after the event loop's
  own work, not at vertical blank.

**What changed (D315):**
- A late drop is judged on the misses after it: unless they halve, or fall
  under the drop line, it is undone and the late rule lowers no further on
  that scene until the scene changes.
- After a late drop, the scale climbs back only after 30 s with (almost) no
  missed refreshes, as well as under D314's GPU prediction.
- Missed refreshes are counted from each window's span (refreshes that
  passed minus frames shown), which is exact whatever the jitter, instead of
  counting intervals over 1.5 periods. A stall counts as two misses at most.
- A control window that is covered by the fullscreen output and starved of
  paints runs only its non-drawing work in its loop-driven frames, with a
  full draw every 5 s (and at once for a key pressed in it), as a minimised
  one does. That is about 12 draws a minute instead of about 117.

## D315 on the projector: the verdict was too strict (2026-10-16, Task M19, D315 -> D316)

The five M18 runs again, on the D315 build (d76e320), 390 s each, numbers
from the `perf:` lines of minutes 2 to 6. The soak was paused for them from
15:03:16 to 15:36:59.

| Run | Build and setup | fps | Late | Control redraws | Scale |
|---|---|---|---|---|---|
| 1 | 12ef170 (D314), OLD 13 | 52.5 | 4.4% | | hunting, as M18 run 2 |
| 2 | d76e320 (D315), OLD 13, control covered | 42.6 | 8.0% | 11-12 a minute | locked at 1.00 |
| 3 | d76e320, OLD 13, control minimised | 42.7 | 9.2% | 0 | locked at 1.00 |
| 4 | `--no-adaptive --render-scale 1`, OLD 13 | 42.2 | | | 1.00 |
| 5 | d76e320, the new scenes, holding `16-acid-mirror` | 49.2 | 0.6% | | locked at 1.00 |

"Late" is the perf line's count of intervals over 1.5 periods; the adaptive
lines count refreshes missed from the window's span (D315), so their shares
are much larger.

**The adaptive lines:**
- **Run 2:** 7.8 s, 1.00 -> 0.90, "97 refreshes missed over the last 240
  frames ... GPU 13.8 ms (D314)". 13.4 s: "left the display's refreshes missed
  (40.4% before, 25.8% after) ... back to 1.00 ... until the scene changes
  (D315)".
- **Run 3:** the same lock, 40.4% to 24.6%.
- **Run 5:** 7.0 s, 1.00 -> 0.90, "55 refreshes missed over the last 240 ...
  GPU 12.0". 12.2 s: "left the display's refreshes missed (22.9% before,
  14.2% after) ... back to 1.00".

**What it showed:**
- **The covered-window part of D315 works.** Run 2's control window drew
  11-12 times a minute, as intended, and covered and minimised now miss
  alike (8.0% against 9.2%).
- **The verdict was a regression.** A 0.10 step that pays cuts the misses by
  35-40% here (40 to 26%, 23 to 14%), and it takes two steps to clear the
  vsync edge. D315 asked for a halving, so it undid the first step and
  locked at 1.00. Scene 16 fell from M18's 59.4 fps at 0.80 to 49.2 fps, and
  the old 13 from about 52 fps at 0.80 to 42.6.
- **Two counts, two meanings.** The perf line's "late" (1-9%) and the
  adaptive lines' "refreshes missed" (25-40%) measured different things,
  which confused the reading.

**What changed (D316):**
- A late step pays when it cuts the missed share by at least a fifth and by
  at least 5 points; the next step is then allowed. Only a step that cuts
  less is undone and locked.
- On the M18/M19 shares (old 13: 40% at 1.0, 25% at 0.90, 15% at 0.80, about
  10.5% at 0.70), that is two steps to 0.80, and a try at 0.70 undone (4.5
  points). On 16 (23%, 14%, about 0) it is two steps to 0.80, both kept.
- The perf line now leads with the same count the controller uses:
  `missed N refreshes (X% of frames)`, then the old `N late of M frames`
  unchanged. The adaptive lines say "missed N refreshes over the last M frames
  (X% of frames)", and an undo says "cut too few missed refreshes (x% of
  frames before, y% after)".

## D316 on the projector: passed, with a slow third step (2026-10-16, Task M20, D316 -> D317)

The five runs again on the D316 build (ccc06de), 390 s each, numbers from the
`perf:` lines of minutes 2 to 6. The soak was paused from 16:06:44 to
16:40:12.

| Run | Build and setup | fps | Missed | Late | GPU ms | Scale | Control |
|---|---|---|---|---|---|---|---|
| 1 | M19 build, OLD 13 | 42.5 | | | | locked at 1.00 | |
| 2 | ccc06de, OLD 13, control covered: three minutes | 52.6 | 14.1% | 1.6% | 9.12 | 0.80 | 11-12 a minute |
| 2 | the same run, after 241 s | 58.5 | 2.6% | 1.7% | 7.29 | 0.70 | 11-12 a minute |
| 3 | ccc06de, OLD 13, control minimised | 58.4 | | | | 0.70 (from 180.5 s) | 0 |
| 4 | `--no-adaptive --render-scale 1`, OLD 13 | 42.6 | 40.7% | | | 1.00 | |
| 5 | ccc06de, the new scenes, holding `16-acid-mirror` | 59.8 | 0.1-0.5% | | | 0.80 | |

**The adaptive lines:**
- **Run 1:** D315's lock, 41.7% -> 25.8%, back to 1.00.
- **Run 2:**
  - 8.0 s, 1.00 -> 0.90: "missed 97 refreshes over the last 240 frames
    (40.4% ...) GPU 13.6";
  - 13.7 s, 0.90 -> 0.80: "missed 64 ... 240 frames (26.7%) GPU 11.2";
  - 241.3 s, 0.80 -> 0.70: "missed 1667 refreshes over the last 11880 frames
    (14.0%) GPU 10.6". It paid (14% -> 2.7%), with no undo and no climb
    back.
- **Run 3:** steps at 7.6 s, 13.3 s and 180.5 s ("1222/8760 frames, 13.9%").
- **Run 5:** 1.00 -> 0.90 at 7.1 s (24.6%), 0.90 -> 0.80 at 12.2 s (14.2%),
  then steady.

**What it showed:**
- **D314-D316 work on the hardware.** Every step that cut the misses was
  kept, nothing hunted, and nothing climbed back. The old 13 went from 42.5
  to 58.5 fps and 16 held 59.8 fps.
- **The missed share explains the frame rate.** Run 4's 40.7% missed is its
  42.6 fps exactly, and the perf line and the adaptive lines now agree.
- **The covered control window** drew 11-12 times a minute, and the minimised
  one not at all.
- **The third step took three to four minutes.** The scene sat at about
  52 fps with 14% missed. The step was gated by the load line, not the
  windows: at 0.80, 13 takes 9.1 ms, under the 10 ms line (0.6 of the
  cadence). The late rule waited until a loud passage lifted the GPU average
  to 10.6 ms. Meanwhile the run of missing windows grew to 99 (11880 frames)
  and was reported whole as the trigger, which is why the log showed 11880
  frames instead of 240.

**What changed (D317):**
- Once a late step on a scene has paid, the load line no longer gates the
  next: the verdict has already shown that the misses follow the scale. The
  next step is judged like every other (undone and locked if it cuts too
  little), so the guarantee against hunting is unchanged.
- The trigger reported, and judged against, is the last two windows, not the
  whole run.

## D317 on the projector: the fast third step, and a false lock (2026-10-17, Task M21, D317 -> D318)

Two hold-13 runs on the D317 build (37880a8), with the control window
covered. The soak was paused from 17:06:47 to 17:11:35.

**Run A, launched about 10 s after the soak's strobe exited:**
- 8.2 s, 1.00 -> 0.90: "missed 102 ... 240 frames (42.5%) GPU 14.9 ms", where
  13 normally takes 13.6 ms.
- 14.5 s: "cut too few missed refreshes (42.5% before, 42.1% after) ... back
  to 1.00 ... until the scene changes (D315, D316)".
- First perf minute: 39.8 fps, 50.4% missed, 1137 late, worst gap 142 ms,
  gpu 14.52. Second minute: 42.6 fps at 1.00, and it stayed locked.
- A transient load disturbed the verdict's windows, most likely DWM or the
  driver tearing down the previous process's fullscreen context. The lock
  that followed lasts as long as the scene is held, which at a show can be
  the whole night, at a cost of about 16 fps.

**Run B, two minutes later, same exe:**

| Time | Step | Missed before | GPU ms |
|---|---|---|---|
| 7.9 s | 1.00 -> 0.90 | 40.8% | 13.6 |
| 13.5 s | 0.90 -> 0.80 | 26.2% | 11.3 |
| 18.7 s | 0.80 -> 0.70 | 14.6% | 9.1 |
| 27.6 s | 0.70 -> 0.60 | 8.3% | 7.2 |

- The third step came at 18.7 s instead of after 3-4 minutes: D317 works.
- A fourth step followed, because 0.70's windows showed 8.3% here against
  2.7% in M20. It paid.
- Minute 1: 55.6 fps. Minute 2: 59.9 fps, 0.1% missed, at 0.60.

**What changed (D318):**
- The late lock expires: 60 s, then 120 s, then 240 s on each repeated
  failure, and a paid step resets it. A false lock heals within a minute.
  A scene whose misses really ignore its scale retries rarely: about 8 s in
  240 at the cap.
- A verdict whose judged windows were disturbed is put off for fresh
  windows, up to four times (16 s), instead of locking. Disturbed means a
  frame over 6 cadences (100 ms at 60 Hz), or the GPU time higher after a
  step down than before it. Run A had both.
- Run B's fourth step stands. It paid (8.3% -> 0.1%) and gave a steady
  59.9 fps; the drop line, not a lower "good enough" level, is where the
  stepping stops (see D318).

## D318 on the projector, and a competitor on the GPU (2026-10-17, Tasks M22 and M23, D318 -> D319)

### M22: launches right after another strobe exits

Two hold-13 runs on the D318 build (76104e9), the control window covered,
one launched 1.4 s after another strobe exited. The soak was paused from
about 17:42 to 17:48:13.

- **Both runs** reached 0.70 by 18.5 s, then ran 58.4-58.6 fps with 2.3-2.8%
  missed. There was no lock and no fourth step.
- **The M21 disturbance did not recur,** so D318's deferral and expiring
  lock were not exercised. Task M24 provokes them.

### M23: a second program loading the GPU

The same hold-13 runs, with `strobe-bench` rendering 10-volume-clouds
fullscreen at 1080p on the same GPU for part of each run. The soak was
paused from 17:49:56 to 17:57:20.

| Run | Load | During the load | Steps | End |
|---|---|---|---|---|
| A | +6.4 s to +21.1 s | no step; worst gap 2.6 s, about 17 fps | 21.8 s (53.8%, GPU 13.6), 27.5 s, 32.7 s | 0.70, 58.6 fps |
| B | +6.1 s to +46.1 s | no step | 46.4 s (68.3%, GPU 15.4, windows mostly within the load), 52.0 s, 57.2 s; then 185.1 s, 0.70 -> 0.60 on "13 refreshes over the last 240 frames (5.4%)" | 0.60, 59.7 fps |

**What it showed:**
- **Under a competitor's stalls the controller waits, then steps normally
  once the load ends.**
  - The windows are counted in presented frames, so under 2.6 s stalls a
    window simply takes longer.
  - Each interval counts as 2 missed refreshes at most (`STALL_CADENCES`), so
    the stalls do not fill a window by themselves.
  - Each step is still judged on the windows after it.
- **Run B crept down on noise.** It held 0.70 for about two minutes at
  2.6-2.7% missed a minute, then took a fourth step because two 120-frame
  windows reached the drop line by chance. At 2.7%, a window holds 5 or
  more misses about one time in four, so two in a row happen about one time
  in twenty.

**What changed (D319):**
- **Settled scales need stronger evidence.** Once a late step has paid, or
  after 30 s at one scale, the scale is settled. The next step then needs
  the recent windows (up to 10 s) to average 1.5 times the drop line
  (6.25% missed).
  - At 2.7%, noise does not get there.
  - A real change (14% or more) still steps within about 4 s.
- **The stall behaviour is unchanged,** and now documented (D319).

## A disturbance inside a verdict, and a long hold (2026-10-17, Task M24, D319 -> D320)

Hold-13 on the D319 build (40ad1b1), the old scenes, control window covered.
The soak was paused from 18:26:53 to 18:44:55.

| Run | Load | Steps | Result |
|---|---|---|---|
| A | bench (640x360, scene 10) +9.4 s to +13.1 s | 8.0 s 1.00 -> 0.90 (40.8%); 21.3 s 0.90 -> 0.80, "over the last 480 frames (36.7%)"; 26.4 s 0.80 -> 0.70 (13.8%) | 58.5 fps at 0.70 from minute 2; worst gap in minute 1, 201 ms |
| B | the same, +9.0 s to +40.0 s | 7.7 s 1.00 -> 0.90; 48.5 s 0.90 -> 0.80, "over the last 600 frames (83.2%)"; 53.7 s 0.80 -> 0.70 | 58.1-58.4 fps at 0.70; no lock |
| C | none, 600 s | 7.7 s, 13.4 s, 18.5 s to 0.70; 244.7 s 0.70 -> 0.60, "missed 58 refreshes over the last 600 frames (9.7%) ... GPU 9.1 ms"; back up 283.6 s (0.65) and 314.1 s (0.70) | held 0.70 at about 58.5 fps to 600 s |

**What it showed:**
- **D318's deferral works.** In run A the first verdict was put off through
  the bench and then paid. In run B it was put off through the whole 31 s
  load, then paid. Neither run locked, so the lock and its expiry still
  have not been seen on the hardware.
- **Why run B's deferral outlasted "4 x 4 s".** The deferral budget is
  counted in windows, not seconds: four deferrals of two 120-frame windows,
  960 presented frames.
  - Under the bench the output presented far fewer frames, so those windows
    took most of the 31 s.
  - Windows during a load are counted, not thrown away. Each stalled
    interval counts as 2 missed refreshes at most.
  - The verdict was taken on the first undisturbed windows once the load
    ended.
- **Run C stepped down for a Windows stall.** The 244.7 s trigger's windows
  held a 915 ms system stall: a Volume Shadow Copy / System Restore snapshot,
  logged as Application/VSS event 8231 in the same second. A settled scale
  stepped on it, then climbed back about 70 s later.

**What changed (D320):**
- A window holding a stall (a frame over 6 cadences), and the one after it,
  no longer count towards a settled scale's evidence. A recurring stall then
  never steps a settled scene down, which is right: stalls do not follow the
  render scale. A scene's first, unsettled half minute keeps the two-window
  rule.
- Every put-off verdict logs a line, for example
  `adaptive resolution: verdict on 1.00 -> 0.90 put off (1 of 4): window
  disturbed (worst gap 201 ms, GPU +12%) (D318)`. A lock running out logs
  `adaptive resolution: the lock at 0.80 ran out; missed refreshes may lower
  the scale again (D318)`. The lock itself was already logged, with its
  length.

**Windows snapshots on this machine.** VSS / System Restore snapshots here
cluster around 18:30-19:10 local each day, about three a day. That is show
hours. Each one can stall every program for close to a second. Since D320
the controller ignores them on a settled scale, but the output still shows
the stall. The fix is on the machine: move or turn off the scheduled
restore points before a show. That belongs in the venue runbook, which is
kept with the show kit in the private `jaja-strobe-assets` repository, not
in this one.

### Task M25: the deferral lines on the projector (2026-10-17, cdfeb5c)

One 600 s hold-13 on the old scenes, the control window covered, with the
640x360 scene-10 bench from +9.0 s to +13.0 s. Soak paused 19:11:36 to
19:22:35.

| Time | Line |
|---|---|
| 7.7 s | 1.00 -> 0.90, 40.8% missed, GPU 13.6 ms |
| 16.2 s | verdict on 1.00 -> 0.90 put off (1 of 4): worst gap 159 ms, GPU -16% |
| 21.2 s | 0.90 -> 0.80, 25.4% missed |
| 26.3 s | 0.80 -> 0.70, 14.2% missed |
| 257.8 s | 0.70 -> 0.60, 9.2% missed over 600 frames, GPU 8.9 ms |
| 378.2 s | 0.60 -> 0.65 (mean rule, GPU 6.0 ms) |
| 408.7 s | 0.65 -> 0.70 (GPU 6.4 ms) |

Per-minute perf from minute 2: 58.1-59.7 fps, 0.4-3.3% missed, worst gaps
28-35 ms, control 11-12 redraws. No VSS event in the run.

- **The put-off line works** and the steps reach 0.70 by 26 s.
- **The step at 257.8 s is content, not noise or a stall:** no gap over
  35 ms in that minute, GPU 8.9 ms against 7.27 at steady state. Old 13 has
  a heavier passage about 245-260 s into a hold; M24 run C's step (244.7 s)
  was the same passage, with the snapshot landing on it. The scale dips to
  0.60 for about 2.5 minutes and returns, at 58-59 fps throughout. This is
  the controller adapting to the scene, as intended.
- **Not a show concern:** the shipped 13 (D310) costs 8.5 ms at full size
  on this machine and needs no scaling.

## The show build's first day: the heavy scenes fixed, and a lock that ran out (2026-10-17, Task M26, 51f2620)

The M9 soak moved from 6ca5248 to master 51f2620 (D310's cheaper scenes,
D311 to D320) at 23:33:06 on 2026-10-16. The playlist resumed at cue 3 of
loop pass 1, from the old soak's resume point; the test card was not
replayed. The first daily report came at 02:57, 3.4 h in.

- **Health:** no reboot, no restart, no WARN or ERROR besides the startup
  robustness line. Private memory was 214 MB at +10 min and 223 MB at 3.4 h,
  with 477 handles both times. The session log was 114 KB, about 0.8 MB a
  day. resume.toml was written 1 s before the check.
- **First 10 minutes:** no prime WARN, so the D311 prime landed. The perf
  lines carry the missed-refresh and scale fields.

Per scene, over 203 perf windows (157 inside one cue):

| Scene | Windows | fps avg (min) | GPU ms | Missed % | Scale | 6ca5248 fps |
|---|---|---|---|---|---|---|
| 01-aurora-gradient | 9 | 59.9 (59.9) | 1.87 | 0.1 | 1.00 | |
| 02-plasma-pulse | 13 | 59.9 (59.9) | 2.26 | 0.1 | 1.00 | |
| 03-kaleido-tunnel | 10 | 59.9 (59.9) | 2.78 | 0.1 | 1.00 | |
| 04-voronoi-glass | 8 | 59.9 (59.9) | 8.21 | 0.1 | 1.00 | 48.3 |
| 05-domain-warp | 12 | 59.9 (59.9) | 7.60 | 0.1 | 1.00 | 47.8 |
| 06-feedback-trails | 16 | 59.9 (59.6) | 2.52 | 0.1 | 1.00 | |
| 07-reaction-diffusion | 13 | 59.9 (59.9) | 4.91 | 0.1 | 1.00 | |
| 08-bloom-orbs | 16 | 59.9 (59.9) | 5.00 | 0.1 | 1.00 | 47.7 |
| 11-script-orbits | 11 | 59.9 (59.9) | 3.57 | 0.1 | 1.00 | |
| 13-chromatic-rise | 12 | 59.8 (59.4) | 6.48 | 0.4 | 1.00 | 42.9 |
| 14-electric-bloom | 17 | 59.5 (59.0) | 8.81 | 0.8 | 0.70-0.80 | 46.7 |
| 15-neon-ribbon-flow | 10 | 58.6 (54.7) | 9.31 | 2.4 | 0.90 | 52.7 |
| 16-acid-mirror | 10 | 59.8 (59.3) | 8.22 | 0.4 | 0.80 | 47.0 |

Overall: 59.5 fps and 0.68% of refreshes missed, against 53.5 fps and 2.5%
late on day 1 of 6ca5248. `15-neon-ribbon-flow` is now the weakest scene.

**The D318 lock and its expiry, on hardware for the first time** (2 of 99
adaptive lines; the scene held at 0.90 was 15-neon-ribbon-flow):

| Time | Line |
|---|---|
| 9797.7 s | lowering 0.90 -> 0.80 cut too few missed refreshes (11.2% before, 6.7% after), so back to 0.90, locked for 3600 frames or until the scene changes (D315, D316, D318) |
| 9863.7 s | the lock at 0.90 ran out; missed refreshes may lower the scale again (D318) |

The step cut 40% of the misses but only 4.5 points, under D316's 5-point
floor, so it was undone. The lock ran out 66 s later, as designed.

Next: rebuild on master 3c1bde1 (D400-D438) and restart, with
strobe-6ca5248.exe kept as the rollback.

## The show build's first full day on 3c1bde1 (2026-10-18, Task M26)

The soak switched to master 3c1bde1 (D400-D438) at 03:12:53 on 2026-10-17
and resumed cue 51 at 160.6 s in. Report at 02:59 the next day, 23.8 h in:

- **Health:** no reboot, no restart, no WARN or ERROR besides the startup
  robustness line. No "repeated N times" line (D422). The session log was
  730 KB, about 0.73 MB a day. resume.toml was written every 5.0 s (D438's
  sync changes nothing visible).
- **Memory:** private 245 MB, working set 64 MB, 478 handles. The old
  builds sat at 200-217 MB over several days; 51f2620 went from 214 to
  223 MB in 3.4 h. With no hourly samples, creep and a plateau cannot be
  told apart yet; hourly sampling is running.
- **Playlist:** 315 cue starts, 5 complete loops.
- **Frames:** 59.6 fps and 0.56% of refreshes missed over 1426 perf
  windows. The worst gap all day was 56 ms. The scenes that still scale
  settled at 14-electric-bloom 0.70-0.75, 15-neon-ribbon-flow 0.85-0.90
  (59.4 fps, up from 58.6) and 16-acid-mirror 0.80.
- **One lock and its expiry, on 05-domain-warp:** at 35778.6 s, 1.00 ->
  0.90 for 12.0% missed; at 35783.9 s it was undone (14.6% after) and
  locked; at 35847.7 s the lock ran out. The worst minute of the day
  (54.5 fps) was inside it. The minute after was 59.9 fps at 1.00. The GPU
  rose from 7.6 to 10.3 ms with no stall, so the misses were not this
  scene's resolution. D315 and D318 did what they are for.

### Scene 13's memory, sampled per minute (2026-10-18, Task M26)

Hourly samples on 3c1bde1 read 245, 251 and 288 MB while
13-chromatic-rise played, against 213-229 MB on every other scene. A
per-minute sampler then followed all four 13 cues of loop pass 11:

| Cue | Plateau (MB) | After the cue (+35-39 s) |
|---|---|---|
| 7 | 245.6 (ramps from 220 over about 2 min) | 229.5 |
| 21 | 245.5 | 229.3 |
| 31 | 245.5 (277.0 for one second at +1 s, the crossfade in) | 229.4 |
| 47 | 245.4 | 213.3 |

Handles stayed at 478 and threads at 14 throughout.

- **No leak, and no growth from pass to pass.** 13 costs a steady +16 MB
  over the scene before it, released when it is parked. Its field pass is
  full-size, float and double-buffered, the only one like it in the show.
  The per-cue hue overrides make no difference.
- **Short spikes do not add up.** The 277 MB reading is the 3 s crossfade,
  when both scenes render. The earlier 288 MB reading (cue 47, pass 10, about
  212 s in) did not recur at the same point in pass 11.

### Day 2: windows over the output cost 5-11% of refreshes (2026-10-19, Task M26)

The day-2 report on 3c1bde1 (47.8 h, no restart, 11 complete loops, only
the startup robustness WARN) showed missed refreshes on the heavy scenes
rising in two steps:

| Hours of the run | 04, 05, 13 missed |
|---|---|
| 0-22 | 0.1-0.2% |
| 24 on (the hourly memory sampler started) | about 5% |
| 36 on (the per-minute scene-13 sampler started) | about 11% |

The GPU time did not change (about 7.4 ms). The cause was the test harness,
not Strobe:
- each sampler launch opened a visible console window, hosted by Windows
  Terminal, on top of the fullscreen output;
- at 03:06 the projector showed five console windows, the taskbar and the
  firewall dialog over the output, which was no longer the foreground
  window;
- Windows then composites the output through DWM instead of presenting it
  directly.

Over the day the adaptive controller stepped 14, 15 and 16 down to
0.60-0.80 (from 0.75-0.90). It also undid about 30 steps that cut too few
misses, each followed by a lock, against 1 on day 1. Every lock expired in
60-70 s, and two repeats backed off to 7200 frames. That is D315 and D318
refusing to chase misses that resolution does not cause.

The harness now launches hidden. The output was made the foreground again
at 03:09. A re-measure is pending.

Memory over the day (24 hourly samples): flat at +0.001 MB/h once the
scene-13 samples are left out. Scene 13 plateaus at 245.4-245.6 MB. Nothing
grows at 47.8 h.

**For a show:** nothing may sit over the output. The unanswered firewall
dialog ("allow public and private networks... Strobe") is now drawn on top
of the projection, and needs someone at the screen (RUNBOOK, "Once, ever").

## Still to do here

- Run a two-machine venue-mode test with the laptop (loopback is M6), and
  Disconnect.
- Move or turn off the scheduled System Restore / VSS snapshots before a
  show (about three a day, 18:30-19:10 local; M24).

- Native strobe-bench and control generation in session 1: does a hidden
  window on AMD WGL draw at all (M3b)?
- The JaJa test card on the projector at native 1920x1200, with
  `--output-monitor 1 --fullscreen --render-scale 1 --no-adaptive`. The
  flags landed after the run above.
