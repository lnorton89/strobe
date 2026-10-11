# strobe-io

Show control: a MIDI controller, a phone running TouchOSC or a script sending
UDP moves controls, recalls presets and runs transport commands. This crate is
everything about that which is not the UI and not the renderer: two worker
threads (MIDI and OSC), the typed events they produce, the mapping model with
learn and soft takeover, the Performance Macro set, and the file that keeps it
all. It has no GL, no window and no egui, so every rule in it runs in a unit
test with no device.

## Role in the workspace

- **Depends on:** `strobe-scene` (for `ControlBank`, the script API's name
  matching and presets), `rosc` (OSC 1.0), `midir` (behind the `midi` feature,
  on by default), `serde`, `serde_json`, `toml`, `thiserror`, `log`.
- **Depended on by:** `strobe-app`, whose `remote` module owns the workers and
  drains them once per scene frame ([`ARCHITECTURE.md`](../../docs/ARCHITECTURE.md),
  frame step 1b).
- **Never depends on:** `strobe-gfx`, a window or egui.
- **Does not:** draw anything, touch a scene's files (presets are in
  `strobe_scene::preset`), or decide what a command does. A command comes back
  as an id and the app runs it through the same table the keys use.
- **`midir` builds for Windows without a C compiler** (D170), so the Windows
  cross-check includes this crate.

## Main types / entry points

| Item | What |
|---|---|
| `message::parse`, `MidiMessage`, `device_key` | Raw MIDI bytes into note on/off, CC, pitch bend, program change; everything else (clock, sysex, aftertouch, channel-mode controllers, malformed) is `None`. `device_key` strips ALSA's `20:0` and WinMM's `2- ` so a device keeps its name across a replug. |
| `osc::decode`, `osc::nesting_ok`, `osc::builtin_target` | A datagram into messages, bounded (bundle depth 4, 256 messages, 8 arguments); the built-in `/strobe/...` address space. |
| `Hub` | `start(&midi, &osc)`, `set_osc`, `set_midi`, `rescan_midi`, `drain(max, &mut events)`. Two threads, one bounded queue, `try_send` so a driver callback never blocks (D171). |
| `Event`, `InputEvent`, `Status`, `DeviceInfo` | What the workers deliver: input, and reports (device found or lost, listener up or failed, messages dropped). |
| `Source`, `Signal` | Where a message came from (CC, note, pitch bend, program, OSC address; a device or any) and what it said (`Analog(0..1)` or `Button`). |
| `Target`, `State`, `target::resolve` | What a mapping acts on, found by name against the playing scene; `Resolved`, `Unavailable`, `Broken`, `Disabled`, `Learning`, `Conflict` (D173). |
| `Mapping`, `Curve`, `Takeover`, `ButtonMode` | Source to target with range, curve, invert, button mode, pickup, threshold. |
| `Engine` | `handle(&source, signal, &mut banks, &mut out)`: writes the two `ControlBank`s directly and returns commands, cue jumps, fav and preset recalls as `Output`. `arm_new`, `arm_relearn`, `conflict`, `resolve_conflict` are learn (D174). |
| `Macro`, `MacroSet` | The curated, ordered, pinned set; a macro is a mapping target and a stable binding (D175). |
| `ShowControl` | `show-control.json`: OSC and MIDI settings, mappings, macros, fav slots, hotkeys. Tolerant of hand edits (D179). |

## Usage

The mapper against a control bank, with no device:

