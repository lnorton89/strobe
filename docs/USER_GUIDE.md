# Strobe user guide

This is for someone running Strobe, not building it. You do not need Rust
or a compiler. Everything here applies to the Windows release; the
command-line flags work the same on Linux, where there is no download yet and
you build Strobe from source (README, "Linux"; settings and logs are in
`~/.local/share/strobe`). For logs in depth, see
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
- **Show output** (the monitor button under the panel's preview, or the
  PLAY view's toolbar), or **F11**, shows the output. Use the
  **Output monitor** menu to put it on the projector, then F11 for
  fullscreen.
- The window is Synesthesia's three columns. The **rail** on the left
  switches the main view: **PLAY** (the live preview at full size),
  **LIB** (the scene library, the default, with its **Scenes** and
  **Controls** pages), **GET** (import a Shadertoy
  shader), **LIST** (playlists), **EDIT** (the scene's code), **MAP** (MIDI,
  OSC, macros), **DIAG** (diagnostics) and **SET** (settings). The **panel**
  on the right stays whatever the view is: the live preview, a status strip
  (output, fullscreen, frame rate, audio level and tempo, and a snapshot
  button), the eight fav slots, and four tabs: **Playlist** (the loaded
  show's cues), **Controls** (the playing scene's controls), **Presets**
  (the scene's saved looks) and **Edit** (tags, picture, media,
  *Generate controls*, *Open in Editor*). Drag its edge to resize it;
  until you do it is about a third of the window, and a double-click on
  the edge puts it back to that. In a short window the fav slots move into
  the Presets tab and the panel drops its preview. In a narrow window it opens
  over the main view, and when closed it is a thin strip you click to open.
  **BLACKOUT** and the macros are always on the bottom strip, and the bar
  under it says what the control under the pointer does.

### Keys

| Key | Does |
|---|---|
| Left, Right | Previous and next scene (or cue) |
| Space | Hold the cue (with a playlist) |
| B | Blackout on and off |
| T | Tap the tempo |
| F11 | Output fullscreen |
| Ctrl+F | Search the library |
| Ctrl+K | The command palette: every command, pane, scene and preset by name |
| Ctrl+L | Switch LIB between Scenes and the live scene's Controls |
| Esc | Close the top pane |

Settings > Keyboard changes any of them.

## Scenes

- **LIB** lists the scenes in the library folder: `scenes\` beside
  `strobe.exe` by default, or the folder given with `--library <dir>`.
  The toolbar has *New scene* (it opens Import), a rescan and the S, M and
  L thumbnail sizes. The row under it has the count ("88/88"), the search
  (names, authors and tags), a sort (library order, name, author,
  favourites first) with a direction arrow, a star that shows only your
  favourites, and a menu that shows only scenes that loaded or that need
  work. Each card shows the picture, the title, the author and a **...**
  menu (also the right-click menu); star favourites. **Click a card to
  play it**, as in Synesthesia: it says PREPARING while it compiles, and if
  it fails the scene that was playing carries on and the card says why. A
  double-click is still one load. With the keyboard, Tab to a card and
  press Enter, or type in the search and press Enter. *Preview without
  loading* (in the **...** menu, or *Preview:* in Ctrl+K) shows a scene
  above the grid without changing what is live. The clicked scene's tags
  show above the grid, and *Edit* changes them. The **Edit** tab does the same for the playing
  scene, and the Import pane for the scene it just imported.
- A scene is a folder with `main.glsl` and `scene.json`, the Synesthesia
  scene format (`.synScene`). Synesthesia scenes you own can be played by
  pointing `--library` at their folder, or by dropping a scene folder on
  the window (it is copied into the library). Dropping a folder *of* scene
  folders adds them all without playing any; a banner says how many were
  added. **Settings > Watched folder** does the same for scene folders
  copied into a folder you choose while Strobe runs: type the folder,
  *Apply*, and the switch turns it off and on. It is remembered, and it
  cannot be the library folder itself. `strobe.exe --watch-folder
  <folder>` watches one for that run only.
- **Synesthesia's scenes.** If Synesthesia is installed on this computer,
  LIB also lists its scenes after your own, each marked *Synesthesia*:
  - the scenes it ships (in `C:\Program Files (x86)\Synesthesia\release`
    and `default`). These are **read-only**: Strobe never changes
    Synesthesia's files. Tagging one, *Use frame as picture*, saving it in
    EDIT or saving a preset for it says so and offers **Copy to my
    library**, which copies the scene into your library folder (under a
    new name if you already have one called that). The copy is yours to
    change, and if it was playing it carries on from the copy;
  - the scene folders you added in Synesthesia's own Settings (Custom
    Scene Folder). Those are yours, so tags and edits work as usual.

  Search for `synesthesia` to see only them. A scene that is in both your
  library and Synesthesia's shows twice; `--scene`, playlists and fav slots
  use your own. To hide them, untick **Settings > Synesthesia's scenes >
  Show Synesthesia's scenes** (remembered), or start with
  `strobe.exe --no-synesthesia` for one run. Strobe ships none of
  Synesthesia's files: it only reads your own install, the same way it
  finds Synesthesia's `lib` folder for scenes that `#include` it. If
  Synesthesia is somewhere unusual, set `STROBE_SYNESTHESIA_DIR` to its
  folder (and `STROBE_SYNESTHESIA_DATA` to its data folder,
  normally `%LOCALAPPDATA%\Synesthesia\Data`).
- **Needs work** (a LIB filter, and a list in DIAG) shows the scenes that
  failed to load this run, each with its reason.
- **Controls** shows the playing scene's sliders, colours and buttons, in
  two places: large, as LIB's **Controls** page (the switch at the top of
  LIB, or Ctrl+L, flips between it and the scenes), and compact, as a tab
  of the panel for when another view is up. While the Controls page is up
  the panel shows Presets beside it rather than the same controls twice.
  The camera button in the status strip, or *Use frame as picture* in the
  Edit tab, saves the current frame as the scene's library picture.
