# strobe-output

Sharing the finished picture with another program, by NDI or Spout (backlog
M-06, D570). It carries one frame out of the engine: the picture as RGBA8,
the two layouts a sender wants, one destination with the show's failure
policy, and a probe for whether the optional runtime is installed. It never
touches GL.

## Role in the workspace

- **Depends on:** `log`, and nothing else. No Strobe crate, no GL, no
  window, no network.
- **Depended on by:** `strobe-app` (`strobe_app::output`), which takes the
  picture from `strobe_gfx::share` and hands it to the senders (D573).
- **Does not:** draw anything, own the render loop, decide what is on
  screen, or block the frame. A frame that cannot be sent is dropped and
  counted, never queued.

## Main types / entry points

| Item | What |
|---|---|
| `Frame` | One picture: RGBA8, top row first, `width * height * 4` bytes. `Frame::black` gives a sender something to advertise before the first frame. |
| `Pixels`, `Layout` | A picture lent for one call: RGBA or UYVY 4:2:2, rows top first. What `Sink::send` takes, so the host's buffer is never copied to cross. |
| `SpoutSender` | Spout (Windows): shares a GL texture through the 2.007 library's vtable (D572, D573), creating the sender on the first frame, `UpdateSender` on a resize, and `Release` on drop. |
| `NdiSender`, `NdiOptions` | NDI (the `ndi` feature): name, groups and frame rate; UYVY or RGBA submitted asynchronously; `receivers()` is NDI's connection count. `ndi::TRADEMARK` and `ndi::TOOLS_URL` are the lines NDI's licence asks for. |
| `pixel::to_bgra` | RGBA to BGRA into a reused buffer, alpha kept. |
| `pixel::to_uyvy` | RGBA to UYVY 4:2:2 (BT.709, limited range) at a chosen stride, chroma averaged over each pixel pair, padded tails zeroed. Half the bytes of BGRA, which is why it goes first. |
| `pixel::bgra_len`, `pixel::uyvy_len` | The buffer a frame needs. |
| `Sink` | One destination: `send(Pixels)`, `send_texture`, `receivers`, and `close` for a backend whose release has a thread. |
| `Output` | The show's policy around a sink: `show`, `show_texture`, `reopen`, `shut`, `status`, `reason`, `receivers`, `sent`, `dropped`. A failing sink is closed, dropped and said once. |
| `probe::ndi_runtime`, `probe::spout_library` | Whether the runtime is on this machine, without loading it, so the app can say what to install. |

## Usage

```rust
use strobe_output::{Frame, Output, Pixels, Problem, Sink};

struct Nowhere;

impl Sink for Nowhere {
    fn send(&mut self, frame: Pixels<'_>) -> Result<(), Problem> {
        println!("{}x{}", frame.width, frame.height);
        Ok(())
    }
}

let mut output = Output::open("Strobe", Box::new(Nowhere));
let mut buffer = Vec::new();
let frame = Frame::black(1920, 1080, &mut buffer);
output.show(frame.pixels());
```

## Testing

```bash
cargo test -p strobe-output
```

The pixel packing is checked against the BT.709 equations for the three
primaries and for black and white, at an odd width and at a padded stride;
the failure policy against a fake sink (one close per outage, frames counted,
reopening only when down); the probes against a scratch folder.

## Status and known gaps

Open work for this crate is tracked in
[`IMPLEMENTATION_BACKLOG.md`](../../docs/IMPLEMENTATION_BACKLOG.md) under the
IDs given here; a gap with no ID is by design, with its decision named.

- The Spout sender shares the renderer's texture with nothing read back,
  calling the installed `SpoutLibrary.dll`'s vtable (D572, D573): it needs a
  **64-bit** Spout library (the `Spout-SDK-binaries` asset of the Spout 2.007
  release, or a 64-bit application's copy; a 32-bit one, such as the copy
  inside Synesthesia, is refused by name) and it needs Windows (backlog
  M-06). Verified on the development machine: a live sender at a fixed
  1280x720 listed by an independent receiver-side probe, and gone from the
  list after a clean exit.
- The NDI sender is built with the app; it needs the NDI 6 runtime at run
  time and has not sent a frame, because no x64 runtime is installed on the
  machine this was built on (backlog M-06). Without one it says where to get
  it, verified.
- No receiver application has been attached: Spout was verified as
  publishing, not as seen on another program's screen (backlog M-06).
- The cost of sharing (the scaled draw, the UYVY pack and its asynchronous
  readback) is unmeasured on real hardware; it is the number backlog M-06's
  acceptance test wants (backlog M-06).
- One sender of each kind, carrying the final picture: a second, clean feed
  and Spout or NDI input are later work (backlog M-06).
- `pixel::to_bgra` and `pixel::to_uyvy` are the CPU reference for the GPU
  packing in `strobe_gfx::share`; the app sends what the GPU packed, by
  design (D573).
- Alpha is carried in the frame and in BGRA, but nothing produces a
  non-opaque picture yet, by design until an output reads it (D248).
