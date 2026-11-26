# strobe-output

Sharing the finished picture with another program, by NDI or Spout (backlog
M-06, D570). It carries one frame out of the engine: the picture as RGBA8,
the two layouts a sender wants, one destination with the show's failure
policy, and a probe for whether the optional runtime is installed. It never
touches GL.

## Role in the workspace

- **Depends on:** `log`, and nothing else. No Strobe crate, no GL, no
  window, no network.
- **Depended on by:** nothing yet. The app's wiring is the next step of
  backlog M-06, with the two backends.
- **Does not:** draw anything, own the render loop, decide what is on
  screen, or block the frame. A frame that cannot be sent is dropped and
  counted, never queued.

## Main types / entry points

| Item | What |
|---|---|
| `Frame` | One picture: RGBA8, top row first, `width * height * 4` bytes. `Frame::black` gives a sender something to advertise before the first frame. |
| `pixel::to_bgra` | RGBA to BGRA into a reused buffer, alpha kept. |
| `pixel::to_uyvy` | RGBA to UYVY 4:2:2 (BT.709, limited range) at a chosen stride, chroma averaged over each pixel pair, padded tails zeroed. Half the bytes of BGRA, which is why it goes first. |
| `pixel::bgra_len`, `pixel::uyvy_len` | The buffer a frame needs. |
| `Sink` | One destination: `send(&Frame)`, and `close` for a backend whose release has a thread. |
| `Output` | The show's policy around a sink: `show`, `reopen`, `shut`, `status`, `reason`, `sent`, `dropped`. A failing sink is closed, dropped and said once. |
| `probe::ndi_runtime`, `probe::spout_library` | Whether the runtime is on this machine, without loading it, so the app can say what to install. |

## Usage

```rust
use strobe_output::{Frame, Output, Sink, Problem};

struct Nowhere;

impl Sink for Nowhere {
    fn send(&mut self, frame: &Frame) -> Result<(), Problem> {
        println!("{}x{}", frame.width, frame.height);
        Ok(())
    }
}

let mut output = Output::open("Strobe", Box::new(Nowhere));
let mut buffer = Vec::new();
let frame = Frame::black(1920, 1080, &mut buffer);
output.show(&frame);
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

- The Spout sender shares the renderer's texture with nothing read back, by
  calling the installed `SpoutLibrary.dll`'s mangled ABI (D571): it needs a
  **64-bit** Spout library (a 32-bit one, such as the copy inside Synesthesia,
  is refused by name) and it needs Windows (backlog M-06).
- The NDI sender is behind the `ndi` feature and is built with the app; it
  needs the NDI 6 runtime at run time and has not been run, because no x64
  runtime is installed on the machine this was built on (backlog M-06).
- No receiver has been watched: Spout was exercised only as far as loading the
  library and reporting why it refused this machine's copy, and NDI not at
  all (backlog M-06).
- The NDI path pays a synchronous readback per frame: the ring is not built
  and the cost is the number backlog M-06's acceptance test still wants
  (backlog M-06).
- `Frame` and the packing are unmeasured on real hardware: the cost of
  reading 1080p back and converting it to UYVY is the number backlog M-06's
  acceptance test wants (backlog M-06).
- Alpha is carried in the frame and in BGRA, but nothing produces a
  non-opaque picture yet, by design until an output reads it (D248).
