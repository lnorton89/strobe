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

<!-- @performance -->

Strobe's floor is OpenGL 3.3 core, the level of a 2012 Intel HD 4000. The slowest
GPUs it has been measured on are an Intel UHD 620 and an AMD Vega 3; no HD 4000
has been tested yet.

<!-- @performance-items -->

### The frame loop

One thread owns the GL context. Audio, MIDI, OSC and the remote run on workers
that hand values over through bounded queues the loop never waits on. Per-frame
paths do not allocate or query the driver, and a render test fails if a state
query runs while a scene plays.

### Adaptive resolution

The loop reads a GPU timer from an earlier frame and lowers the render scale when
the average goes over budget, or when about 4% of frames miss their refresh for
four seconds. The second rule exists because a mean under budget is not a smooth
show: on a Vega 3, scenes at 12.4 to 13.8 ms still missed about one refresh in
eleven.

### ANGLE fallback

If native GL 3.3 will not start, Strobe opens OpenGL ES 3.0 through ANGLE on
Direct3D 11, the path Chrome uses. Everything above the context is the same code,
and on the T480 ANGLE's frame times were within 5 to 10% of native.

### When something fails

A lost GL context is rebuilt and the show carries on. A script gets 4 ms a frame,
so a `while(true){}` is interrupted and the scene keeps rendering. A crash leaves
a report and a minidump that `cargo xtask symbolicate` turns into a stack with
file and line.

<!-- @performance-source -->

Mean milliseconds per frame, timed with `glFinish` over 60 frames, on a ThinkPad
T480 (Intel UHD 620) through native OpenGL. The tick is the 16.7 ms budget of a
60 Hz refresh. Volume Clouds is the stress scene at the top of the load ladder and
does not run at this GPU's frame rate at any scale.

<!-- @scenes -->

The library is original work. Ten scenes span five tiers of GPU load, from a
drifting gradient to raymarched clouds, and they are what `strobe-bench` times on
every machine. The rest are a scripted physics scene, a fictional circuit board
and five audio-reactive show pieces. Open any of them in the browser sandbox,
which runs Strobe's own player as WebAssembly.

<!-- @compare-note -->

From the README

<!-- @compare-disclaimer -->

Strobe is an independent project. It is not made by, endorsed by or affiliated
with Synesthesia's makers, and it ships none of their scenes.

<!-- @status-title -->

It is for testing, not yet for production.

<!-- @status -->

Windows is the target. It has run on a ThinkPad T480 (Intel UHD 620), desktops
with a GTX 1080 Ti and an RX 580, and a venue mini PC (Ryzen 3 3200U, Vega 3)
driving a projector. Linux builds and runs on X11 and Wayland, so far on Mesa's
software renderer. macOS does not run yet.

<!-- @status-unverified -->

Not verified yet: MIDI hardware, a live Shadertoy import, phones, and an HD 4000.

<!-- @licence -->

Source-available: MIT with the Commons Clause and an added permission. You may
use, change and share Strobe and run it for pay. You may not sell Strobe itself.
