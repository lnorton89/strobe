<!-- @tagline -->

Audio-reactive shaders for screens, projectors and LED walls.

<!-- @hero-why -->

Strobe is an open-source VJ engine in Rust. It plays **Synesthesia scenes as
they are** — GLSL, controls and JavaScript — imports **Shadertoy** shaders,
and runs long shows from playlists. It is built to stay up on old laptops:
the floor is **OpenGL 3.3**, roughly a 2012 Intel HD 4000.

<!-- @hero-proof -->

The control window at 1440x900, playing Script Orbits: the live preview, the
scene's controls in the side panel, and the transport, macros and BLACKOUT
along the bottom.

<!-- @features-note -->

From the README, in practical terms

<!-- @features -->

### Plays Synesthesia scenes

Reads `main.glsl`, `scene.json` and `script.js` unchanged: multipass, the SSF
function library, every control type, images, media, and the JavaScript API.

### Imports Shadertoy

A "Send to Strobe" userscript posts a shader to the app. Buffers become
passes, the audio channel becomes your live sound, and the licence is kept.

### Generates controls

Finds the literals that shape a scene and turns them into controls, keeping
only those that measurably change the picture. The original is never touched.

### Runs long shows

Playlists in hand-writable TOML with inherited settings. Every scene is
compiled before it plays, the next one warms up off screen, and a crash resumes.

### Takes show control

MIDI and OSC in with learn, soft takeover and curves; OSC feedback out;
Performance Macros, presets and eight fav slots; rebindable keys.

### Remote from a phone

Pair a phone or another Strobe desktop over the LAN, with scoped pairing and
revocation, and an unattended venue mode for show machines.

### A performer-first UI

The live preview with the transport underneath, a command palette, a scene
editor that keeps the last good scene playing, and tag search with thumbnails.

### Stays up on bad hardware

Adaptive resolution, an ANGLE fallback when the GL driver fails, context-loss
recovery, runaway scripts stopped within a frame budget, and crash minidumps.

<!-- @scenes-note -->

Rendered from scenes/ at build time

<!-- @scenes -->

Strobe ships with an original scene library spanning five tiers of GPU load,
from a drifting gradient to raymarched clouds, plus a scripted physics scene
and a set of feedback-driven paint studies. Every thumbnail below is a real
frame from the scene it names.

<!-- @compare-note -->

Where the two differ

<!-- @numbers-note -->

Real-world results, with the evidence in the repository

<!-- @status-title -->

v0.1.0 is the first release. It is for testing, not production.

<!-- @status -->

The engine, the performer UI, playlists, show control and Shadertoy import are
built and tested. Windows is the target and has been checked on real Intel,
NVIDIA and AMD machines; Linux builds and runs on X11 and Wayland; macOS does
not run yet.
What is verified, and where, is tracked in the compatibility matrix.