- **Presets** (a tab of the panel): save the current look under a name,
  recall it later or from a controller. A preset saved while a picture is
  chosen for `syn_Media` remembers it.
- **EDIT** opens the scene's files. *Save* writes them and the show keeps
  what it has; *Save and reload* writes them and loads the scene again.
  If `scene.json` would not load, *Save and reload* writes nothing and
  shows the error with a button to its line and column.

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
- `--audio ambient` is the built-in pattern with no beat: hits and beat
  triggers stay at zero and the levels swell slowly, so scenes keep moving
  without flashing on a beat the room cannot hear. Use it for a show with
  no microphone.

## The output

- **Show output** / F11 as above. **Blackout** (B, or the button) darkens
  the output while the scene keeps running in the preview.
- For a machine that starts Strobe by itself (a venue PC), put the output
  on the projector from the command line:
  `strobe --output-monitor 2 --fullscreen`. Monitors are numbered as the
  **Output monitor** menu numbers them; `--fullscreen` alone uses the
  monitor the output opens on.
- **Settings > Performance** holds the render scale and adaptive
  resolution, which lowers the render size by itself when the GPU falls
  behind; `--no-adaptive` turns it off for a run.
- The output's look (brightness, hue, mirror, colour limits and the like)
  is under *Output (meta)* in Controls (the page or the panel's tab). It
  carries over from scene to scene.

## Media behind a scene

Scenes that read `syn_Media` show a picture or a looping folder of
pictures (PNG or JPEG).

- In the panel's Edit tab, **Media > Choose** takes a file or folder
  path; *Scene's own* gives the scene its own back. Dropping a picture or a folder of
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
  library by tag and length. Its *Exclude* field leaves out scenes with a
  tag (`heavy`) or a name, and the preview says which were skipped.
- Select a cue to change its length, transition, preset or media; the
  card says where each setting comes from. Ctrl-click several cues to
  change them together. Drag a scene from LIB onto a cue to add it after.
- A cue can also end on the music: `advance = "audio-drop"`,
  `"audio-breakdown"` or `"audio-song"` (the next track started: a gap
  between tracks, or a new tempo), alone or in a list such as
  `["timer", "audio-song"]` for whichever comes first. They need live
  audio. A DJ mix that keeps one tempo with no gap gives no song change.
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

## Remote control from a phone

A phone or tablet on the same Wi-Fi can run the show: Next and Previous,
Hold, Restart, +30 s, Skip section, Tap tempo and Blackout, with the
current and next scene and the cue's time left. It is a web page that
Strobe serves itself; there is nothing to install, and no account or
internet connection is involved. **It has not yet been tried on real
phones**; please report what you find.

**It is off until you switch it on**, and it never switches itself on at
launch.

1. Open **SET** (Settings), find **Remote**, and press **Start remote**. Leave
   *Address* empty to use this computer's address on its network, or type
   one of its own addresses. Strobe refuses a public internet address.
   To start it with Strobe instead, add `--remote` to the shortcut's
   *Target* (and `--remote-bind 192.168.1.20` or `--remote-port 47812` if
   you need them).
2. **Windows Firewall** asks, the first time, whether Strobe may accept
   connections. Allow it on **private networks only**, never public ones.
   If you dismissed the prompt, allow `strobe.exe` for private networks in
   *Windows Security > Firewall & network protection > Allow an app through
   firewall*. Your network must also be set to *Private* in Windows'
   network settings.
3. The section shows an address such as `http://192.168.1.20:47812/`.
   Type it into the phone's browser exactly as shown: a name instead of
   the numbers is refused, on purpose.
4. Press **Pair a device**. A six-digit code shows for two minutes. Type it
   on the phone and press *Pair*.
5. Strobe shows *"iPhone at 192.168.1.23 wants to control the show"*.
   Press **Approve** if that is your phone, **Deny** if it is not. The phone
   is paired and keeps working after both restart; it does not need the
   code again.

**The title bar shows "Remote on"** with the number of connected phones
whenever it is listening, and a **Stop remote** button beside it. Stop
drops every phone at once. **Revoke** (or **Revoke all**) under *Paired
devices* makes a phone pair again; a connected one is dropped at once.
Revoking works with the remote off too.

On the phone, Blackout sends "black out" or "bring it back" according to
what the screen last showed, never a plain toggle. If the connection
drops (the phone slept, the Wi-Fi roamed) the page says so, disables its
buttons and reconnects by itself. *Forget this device* on the page
removes its pairing from that browser.

**Use it only on a network you trust.** The remote is plain HTTP, not
HTTPS: someone on the same Wi-Fi who is watching the traffic can see the
pairing code as you type it and the phone's key as it is issued, and could
then control the show until you revoke it. A home or venue network you
control is fine; a public or shared Wi-Fi is not. There is no installable
app (PWA) version yet, because that needs HTTPS.

### Control another Strobe desktop

Both computers must run a build with desktop control support. On the host,
open **Settings > Remote**, click **Start remote**, enable **Allow full
desktop control**, then click **Pair a device**. On the controlling computer,
open **Settings > Remote > Control another Strobe desktop**. Enter the host's
IP and port (for example `192.168.1.20:47812`) and its pairing code, then
click **Connect to desktop**. Approve the request labelled **Full desktop**
on the host.

The host's entire Strobe control window replaces the controller's local UI.
Click in the image to focus it, then use its controls, keyboard shortcuts,
editor, playlist, presets and file operations as usual. All edits, imports
and file paths refer to the **host computer**. Clipboard text can be pasted
into the host; Copy/Cut in a host widget returns selected text to the
controller. Paste at most 64 KiB at a time. Local file drops/uploads and
operating-system dialogs are outside this Strobe window stream.

**Return to local Strobe** always belongs to the controlling computer;
Ctrl+Shift+F12 is its keyboard shortcut (Command+Shift+F12 on macOS).
Only one desktop can control a host at once. The host title bar says
**Desktop access on**, and **Desktop controlled remotely** (with a warning
sign) while a controller holds the controls; **Stop remote**, disabling full
desktop access, or revoking the controlling device disconnects it and
releases held inputs. Existing phone tokens retain performance-only access,
even while desktop access is enabled. Desktop approval is a separate
permission.

A controller can do anything in the host's Strobe window except widen who
can reach the host: pairing or approving devices, starting a listener,
switching desktop access on, changing OSC and connecting the host to another
desktop are refused from the remote and must be done at the host itself.
Treat a full desktop approval like handing someone the host's keyboard: it
includes editing and saving scenes, typing host file paths and quitting.

To reconnect during the same controller app session, leave the code blank
and connect to the same IP and port. The credential is rotated on each
connection and kept only in memory. After restarting the controller, pair
again. On the host, a desktop pairing is never saved: switching full desktop
access off, **Stop remote** or restarting Strobe revokes every desktop
pairing, so each new session is approved again. Phone pairings are kept.

The native client uses a private IP connection on the trusted LAN; it does
not use the HTTPS proxy described below. The control window stream is
limited to five frames per second and 4096 pixels per edge. Keep the host
control window open; resize it if capture exceeds this limit. Input is
disabled while frames are stale, and queued input is never replayed after
reconnecting. Rendering the projector output continues on the host.

### Venue mode: an unattended show machine

Normally pairing a desktop needs someone to click **Approve** at the host.
At a venue the show machine may have no screen, so venue mode replaces the
click with a **trust file**: a small text file holding a secret. A controller
that proves it has the same file is paired for full desktop control, with no
approval.

**1. Make the file, once, on any computer you own.**

```text
strobe --make-remote-trust "C:\Users\you\venue.trust"
```

Strobe writes 32 random bytes as hex, tells you what to do next, and refuses
to overwrite a file that is already there. Keep it in your user profile (the
default permissions on your profile folder let only you and administrators
read it) and never post it, email it or commit it. On Linux and macOS the
file is created readable by you only, and Strobe will not use one that other
users can read.

**2. Copy it to the other computer.** Use a USB stick or a copy over a
channel you trust, and delete the stick's copy afterwards. Both computers
use the same file.

**3. Start the show machine** (start-up shortcut or script):

```text
strobe --remote --allow-desktop-control --remote-trust "C:\Users\you\venue.trust" --fullscreen
```

`--remote` starts the listener (it retries every few seconds if the network
is not up yet at boot); `--allow-desktop-control` switches **Allow full
desktop control** on for this run; `--remote-trust` names the file, which is
read once at launch. If the file cannot be used, Strobe says so in its log and
carries on without trusted pairing, so the show still starts. Nothing here is
saved: after a restart the switches come from the command line again.

**4. The laptop.** Copy the trust file to the laptop (step 2), then start
Strobe there with the show machine's address:

```text
strobe --control 192.168.1.50:47812 --remote-trust "C:\Users\you\venue.trust"
```

`192.168.1.50` is the show machine's address, shown in its **Settings >
Remote** (and in its log); the port is `47812` unless you started it with
`--remote-port`. A bare IP takes `47812`. The laptop opens straight into the
show machine's window, with no click anywhere, and reads the file when it
connects. `--control` is for the laptop only: it cannot be combined with
`--remote`. Without `--remote-trust`, `--control HOST:PORT` opens
**Settings > Control another Strobe** with the address filled in, for pairing
with a code.

Or, from the running laptop, open **Settings > Control another Strobe**, type
the address, tick **Use a trust file (venue mode)**, type or paste the file's
path and press **Connect to desktop**. Strobe remembers the path (never the
file's contents) and the choice for next time.

**If the connection drops, the laptop reconnects by itself.** After a host
restart, a Wi-Fi blip or the show machine booting later than the laptop, the
viewer keeps the last image dimmed and says **Reconnecting... (attempt N)**
and why. It tries again after 1 s, 2 s, 4 s, 8 s and then every 15 s, for as
long as it takes, and takes the controls back when the host answers. Press
**Disconnect** to stop. If the host says no, it stops and says why instead:
the file is not the one the host was started with, the controller was
revoked at the host, or the host was not started with `--remote
--allow-desktop-control --remote-trust FILE`. **Reconnect** tries the file
again once you have fixed it. (Pairing by code keeps its manual **Reconnect**.)

What you see at the host: the title bar says **Desktop controlled remotely**
while the laptop is in control, and **Settings > Remote** lists the laptop as
*Full desktop: trusted ...* with a *trusted* mark. **Revoke** there ends that
session and refuses that controller until Strobe restarts. **Revoke all**, or
switching **Allow full desktop control** off, shuts the trust file out;
switching it off ends every desktop session at once, and trusted pairing is
refused while it is off.

**What it protects, and what it does not.**

- **Whoever has the file has full control** of the show machine: scenes,
  files, quitting. Treat it like a key. If it may have leaked, make a new one
  and relaunch both computers.
- **LAN only, plain HTTP.** The secret is never sent. The laptop answers a
  fresh random challenge with a keyed hash that is good once, for that host
  only, so someone watching the network cannot learn the secret or replay an
  earlier answer. But the picture and your clicks and typing are not
  encrypted, and someone able to sit between the two machines on your
  network could hijack a session. Use a network you control, not a public one.
- Wrong guesses are slowed and logged: a few free tries per address, then
  pauses that grow to five minutes, and a shared pause if many addresses fail
  at once. A flood on the network can therefore delay a *new* connection for a
  while; a session already open is not affected.
- Keep the file out of any folder Strobe writes scenes to: a controller with
  the full UI can save files on the host.

### HTTPS through your own proxy (advanced)

Strobe has no certificate of its own, and it will never ask you to click
past a browser's security warning. If you own a domain name, you can put
the remote behind HTTPS yourself, with a reverse proxy on the same
computer. That encrypts the traffic, so the pairing code and the phone's
key can no longer be read off the Wi-Fi. **This has not been tried on real
phones yet.**

1. In your domain's DNS, point a name at this computer's LAN address, for
   example `strobe.example.com` to `192.168.1.20`. Some routers refuse to
   answer a public name with a private address ("DNS rebinding
   protection"); allow your name there if the phone cannot find it.
2. Run a reverse proxy that gets a real certificate for that name.
   [Caddy](https://caddyserver.com) with a DNS-provider plugin can do it,
   since a LAN-only address cannot pass the usual web challenge. Its
   configuration is one block:

   ```
   strobe.example.com {
       reverse_proxy 127.0.0.1:47812
   }
   ```

3. Start Strobe with `--remote --remote-proxy strobe.example.com`. With no
   `--remote-bind`, the remote then listens on `127.0.0.1` only, where the
   proxy reaches it. Strobe accepts that name only from this computer,
   which means only from the proxy. It accepts the socket only from the
   page at `https://strobe.example.com`.
4. Open `https://strobe.example.com` on the phone and pair as above.

The proxy, its certificate and its renewals are yours to run: Strobe
does not run them or check them.

## Importing from Shadertoy

The **Import** pane (via LIB) takes a Shadertoy URL or id, or an exported
`.json` file, and writes a scene into the library. Dropping a `.json` on
the window does the same.

**Licences are your responsibility.** Most Shadertoy shaders are
CC BY-NC-SA 3.0: no commercial use without the author's permission.
Strobe records each import's author and source in the scene; check them
before using a shader at a paid event.

*Generate controls* (the Edit tab) measures a scene's numbers and writes
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
| `remote-clients.json` | Phones paired with the remote (no usable keys: only their fingerprints). Deleting it un-pairs every phone. |
| `backups\` | The Editor's copies of files before it saved over them. |

- **The DIAG pane** shows the GPU, the backend, frame timing and recent
  failures with their reasons, without opening a file.
- **A scene fails to load:** a red banner names the error and offers *Open
  in Editor* at the line (and the column, on drivers that give one); the
  show keeps the previous scene.
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
- [COMPATIBILITY.md](COMPATIBILITY.md): what works, what is degraded and
  what is not, with evidence.
