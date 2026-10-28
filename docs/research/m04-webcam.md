# M-04: webcam input behind `syn_Media` (spike and proposal)

Backlog [M-04](../IMPLEMENTATION_BACKLOG.md#m-04-video-and-webcam-input)
asks for a proposal covering binary size, supported formats, device removal
and reconnect, latency, GPU upload cost and a license review before any
webcam or video code lands. This page is that proposal for **webcam** input.
Video files (`syn_MediaType` 2) need a decoder and are not covered; see
[Not covered](#not-covered).

The numbers come from a throwaway spike on branch `spike/m04-webcam`,
measured on the AMD desktop ([hardware/AMD.md](../hardware/AMD.md):
Ryzen 5 5600X, RX 580, native GL 3.3 and ANGLE on D3D11), 2026-10-28.
**That machine has no camera.** Capture and upload are measured with
synthetic pictures through the same code a camera would feed. The
camera-only questions (real latency, unplug and replug, the formats real
cameras offer, the T480's cost) are listed in the
[checklist](#checklist-for-a-machine-with-a-camera).

## How Synesthesia does it

From Synesthesia's own documentation (its Video & Media FAQ and the
manual's Media controls; not yet checked against a running copy):

- A webcam is a **live source in the Media panel**, listed with the media
  files. On Windows, "Video Inputs" lists anything DirectShow sees: USB
  cameras and HDMI/SDI capture cards. Spout and NDI inputs sit in the same
  list, one of each active at a time.
- It reaches a scene **two ways**:
  1. Scenes that read `syn_Media` use it in their own maths, with
     `syn_MediaType` 3 for a webcam (Q22).
  2. The **Media Overlay** meta controls (a mode, such as Add, and a
     strength knob) blend the media over *any* scene's output. Per R16 the
     overlay runs near the end of the meta stage, after the clamp and before
     Master. This is how a camera shows on a scene that never samples media.
- The media group also has Fit or Fill, Media Scale, flips, its own
  colour correction, playback controls, and a Lock that keeps the media
  across scene changes.

Strobe has path 1 for stills and image sequences (D186, D217, D223,
D225). Path 2 is part of the media group that C-01 lists as missing. **None
of Strobe's own scenes in `scenes/` read `syn_Media`**, and only a few of
Synesthesia's do (Q22), so a webcam with only path 1 shows almost nowhere.
The overlay is what makes a camera useful.

## What the spike built

- `strobe_gfx::live`: a one-picture **mailbox** between a capture thread
  and the render thread. The newest picture wins, and an older one not yet
  taken is dropped and counted. Buffers go back and forth, so neither side
  allocates once both have one. Also integer BT.601 YUY2 and NV12 to RGBA
  converters, which flip the picture upright while they convert.
- `Media` gains a live variant. `Media::poll`, already called once per
  frame, takes at most one picture and writes it with the existing
  `Gl::upload` (`glTexSubImage2D`) into **one reused texture**. A new size
  makes a new texture. While it shows, `syn_MediaType` is **3**.
  `ScenePlayer::set_live` installs it. No new GL leaf was needed, apart from
  a `flush` used by the bench.
- `strobe_app::webcam`: a capture thread on nokhwa 0.10.11, Media
  Foundation backend, `default-features = false` (see
  [Dependency](#dependency-and-license)). It asks for 1280x720 YUY2 at
  30 fps or the closest offered. It converts YUY2 and NV12 with the
  converters above and MJPEG with `image`. When a read fails it says why
  through the mailbox, lists cameras once a second, and reopens the one
  with the same name.
- `strobe --webcam <N|list>` and `strobe-bench --live`.

## Measurements

### Capture thread: one picture to RGBA (Ryzen 5 5600X)

| Camera | YUY2 | NV12 | MJPEG (`image`) | MJPEG size |
|---|---:|---:|---:|---:|
| 640x480 | 0.57 ms | 0.57 ms | 1.8 ms | 46 KiB |
| 1280x720 | 1.75 ms | 1.75 ms | 5.6-6.3 ms | 136 KiB |
| 1920x1080 | 3.9 ms | 3.9 ms | 12.8-13.8 ms | 305 KiB |

This runs on the capture thread, never the render thread. At 30 fps,
720p YUY2 uses about 5 % of one core here. MJPEG costs three times as
much, and a 1080p MJPEG camera about 40 % of a core. The T480's
i7-8550U is slower; measure it there.

### Render thread: the upload

The cost of a frame of a scene that shows only `syn_Media`, with a new
1280x720-output picture every frame:

| Camera | Whole cost, `glFinish` each frame (native / ANGLE) | Upload call, paced at 60 fps, no `glFinish` (native / ANGLE) |
|---|---:|---:|
| 640x480 | +0.25-0.36 / +0.21 ms | 0.10-0.13 / 0.10 ms |
| 1280x720 | +0.94-1.08 / +0.90 ms | 0.18-0.22 / 0.20 ms |
| 1920x1080 | +2.4-2.7 / +2.5 ms | 0.42-0.49 / 0.52 ms |

- The **paced** column is the one a show sees: the CPU time of the upload
  call when the loop runs at 60 fps like a vsynced output. It is 0.2 ms for
  720p.
- The fenced column is an upper bound: the GPU's copy plus the call, with
  nothing overlapping.
- **In an unpaced loop** (frames back to back, no swap, no fence) the
  upload call took 6-7 ms at 720p and 12-20 ms at 1080p, on native GL. That
  is the driver throttling a queue that only fills, not a texture conflict:
  a ring of two or three textures did not change it. A show is paced by
  vsync, so one texture is enough. If the T480 disagrees, the next step is a
  pixel-unpack buffer, not a ring.
- One-off stalls of 30-250 ms showed up in some runs, both backends. Each
  time, other cargo builds were running on the machine. Clean reruns had
  none.

### Latency added by Strobe

A thread put pictures at 30 fps into a loop rendering at 60 fps, paced by
sleeping. A picture waited **at most 16.7 ms** (one render frame) from
being put to being uploaded, with mean 10-15 ms (the phase of 30 against
60). Nothing was dropped when the loop kept up. Strobe's total share is
that wait, plus the conversion above, plus the frame and swap that every
output already has. The camera's exposure and USB transfer, and Media
Foundation's buffering, come on top and need a camera.

### Binary size

Release `strobe.exe` (fat LTO, line tables): 15,894,016 bytes without
nokhwa, 15,983,104 with the whole spike: **+89,088 bytes (87 KiB,
+0.56 %)**. That covers nokhwa, its bindings, the extra `windows`
features they turn on, and the spike's own capture and mailbox code.
`image`'s JPEG decoder was already linked.

### No camera, and failure

On this camera-less machine:

- `nokhwa::query(MediaFoundation)` returned an empty list in 5-48 ms, in
  release and in a debug build, so nokhwa's old no-camera panic (upstream
  #192, fixed) does not reproduce with 0.4.6 bindings. #192 was a
  `slice::from_raw_parts` precondition, checked only with debug assertions
  on; the debug build listed nothing in 5.2 ms without panicking.
- `--webcam 0`: the open failed with "No device". The failure went
  through `strobe_diag::failure` once, with the existing action ("the
  scene runs without a media picture (syn_Media is black)"). The scene kept
  running, and the thread went on listing cameras once a second in the
  background.

## Dependency and license

`nokhwa = { version = "0.10.11", default-features = false, features = ["input-msmf"] }`

- **Eight new crates:** `nokhwa`, `nokhwa-core`, `nokhwa-bindings-windows`
  (Apache-2.0), `flume`, `paste` (MIT/Apache-2.0), `futures-sink`,
  `nanorand`, `spin`. `windows` 0.62 and `image` 0.25 are already in the
  tree, through cpal, midir and our own crates.
- **Default features must stay off.** nokhwa's default `decoding` feature
  pulls `mozjpeg`, which is C (libjpeg-turbo) built from source. MJPEG
  decodes fine through `image`, which is pure Rust and already linked.
- **`input-native` must not be used.** It turns on the Linux backend too,
  whose `v4l2-sys` runs bindgen and needs libclang on CI. Linux and macOS
  are their own later steps.
- **cargo-deny fails** on `paste`: RUSTSEC-2024-0436, unmaintained. It is
  a proc-macro with no runtime code. Landing nokhwa needs either an
  `ignore` in `deny.toml` with its reason, recorded as a decision, or a
  nokhwa fork without it. Licenses, bans and sources pass.
- `THIRD_PARTY.md` needs `cargo xtask credits` when it lands.

### Known upstream problems (nokhwa issue tracker, read 2026-10-28)

- **#251, open: one `IMFSample` leaks per frame on Media Foundation.** The
  reporter measured about 172 bytes a read, roughly 20 MB an hour at
  30 fps, and posted a two-line fix (pass an empty `Option` to
  `ReadSample` instead of a pre-made sample). For a show that runs for
  hours this must be fixed before release: upstream, or a
  `[patch.crates-io]` fork carrying the fix.
- **A read blocks until a frame comes.** `Camera::frame` has no timeout.
  Downstream projects report it waiting for ever when a camera stops
  sending (on macOS for certain, on Media Foundation reportedly). The
  capture thread is therefore never joined (the spike sets a stop flag and
  lets it go), and the render thread never waits on it. A real version
  needs a watchdog: no picture for a few seconds means "the camera stopped
  sending", said once.
- **Errors are strings.** nokhwa keeps the HRESULT's text, not the code,
  so "unplugged", "in use by another app" and "blocked by Windows'
  camera privacy setting" can only be told apart by their wording. That
  matters for the message a person sees, since Windows 10 and 11 can turn
  off camera access for desktop apps.
- #165 (open): a deadlock in stop-stream with nokhwa's threaded camera.
  The spike does not use the threaded camera.

### Alternatives considered

- **Media Foundation directly through the `windows` crate.** Full control:
  a timeout on `ReadSample`, real HRESULTs, no leak. But it would put a
  third `unsafe` zone in the codebase, outside `strobe-gfx/src/gl/` and
  `strobe-diag/src/native.rs`, with COM lifetimes to get right. Worth it
  only if the nokhwa problems above can't be patched.
- **DirectShow**, which Synesthesia uses. On Windows 10 it is the only API
  that sees DirectShow-only virtual cameras. OBS's virtual camera is the
  usual one, which matters to VJs. Whether Media Foundation sees OBS's
  camera on the owner's machines is unverified (see the checklist). Spout
  and NDI input are the better route for OBS anyway, and are separate items.
- **`ccap-rs`**: a C++ library behind a Rust wrapper (DirectShow, optional
  MF). It adds a C++ build.
- **OpenCV through nokhwa's `input-opencv`**: far too heavy.

## Proposal

1. **Capture in its own crate**, `strobe-capture`, with no GL, like
   `strobe-audio` and its cpal thread. It owns device listing, open, the
   capture thread, pixel conversion, the watchdog and reconnect by name. It
   writes into a sink trait the app implements over `strobe_gfx::live`'s
   mailbox, so neither crate depends on the other and no picture is copied
   twice. nokhwa is a Windows-only target dependency of that crate.
2. **Pixels:** ask for YUY2 or NV12 at 1280x720, 30 fps. Use MJPEG only
   when that size isn't offered uncompressed. Convert on the capture
   thread. A shader that converts YUY2 on the GPU (half the upload bytes,
   no CPU conversion) is the optimisation to reach for only if the T480
   needs it.
3. **Upload:** at most one per frame, into one reused texture, through
   `Gl::upload`, as the spike does. No allocation and no driver query on
   the frame path; a size change makes a new texture once.
4. **Media semantics:** a camera is a media choice beside "a file or a
   folder" in the Inspector's Media row, by camera name, with
   `syn_MediaType` 3. Like the operator's choice now (D217) it holds across
   scene changes; that is Synesthesia's Lock. A cue or preset may name a
   camera, with the same precedence as today (cue > operator > preset >
   scene). `FPS` and `LOOP` don't apply; the slot's `WRAP` and `FILTER` do
   (D238).
5. **Failure:** a missing, unplugged, busy or blocked camera reads black,
   is said once through `strobe_diag::failure` with an action that names
   the likely cause, and comes back by itself when the camera does.
6. **The Media Overlay first, or with it.** Mode and strength, in the post
   stage after the clamp and before Master (R16), for stills and sequences
   as much as for cameras. It belongs to C-01's media group and waits on
   that item's reorder decision. Without it a camera shows only in scenes
   that read `syn_Media`.
7. **Before release:** the #251 leak fixed (upstream or patched), the
   `paste` advisory decided, and the checklist below run on the T480.

## Checklist for a machine with a camera

A ThinkPad T480 usually has a built-in camera; [hardware/T480.md](../hardware/T480.md)
doesn't list one yet. A USB camera is needed for the unplug items. The
T480 runs Windows 11, so the Windows 10 items need the AMD desktop and a
USB camera.

- [ ] `strobe --webcam list`: the cameras, their names, and how long
      listing takes.
- [ ] The formats each camera offers at 640x480, 720p and 1080p (YUY2,
      NV12 or MJPEG only), and what the spike's "closest to 720p YUY2 30"
      request picks.
- [ ] **Glass-to-glass latency:** a scene drawing a large frame counter
      beside `syn_Media`, the camera pointed at the output, a screenshot
      or phone photo: the difference between the two counters, times the
      frame time.
- [ ] Capture-thread CPU and the paced upload cost on the i7-8550U and
      UHD 620, native and ANGLE (`strobe-bench --live --context window`).
- [ ] **Unplug while showing:** the error, how long until it is said,
      whether `frame()` returns or blocks.
- [ ] **Replug:** reopened by name? After how long? The same index?
- [ ] Camera in use by another app (Teams, the Camera app), and Windows'
      camera privacy switch off: the error text of each.
- [ ] Sleep and resume with the camera open.
- [ ] A 4-hour soak at 30 fps: private memory over time, with and without
      the #251 fix.
- [ ] OBS's virtual camera: does Media Foundation list it on Windows 10?
      On Windows 11?

## Not covered

- **Video files** (`syn_MediaType` 2): a decoder decision of their own. Media
  Foundation's Source Reader decodes MP4/H.264 with hardware help on
  Windows through the same `windows` crate, but nokhwa doesn't read
  files, and HAP/DXV (which Synesthesia recommends) need their own
  decoders.
- **Spout and NDI input**, Linux (V4L2) and macOS (AVFoundation)
  capture: separate items.
