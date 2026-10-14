# strobe-audio

Turns sound into the values an SSF scene reads: the scalar `syn_*` audio
uniforms (levels, hits, BPM and beat events, per band), the `syn_Spectrum`
and `syn_LevelTrail` textures, and the Shadertoy audio texture. The analysis
is pure DSP with no I/O, threads or clock, so it is deterministic and tested
from synthetic signals. An optional `capture` feature (on by default) adds
live capture from an audio device.

## Role in the workspace

- **Depends on:** `strobe-scene` (only for the uniform names in
  `strobe_scene::audio::SCALARS` and their silent values), `realfft`, `log`
  and `thiserror`. With the `capture` feature it also uses `cpal`, `rtrb` and
  `arc-swap`.
- **Depended on by:** `strobe-app`.
- **Does not:** depend on `strobe-gfx` or touch GL. It produces an
  `AudioFrame` and the app hands the values to the player. It does not
  decide when to fall back to synthetic audio either: that is the app's
  policy (D61), using `strobe_scene::synthetic` as the stand-in.

## Main types / entry points

| Item | What |
|---|---|
| `Analyzer` | The pure analyser. `push(samples)`, then `analyze(dt)` returns the latest `&AudioFrame`. Also `set_reactivity`, `set_audio_speed`, `tap`, `nudge_bpm`, `clear_manual_tempo`. `FFT_LEN` is 2048 and `HOP` is 512. |
| `AudioFrame`, `Layout` | One snapshot: `scalars` (in `SCALARS` order, `scalar(name)` looks one up), `spectrum`, `level_trail` and `shadertoy` buffers. `Layout` sizes the textures (default 256 / 256 / 512). |
| `Band`, `BandLayout`, `BAND_COUNT` | The four bands: Bass, Mid, MidHigh, High. |
| `BPM_MIN`, `BPM_MAX`, `BPM_DEFAULT` | 50, 220 and 120. |
| `AudioFrame::downbeat`, `downbeat::Downbeat` | `[offset, confidence]`: a bar starts when `syn_BeatTime` reaches `4n + offset`. Confidence 0 means no estimate (D233). Not a uniform. |
| `offline::run` | Drives an `Analyzer` over a finished buffer at a fixed frame rate, calling back per frame. Used by tests and the tuning example. |
| `capture::Capture` | (feature `capture`) `start` fails if no device opens; `start_resilient` always returns a handle, serves silence and retries every 2 s. `latest()` is one wait-free load; `state()`, `status()`, `select_device`, `tap`, `nudge_bpm`, `set_reactivity` and `set_audio_speed` are safe to call from a UI thread. |
| `capture::list_devices`, `DeviceChoice`, `DeviceInfo` | Device listing and selection. Windows output devices are listed as `DeviceKind::Loopback`. |
| `CaptureState` | `Running`, `Recovering` or `SilentFallback`. |

## Usage

Offline analysis and live capture, both against the current API (this
compiles and runs; with no audio device the capture handle reports
`SilentFallback`):

```rust
use strobe_audio::capture::{Capture, CaptureConfig};
use strobe_audio::{Analyzer, offline};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Offline: feed a buffer through the analyser at 60 frames a second.
    let mut analyzer = Analyzer::new(48_000);
    let samples = vec![0.0_f32; 48_000]; // one second of silence
    let mut last_bpm = 0.0;
    offline::run(&mut analyzer, &samples, 60.0, |_seconds, frame| {
        last_bpm = frame.scalar("syn_BPM").unwrap_or(0.0);
    });
    println!("syn_BPM after silence: {last_bpm}");

    // Live: never fails for lack of a device; serves silence and retries.
    let capture = Capture::start_resilient(CaptureConfig::default())?;
    let frame = capture.latest(); // wait-free, safe every render frame
    println!("{:?}, {} scalars", capture.state(), frame.scalars.len());
    Ok(())
}
```

Two examples are worth knowing:

```bash
# WAV in, CSV of every audio uniform per frame out ("-" writes to stdout)
cargo run --release -p strobe-audio --example wav-analyze -- track.wav out.csv --fps 60
# Per-call CPU cost of Analyzer::analyze on a busy synthetic signal
cargo run --release -p strobe-audio --example analyze-cost
```

## Testing

```bash
cargo test -p strobe-audio
cargo test -p strobe-audio --no-default-features   # pure DSP, no system audio library
```

The tests are unit and behaviour tests on synthetic signals (kick patterns,
band isolation, silence, value ranges, frame-rate independence, texture
shapes, tap tempo and nudge) plus capture tests that need no device:
starting with a missing device is an error rather than a panic. No GPU or
audio hardware is required. On Linux the default build links ALSA through
`cpal`, so it needs the ALSA development headers; `--no-default-features`
avoids that.

## Design notes

- **Two clocks** (D20): spectral analysis runs on audio time (one step per
  512-sample hop) and only writes targets; `analyze(dt)` then glides and
  decays toward them on the caller's `dt`. Hits keep the peak of every hop
  since the last call so a slow render loop cannot miss a kick.
- **Tempo by autocorrelation comb with an octave prior** (D21), 50 to 220
  BPM. A track near the range edges can still report double or half time,
  which tap tempo and nudge exist to override.
- **Capture is one thread that owns the stream** (D22): the audio callback
  feeds a lock-free ring, the analysis thread runs at about 100 Hz and
  publishes through `ArcSwap`, and the render thread only does an atomic
  load. Device loss is handled in the worker (`Running`, `Recovering`,
  `SilentFallback`).
- **Texture widths are a `Layout`, not constants** (D23), because the layout
  real scenes expect is unverified (Q10).
- **Numbers are tuned by ear, not matched to Synesthesia**: tests pin ranges
  and shapes, not exact values.

## Status and known gaps

- Built and wired into the app (phase 3, D61): capture with a synthetic
  fallback, textures uploaded each frame, settings UI
  ([`ROADMAP.md`](../../docs/ROADMAP.md)). The bench still uses the synthetic
  stand-in and uploads placeholder textures.
- On the T480 live capture opened the Realtek microphone at 48 kHz
  ([`T480.md`](../../docs/hardware/T480.md)); that is the only real-device
  record there. Device loss is recovered by retrying every 2 seconds, not
  by a device-change notification (D61).
- The spectrum is 1024 texels, Synesthesia's width (D269). The channel
  meanings are assumptions (Q10 in
  [`OPEN_QUESTIONS.md`](../../docs/OPEN_QUESTIONS.md)): raw linear 0 to
  Nyquist, juiced log 30 Hz to 16 kHz, trail index 0 newest.
- The downbeat estimate (`downbeat`, D233) says which beat starts a bar
  from each beat's bass energy and band-profile rise, with a confidence
  that stays near zero when every beat sounds alike. Tested on synthetic
  kicks at several tempos and phases, not yet on real music.
