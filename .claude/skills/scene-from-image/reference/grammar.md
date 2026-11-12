# The shared grammar

Every image scene so far was built from the same parts. Choose from these
before inventing new ones.

## Layout archetypes

| Archetype | When | How | 16:9 filler |
|---|---|---|---|
| Grid | Tiles, panels, a board of glyphs | Whole cells; rows chosen, columns from aspect; each cell hashes to its content | More cells |
| Polar | Mandalas, wheels, rosettes | Bands of near-square cells, each band with its own turn | Side fields of dimmed marks, twins |
| Axial | Totems, staffs, trunks, figures standing | One vertical axis, slots down it, mirror with `abs(x)` where the art is symmetric | Repeat the plank, twins, extend the substrate |
| Path | Serpents, rivers, vines, tentacles | Walk segments and arcs once to get arc length `s` and signed cross distance `t`; draw in `(s, t)` | Let the path meander across the width |
| Windows | Several scenes in one painting | Hand-placed rectangles, each a small scene | Extend the outer ones |
| Field | Scattered motifs with no structure | Jittered cells; each picks a motif by hash and weight | Natural |

A figurative piece (a face, a figure, a creature) is usually axial with a
hand-placed hero: place its parts in normalised painting space from the
photo, then add the rule-driven parts (hair, fins, ruffs, stars) around it.

## Recipes found while building

- **Paths in `(s, t)`**: return the true distance past each end as well as
  the cross distance, and an "inside the limb" flag. Without them, ribs,
  spines and hatching smear into halos past a limb's tip.
- **Outlines that echo a shape** (a line following every black edge at a
  gap, then running on as a channel): take the smoothed union of all the
  shape's paths, draw lines at offsets of that distance, and cap them by arc
  length where they run on.
- **Meanders from noise contours** (maze channels, roots): use the iso-lines
  of a low-frequency noise at two levels, distance by value over gradient,
  with a floor on the gradient so saddles don't blob. Raise the
  low-frequency share to get long joined meanders instead of small loops.
- **Polar hair, petals and flames**: build them from the gaps between
  tufts (angle cells, one tuft per cell, shaped within it), or seams appear
  where neighbouring tufts overlap.
- **Helical ridges** (shells, screws, twisted horns): the ridge coordinate is
  `y - k * x` inside the body, so each lip runs diagonally and passes
  behind the flank. Level ridges read as a pagoda or a Christmas tree.

## Marks

Segment, tapered segment, arc, ring, spiral, hook, crescent, 4- and 8-point
star, comb, hatch, ladder, zigzag, wave, dot row, box, diamond, cross, flame
lobe, rays, lightning bolt. Combine them into the artwork's own signs.
Writing (glyphs, sigils, runes) is a stroke grammar on a small lattice:
weighted stroke types, each glyph hashed from its cell and a generation
counter, with a nib model (width depends on stroke direction) when the
original is calligraphic.

## The hand

- Domain wobble from a noise gradient (`wobble` control).
- Pen pressure: width varies along the stroke and across the frame.
- Overshoot and gaps at joins for paint pen and brush.
- Hash jitter in size, angle and spacing so no two copies match.

## Materials

| Material | Model |
|---|---|
| Paint pen or acrylic on a dark ground | Flat colour, small even halo, faint coat texture |
| Fluorescent under UV | `mix(day, colour, uv)` with a violet room cast and a stronger halo |
| Chalk or pastel | Broken coverage: threshold a fine noise across the stroke, grain along it |
| Puff paint, raised gold | Bead normal from the stroke's cross-section, lit by a moving rake light |
| Holographic foil or stickers | Rainbow from the angle between a moving light and a noise normal |
| Burned or carved wood | Darkened, slightly sunk strokes; grain lines bending round knots |
| Bone, shell, driftwood | Pale warm base, soft shading from a distance-based normal, pits and grain |
| Gloss resin | A broad sheen and a narrow streak drifting across, over everything |

Matte paintings want little glow (default 0.3-0.5). Neon tubes, plastic
bevels and bronze-looking gold were all rejections.

## Motion devices

Write, hold and unwrite with a glint; band rotation; slide along a path;
wave phase roll; rake-light sweep; damped spring (integrated in the clock);
twinkle; a beat ripple along an axis (`fract(syn_BeatTime)`); chase lights;
drift of hair or smoke as a domain warp whose phase is a clock.

## Audio

Only `syn_BassHits`, `syn_BassLevel`, `syn_MidLevel`, `syn_HighLevel` and
`syn_BeatTime` are needed. Kicks are `clamp(syn_BassHits, 0, 1) * reactivity`
and only ever add to integrated rates or a brief brightness. Bass swells
width, size and glow. Mids sway, lean or turn. Highs twinkle and sparkle.
