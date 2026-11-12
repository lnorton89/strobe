---
name: scene-from-image
description: Make a Strobe .synScene from a photo of a painting or object, as a well-made, performable, audio-reactive recreation in the artwork's own style. Use when the owner hands over pictures and asks for shaders or scenes "of these", "based on this" or "inspired by" them. Covers reading the art, the shared GLSL template, controls and audio conventions, the render-and-critique loop with strobe-forge, and the quality gates.
---

# Scene from image

A repeatable way to turn a photographed artwork into a `.synScene`. It is the
method that made the scenes in `user-scenes/`, written down with its failures
fixed in advance (D562). Follow it step by step and keep the notes it asks for:
another agent, or the `strobe-forge make` loop, should reach a similar scene
from the same photo.

## Ground rules

- **Private by default.** Photos of friends' art and the scenes made from them
  stay in `user-scenes/`, which Strobe's `.gitignore` keeps out and which is
  the owner's own private repository. Scenes go in
  `user-scenes/<artist>/<slug>.synScene/` and their photos in
  `user-scenes/<artist>/refs/`. Never copy either into `scenes/`, a Strobe
  commit, a PR, a public page or a share link unless the owner says so for
  that piece (D526, D532). Commit to that repository only once the owner has
  seen the scene.
- **No licence stamp.** Leave `LICENSE` out of `scene.json`. `CREDIT` says
  "Strobe interpretation of <what it is>", and names the artist only if the
  owner named them.
- **From a worktree session**, the Write and Edit tools refuse paths outside
  the worktree, `user-scenes/` included. Draft in your scratchpad under
  names unique to your scene (builders running in parallel share it), and
  copy the folder across with the shell (`Copy-Item -Recurse -Force`).
- **Procedural, no reference media.** Never sample, trace or embed the photo.
  Shapes come from rules you invent that produce the same marks.
- **Judge it yourself.** Compare your render with the photo and list the
  differences concretely before calling it done. Never ask the owner what is
  wrong; ask only about intent.

## 1. Read the photo (write it down first)

Phone photos often carry an EXIF rotation: apply it before you crop or
measure. Lay a coordinate grid over a straightened crop of the artwork (a
small script into the scratchpad) and read positions and sizes off it in
frame-height units; it was the most useful step for placed heroes.

Before any code, write the reading as the header comment of `main.glsl`:

1. **What is the artwork, and what is room?** Walls, floors, frames, glare,
   reflections, other artworks in the shot. Name each and ignore it. If one
   photo holds several works, pick the one it is centred on and say so.
2. **Subject and composition:** what is depicted, where the mass sits, the
   axis or symmetry, how much of the frame the hero fills, the reading order.
3. **Materials:** paint pen, acrylic, chalk, puff paint, burned wood,
   driftwood, bone, shell, foil or holographic stickers, resin gloss. Each
   needs its own material model (see `reference/grammar.md`).
4. **Marks:** stroke widths relative to the frame height, taper, nib angle,
   hatching, dots, stars, spirals, how hand-made the line is (wobble,
   pressure, overshoot). Measure the stroke width: a too-thin stroke was the
   commonest rejection.
5. **Palette as paint, not as photographed:** take hue from the photo, but
   push value and saturation to what the paint is under a normal light. Note
   the substrate colour separately.
6. **Motion from meaning:** what in the subject could move (water rolls, a
   spring bounces, a sun sets, writing is written, a crab scuttles, hair
   drifts). Motion the still cannot show is where the scene earns its place.

## 2. Plan the scene

- **Archetype** (`reference/grammar.md`): grid, polar, axial stack, swept
  path, layered windows or scatter field. Name it.
- **Recompose to 16:9.** The hero fills most of the height and the sides are
  not dead: extend the substrate, add twins or a dimmed field of the same
  marks, repeat the plank, run the sea across. A portrait plank alone in the
  middle third was a rejection.
- **Controls** (`reference/controls.md`): the universal set, one verb `bang`,
  a layout `dropdown`, one colour control per paint, and the subject's own
  physical parameters, each with a description saying what zero means.
- **Audio:** kicks push integrated clocks (never jump a value), bass swells
  width and glow, mids sway or turn, highs twinkle, all times `reactivity`.

## 3. Build from the template

Copy `reference/template.glsl` and `reference/template.scene.json` into the
scene folder and grow them. The template already has the clock pass, `dt`
guard, value noise, the `paint`/`ink` compositing with halos that end inside
their bounds, analytic pixel size, the tone shoulder and vignette. Keep them:
they are the fixes for most past defects.

