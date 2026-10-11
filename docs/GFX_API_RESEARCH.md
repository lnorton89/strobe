# Modern graphics APIs for Strobe: research and implementation plan

**Status:** proposal, not an implementation.  
**Scope:** evaluate Vulkan and other modern graphics APIs without sacrificing Synesthesia Shader Format (SSF) or Shadertoy compatibility, the existing OpenGL 3.3 floor, ANGLE fallback, or the old-laptop performance target.

## Executive recommendation

Build an API-independent rendering contract around the existing renderer, then prototype **Vulkan through a compatibility-focused backend**. Do not replace the current OpenGL renderer or switch the whole project to `wgpu` in one step.

There are two separate problems:

1. **Graphics API coverage:** Vulkan, Direct3D 12, Metal, and eventually browser WebGPU.
2. **Shader-language compatibility:** Strobe runs real GLSL from SSF and Shadertoy. An API backend is not useful if its shader frontend rejects scenes that work today.

The second problem is the primary risk. `wgpu` is attractive because it offers a single safe Rust API over Vulkan, DX12, Metal, and browser WebGPU, but its GLSL input path goes through Naga. The project already records concrete incompatibilities in [PLAN.md](PLAN.md) and [DECISIONS.md](DECISIONS.md), including sampler parameters, combined image/samplers, and real-world constant/switch patterns. Treat these as a compatibility test corpus, not as reasons to assume all future Naga versions are unsuitable.

**Recommended direction:** keep OpenGL as the production reference and fallback; add a small renderer abstraction at the scene/pass boundary; build a time-boxed Vulkan proof of concept using a GLSL-to-SPIR-V route that preserves the existing GLSL semantics; evaluate `wgpu` against the same fixtures in parallel. Choose based on measured compatibility, not API fashion. If Naga's GLSL frontend passes the corpus at acceptable fidelity by the time implementation begins, `wgpu` is likely the lowest-maintenance multi-API option. If not, a direct Vulkan backend using shaderc/glslang or another maintained GLSL compiler may be necessary, at a significantly higher engineering cost.

## What Strobe actually needs from a backend

This is not a generic 3D engine. The workload is a full-screen fragment-shader instrument, with multiple passes, feedback, audio textures, live control updates, transitions, and output to a projector or LED wall.

A backend must preserve:

- **SSF:** the scene's GLSL, generated SSF preamble/library, controls, and per-pass `PASSINDEX` behaviour.
- **Shadertoy:** GLSL ES 3.00, `mainImage` conversion, `iChannel0..3`, multipass buffers, audio texture, texture filtering/wrapping, and frame/time uniforms.
- **Feedback:** double-buffered pass targets and well-defined frame-to-frame ping-pong.
- **Pixel equivalence:** coordinate origin, render-target orientation, texture origin, sRGB policy, precision, alpha, and resize semantics must be explicit.
- **No compilation during playback:** scene/pass pipelines are prepared before a show starts. A failed compile must be a load error, not a surprise in a live set.
- **Non-blocking frame path:** no device-wide idle, synchronous GPU readback, or per-frame allocation. GPU timing and readback remain asynchronous.
- **Recovery and diagnostics:** adapter/backend/driver identity in diagnostics, device-loss handling, resource recreation, and useful shader compile errors.
- **Old hardware:** adaptive resolution and low overhead matter more than theoretical feature count. Vulkan must not become a new minimum requirement.
- **Headless work:** benchmark, auto-control analysis, tests, and screenshot tooling need an offscreen path that behaves consistently with the windowed renderer.

See [strobe-gfx](../crates/strobe-gfx/README.md), [Architecture](ARCHITECTURE.md), [Benchmark results](BENCHMARKS.md), and the current decisions for the actual constraints.

## API options

### 1. OpenGL (current reference)

**Keep it.** It is the reference implementation and the compatibility oracle for the scene corpus. It hands GLSL to the driver's compiler and is the current path for native desktop GL and GLES through ANGLE.

Advantages:
- Direct GLSL compatibility with the existing pipeline.
- GL 3.3 is the intentional floor and works on older Intel GPUs.
- Mature headless EGL test path and current benchmark.
- Lowest migration risk.

Costs:
- Driver/context creation varies by platform.
- Windows hybrid-GPU and driver problems motivated ANGLE.
- Apple has deprecated OpenGL, and future platform coverage is constrained.

### 2. Vulkan

