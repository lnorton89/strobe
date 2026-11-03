# strobe-capture

Webcams for `syn_Media` (backlog M-04, D528). It lists the cameras the
system sees, opens one by name at 720p, reads its pictures on a thread of
its own, converts each to RGBA and hands it to a `Sink`. A watchdog covers a
read that never returns, and the camera is reopened by name when it comes
back. It never touches GL.

## Role in the workspace

- **Depends on:** `log`, `image` (MJPEG), and on Windows `nokhwa` with only
  its Media Foundation backend (`input-msmf`, no default features).
- **Depended on by:** `strobe-app`, whose `camera` module implements the
  `Sink` over a `strobe_gfx::live::Mailbox` and installs the player's
  `LiveSources`.
- **Does not:** depend on `strobe-gfx` or any other Strobe crate, or draw
  anything. The render thread never waits on it: a picture goes into a
  one-picture mailbox, and the newest wins.

## Main types / entry points

| Item | What |
|---|---|
| `list()` | The cameras by name, as `--list-cameras` prints them. Tens of milliseconds, so a UI calls it off its frame thread. |
| `start(name, sink)` | Starts the `camera` (watchdog) and `camera-read` threads. Returns at once; a camera not there yet is waited for. |
| `Sink` | `buffer`, `put(Picture)`, `failed(&Failure)` once per outage, `recovered`, and `wanted`, which stops the threads when it says no. |
| `Picture` | RGBA8, bottom row first (GL's order), with the time it was read. |
| `Cause`, `Failure` | Why a camera shows nothing (not found, unplugged, stalled, in use, blocked, unreadable, unsupported) and the action line a person reads. |
| `format::pick`, `Offer`, `PixelFormat` | Which of a camera's formats to ask for: 720p30 uncompressed first. |
| `convert::to_rgba` | YUY2, NV12, MJPEG, grey, RGB and BGR to RGBA, flipped, into a reused buffer. |
| `Backend`, `Stream`, `Timing`, `start_with` | The seam the tests use: a fake camera and short times. |

## Usage

```rust
use std::sync::Arc;
use strobe_capture::{Failure, Picture, Sink};

struct Print;

impl Sink for Print {
    fn buffer(&self) -> Vec<u8> { Vec::new() }
    fn put(&self, picture: Picture) { println!("{}x{}", picture.width, picture.height); }
    fn failed(&self, failure: &Failure) { println!("{failure}: {}", failure.cause.action()); }
    fn recovered(&self, camera: &str) { println!("{camera} is back"); }
    fn wanted(&self) -> bool { true }
}

fn main() -> Result<(), strobe_capture::Problem> {
    for name in strobe_capture::list()? {
        println!("camera:{name}");
    }
    strobe_capture::start("Integrated Camera", Arc::new(Print));
    Ok(())
}
```

In the app a camera is chosen as the media path `camera:<name>`: in the
Inspector's Media row (Camera), with `--media "camera:<name>"`, in a cue's
`media` or a preset's media channel.

## Testing

```bash
cargo test -p strobe-capture
```

Everything but the Windows backend is tested against a fake camera on every
OS: each pixel format, the format choice, the failure wording, and the
supervisor (pictures flow and stop when unwanted, a missing camera is said
once and shown when it comes, an unplugged one is reopened, a hung read is
left behind and hung readers are capped). The render side is
`crates/strobe-gfx/tests/live.rs`.

## Status and known gaps

Open work for this crate is tracked in
[`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md) under the
IDs given here; a gap with no ID is by design, with its decision named.

- Built and wired into the app on Windows (D528), with nokhwa's frame leak
  patched in `vendor/` (D529). The machine it was built on has no camera,
  so no real camera has been opened by it yet: the camera checklist in
  [`research/m04-webcam.md`](../../docs/research/m04-webcam.md) is still
  open (backlog M-04).
- The failure wording (`Cause::from_text`) is unverified against the error
  text real cameras give when unplugged, in use or blocked (backlog M-04's
  checklist). A wrong guess only changes the advice shown.
- Linux and macOS have no backend yet: listing says so, by design for
  this stage (D528); V4L2 and AVFoundation are their own later steps
  (backlog M-04).
- Two identical cameras share a name, and the first listed is the one
  opened, by design for now (D528).
- A camera shows only in scenes that read `syn_Media`; the Media Overlay
  that blends it over any scene is not built and waits on backlog C-01.
