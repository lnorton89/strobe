# Controls

Group order: motion, then the scene's own groups, then colour, then audio.
Every slider is `slider smooth` with `PARAMS 0.03` and a `DESCRIPTION` that
says what it changes and what zero means. Aim for 14 to 22 controls.

## Always

| Name | Range / default | Group |
|---|---|---|
| `speed` | 0-3 / 1 | motion |
| `glow` | 0-2 / 1 (0.4 for matte work) | color |
| `brightness` | 0.4-1.6 / 1 | color |
| `reactivity` | 0-2 / 0.8 | audio |

## Usually

`wobble` 0-1/0.5 (the hand), `weight` 0.5-1.8/1 (pen width), `uv` 0-1/0.5
when the art is fluorescent, `gloss` 0-1/0.5 for resin or varnish, `stars`
0-1 for sparkle fields.

## One per scene

- **A bang**, `bang counter smooth`, `PARAMS 0.04`, named as a verb the
  performer does: `gust`, `inscribe`, `strike`, `flash`, `sunrise`, `snap`,
  `scuttle`. It drives a clock or spring through the fraction of its counter,
  so it rises and settles once per press.
- **A dropdown** for a layout or a count, with `LABELS` and `VALUES`, and
  `DEFAULT` as an index: `rows`, `rings`, `view`, `frame`, `twins`.
- **A colour control per paint** (`color smooth`, `DEF_COLOR` as `vec3`).

## The scene's own

Physical parameters of the marks and the subject: `droop` (how far fronds
curl), `barbs` (spike length), `thorn`, `girth`, `flames`, `rewrite` (share of
glyphs rewritten per beat). Name them as the painter would. A control that
barely changes the image is worse than none: the forge checks every one at
its extremes.