**Highest-value native API to investigate first.** It offers explicit resource management and a modern native path on Windows and Linux. It can also be used on Apple platforms through MoltenVK, but that adds a portability layer and must be tested separately.

Advantages:
- Strong control over command submission, memory, and synchronization.
- Broad Windows/Linux driver support and useful validation tooling.
- SPIR-V is a standard intermediate representation for shader modules.

Costs and risks:
- More implementation surface than GL: instance/device selection, queues, swapchains, image layouts, barriers, descriptors, pipeline layouts, and explicit synchronization.
- A Vulkan implementation must manage swapchain recreation and device loss carefully.
- GLSL source still needs a compiler and a compatibility strategy. Vulkan itself does not solve Naga's GLSL frontend gaps.
- Shader compilation and pipeline creation can stall if done on the show path; pipeline preparation must be a load-time operation.
- Old GPUs and drivers must be tested rather than assumed to benefit.

### 3. wgpu as a multi-API abstraction

Evaluate `wgpu` as a candidate implementation layer, not as an automatic decision.

Its backend coverage includes Vulkan, Direct3D 12, Metal, and browser WebGPU; OpenGL/GLES is also available as a secondary/best-effort path depending on target and features. This could avoid maintaining separate Vulkan, DX12, and Metal renderers.

The central blocker is shader ingress. The existing decision log identifies concrete GLSL constructs that have failed through Naga. Before committing, build a compatibility harness that runs all existing shader fixtures and a licensed set of representative real-world shaders through the exact Naga version and features proposed for Strobe. Record compile failures by construct and determine whether a semantics-preserving source rewrite is safe. Do not build a general GLSL transpiler as a side effect of the API migration.

If wgpu passes, prefer it for the multi-API backend: less platform-specific unsafe code, a common resource/pipeline model, and a better long-term maintenance story. Keep native GL available for shaders that cannot be translated and for the existing GL 3.3 hardware floor.

### 4. Direct3D 12

DX12 is relevant for Windows performance and driver coverage, but a dedicated backend duplicates much of the resource and pipeline work. Prefer it through wgpu unless a measured limitation blocks a required Strobe feature. Existing ANGLE/D3D11 fallback is valuable and should remain independent of this decision.

### 5. Metal

Metal is important if native macOS support becomes a real project goal. MoltenVK can make a Vulkan renderer run on Apple platforms, but it is not equivalent to validating native Metal behaviour or performance. Prefer wgpu's Metal backend if its shader path and compatibility tests pass. Keep platform support claims separate for native Metal and Vulkan-over-MoltenVK.

### 6. WebGPU in a browser

This is a possible future output/remote-rendering direction, not a near-term desktop renderer replacement. Browser WebGPU has different presentation, lifecycle, audio, and file-access constraints. The existing PWA remote-control proposal is a control plane and should not be conflated with moving the rendering engine into a browser.

### 7. OpenGL ES / WebGL

Retain GLES 3.0 through ANGLE as the current fallback. WebGL 2 may be useful for a future browser renderer, but it is not a modern native API replacement and should be evaluated separately from WebGPU.

## Architecture proposal

Avoid abstracting every GPU operation before there is a second backend. Extract only the contracts needed by the current renderer and a vertical-slice prototype.

1. **Scene and shader preparation stays backend-neutral.** Keep SSF parsing, control evaluation, audio analysis, JS, playlists, and Shadertoy conversion outside backend-specific crates.
2. **Define an explicit renderer contract** for creating a device/context, preparing a scene, rendering one frame, resizing targets, reading diagnostic identity, polling GPU timings, and recovering from device/context loss.
3. **Keep shader compilation behind a backend-specific interface.** Inputs include dialect, source, pass index/specialization, and diagnostics mapping. Outputs are compiled modules/programs plus a structured error. Avoid assuming GL program objects map directly to Vulkan pipelines.
4. **Represent a scene as a render graph or ordered pass plan.** A pass declares inputs, output format/size, feedback relationship, and dependencies. GL can execute this plan with FBOs; Vulkan/wgpu can use textures, views, and explicit transitions.
5. **Make synchronization semantics explicit in the plan.** Feedback reads the previous frame, while ordinary pass inputs read the current frame's completed producer. Backend code owns the barriers/layout transitions or GL ordering required to implement this.
6. **Separate presentation from scene rendering.** Scene output, playlist compositor, meta post-processing, preview, screenshot, and future capture/NDI consume a common final-image contract. The first prototype may retain GL for UI and use a shared-image/readback bridge only as a diagnostic experiment; that bridge is not an acceptable final high-performance architecture.
7. **Preserve the app's UI and event loop.** Do not migrate egui/winit just to change the scene API. If a backend requires different surface integration, isolate it in the backend provider.
8. **Make backend selection observable.** A user-facing choice should distinguish `auto`, `gl`, `vulkan`, and any eventual `wgpu` backend. Auto selection must log the chosen API, adapter, driver, and reason for fallback. Forced selection should fail clearly rather than silently switching APIs.
9. **Do not expose backend types in scene/runtime crates.** A backend change must not alter scene files, playlists, JS APIs, or importer output.

