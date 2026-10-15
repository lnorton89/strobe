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
| SPIR-V, samplers split -> naga spv-in | the same SPIR-V after `spirv-webgpu-transform`'s `combimgsampsplitter` splits every combined image sampler into a texture and a sampler, then naga (added 2026-10-15, see the follow-up) |
| SPIR-V -> HLSL / MSL | `spirv-cross --hlsl --shader-model 50` and `--msl` on the same SPIR-V |

It writes `results.csv` (one row per pass, each column's result, time and
first error line), `summary.md`, and every source and SPIR-V module under
`src/`, so any row can be checked by hand. **Point `--out` outside the
repository when the library is private:** `src/` holds every assembled
shader.

A scene with a `script.js` is assembled with the uniforms its script
creates, found as `Session::load` finds them: the script runs one frame
before the shaders are assembled (`strobe-script`), and names the scene
already declares are dropped. Before 2026-10-15 the tool passed no script
uniforms. The 26-scene corpus below never noticed: its one scripted scene
creates no uniforms (it drives controls). The owner's corpus did notice.

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

## Follow-up (2026-10-15): the full corpus, and naga today

The owner asked for the run over the private Synesthesia corpus, a check
of the newest naga, and ANGLE's other backends as an alternative. The
ANGLE part is [`../angle-backends.md`](../angle-backends.md), and the
decision is D292.

### The corpus

- **The owner's 90 Synesthesia scenes.** These are the defaults of
  Synesthesia 1.25.4: 48 with a `script.js`, 63 multipass, up to 11
  passes. The corpus is private and copyrighted, so it is not in the
  repository. This page gives scene names and counts only, and the tool's
  `src/` dump went to a scratch folder outside the checkout.
- **`#include` resolution.** `STROBE_GLSL_PATH` pointed at the corpus's
  copy of Synesthesia's `lib`. The tool resolves includes exactly as the
  app and `strobe-corpus` do: `SceneFolder::load` calls
  `include::search_path`, which takes `STROBE_GLSL_PATH`, then a detected
  Synesthesia install's `lib`, then its user `glsl` folder.
- **The library**, now 16 scenes in `scenes/`, plus the 14 Shadertoy
  fixtures as before.

That is 120 scenes and 288 pass sources.

Tools: naga 30.0.1, spirv-webgpu-transform 0.1.7, glslang 15.1.0,
SPIRV-Cross 2021.01.15.

### Result

All 120 scenes and 288 pass sources:

| Candidate | ok | failed | not run | scenes failing | median ms |
|---|---|---|---|---|---|
| naga glsl-in | 0 | 288 | 0 | 120 | |
| naga glsl-in (as 450 core) | 0 | 288 | 0 | 120 | |
| naga glsl-in (450, uniforms in a block) | 0 | 288 | 0 | 120 | |
| glslang -> SPIR-V (GL) | 288 | 0 | 0 | 0 | 51 |
| glslang -> SPIR-V (Vulkan, relaxed) | 287 | 1 | 0 | 1 | 52 |
| SPIR-V -> naga spv-in | 16 | 271 | 1 | 106 | 0.4 |
| SPIR-V, samplers split -> naga spv-in | 116 | 171 | 1 | 60 | 0.7 |
| SPIR-V -> HLSL (spirv-cross) | 287 | 0 | 1 | 0 | 8 |
| SPIR-V -> MSL (spirv-cross) | 287 | 0 | 1 | 0 | 8 |

Split by corpus (passes ok of total, and scenes failing):

| Candidate | Synesthesia, 90 scenes / 227 passes | Library + fixtures, 30 scenes / 61 passes |
|---|---|---|
| naga glsl-in (any variant) | 0 / 227, 90 scenes failing | 0 / 61, 30 scenes failing |
| glslang -> SPIR-V (GL) | 227 / 227 | 61 / 61 |
| glslang -> SPIR-V (Vulkan, relaxed) | 227 / 227 | 60 / 61, 1 (the precision overload) |
| SPIR-V -> naga spv-in | 5 / 227, 85 | 11 / 61, 21 |
| SPIR-V, samplers split -> naga spv-in | 56 / 227, 60 | 60 / 60, 0 |
| SPIR-V -> HLSL / MSL | 227 / 227 | 60 / 60 |

