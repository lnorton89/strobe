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
| Q8 | ANGLE path for SSF code: how much real SSF code fails under GLSL ES (implicit conversions, `PASSINDEX == 0.0`)? | Much of it. Needs a source-fixup pass or a GLSL parser (e.g. `glsl-lang`) to insert casts | Run the community fixtures through `strobe-bench --api gles`. **Shadertoy half (Phase 4 stage 1):** valid Shadertoy source is ES-clean by construction, and all nine fixtures compile on GLES 3.0 under Mesa (D94). Still unsolved: implicit `int` to `float` in SSF scenes (needs types, so a parser), and the packing built-ins (`packHalf2x16`, `unpackUnorm2x16`...), which are core in ES 3.00 but need GLSL 4.00 on desktop, so a Shadertoy shader using them fails on a 3.30 context |
| Q9 | What is the root `"UNIFORMS"` key in some community scenes? | Probably non-standard (from AI-generated scenes); ignored | Look for it in a real Synesthesia-authored scene |
| Q10 | What exact layout do real scenes expect from `syn_Spectrum` and `syn_LevelTrail`: texel count, frequency axis of the raw channel, which end of the trail is "now", and what the waveform channel's range is? | `strobe-audio` assumes 256 texels; raw linear 0..Nyquist; juiced log 30 Hz to 16 kHz; waveform 0.5 at zero; trail index 0 = newest, channels whole/bass/mid/high. All sizes are configurable (`Layout`) | A real licensed scene that samples the textures in known ways |
| Q11 | Does the release build (`strip = "none"`, `debug = "line-tables-only"`) actually produce a `strobe.pdb` that symbolicates a minidump from the diagnostics crate? | Yes: MSVC writes a PDB unless rustc passes `/DEBUG:NONE`, which `strip = "debuginfo"` does (D53) | Cut a tag-less release build on Windows, crash it, and symbolicate the dump with the PDB |
| Q12 | Does `release.yml` run end to end (ANGLE fetch, packaging paths, cargo-cyclonedx, attestations)? | Yes; every command was run in pieces locally and the YAML passes actionlint | Push a `v0.0.0-test` tag to a fork (or use `workflow_dispatch` on a throwaway tag), then delete the draft |
| Q13 | What does `_click` hold? `strobe-gfx` documents it as left/right/middle held; `strobe-script` documents it as the last click's x and y with z the button state | The app feeds both the left/right/middle-held reading (`mouse.rs`), and `_mouse.zw` carries the last click position | A real scene or script that reads `_click` |
| Q14 | Does `textureFinalPass` need the previous frame's 64x36 downsample on the first frame, and how stale may it be? | Not provided yet: it returns black. A per-frame GPU readback stalls an old laptop's pipeline, so it wants an asynchronous (PBO) readback one or two frames late | A real script that calls `textureFinalPass` |
| Q15 | Shadertoy's exact `iDate`, `iChannelTime`, `iMouse`, `iSampleRate` and `iFrameRate` semantics, and its licence terms | `iDate` is UTC here (year, month 0 to 11, day, seconds since midnight; Shadertoy uses local time); `iChannelTime[n]` is `iTime` for audio channels and 0 otherwise; `iMouse` is the player's `_mouse` (xy while held, zw click position; Shadertoy flips the sign of z and w with button state, which `_mouse` may not); `iSampleRate` is 44100; `iFrameRate` is 1/dt of the last frame; shaders are CC BY-NC-SA 3.0 unless the author says otherwise | Read Shadertoy's page source (`shadertoy.com/js/effect.js`) once a browser can reach it, and Shadertoy's terms page |
| Q16 | Does a Shadertoy `srgb: true` texture need decoding on sampling, and does `internal: float` matter? | Neither is applied; the importer warns for `srgb`. `Format` has no sRGB entry and `strobe-gfx/src/gl/` was another agent's | A real shader that sets it, next to Shadertoy's own picture |
| Q17 | Do strict desktop drivers (Intel HD 4000 on Windows, NVIDIA) accept a shader that defines a built-in function, such as `float tanh(float)`? | Mesa accepts it on 3.30 and rejects it on ES 3.00, so the converter aliases it only under `GL_ES` (D94). A strict desktop driver would need the alias there too, at the cost of breaking a call to an overload the shader did not define | Run the `legacy` fixture on the T480 |
| Q18 | Does the audio texture's scale match Shadertoy's closely enough for shaders written against it? | Row 0 is the raw FFT converted from -100..-20 dB to the browser's -100..-30 dB window with a 6 dB offset, with none of the browser analyser's smoothing; row 1 is the waveform | Compare a known audio shader against Shadertoy with the same track; then feed `AudioFrame::shadertoy` through (D93) |

## Resolved

| # | Question | Answer | Evidence |
|---|---|---|---|
| R1 | Does the final pass need a `PASSES` entry? | No. The final pass is implicit, with `PASSINDEX == len(PASSES)` | Community scenes `filter-brown` and `thresholder` (two listed passes, `PASSINDEX == 2.0` for the output) |
| R2 | What is a `color` control's uniform type? | `vec3`, with its default in `DEF_COLOR` | SSF JSON docs, and the `thresholder` scene |
| R3 | Do real scenes always give MIN/MAX/DEFAULT? | No. They're often missing, and sometimes MIN > MAX on purpose | `thresholder`, `filter-brown` |
| R4 | What type does real code compare `PASSINDEX` with? | Both int (`== 0`) and float (`== 0.0`), which needs desktop GLSL | Community scenes |
