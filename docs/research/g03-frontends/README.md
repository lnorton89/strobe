# G-03: GLSL front ends for a second backend (spike)

Backlog G-03 asks whether Strobe's GLSL can reach a non-GL backend
(Vulkan, D3D12, Metal) by either route the roadmap names:

- **wgpu/naga**: naga parses the GLSL itself, or takes SPIR-V that glslang
  made.
- **direct GLSL to SPIR-V**: glslang compiles it, then SPIRV-Cross writes
  HLSL or MSL from that.

This folder is the tool that measured it and the result. It is outside
the workspace and outside CI: it is research, not product.

## What it does

`cargo run --release -- <library>... --out <dir>`

For every pass of every scene in the libraries, it assembles the fragment
shader exactly as the GL path compiles it (`strobe_scene::assemble::fragment`,
desktop dialect, the pass index a constant), then runs:

| Column | What |
|---|---|
| naga glsl-in | naga's GLSL front end and validator on the source as written |
| naga glsl-in (as 450 core) | the same, with only the `#version` line changed, since naga takes 440 and 450 only |
| naga glsl-in (450, uniforms in a block) | and, by hand, what glslang's relaxed mode does automatically: loose uniforms moved into one block with a binding, bindings on samplers, a location on the output |
| glslang -> SPIR-V (GL) | `glslangValidator -G` (OpenGL SPIR-V, auto-mapped locations and bindings) |
| glslang -> SPIR-V (Vulkan, relaxed) | `glslangValidator -V -R` (Vulkan SPIR-V; `-R` accepts loose uniforms) |
| SPIR-V -> naga spv-in | that Vulkan SPIR-V into naga and its validator: the path wgpu takes for a SPIR-V module |
| SPIR-V -> HLSL / MSL | `spirv-cross --hlsl --shader-model 50` and `--msl` on the same SPIR-V |

It writes `results.csv` (one row per pass, each column's result, time and
first error line), `summary.md`, and every source and SPIR-V module under
`src/`, so any row can be checked by hand.

