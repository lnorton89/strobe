# Open questions

These are things not yet verified against real Synesthesia behaviour,
along with the current assumption for each. Once one is verified or
disproved, move it to **Resolved** with the evidence, and log a decision
in DECISIONS.md if the code changes.

The best evidence is a real `.synScene` that depends on the behaviour.
The owner can supply their own scenes. Unlicensed scenes may be used
locally but must not be committed.

## Open

| # | Question | Current assumption | How to settle |
|---|---|---|---|
| Q1 | Are `syn_Spectrum` / `syn_LevelTrail` `sampler1D` or `sampler2D` N×1? | `sampler1D` on desktop (the docs say "1DSamplers"); `sampler2D` N×1 on GLES, where 1D textures don't exist | Find a real scene that samples them, and check whether it calls `texture(syn_Spectrum, float)` or `vec2` |
| Q2 | What should `_mouse` / `_muv` hold when the mouse is outside the preview? | All zeros | Low priority |
| Q3 | Is `_loadMedia(vec2 offset)`'s offset in UV units or pixels? | UV units | A scene that uses it |
| Q4 | How does `bang smooth` shape its envelope with PARAMS? | Fast attack (≥25% of the remaining distance per 60th of a second), then exponential decay at PARAMS | Compare by eye with Synesthesia |
| Q5 | How should a non-`DOUBLE_BUFFER` pass behave when it samples itself? | It binds the same texture it renders to, which is a GL feedback loop, and SSF's docs say to expect artifacts | Confirm that no real scene relies on it |
| Q6 | Does Synesthesia ignore unknown `scene.json` keys? | Yes. Real scenes carry `IS_META`, `GPU`, `TAGS`, `UNIFORMS`, `LABEL` | This only matters if Strobe writes scenes back for Synesthesia |
| Q7 | Do the EULA terms allow running Synesthesia's own built-in scenes in another engine? | Unknown, so Strobe doesn't do it | Read the EULA |
| Q8 | ANGLE path for SSF code: how much real SSF code fails under GLSL ES (implicit conversions, `PASSINDEX == 0.0`)? | Much of it. Needs a source-fixup pass or a GLSL parser (e.g. `glsl-lang`) to insert casts | Run the community fixtures through `strobe-bench --api gles` |
| Q9 | What is the root `"UNIFORMS"` key in some community scenes? | Probably non-standard (from AI-generated scenes); ignored | Look for it in a real Synesthesia-authored scene |

## Resolved

| # | Question | Answer | Evidence |
|---|---|---|---|
| R1 | Does the final pass need a `PASSES` entry? | No. The final pass is implicit, with `PASSINDEX == len(PASSES)` | Community scenes `filter-brown` and `thresholder` (two listed passes, `PASSINDEX == 2.0` for the output) |
| R2 | What is a `color` control's uniform type? | `vec3`, with its default in `DEF_COLOR` | SSF JSON docs, and the `thresholder` scene |
| R3 | Do real scenes always give MIN/MAX/DEFAULT? | No. They're often missing, and sometimes MIN > MAX on purpose | `thresholder`, `filter-brown` |
| R4 | What type does real code compare `PASSINDEX` with? | Both int (`== 0`) and float (`== 0.0`), which needs desktop GLSL | Community scenes |