This abstraction is a migration seam, not a mandate to port every GL wrapper one-for-one. Vulkan's explicit resource model should shape its implementation rather than be hidden behind a lowest-common-denominator GL facade.

## Staged implementation plan

### Stage 0 — compatibility and baseline (no renderer changes)

- Pin the current toolchain/dependency versions and capture the current test and benchmark outputs.
- Enumerate every existing shader fixture, library scene, pass type, texture type, and GLSL dialect.
- Add a machine-readable compatibility report: compile success, runtime result, output checksum/image diff, frame time, and failure category.
- Add real-world fixtures only where redistribution is permitted; otherwise document reproducible acquisition and keep local-only fixtures uncommitted.
- Establish baseline runs for native GL, ANGLE/GLES, and headless llvmpipe.
- Record API/compiler versions so results are repeatable.

**Exit:** a repeatable corpus test catches a compatibility regression before any new backend is merged.

### Stage 1 — renderer seam, while GL remains the only backend

- Extract the narrow scene-rendering and presentation contracts described above.
- Keep the current GL implementation behaviourally unchanged.
- Test the contract against existing headless rendering, pass feedback, resize, post stage, readback, context-loss recovery, and benchmark paths.
- Avoid introducing a second abstraction for each GL object.

**Exit:** all existing tests and golden images pass with no meaningful regression in CPU/GPU frame cost.

### Stage 2 — shader frontend spike

Run two small prototypes against the same fixture corpus:

A. wgpu + Naga GLSL input, using the exact features and versions under consideration.  
B. Vulkan + GLSL-to-SPIR-V compiler (evaluate shaderc/glslang maintenance, licensing, deployment, and diagnostics).

Measure:
- Compile pass rate by fixture and by unsupported construct.
- Whether any source rewriting changes semantics.
- Startup and pipeline-preparation time for the full scene library.
- Shader error quality and line mapping.
- Runtime correctness for multipass, feedback, audio textures, and Shadertoy channels.

Do not silently skip unsupported scenes. Every failure must be listed, and any fallback to GL must be explicit and observable.

**Exit:** one route renders the required corpus, or the project makes a documented decision to support a clearly defined subset with visible compatibility reporting.

### Stage 3 — vertical-slice Vulkan prototype

Implement only enough to load and render a scene with:
- one output texture and a full-screen fragment pass;
- uniform/control updates;
- resize and swapchain recreation;
- explicit device/adapter selection;
- structured shader and device errors;
- validation layers in development builds.

Then add, in order: two dependent passes, double-buffer feedback, sampled images/cubemaps, audio textures, compositor/post stage, asynchronous GPU timing, and asynchronous final-pass readback.

Keep the prototype behind a feature flag or non-default backend selection until the acceptance matrix passes. Do not make Vulkan a required runtime dependency for existing users.

### Stage 4 — correctness and performance parity

Test all library scenes and compatible fixtures on GL, ANGLE, and the new backend. Use image diffs with a tolerance for floating-point differences, not just “it compiled.”

Measure:
- median, p95, and worst frame time;
- CPU submission time and GPU time separately;
- shader/pipeline warm-up and scene-switch latency;
- memory use and target allocations;
- dropped frames during scene changes and transitions;
- time to recover from resize, display changes, and device loss.

Run on at least one older integrated GPU and one current discrete GPU. Vulkan is not accepted solely because it is faster on a modern desktop.

### Stage 5 — shipping and support

- Add CI compile checks where runners support the target; do not treat a successful cross-compile as runtime validation.
- Document platform-specific loader/driver requirements and any bundled compiler/runtime assets.
- Record backend/compiler/adapter details in `--diagnose`, logs, crash reports, and bug reports.
- Keep GL/ANGLE as the safe default until real hardware evidence supports changing the policy.
- Publish the compatibility matrix and known shader limitations with each release.