The ES dialect (ANGLE's) is not measured: SPIR-V needs GLSL ES 3.10 or
later, so ES 3.00 cannot take this route at all, and a SPIR-V backend would
compile the desktop dialect anyway.

## The corpus

26 scenes, 51 pass sources:

- the 12 scenes in `scenes/`;
- the 14 Shadertoy fixtures (the converter's 9 and the auto-control
  engine's 5), converted with `strobe-import --offline`.

The owner's 65 Synesthesia scenes are not in the repository. Running this
tool over them on TORNADO is the next step, and only needs `glslangValidator`
and `spirv-cross` on the PATH (both are in the Vulkan SDK).

Tools: naga 30, glslang 15.1.0, SPIRV-Cross 2021.01.15 (Ubuntu 24.04
packages), on 2026-10-13.

## Result

| Candidate | ok | failed | not run | scenes failing | median ms |
|---|---|---|---|---|---|
| naga glsl-in | 0 | 51 | 0 | 26 | |
| naga glsl-in (as 450 core) | 0 | 51 | 0 | 26 | |
| naga glsl-in (450, uniforms in a block) | 0 | 51 | 0 | 26 | |
| glslang -> SPIR-V (GL) | 51 | 0 | 0 | 0 | 45 |
| glslang -> SPIR-V (Vulkan, relaxed) | 50 | 1 | 0 | 1 | 46 |
| SPIR-V -> naga spv-in | 11 | 39 | 1 | 17 | 0.5 |
| SPIR-V -> HLSL (spirv-cross) | 50 | 0 | 1 | 0 | 4 |
| SPIR-V -> MSL (spirv-cross) | 50 | 0 | 1 | 0 | 4 |

The glslang and spirv-cross times include starting a process. The SPIR-V
modules total 462 KB for 50 passes.

Why each failure:

- **naga glsl-in, as written:** `Invalid version: 330`. naga only accepts
  `#version 440` and `450`.
- **naga glsl-in, as 450:** `uniform/buffer blocks require
  layout(binding=X)`. SSF declares every uniform loose, at global scope.
- **naga glsl-in, uniforms in a block:** `Not implemented: variable
  qualifier`, on the first `uniform sampler2D`. naga's GLSL front end does
  not implement combined image samplers. It wants Vulkan GLSL's separate
  `texture2D` and `sampler`, joined at each call. Every SSF and Shadertoy
  scene samples through `sampler2D`, and so does the SSF library's own code,
  including its function parameters. Getting past this means rewriting
  every sampling call and every function that takes a sampler, which is a
  GLSL rewriter, not an adapter.
- **glslang Vulkan, 1 pass:** a Shadertoy fixture overloads a function with
  different precision qualifiers, which Vulkan GLSL refuses and desktop GL
  accepts. The GL SPIR-V target compiles it.
- **naga spv-in, 39 passes:** `invalid id`, on loading a combined
  sampled image from a global (`%189 = OpLoad %sampled_image
  %syn_FinalPass`). The 11 passes that pass are exactly the ones that sample
  no texture. So naga rejects SSF's sampling whichever way the GLSL
  arrives.
- **naga glsl-in also fails the 9 scenes that sample nothing,** because
  Strobe's preamble always declares `syn_FinalPass`, `syn_Media` and the
  audio textures. A backend could leave unused ones out, but that would
  only rescue scenes that read no texture.

## Go / no-go

- **wgpu/naga: no-go.** Neither of naga's front ends takes a scene that
  samples a texture. 17 of the corpus's 26 do, in at least one pass (a
  buffer, a picture, `syn_FinalPass` or an audio texture). In the owner's
  65 Synesthesia scenes, at least 18 read pictures and 11 are multipass
  (TORNADO T24). Every feedback, buffer and media scene is in that group:
  the scenes a VJ engine exists for. The gap
  is structural (combined samplers), not a list of small fixes. A shader
  must be fully validated by naga before wgpu will use it, so wgpu's SPIR-V
  passthrough would need its unsafe, Vulkan-only escape hatch, which gives up
  what wgpu is for. This confirms D1 from the start of the project: native GL,
  because naga's GLSL front end rejects too much real code.
- **glslang -> SPIR-V -> SPIRV-Cross: viable as a front end, not yet a
  backend.** 51 of 51 compile to OpenGL SPIR-V and 50 of 51 to Vulkan
  SPIR-V, and SPIRV-Cross writes HLSL and MSL for all 50. But this measures
  only that the code translates. Measuring a picture or a frame time needs
  a renderer behind it, on each API, and none exists. A second backend is
  therefore not enabled and not started: the GL 3.3 path and ANGLE stay the
  only ones (G-04 stays deferred until a platform needs it, which macOS's
  deprecated GL may one day force).

Costs a glslang path would bring, for when it is reopened:

- **Dependencies:** glslang is C++, and so is SPIRV-Cross. Built from
  source, that is the `shaderc` crate's cmake build, or binaries shipped
  beside the exe.
- **Speed:** glslang takes about 40 ms per pass here, more than the GL
  driver's own compile of the same source on the T480. Every scene would
  then compile twice, once to SPIR-V and once in the driver.
- **Fidelity:** unmeasured. The two differences D199 and `fixups` already
  handle for ES (int-to-float, `#if` on non-macros) would need checking on
  each API.

## Reproduce

```bash
apt-get install -y glslang-tools spirv-cross
for f in crates/strobe-shadertoy/tests/fixtures/*.json crates/strobe-autocontrol/tests/fixtures/*.json; do
  target/debug/strobe-import "$f" --out /tmp/g03lib --name "st-$(basename "$f" .json)" --offline --force
done
cd docs/research/g03-frontends
cargo run --release -- ../../../scenes /tmp/g03lib --out /tmp/g03out
```

`G03_VERBOSE=1` prints naga's full error for the adapted source of each pass.
