<!-- @tagline -->

A VJ engine for audio-reactive GLSL scenes.

<!-- @hero-why -->

Strobe plays Synesthesia scenes unchanged, including their controls and
JavaScript, imports Shadertoy shaders, and runs shows from playlists. It is
written in Rust and draws through OpenGL 3.3, with ANGLE on Direct3D 11 for
machines whose GL driver fails.

<!-- @hero-proof -->

The control window at 1440x900, playing Electric Bloom: the scene library, the
live preview and its controls, and the transport, macros and BLACKOUT along the
bottom.

<!-- @features -->

### Scenes

Reads a `.synScene` folder as it is: `main.glsl`, `scene.json` and `script.js`.
That covers multipass, the SSF function library, every control type, images,
cube maps, media, and the JavaScript API under QuickJS. Of 65 real Synesthesia
scenes run through the test harness on NVIDIA, 63 play with their scripts; the
other two have bugs of their own.

### Shadertoy import

A "Send to Strobe" userscript posts a shader from its page to the app, which
converts it into an ordinary scene folder. Buffers A to D become passes, the
audio channel takes your live sound, and the source, author and licence are
recorded. The converter is tested with fixtures; a live import from the site has
not been verified yet.

### Generated controls

For a scene with no controls, Strobe finds the literals that shape it (colours,
scales, speeds, loop counts), renders with each one changed, and keeps those
that measurably change the picture. It runs in a worker process while the show
plays, and saving backs up the files it replaces.

### Playlists

A show is TOML you can write by hand. A cue that names only a scene inherits its
duration, advance and transition from its section and the playlist, and a seeded
generator can build a section from scene tags. Every scene is compiled before
the show starts, the next one warms up off screen, and a show resumes after a
crash.

### Show control

MIDI and OSC in, with learn, soft takeover and curves, and OSC feedback out for
motorised faders and TouchOSC. Macros, presets, eight fav slots and rebindable
keys. The transport and BLACKOUT stay in one place in every view. MIDI has been
tested in software and not yet with hardware.

### LAN remote

Opt-in, for trusted networks. A phone opens the address Settings shows, gives
itself a name, and the laptop approves it. Another Strobe desktop can ask the same
way, and venue mode uses a trust file so a laptop reconnects to a show machine by
itself.

<!-- @scenes-note -->

Rendered from scenes/ at build time

<!-- @scenes -->

Strobe ships with an original scene library spanning five tiers of GPU load,
from a drifting gradient to raymarched clouds, plus a scripted physics scene
and a set of feedback-driven paint studies. Every thumbnail below is a real
frame from the scene it names.

<!-- @compare-note -->

Where the two differ

<!-- @status-title -->

It is for testing, not yet for production.

<!-- @status -->

The engine, the performer UI, playlists, show control, Shadertoy import and the
LAN remote are built and tested. The current release rebuilt the app's UI,
added approve-only pairing and venue mode for controlling a show machine, and
lets you choose ANGLE's backend when a GL driver misbehaves. Windows is the
target and has been checked on real Intel, NVIDIA and AMD machines; Linux
builds and runs on X11 and Wayland; macOS does not run yet.
What is verified, and where, is tracked in the compatibility matrix.
