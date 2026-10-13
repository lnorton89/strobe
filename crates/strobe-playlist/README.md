# strobe-playlist

Playlists, without a GPU, a thread or a clock of their own. It owns the
playlist data model, the inheritance rules that make a cue naming only a
scene complete, the TOML reader and writer, a deterministic generator that
builds a section from a library snapshot, the timeline (every cue's start and
length, with wall-clock placement), and the playback state machine the frame
loop ticks.

## Role in the workspace

- **Depends on:** `serde`, `toml` and `thiserror` only (`anyhow` for tests).
  It does not depend on `strobe-audio` or `strobe-scene`: the clock takes a
  small `BeatInfo`, and control overrides use the crate's own `ControlValue`
  so the file format does not move when the scene crate does. The app
  converts at the boundary.
- **Depended on by:** `strobe-app`, whose `show` module is the glue: it
  compiles `distinct_scenes()` at load, ticks the clock each frame, warms up
  and mixes scenes, and saves `resume_point()` (D150-D158).
- **Does not:** load or compile scenes, render transitions, read the system
  clock, spawn threads, or touch GL. Its only I/O is `file::load` / `save`
  and `ResumePoint::save` / `load`, both atomic.

## Main types / entry points

| Item | What |
|---|---|
| `Playlist`, `Section`, `Cue`, `CueSettings` | The authored model. Every inheritable field is an `Option`. `SceneRef`, `TransitionSpec` / `TransitionKind` (cut, crossfade, fade-through-black, custom shader), `AdvanceMode`, `LoopMode`, `WallClockAnchor`, `ShuffleSpec`. |
| `DurationSpec` | The duration grammar: `90s`, `1:30`, `3m`, `1h30m`, `32 bars`, `rand(2m, 4m)`. |
| `resolve()`, `ResolvedCue`, `identity()`, `distinct_scenes()` | Flatten cue, section, playlist and built-in defaults (180 s, timer, 2 s crossfade) into cues with nothing optional; `distinct_scenes` is the list to precompile (enabled cues, first-use order). |
| `parse::parse`, `write::to_toml`, `file::load` / `save` | The TOML format. Errors carry line, column and the offending text, and unknown keys are errors. |
| `generate()`, `GenerateSpec`, `LibrarySnapshot` | A section from a scene library, pure in its seed (`rng::SplitMix64`). `PoolFilter::why` says which rule put a scene in the pool or kept it out (`Why`), and `Generated::pool` lists each pooled scene with its reason. |
| `Timeline`, `TimelineConfig` | `from_playlist`, `total_runtime`, `has_open_ended`, `wall_time`, plus `schedule::Placement` for wall-clock placement. |
| `PlaylistClock` | The state machine: `tick(dt, &BeatInfo)`, `apply(Command)` (`Next`, `Previous`, `Hold`, `Extend`, `JumpTo`, `SkipSection`, `ToggleBlackout`, ...), `drain_events()`, `transition()`, `mark_scene_failed(..)`, `resume_point()`. |
| `ClockEvent` | `CueStarted`, `TransitionStarted`, `TransitionFinished`, `WarmUp`, `CueSkipped`, `HoldChanged`, `BlackoutChanged`, `PlaylistEnded`. |
| `ResumePoint` | Where playback was, as a few lines of TOML. |

## Usage

Load the shipped example, plan it and play its first ten simulated seconds
(this compiles against the current API and prints the first cue start):

```rust
use std::path::Path;
use strobe_playlist::{
    BeatInfo, ClockConfig, PlaylistClock, Timeline, TimelineConfig, distinct_scenes, file,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let playlist = file::load(Path::new("playlists/example.toml"))?;
    println!("{} scenes to precompile", distinct_scenes(&playlist).len());

    let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
    println!("nominal runtime: {:.0} s", timeline.total_runtime());

    let mut clock = PlaylistClock::new(timeline, ClockConfig::default());
    for _ in 0..60 * 10 {
        clock.tick(1.0 / 60.0, &BeatInfo::SILENT); // dt in seconds, plus the beat state
        for event in clock.drain_events() {
            println!("{event:?}");
        }
    }
    Ok(())
}
```

`playlists/example.toml` is the format by example (defaults, a shuffle seed,
three sections, per-cue overrides, bar-based durations, a wall-clock anchor).
`crates/strobe-playlist/tests/example.rs` checks it parses, names only scenes
that exist in `scenes/`, round-trips and plays. The module docs in
`src/lib.rs` and the "Playlists" section of
[`ARCHITECTURE.md`](../../docs/ARCHITECTURE.md) show how the app is meant to
drive it.

## Testing

```bash
cargo test -p strobe-playlist
```

- Unit tests live beside each module (parsing errors, inheritance, the
  generator's golden output, the clock, resume), 160 of them at the time of
  writing, plus `tests/example.rs` (4) and `tests/soak.rs` (2).
- **Soak tests** (`tests/soak.rs`): two simulated four-hour runs at 60 Hz, 864,000
  ticks each. One is undisturbed and checked against the static timeline; the
  other runs a generated playlist under a seeded random stream of commands,
  failures and wobbling beat confidence, checking invariants after every tick
  (the active cue exists and is not failed, at most one transition is open, no
  transition id is reused, progress stays in 0..1, the event queue never grows
  past its reservation). They run in under a second in a debug build because
  nothing reads a real clock.
- No GPU, window or audio device is needed.

## Design notes

- **Authored data, resolved once into a flat view** (D73): `Playlist` is what a
  person wrote; `resolve()` yields `ResolvedCue`s that record which level
  supplied each field. A cue's `transition` is how the show arrives at it, and
  is capped at the cue's length, so total runtime is the plain sum of durations.
- **Our own SplitMix64, not `rand`** (D74): a seed in a playlist is a promise
  that the same section comes back. A golden test pins a generated section.
- **Beat-locked ends** (D75): a `bars` or `beats` cue's length is fixed at cue
  start, then it ends on the next downbeat, abandoning the wait if confidence
  is lost or after six beats.
- **Time is always an argument** (D76): the scene clock (`dt`) and the wall
  clock (time of day) are separate and meet in `sync_to_wall_clock`. Events are
  indices in a pre-reserved queue, so `tick` does not allocate.
- **Failed scenes are marked on the clock** (D77), so a cue whose scene failed
  to compile is skipped with a `CueSkipped` event wherever the clock would have
  gone.

## Status and known gaps

- Stage 1 is done: the pure core, with 166 tests and the soak runs
  ([`ROADMAP.md`](../../docs/ROADMAP.md)).
- **Stage 2 (app integration) is built** (D150-D158): precompile at load with
  progress, warm-up, the compositor (cut, crossfade, fade-through-black,
  custom GLSL), the Playlist pane and generate card, a file watcher, cue
  commands from the UI and keyboard, `resume_point()` saved every 5 s and
  `sync_to_wall_clock` at start-up. Verified on llvmpipe and headless GL;
  not on real hardware. Still to do: hotkeys beyond the table, MIDI and OSC
  mapping, audio-event advance, a downbeat detector.
- The app uses this crate's `restore_lenient` for hot reloads and `restore`
  (with the identity check) for the resume file; `mark_scene_failed` for scenes
  that do not compile or resolve; `peek_next`, `elapsed` and `remaining` for the
  transport strip.
- `strobe-audio` has no downbeat detector, so a "bar" is counted from the beat
  grid's origin until one exists (D75). Audio-event advance modes depend on
  phase 7 detection calling `clock.audio_event()`.
- `[shuffle]` supplies the seed for `rand(a, b)` durations and generator
  defaults; it does not reorder authored cues at run time (D73).
