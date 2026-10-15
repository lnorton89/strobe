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
D296. The cause is not yet known. Run this in session 1 (a one-shot scheduled
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

## Re-measuring the D310 scene pass (not run yet)

D310 makes 04, 05, 08, 13, 14 and 16 cheaper. Its numbers are llvmpipe
ratios ([BENCHMARKS.md](../BENCHMARKS.md)), not Vega 3 measurements. To
measure them here, build the branch that carries D310 and time each scene
before and after, alternating, at 1920x1080:

```powershell
cd $env:USERPROFILE\strobe
git fetch origin
git worktree add ..\strobe-before origin/master   # the scenes before D310
git switch <the D310 branch>
cargo build --release -p strobe-bench
$b = ".\target\release\strobe-bench.exe"
foreach ($s in "04-", "05-", "08-", "13-", "14-", "16-") {
  & $b --context window --gl native --show-window --window-size 1920x1080 --size 1920x1080 --library ..\strobe-before\scenes --filter $s --out "target\vega-before\$s"
  & $b --context window --gl native --show-window --window-size 1920x1080 --size 1920x1080 --library scenes --filter $s --out "target\vega-after\$s"
}
```

Open the PNGs before trusting a time. Native WGL drew black above about
160x90 here (D296, section above), in about 0.2 ms. If the frames are
black, the times mean nothing. Run the same loop with `--gl angle` and
`target\vega-angle-before` / `target\vega-angle-after` instead; ANGLE
matched the app in session 1. The figures D310 started from are the app's
GPU times at 1920x1080, so playing each scene in the app and reading its
GPU ms is the other check.

## Still to do here

- Run a two-machine venue-mode test with the laptop (loopback is M6), and
  Disconnect.
- Re-measure the D310 scenes (section above). `14-electric-bloom` is
  expected to be the one still above 10 ms.

- Native strobe-bench and control generation in session 1: does a hidden
  window on AMD WGL draw at all (M3b)?
- The JaJa test card on the projector at native 1920x1200, with
  `--output-monitor 1 --fullscreen --render-scale 1 --no-adaptive`. The
  flags landed after the run above.