## Acceptance matrix

| Area | Required evidence |
|---|---|
| Existing scenes | All original library scenes compile and render; any exception has an explicit reason |
| SSF multipass | Targeted passes, implicit final pass, pass index specialization, and current-frame dependencies match GL |
| Feedback | Double-buffered pass reads previous-frame data and remains stable across resize |
| Shadertoy | Single/multipass, channel textures, wrap/filter modes, audio texture, time/frame uniforms |
| Coordinates | Origin, UVs, pixel centres, aspect ratio, orientation, and resize behaviour match reference |
| Colour | RGBA8/float targets, alpha, filtering, and sRGB behaviour are documented and tested |
| Live performance | No shader compilation, blocking readback, or device-wide wait in steady-state playback |
| UI/presentation | Preview and output show the same final image; fullscreen output and multi-window operation survive resize |
| Diagnostics | Forced backend failures explain why; auto mode records selection and fallback |
| Recovery | Device/context loss can rebuild targets and pipelines without corrupting the session |
| Performance | Reproducible GL comparison on old integrated and current discrete hardware |
| Distribution | Clean-machine test with documented driver/runtime prerequisites |

## Risks and decisions still open

- **Shader compatibility is the gating risk.** API coverage without shader coverage is not a win for this application.
- **Dual backend cost:** every feature added to GL (media, post, readback, transitions, capture) increases parity work. Keep the matrix visible and avoid a long-lived half-supported backend.
- **Compiler deployment:** evaluate licensing, binary size, update cadence, diagnostics, and Windows packaging before selecting shaderc/glslang.
- **Precision differences:** SPIR-V and different drivers may produce small or visible image differences in raymarching, feedback, and noise-heavy shaders.
- **Driver support:** feature availability and stability vary by GPU/driver; enumerate capabilities rather than infer them from API name.
- **Old hardware:** some target GPUs support Vulkan only through older drivers or not at all. OpenGL 3.3 and ANGLE remain essential.
- **Apple:** distinguish native Metal, Vulkan via MoltenVK, and GL compatibility; they have different validation needs.
- **No premature API promises:** do not advertise DX12, Metal, Vulkan, or WebGPU support until the corresponding runtime tests pass on real hardware.

## Research references

These are primary documentation and upstream project sources; version-specific behaviour must be rechecked when the prototype is built.

1. [wgpu backend support and environment variables](https://github.com/gfx-rs/wgpu/blob/trunk/README.md) — backend matrix, adapter selection, and `WGPU_BACKEND`.
2. [wgpu backend feature flags](https://github.com/gfx-rs/wgpu/blob/trunk/wgpu/Cargo.toml) — Vulkan, DX12, Metal, GLES, browser WebGPU and Vulkan portability features.
3. [wgpu `Backends` API documentation](https://docs.rs/wgpu/latest/wgpu/struct.Backends.html) — platform/backend support and portability notes.
4. [Vulkan Guide: queues](https://docs.vulkan.org/guide/latest/queues.html) — queue families and explicit inter-queue synchronization.
5. [Vulkan Guide: device selection](https://docs.vulkan.org/guide/latest/initialization.html) — instance/device setup and capability discovery.
6. [Vulkan Tutorial: swapchain](https://docs.vulkan.org/tutorial/latest/03_Drawing_a_triangle/01_Presentation/01_Swap_chain.html) — surface support, swapchain creation and presentation.
7. [Vulkan Guide: synchronization examples](https://docs.vulkan.org/guide/latest/synchronization_examples.html) — image transitions and synchronization.
8. Strobe's own [PLAN.md](PLAN.md), [DECISIONS.md](DECISIONS.md), [strobe-gfx README](../crates/strobe-gfx/README.md), [ARCHITECTURE.md](ARCHITECTURE.md), and [BENCHMARKS.md](BENCHMARKS.md) — existing shader constraints, GL/ANGLE design, renderer hot-path rules, and performance baseline.

## Decision to make after the spike

Choose **wgpu** if the real corpus passes Naga with a maintainable, semantics-preserving compatibility strategy. Choose a **direct Vulkan backend** only if the wgpu/Naga path cannot meet the compatibility requirement and the project is willing to own explicit GPU resource/synchronization code. In either case, retain the current OpenGL/ANGLE implementation until the new backend proves compatibility and performance on actual target hardware.