- One function per part (`sky`, `hero`, `border`, `field`), painter's order.
- Hash-driven variation per cell or slot so copies differ (`_hash12` only).
- Bounds tests before costly shapes, with a margin at least 6x the glow width.
- Cell lookups, not loops over every feature; no big constant arrays.
- No `fwidth`/`dFdx`; no `texture(` of anything but the clock.
- The engine predefines `PI` (and `_hash12`, `_uvc`, `_xy`, `RENDERSIZE`,
  `TIME`); defining `PI` again is a compile error. Use `TAU` from the template.
- GLSL ES reserves more words than desktop GLSL: never name anything
  `patch`, `sample`, `input`, `output`, `filter`, `common`, `partition` or
  `active`.
- Dropdown values arrive as floats: compare with `floor(x + 0.5)`.
- `smoothstep(e0, e1, x)` is undefined when `e0 >= e1`; write
  `1.0 - smoothstep(e1, e0, x)` instead.
- Bangs: use the template's `bangPulse()`. Detecting a smooth counter at its
  half step lags about 0.3 s.
- Dark ink on a light ground (white boards, paper): the halo must darken,
  so mix toward the ink rather than adding light.
- ANGLE inlines every call site: five calls to a big `totem()` took 417 s
  to compile on D3D11. Per pixel, find the nearest copy or slot and call the
  drawing function once.
- Fade an old glyph or mark out by alpha while the new one is drawn over it;
  shrinking its width leaves hairline webs and empty slots.
- Neighbour-cell culling and band tests must include `GLOW_REACH` and the
  mark's own reach, or seams appear.
- High-frequency `vnoise` (above about 200 cycles a frame) shows 2x2 blocks;
  use a per-cell hash for stipple and grain.

## 4. Check, look, critique, repeat

Render with the forge (D562):

```bash
cargo run --profile release-fast -p strobe-forge -- check user-scenes/<artist>/<slug>.synScene --reference user-scenes/<artist>/refs/<slug>.jpg
```

That writes `target/forge/<slug>/`: frames at 2, 7, 15, 40 and 300 s, a sheet
with the photo beside them, every control at its extremes, and `report.md`
with the gates. If Windows refuses with "Access denied" because another
forge is running, call `target/release-fast/strobe-forge.exe` directly. The
bench renders frames too, but cannot set controls, so only the forge checks
the extremes:

```bash
cargo run --profile release-fast -p strobe-bench -- --library user-scenes/<artist> --filter <slug> --size 1280x720 --start 7 --out target/bench/<slug>
```

Timings are meaningless while other jobs use the GPU (parallel builders
saw 1.5 ms read as 30 ms). Time on a quiet GPU, or take the lowest of
several runs, and say which. A "does little" warning on a control that only
touches a small feature (a pupil, a foil disc, sparkles) is acceptable if you
can see the change in the controls grid; say so in your report.

Open the frames with the Read tool and go through `reference/critique.md`
against the photo. Write the differences down as a list, fix the worst three,
render again. At least two rounds; stop when the list holds only things you
chose on purpose. Typical first-render faults: hero too small, strokes too
thin, everything too regular, neon or plastic look, glow clipped into
rectangles, colours drifted from the paint.

## 5. Gates before you hand it over

- Compiles in both dialects (desktop 330 and GLSL ES; the forge does both).
- No black, flat, blown-out or frozen frame; passes the flash rule.
- Under 2.5 ms a frame at 1920x1080 on LIGHTNING's RX 580 (the forge's
  timing), or a stated reason.
- Every control visibly changes the image between its extremes, and no
  extreme breaks it. Cap `weight` where fat strokes start merging into blobs.
- In write-and-wipe scenes, the hero is never blank: at every sampled time
  it is at least about 40% drawn, and only one part is mid-wipe at once.
- The header comment holds the reading, the archetype and what moves.

Report back with: the slug, the reading in two lines, the archetype, the
controls, the timing, the last critique list and anything left undone.

## 6. Share it, only when asked

A share link publishes the scene, so make one only when the owner asks for
that piece (see the ground rules). Give the short `/s/<id>` link, never the
long `#s=` fragment link: the fragment carries the whole scene and runs to
about 10 KB, and handing one over was a rejection.

POST the scene to the site's Worker as the sandbox's Share button does
(D527). It stores the scene by content hash and answers `201` with
`{id, url}`:

```bash
curl -s -X POST https://strobe.lnorton89.workers.dev/api/share \
  -H 'content-type: application/json' --data-binary @share.json
```

`share.json` is `{"title": "<TITLE>", "files": {"main.glsl": "…",
"scene.json": "…"}}`, with each file's text as a JSON string; build it with a
script rather than by hand. The limit is 256 KB and 16 files
(`site/worker/scene-store.ts`). Open the returned `url` in a browser and
wait for the status line to say Compiled before reporting it. Save the link
next to the scene as `user-scenes/<artist>/<slug>-share-link.txt`. Use the
fragment link only when the Worker cannot be reached, and say so.
