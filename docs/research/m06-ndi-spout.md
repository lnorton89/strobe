# M-06: NDI and Spout output — research and proposal

> **Status:** research done 2026-11-26 on the owner's Windows machine; the
> frame seam it proposes is built (`strobe-output`, D570), the two senders
> and the app wiring are not. This page replaces the "Deferred research"
> line in backlog M-06.

## What was recorded before this page

There was no research on NDI or Spout. The repository held four pointers,
and they are worth knowing because they still hold:

- [`PLAN.md`](../PLAN.md) put NDI out in v2 "via `grafton-ndi` … requires
  GPU→CPU readback via PBO ring", and put Syphon/Spout out of scope for v1.
- [`hardware/TORNADO.md`](../hardware/TORNADO.md) T21 and
  [`OPEN_QUESTIONS.md`](../OPEN_QUESTIONS.md) Q22 settled that SSF gives no
  `syn_MediaType` value for NDI or Spout, so both are **outputs** here and
  not inputs.
- [D248](../decisions/D248.md): `Alpha Channel` waits for an output that
  carries alpha, and calls NDI and Spout "the first thing to add".
- [D181](../decisions/D181.md) and Q20: blackout clears only the shown
  window; whether a shared output honours it was left for this stage.

So: no prior research, no SDK evaluation, no crate evaluation, no numbers.
The rest of this page is that work.

## The two ecosystems, and what changed since PLAN.md

### NDI