```rust
use strobe_io::{Banks, Engine, Mapping, Signal, Source, Target};
use strobe_scene::control::{ControlBank, ControlDef};
use strobe_scene::json::RawControl;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw: RawControl =
        serde_json::from_str(r#"{ "NAME": "speed", "TYPE": "slider", "MAX": 10 }"#)?;
    let mut scene = ControlBank::new(vec![ControlDef::from_raw(&raw)?])?;

    let mut engine = Engine::default();
    let source = Source::MidiCc { device: None, channel: 1, controller: 21 };
    let mut mapping = Mapping::new(source.clone(), Target::control("speed"));
    mapping.takeover = strobe_io::Takeover::Jump; // follow the fader at once
    engine.mappings.push(mapping);

    let mut banks = Banks { scene: Some(&mut scene), meta: None };
    let mut out = Vec::new();
    engine.handle(&source, Signal::Analog(0.5), &mut banks, &mut out);
    println!("speed target: {:?}", scene.target(0)); // 5.0, through ControlBank
    Ok(())
}
```

Sending OSC to a running Strobe (`strobe --osc`), with no library at all:

```bash
python3 - <<'EOF'
import socket, struct
def pad(b): b += b"\0"; return b + b"\0" * (-len(b) % 4)
msg = pad(b"/strobe/control/hue_shift") + pad(b",f") + struct.pack(">f", 0.8)
socket.socket(socket.AF_INET, socket.SOCK_DGRAM).sendto(msg, ("127.0.0.1", 9000))
EOF
```

### The built-in OSC addresses

No mapping is needed. Values are normalised 0 to 1.

| Address | Argument | Does |
|---|---|---|
| `/strobe/control/<name>` | float | sets the scene control across its range |
| `/strobe/control/<name>/<x\|y\|z\|r\|g\|b>` | float | one component of a pad or colour |
| `/strobe/meta/<name>` | float | sets the meta control |
| `/strobe/cmd/<id>` | none, `1`, `T` | runs the command (`next_scene`, `prev_scene`, `toggle_hold`, `extend_cue`, `restart_cue`, `skip_section`, `toggle_blackout`, `toggle_fullscreen`, `tap_tempo`) |
| `/strobe/cue/<n>` | none, `1` | jumps to cue `n`, from 1 |
| `/strobe/fav/<n>` | none, `1` | recalls fav slot `n`, 1 to 8 |
| `/strobe/preset/<name>` | none, `1` | recalls a preset of the playing scene (`_` reads as a space) |
| `/strobe/macro/<id or name>` | float | drives a Performance Macro |

A mapping for an address always wins over its built-in meaning.

## Testing

```bash
cargo test -p strobe-io
```

Over a hundred tests, none needing a device: recorded MIDI byte sequences,
bounded OSC decoding (a 3000-deep bundle, lying sizes, garbage), the mapping
math, soft takeover, learn and conflicts, resolution, the macro set, the file's
tolerance, and a real loopback UDP listener (`hub.rs`). The MIDI worker's
device code cannot be exercised without ALSA's sequencer or a driver, so it is
checked to start and to report "no MIDI service" where there is none; **a real
controller is the open check** (D171, Q23).

## Design notes

- **One control model** (D170). The mapper writes `ControlBank::set_normalized`,
  `set` and `press` and nothing else, so smoothing, `speed` integration, bang
  envelopes and clamping are not duplicated.
- **Bounded and non-blocking** (D171). A full queue drops the newest message
  and counts it. The app drains at most 256 events per scene frame.
- **OSC is off until asked for, loopback-only unless chosen, and a command-line
  flag is never saved** (D172). The datagram is treated as hostile: bundle
  nesting is checked iteratively before `rosc` recurses.
- **Names, never indices** (D173). A mapping to a control the scene lacks is
  *unavailable*, kept and harmless.
- **Soft takeover and learn** (D174): pickup by reaching or crossing the
  control's value; learn binds the first usable message and reports a conflict
  instead of stealing a source.
- **Tolerant file** (D179): a mapping that does not parse is skipped and named,
  the rest load, and the original is kept as a `.bak`.

## Status and known gaps

- Built and tested without hardware; **no MIDI controller has been used**.
- No OSC or MIDI **output** (feedback to motorised faders or TouchOSC).
- OSC address layout is Strobe's own, not Synesthesia's (Q23).
- Blackout is a toggle only; there is no explicit on or off command.
- A macro's range and colour have data and engine support but no editor.