**What it says:**

- **naga's GLSL front end takes nothing**, in the same three steps and
  with the same errors as the 26-scene run:
  - `#version 330` is refused;
  - then loose uniforms need a block;
  - then `Not implemented: variable qualifier` on the first
    `uniform sampler2D`.
- **glslang compiles the whole Synesthesia corpus**, 227 of 227 passes, to
  OpenGL and to Vulkan SPIR-V, and SPIRV-Cross writes HLSL and MSL for
  every one.
  - The first run failed 105 passes in 43 scenes on an undeclared
    identifier. All 43 were scripted scenes, and the identifiers were the
    uniforms their scripts create. The tool now passes them as the app
    does (above), and all 105 compile.
  - Median times: glslang about 51 ms per pass, SPIRV-Cross about 8 ms
    (both include starting a process).
  - The Vulkan SPIR-V totals 9.0 MB for 287 passes. A corpus pass is
    larger than a library pass because of the SSF library and lygia
    includes.
- **naga's SPIR-V front end still passes only what samples nothing**: 5
  corpus passes, one each from the only scenes that come through whole
  (`hello_world_start`, `hello_world_end`, `midi_mapping`,
  `scaffold_fractal`, `test_card`), and none of them samples a texture.
  The error is the same
  `invalid id` on loading a combined sampled image.
- **New: splitting the samplers first gets naga much further, not all the
  way.**
  - `spirv-webgpu-transform` is a pure-Rust (BSD-2-Clause) SPIR-V pass
    that rewrites combined image samplers as separate textures and
    samplers, function parameters included. After it, naga takes all 60
    library and fixture passes.
  - It takes only 56 of 227 corpus passes, and 30 of 90 scenes whole. The
    other 171 fail naga's *validator* on what its SPIR-V reader built:
    - 155 passes in 58 scenes with `Expression [N] can't be introduced -
      it's already in scope`, 148 of them in `renderMain`. naga's SPIR-V reader emits
      one expression twice on glslang's unoptimised control flow, which
      is a naga bug, not an SSF construct.
    - 11 passes, all of `fluid_body`: `Operation Multiply can't work
      with` a 64-bit and a 32-bit integer vector.
    - 5 passes, all of `churning`: `Special type is not registered
      within the module`.
  - So the structural gap D255 named, combined samplers, can be bridged
    in SPIR-V. A wgpu backend would then hit a list of naga SPIR-V reader
    bugs on two thirds of the real corpus, and would still carry glslang,
    which is C++.

### naga today

- **No newer naga.** crates.io's newest is **30.0.1** (2026-08-21), the
  version the spike already used. There is nothing to bump, and the tool
  stays on 30.
- **No sampler work in the changelog.** wgpu's `CHANGELOG.md` (trunk,
  read 2026-10-15) has no `glsl-in` or `spv-in` work on combined samplers
  in v30.0.0, v30.0.1 or *Unreleased*. Its naga entries are ray tracing,
  mesh shaders, validation and backend fixes.
- **The code is unchanged.** On trunk:
  - `front/glsl/parser/declarations.rs` still answers
    `NotImplemented("variable qualifier")` at the same point. `sampler2D`
    after `uniform` is not a type to it.
  - `front/spv/image.rs` is identical to 30.0.1's.
  - naga's docs (Context7, docs.rs) still describe the GLSL front end as
    Vulkan GLSL 450/460, with 440 partial.
- **The maintainers' tracking issue** for samplers as function parameters
  is wgpu #5256, labelled help-required. In January 2025 they said they
  have no bandwidth for it.

**D255's no-go holds** for naga as Strobe's GLSL front end. The split-sampler
route is noted in D292 as the one thing that would change the wgpu
picture: naga's SPIR-V reader taking the corpus after the split. The tool
measures it, so rerunning it after a naga release answers that.

### Reproduce (private corpus)

```bash
export STROBE_GLSL_PATH=/path/to/strobe-syn-corpus/engine/lib
cd docs/research/g03-frontends
# --out outside the checkout: src/ holds every assembled corpus shader
cargo run --release -- /path/to/strobe-syn-corpus/scenes ../../../scenes /tmp/g03lib --out /tmp/g03-corpus
```