The 2026 picture is much better than PLAN.md's: **`grafton-ndi` 3.0 needs
no SDK at build time** on Windows, macOS, Linux and Android. Its own
[crates.io page](https://crates.io/crates/grafton-ndi) says default builds
"need no SDK at build time", and its
[setup guide](https://github.com/GrantSparks/grafton-ndi/blob/main/docs/setup.md)
says a runtime-loaded build finds an **NDI 6 runtime** beside the executable,
in `NDI_RUNTIME_DIR_V6`, or through the platform's loader. Only the
`advanced` feature (compressed and native packet paths) wants SDK headers
and `libclang`. That removes the packaging question PLAN.md was worried
about: no C, no headers, no libclang, and the Windows cross-check keeps
compiling.

The sender API, from its
[usage guide](https://github.com/GrantSparks/grafton-ndi/blob/main/docs/usage.md):

| Need | Call |
|---|---|
| Create | `Sender::new(&ndi, &SenderOptions::new("Strobe"))` |
| Frame size and rate | explicit `FrameRate`; 60 fps is the default |
| Send, copying | `send_video(&frame)` — storage is free when the call returns |
| Send async, no copy | `submit_video(owned)` plus `reclaim_video()`, reusing **two** owned frames |
| Send a borrowed slice | `submit_video_copy` |
| Clocking | on by default; `clock_video(false)` sends at the caller's cadence |

The SDK's async pipeline is not a Rust future and "may block while
synchronizing", so a sender must hand over storage it keeps, never a slice
of the renderer's target.

**Licensing.** NDI's own
[licensing page](https://docs.ndi.video/all/developing-with-ndi/sdk/licensing)
says the SDK is royalty-free subject to its terms, and lists the
distribution duties: a link to [ndi.video](https://ndi.video) "in a
location close to all locations where NDI is used/selected within the
product, on your website, and in its documentation", and no redistribution
of the NDI tools. That is a doc, an About-page and a Settings-row obligation
— cheap, but it has to be in the plan rather than discovered at release.
The page also mentions a **30-minute trial timeout** for the Advanced SDK on
non-mobile platforms; whether any timeout applies to the standard runtime is
not something this page could confirm from the terms, and it is the one
licensing question to settle with the real SDK (below).

### Spout

Spout shares a **Direct3D 11 texture** with another process on the same
machine; a GL renderer registers its texture with
`WGL_NV_DX_interop` and Spout copies GPU-to-GPU, so there is **no CPU
readback and nothing encoded**. That is the opposite trade from NDI, and it
is the right one on Windows: the frame is already on the GPU and staying
there costs about nothing.

Two ways in:

1. **`SpoutLibrary.dll`** — Spout's own C-ABI build, which is what the
   ecosystem ships (Synesthesia installs one; see the machine evidence
   below). It exposes the sender calls as C functions, so it can be loaded
   at runtime, but the calls are `unsafe` and would need a new leaf in the
   unsafe boundary ([`strobe-conventions`](../../crates/strobe-conventions/src/unsafe_boundary.rs)
   allows exactly two places today).
2. **A Rust binding crate.** Three exist and all are young:
   [`spout-rs`](https://crates.io/crates/spout-rs) 0.2.0 (BSD-2-Clause,
   safe API with `send_texture`, and an empty API on non-Windows so it can
   sit in a cross-platform tree), `spout2-rs` 0.1.1 with a hand-written C++
   shim, and `rust-spout2` 0.1.5 (MPL-2.0). `spout-rs`'s documented call is
   `SpoutSender::new(name)` then `send_texture(tex_id, target, w, h, …)`,
   which is exactly the zero-copy path. Its stated repository URL returns
   **404** at the time of writing, which is a supply-chain fact the
   decision has to weigh, not a detail.

Spout is Windows-only by construction. Syphon is its macOS counterpart and
is a separate item, not covered by M-06.

## Machine evidence

Checked on the owner's Windows machine, 2026-11-26:

- `C:\Program Files (x86)\Synesthesia\SpoutLibrary.dll` — **present**, so a
  Spout sender can be tried here without installing anything.
- `C:\Program Files (x86)\Synesthesia\Processing.NDI.Lib.x86.dll` —
  present, and **32-bit**. An x86_64 `strobe.exe` cannot load it, and NDI's
  loader does not look inside another application's folder. **NDI output
  therefore needs the NDI 6 runtime installed for x64 (NDI Tools) before it
  can be verified on this machine.**
- No `NDI_SDK_DIR`, no `C:\Program Files\NDI`.

## What it costs

A 1920x1080 frame, and what one second of it weighs:

| Path | Bytes a frame | At 60 fps | At 30 fps |
|---|---|---|---|
| RGBA / BGRA (NDI lossless, readback) | 8.29 MB | 498 MB/s ≈ 3.98 Gbit/s | 249 MB/s |
| UYVY 4:2:2 (NDI default) | 4.15 MB | 249 MB/s ≈ 1.99 Gbit/s | 124 MB/s |
| UYVY at 1280x720 | 1.84 MB | 111 MB/s ≈ 0.88 Gbit/s | 55 MB/s |

Three consequences, and they decide the design:

1. **NDI wants UYVY**, and it wants the output at the show's size rather
   than at some larger capture size. BGRA is the fallback, not the default;
   it doubles both the readback and the wire.
2. **NDI at 1080p60 needs a gigabit link to itself, and a 2.5G/10G link to
   be comfortable** once discovery, audio and other traffic are on it. A
   venue Wi-Fi is not a plan. This is worth saying in the user guide next to
   the switch, because "it stutters" will otherwise be blamed on the
   renderer.
3. **The readback is the cost that lands on the frame**, not the send. A
   full-size RGBA read of 8.29 MB per frame is 498 MB/s of memory traffic;
   on an HD 4000 with shared memory that is real. `textureFinalPass`
   already avoids the stall with a 3-slot pixel-pack ring and a fence
   ([`strobe-gfx/src/readback.rs`](../../crates/strobe-gfx/src/readback.rs));
   the output path wants the same shape at full size, plus the UYVY pack on
   the CPU. **Spout has no readback at all**, which is why it should be the
   zero-cost path on Windows and NDI the interoperable one.

`Frame` and the packing pass on the CPU can be bounded and counted, but the
real number is the one backlog M-06's acceptance test asks for: a
`strobe-bench` run with an output on, against one with it off, on the T480.

## Where the picture is taken from

The tap belongs **after the output post stage** (D180), on the picture
rather than the window:

- Post is the last thing that touches pixels and it runs after the
  compositor, so a crossfade goes out as one mixed picture, and `master`,
  colour, mirrors and `Limit Colors` reach the stream exactly as they reach
  the projector.
- The output window letterboxes; a stream must not. So the source is the
  post stage's target when it ran, else the compositor's, else the scene's
  output — the same three-way choice the app already makes for a hidden
  window's screenshot (`app.rs`, `scene_frame`).
- **Alpha**: the post stage currently forces alpha to 1. With a reader that
  can carry it (BGRA for NDI, an RGBA Spout texture), `Alpha Channel`
  (D248) stops being a control with nowhere to go. It should be added in
  the post stage, not in a sender.
- **Blackout**: it is a present-time clear of the shown window (D135, D181).
  A shared output follows `master` and not blackout, so a panic key that
  darkens the room does not also cut the recording; a cue or a macro can
  send black if the stream should go dark too.

## Failure handling, which is most of the work

A show must not die because a receiver went away. The rules this stage
proposes, and the ones `strobe-output` already enforces:

- **Said once per outage**, through the asset/failure path
  (`strobe_diag::failure`), with an action: install the NDI runtime, check
  the network, close the program holding the Spout sender.
- **Never queued, never waited for**: a frame that cannot be sent is
  dropped, counted, and the show keeps its cadence. The sink is closed and
  dropped so a half-broken sender is not reused.
- **Reopened on the host's schedule**, not per frame: a failed sender is
  replaced after a backoff (one second, doubling to thirty), and a
  reconnect is not a song change, a scene reload or a resize.
- **Runtime absent**: probed before the sender is created, so the console
  says "NDI output needs the NDI 6 runtime; install NDI Tools from
  https://ndi.video" instead of an SDK error from inside the frame.
- **Sending is off by default**, and the source of truth is a setting the
  show reads once, like the LAN remote.

## Proposal

1. **`strobe-output`** (built): `Frame`, BGRA and UYVY packing, the `Sink`
   seam with the failure policy above, and the runtime probes. No GL, no
   Strobe dependencies, tested everywhere.
2. **NDI sender** behind a `ndi` feature: `grafton-ndi` 3.0, one sender
   named from the setting, UYVY first with BGRA as the fallback,
   `clock_video(false)` so the show's cadence is the only clock,
   `submit_video` with two reclaimed frames so a send never copies twice.
3. **Spout sender** behind a `spout` feature: zero copy from the post
   stage's texture, on the render thread with the GL context current. Its
   route (a binding crate, or `SpoutLibrary.dll` through one new unsafe
   leaf) is the open choice below.
4. **The tap** in `strobe-gfx`: a full-size readback ring for the CPU path,
   and a safe way to lend the texture for the Spout path, both sized at the
   output and rebuilt on resize.
5. **The app**: Settings > Output gains NDI and Spout rows with the
   runtime's state, `--ndi-name NAME` / `--spout` for a headless or scripted
   show, the failure once with its action, and the NDI link the licence
   asks for.
6. **Docs**: USER_GUIDE ("sharing the output"), COMPATIBILITY, and the
   THIRD_PARTY notices once the two crates are in the tree.

## Acceptance test (backlog M-06)

- `strobe-bench --output none` against `--output ndi|spout` at 1280x720 and
  1920x1080 on the T480, frame time and dropped frames recorded in
  [BENCHMARKS.md](../BENCHMARKS.md).
- An NDI receiver (NDI Studio Monitor) on a second machine: the stream
  appears by name, keeps its cadence for 30 minutes, survives the receiver
  closing and reopening, and the console shows one line for each of those.
- A Spout receiver (OBS, or Resolume) on the same machine: the sender
  appears, the picture matches the projector, blackout does not blacken it
  while `master` does, and closing the receiver does not stop the show.
- The runtime absent: both rows say what to install, and neither is
  attempted.
- A 30-minute soak with an output on: flat memory, no runtime compiles, no
  frame time regression against the same show with it off.

## What issue #116 asks for, and where each part lands

The Spout request ([lnorton89/strobe#116](https://github.com/lnorton89/strobe/issues/116))
asks for more than one sender, and for input as well as output. Each part
has a place, and they are not the same size:

- **Native GPU sharing, not a pixel copy.** That is what Spout is, and it
  is why the Spout path in this design never touches the CPU: the post
  stage's texture goes to the sender with the GL context current.
- **Several senders at once, each with its own name.** One `Output`
  ([`strobe-output`](../../crates/strobe-output/src/sink.rs)) is one named
  destination, so several senders are several outputs, each opened on its
  own name. What that needs from the app is a list rather than a switch:
  Settings > Output with one row per sender (name, kind, state), a name
  that is stable across restarts so a Resolume or OBS rig finds the same
  sender again, and a refusal of two senders with one name. Spout itself
  lists senders by name per machine, so the names have to be the operator's
  and not generated.
- **A GUI for it.** The same Settings > Output rows, with the runtime's
  state and the failure's action, as this page's proposal already says.
- **Spout input.** A different feature, and not in this slice: it is a
  *media source*, the mirror of the webcam (D528). A receiver would be the
  media choice `spout:<name>`, taken through the same `LiveSources` hook
  and one reused texture, with `syn_MediaType` 3 (the value D528 uses for
  a live input; Q22 records that SSF has no value of its own for a shared
  texture). It shares nothing with the sender beyond the library, it needs
  a receive path and its own failure wording, and only scenes that read
  `syn_Media` would see it until the Media Overlay (C-01) lands. It belongs
  in M-04's family, not M-06, and should be its own item.
- **ISF import**, mentioned in passing in the issue, is a separate item
  (PLAN.md puts ISF import out of v1).

## Open questions for the owner

1. **Which Spout route** — `spout-rs` 0.2.0 (safe API, but its repository
   is unreachable, so pinning it is a supply-chain decision) or
   `SpoutLibrary.dll` through one new unsafe leaf in `strobe-output`, which
   is what the installed Synesthesia build provides? The second is more
   code and more honest about what it depends on.
2. **Is NDI's 30-minute timeout** anything to do with the standard runtime,
   or only Advanced SDK trials? Confirm from the SDK's own licence text on
   the machine that installs it.
3. **Send rate**: every presented frame (249 MB/s at 1080p60 in UYVY), or
   a capped 30 or 60? A default of "every frame" is right for a show; a
   venue network may want the lever.
4. **Does the owner want audio over NDI**? A VJ engine sending picture
   without its track is a different integration (analysis here, audio
   capture from an interface), and the sender is where `send_audio` goes if
   so.
5. **How many senders, and at what cost?** Issue #116 wants several at
   once. Each NDI sender is its own stream and its own readback unless the
   pack is shared; each Spout sender is nearly free. One readback packed
   once and handed to every NDI sender is the design to aim for, but the
   number is the owner's call.
