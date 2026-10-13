# Strobe user guide

This is for someone running Strobe, not building it. You do not need Rust
or a compiler. Everything here applies to the Windows release; the
command-line flags work the same on Linux. For logs in depth, see
[DEBUGGING.md](DEBUGGING.md).

## Install

1. Download `strobe-vX.Y.Z-x86_64-pc-windows-msvc.zip` from the project's
   GitHub Releases page and unzip it anywhere you can write to, for example
   `C:\Strobe`. There is no installer and nothing goes in the registry.
2. The folder holds:

   | File | What it is |
   |---|---|
   | `strobe.exe` | The app. |
   | `libEGL.dll`, `libGLESv2.dll`, `d3dcompiler_47.dll` | ANGLE: OpenGL ES on Direct3D 11, for machines whose OpenGL driver is broken. These come from an official Electron release, are checked against pinned hashes when the release is built, and are covered by `LICENSES.chromium.html` beside them. Keep them next to `strobe.exe`. |
   | `scenes\` | The twelve scenes that ship with Strobe. |
   | `README.md`, `LICENSE` | The project and its MIT licence. |

   A separate `...-symbols.zip` holds `strobe.pdb`. You only need it to
   turn a crash report into function names (see *When something goes
   wrong*).
3. Double-click `strobe.exe`. Windows SmartScreen may warn about an
   unrecognised app the first time: choose *More info*, then *Run anyway*.

**What the machine needs:** a GPU with OpenGL 3.3 (a 2012 Intel HD 4000 or
newer) or, failing that, Direct3D 11. Strobe picks one by itself; see
*Graphics* below.

## First run

Strobe opens one window, the **control window**. The **output window**
(what the projector shows) starts hidden, so nothing appears on a second
screen until you ask.

- The first scene of the library plays in the preview, and the scene you
  last played is reopened on later launches.
- **Show output** in the toolbar, or **F11**, shows the output. Use the
  **Output monitor** menu to put it on the projector, then F11 for
  fullscreen.
- The left rail opens panes: **LIB** (the scene library), **LIST**
  (playlists), **EDIT** (the scene's code), **MAP** (MIDI, OSC, macros),
  **DIAG** (diagnostics) and **SET** (settings). The Controls pane on the
  right is the playing scene's controls.

### Keys

| Key | Does |
|---|---|
| Left, Right | Previous and next scene (or cue) |
| Space | Hold the cue (with a playlist) |
| B | Blackout on and off |
| T | Tap the tempo |
| F11 | Output fullscreen |
| Ctrl+F | Search the library |
| Ctrl+K | The command palette: every command by name |
| Esc | Close the top pane |

Settings > Keyboard changes any of them.

## Scenes

- **LIB** lists the scenes in the library folder: `scenes\` beside
  `strobe.exe` by default, or the folder given with `--library <dir>`.
  Search by name or tag; star favourites; double-click to play.
- A scene is a folder with `main.glsl` and `scene.json`, the Synesthesia
  scene format (`.synScene`). Synesthesia scenes you own can be played by
  pointing `--library` at their folder, or by dropping a scene folder on
  the window (it is copied into the library). Dropping a folder *of* scene
  folders adds them all without playing any; a banner says how many were
  added.
- **Needs work** (a LIB filter, and a list in DIAG) shows the scenes that
  failed to load this run, each with its reason.
- **Controls** shows the playing scene's sliders, colours and buttons.
  *Use frame as picture* saves the current frame as the scene's library
  picture.
- **Presets** (in the Controls pane): save the current look under a name,
  recall it later or from a controller. A preset saved while a picture is
  chosen for `syn_Media` remembers it.

## Audio

Scenes react to sound through the default **recording** device.

- **Settings > Audio** chooses the device. If none can be opened, Strobe
  uses a built-in 120 BPM pattern so scenes still move, and the title bar
  says *Synthetic audio*.
- **Windows privacy:** if the live device never appears, check
  *Settings > Privacy & security > Microphone*. *Microphone access* and
  *Let desktop apps access your microphone* must both be on.
- To react to what the computer plays rather than a microphone, enable
  *Stereo Mix* (Sound control panel > Recording, show disabled devices)
  or use a loopback device, and choose it in Settings > Audio.
- `--audio synthetic` forces the built-in pattern, for testing without
  sound.

## The output

- **Show output** / F11 as above. **Blackout** (B, or the button) darkens
  the output while the scene keeps running in the preview.
- **Settings > Performance** holds the render scale and adaptive
  resolution, which lowers the render size by itself when the GPU falls
  behind; `--no-adaptive` turns it off for a run.
- The output's look (brightness, hue, mirror, colour limits and the like)
  is in the Controls pane under *Output (meta)*. It carries over from scene
  to scene.

## Media behind a scene

Scenes that read `syn_Media` show a picture or a looping folder of
pictures (PNG or JPEG).

- In Controls, **Media > Choose** takes a file or folder path; *Scene's
  own* gives the scene its own back. Dropping a picture or a folder of
  pictures on the window does the same.
- A playlist cue can name its own media, and that wins for the cue.
- Very long sequences are shrunk to fit 192 MiB of memory rather than cut
  short.

## Playlists

A playlist is a TOML file listing scenes in sections, with how long each
plays and how it hands over. `playlists\example.toml` (in the source
repository) shows the format.

- **LIST > Load** opens one, or start with `--playlist <file>`, or drop
  the `.toml` file on the window. **Generate** builds a section from your
  library by tag and length.
- Select a cue to change its length, transition, preset or media; the
  card says where each setting comes from. Ctrl-click several cues to
  change them together. Drag a scene from LIB onto a cue to add it after.
- **Save** writes the file. Comments you wrote in it are kept.
- A playlist resumes where it was if Strobe is closed mid-show;
  `--no-resume` starts from the top.

## Controllers: MIDI and OSC

**MAP** connects MIDI devices and OSC (off by default; turn it on and
choose a port). *Learn* binds the next knob, fader, pad or OSC message to
whatever you click. **Macros** are named controls that keep working across
scenes; each can have a reach, a colour, a glide (smoothing) and a
keyboard key that drives it while held.

With OSC on, nothing needs mapping: `/strobe/control/<name>`,
`/strobe/cmd/next_scene` and the like work at once. A TouchOSC layout
made for Synesthesia also works for scene and meta controls (by name or as
`/controls/global/slider/1`), fav slots and playlist next, previous and
play; its other addresses are ignored.

## Importing from Shadertoy

The **Import** pane (via LIB) takes a Shadertoy URL or id, or an exported
`.json` file, and writes a scene into the library. Dropping a `.json` on
the window does the same.

**Licences are your responsibility.** Most Shadertoy shaders are
CC BY-NC-SA 3.0: no commercial use without the author's permission.
Strobe records each import's author and source in the scene; check them
before using a shader at a paid event.

*Generate controls* (Controls pane) measures a scene's numbers and writes
a copy with sliders for the ones that matter; your original is never
changed.

## Graphics

Strobe tries the GPU's OpenGL driver first and falls back to ANGLE
(Direct3D 11) by itself if it fails. You can choose:

- `strobe.exe --gl angle` forces ANGLE (try this first if scenes draw
  wrongly or the app will not start);
- `strobe.exe --gl native` forces OpenGL;
- `strobe.exe --diagnose` writes a report of the machine, GPU, driver and
  audio devices, for a bug report.

To run with a flag, make a shortcut to `strobe.exe` and add the flag at
the end of its *Target*.

## When something goes wrong

Everything Strobe writes lives in `%LOCALAPPDATA%\strobe`:

| Folder or file | What |
|---|---|
| `logs\` | A log per session, newest last. |
| `crashes\` | A report (`.txt`) and a minidump (`.dmp`) per crash. |
| `ui.toml` | Window and pane layout, theme, density, favourites. |
| `show-control.json` | MIDI and OSC settings, mappings, macros, keys. |
| `backups\` | The Editor's copies of files before it saved over them. |

- **The DIAG pane** shows the GPU, the backend, frame timing and recent
  failures with their reasons, without opening a file.
- **A scene fails to load:** a red banner names the error and offers *Open
  in Editor* at the line; the show keeps the previous scene.
- **Strobe stops:** a dialog names the log. The crash report and minidump
  are in `crashes\`. Send the `.txt`, the `.dmp` and the session log with
  a bug report; the maintainer turns the dump into function names with the
  matching symbols zip.
- **Black output on a laptop with two GPUs:** in Windows *Graphics
  settings*, set `strobe.exe` to *High performance*.
- **Choppy output with the control window beside it (NVIDIA):** Strobe
  detects this and adapts (the log says *the output now waits for its own
  swap*). Fullscreen output on the projector avoids it entirely.
- **More detail in the log:** set the environment variable `STROBE_LOG`
  to `debug` before starting (DEBUGGING.md explains the options).

## Where to read more

- [README.md](../README.md): what Strobe is, and building it from source.
- [DEBUGGING.md](DEBUGGING.md): logs, `STROBE_LOG`, GL debug output, crash
  symbolication.
- [ROADMAP.md](ROADMAP.md): what is built and what is next.
