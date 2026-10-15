# ANGLE's other backends as Strobe's second backend (research, 2026-10-15)

G-03 follow-up (D292). D255 left a second backend unstarted because no
GLSL front end could feed one. This page asks a different question: Strobe
already renders through ANGLE (GLES 3.0 on Direct3D 11) on Windows. ANGLE
also has Metal and Vulkan backends. Could choosing ANGLE's backend be the
second backend, with no new renderer code?

**Short answer: yes for Vulkan, and very likely for Metal, but not for free
on macOS.**

- **Vulkan works today, with no code change.** The ANGLE build Strobe
  already ships (Electron v38.2.0) contains a Vulkan backend on Windows and
  Linux. In this container, the unmodified `strobe-bench` rendered all 16
  library scenes on ANGLE's Vulkan backend, on Mesa's lavapipe and on
  SwiftShader, and the corpus harness ran all 90 private Synesthesia
  scenes on it.
  - On lavapipe the library's pictures matched Mesa's own GLES
    bit-for-bit (12 scenes) or to within feedback-scene noise (4).
  - Frame times were within this container's run-to-run noise of
    Mesa's.
  - See [the experiment](#5-the-experiment-angle-vulkan-in-this-container).
  The backend was picked with ANGLE's `ANGLE_DEFAULT_PLATFORM` environment
  variable, because glutin cannot pass the attribute (section 3).
- **On macOS the obstacle is glutin, not ANGLE.**
  - ANGLE's Metal backend is GLES 3.0 conformant.
  - Apple's own Safari runs WebGL 2 on it, and Chrome has used it on
    macOS since about Chrome 115.
  - Electron ships it as two dylibs (7 MB, macOS 12+).
  - But glutin compiles its EGL backend out on Apple targets, and it never
    passes ANGLE's platform attributes. A macOS ANGLE path therefore needs
    its own small EGL provider in `strobe-gfx/src/gl/`, or a glutin
    patch.
  - Also, ANGLE's *own* default on macOS is its OpenGL backend, not Metal,
    so the Metal type has to be requested explicitly.

## 1. Which backends ANGLE has

From ANGLE's [README](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/README.md)
(main, read 2026-10-15):

| ES version | D3D9 | D3D11 | Desktop GL | GL ES | Vulkan | Metal |
|---|---|---|---|---|---|---|
| ES 2.0 | complete | complete | complete | complete | complete | complete |
| ES 3.0 | | complete | complete | complete | complete | complete |
| ES 3.1 | | | complete | complete | complete | |
| ES 3.2 | | | in progress | in progress | complete | |

| Platform | Backends ANGLE supports there |
|---|---|
| Windows | D3D9, D3D11, desktop GL, GLES, Vulkan |
| Linux | desktop GL, Vulkan |
| macOS | desktop GL (marked *deprecated*), Metal (macOS 10.14+) |
| iOS | Metal (iOS 12+) |

ANGLE's Vulkan backend has formal Khronos conformance for ES 2.0, 3.0, 3.1
and 3.2. The Metal backend reports ES 3.0 as its maximum *conformant*
version (`DisplayMtl::getMaxConformantESVersion`). It needs Metal GPU family
Mac1/Apple4 for ES 3.0 and falls back to ES 2.0 below that. Every Mac that
runs a supported macOS has that family. A WebGPU backend exists but is not
built in official builds (`angle_enable_wgpu` needs `!is_official_build`).

**What is compiled in is a build choice** (`gni/angle.gni`):

- `angle_enable_d3d11 = is_win`;
- `angle_enable_metal = is_apple`;
- `angle_enable_vulkan` on Windows, Linux and ChromeOS;
- `angle_enable_gl` everywhere except iOS, Fuchsia and Windows on ARM,
  **macOS included**.

The Electron v38.2.0 binaries Strobe pins (D12) were checked by their
extension strings:

| Electron v38.2.0 build | Files | Backends inside |
|---|---|---|
| `win32-x64` (what Strobe ships) | `libEGL.dll`, `libGLESv2.dll` (8.1 MB), `d3dcompiler_47.dll`, plus `vulkan-1.dll`, `vk_swiftshader.dll` in the zip | D3D11, D3D9, desktop GL (WGL), Vulkan |
| `linux-x64` | `libEGL.so`, `libGLESv2.so` (6.4 MB), `libvulkan.so.1`, `libvk_swiftshader.so` | desktop GL (GLX/EGL), Vulkan, SwiftShader |
| `darwin-arm64` | `Electron Framework.framework/Versions/A/Libraries/libEGL.dylib` (0.1 MB), `libGLESv2.dylib` (7.0 MB), `libvk_swiftshader.dylib` | Metal, desktop GL (CGL), Vulkan on SwiftShader only. `LC_BUILD_VERSION` minimum macOS 12.0, built against SDK 26.0 |

All three report `ANGLE 2.1.25847 git hash: a8c8a6febe63`.

## 2. Selecting a backend

**The EGL way.** [`EGL_ANGLE_platform_angle`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/extensions/EGL_ANGLE_platform_angle.txt):
call `eglGetPlatformDisplayEXT(EGL_PLATFORM_ANGLE_ANGLE /*0x3202*/,
EGL_DEFAULT_DISPLAY, attribs)` with `EGL_PLATFORM_ANGLE_TYPE_ANGLE
(0x3203)` set to one of:

| Attribute value | Hex | Extension that advertises it |
|---|---|---|
| `EGL_PLATFORM_ANGLE_TYPE_D3D11_ANGLE` | 0x3208 | `EGL_ANGLE_platform_angle_d3d` |
| `EGL_PLATFORM_ANGLE_TYPE_OPENGL_ANGLE` | 0x320D | `EGL_ANGLE_platform_angle_opengl` |
| `EGL_PLATFORM_ANGLE_TYPE_OPENGLES_ANGLE` | 0x320E | `EGL_ANGLE_platform_angle_opengl` |
| `EGL_PLATFORM_ANGLE_TYPE_VULKAN_ANGLE` | 0x3450 | [`EGL_ANGLE_platform_angle_vulkan`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/extensions/EGL_ANGLE_platform_angle_vulkan.txt) |
| `EGL_PLATFORM_ANGLE_TYPE_METAL_ANGLE` | 0x3489 | [`EGL_ANGLE_platform_angle_metal`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/extensions/EGL_ANGLE_platform_angle_metal.txt) |

`EGL_PLATFORM_ANGLE_DEVICE_TYPE_ANGLE` can add
`..._DEVICE_TYPE_SWIFTSHADER_ANGLE` (0x3487) for a CPU Vulkan device. Values
are in [`eglext_angle.h`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/include/EGL/eglext_angle.h).
A client checks the client-extension string for the backend's extension
before asking for it. That string is how "this ANGLE has Metal" is
detected.

**The environment-variable way.** When the caller passes no type, ANGLE
reads `ANGLE_DEFAULT_PLATFORM` (`src/libANGLE/Display.cpp`,
`GetDisplayTypeFromEnvironment`). Its values are `vulkan`, `swiftshader`
(Vulkan on SwiftShader), `vulkan-null`, `gl`, `d3d11`, `metal`, `webgpu` and
`null`, each honoured only if compiled in. The comment says it exists "to
run angle with benchmarks without having to modify the benchmark source".
That is exactly how the experiment below selected Vulkan.

**ANGLE's own default, when nothing is said.** The same function falls
through in this order: D3D11 if built; Vulkan on Android; **OpenGL if
built**; then Metal; then Vulkan. On macOS the GL backend is built (see
above), so **a bare `eglGetDisplay` on macOS gives ANGLE-on-CGL, not
Metal**. That is ANGLE on Apple's deprecated GL. Chrome gets Metal on
macOS because it passes the type explicitly (its `--use-angle=metal`
default). Strobe would have to do the same.

Precedent: GLFW 3.4 added a `GLFW_ANGLE_PLATFORM_TYPE` init hint
(`GLFW_ANGLE_PLATFORM_TYPE_METAL`, `_VULKAN`, `_D3D11`, ...) that passes
exactly this attribute.

## 3. What Strobe does today, and what glutin allows

**Today:**

- `crates/strobe-gfx/src/gl/window.rs`: `Backend::Angle` asks glutin for
  `DisplayApiPreference::Egl` and a GLES 3.0 context, and glutin loads
  `libEGL.dll` (Windows) or `libEGL.so.1` (elsewhere).
- `--gl angle` (app, bench) and `GlChoice::Auto`'s fallback choose it.
- The headless context (`gl::context::Headless`) uses EGL *device*
  displays (`EGL_EXT_device_enumeration`), which ANGLE does not implement.
  So ANGLE is reached only through the window path, and the bench's
  `--context window --gl angle`.

**glutin 0.32.3** (Strobe's version, and still the newest on crates.io;
`master` is the same in both respects):

- **No platform attributes.**
  - `api/egl/display.rs`, `get_platform_display_ext`: for a Windows display
    handle with `EGL_ANGLE_platform_angle` it calls
    `eglGetPlatformDisplayEXT(EGL_PLATFORM_ANGLE_ANGLE, EGL_DEFAULT_DISPLAY,
    [EGL_TRACK_REFERENCES_KHR?, EGL_NONE])`. There is no type attribute and
    no API to add one.
  - There is also no `Display::from_raw(EGLDisplay)` to hand glutin a
    display made elsewhere.
  - On Linux, glutin uses the X11/Wayland platforms. ANGLE does not accept
    those, so glutin falls back to `eglGetDisplay(native)`, which still has
    no type.
- **No EGL on Apple.** `build.rs` defines `egl_backend` as
  `all(feature = "egl", any(windows, unix), not(apple), not(wasm_platform))`.
  On macOS, `DisplayApiPreference::Egl` does not exist, and `Backend::Angle`
  cannot even be expressed. This is the same limit D55 hit for the
  headless context.

**What it would take, per platform:**

| Platform | Backend | Cost |
|---|---|---|
| Windows | Vulkan (or GL) | **Zero code** to try it: set `ANGLE_DEFAULT_PLATFORM=vulkan` before launch, beside the shipped DLLs. The release zip would also need `vulkan-1.dll`, unless the GPU driver's loader in `System32` is relied on (every Vulkan driver installs one). To make it a `--gl angle-vulkan` choice in the app, see below. |
| Linux | Vulkan | Same: Electron's `libEGL.so` and `libGLESv2.so` beside the binary, `LD_LIBRARY_PATH` or an rpath, and the variable. Tried below. |
| macOS | Metal | **New code.** An EGL provider of our own in `strobe-gfx/src/gl/`, since glutin has none on Apple. It would `dlopen` `libEGL.dylib` (the `khronos-egl` crate's dynamic loading, or `libloading` and a handful of leaf wrappers). It calls `eglGetPlatformDisplayEXT` with `TYPE_METAL_ANGLE`, chooses a config, creates a GLES 3.0 context, and makes a window surface from a **`CALayer *`**: ANGLE-Metal's `isValidNativeWindow` accepts only a `CALayer` (`DisplayMtl.mm`), and adds its own `CAMetalLayer` beneath it (`SurfaceMtl.mm`). So winit's `NSView` gets `setWantsLayer:YES`, and its layer is passed. Swap interval, `make_current`, `get_proc_address` and robustness are the other leaf calls. That is roughly `gl::window`'s size again, plus the macOS-only window plumbing, and it sits under the unsafe-boundary rules (D-rules in CLAUDE.md). |

**Options for passing the type** on Windows and Linux, where glutin does
work:

1. **The environment variable, set by the process before EGL loads.** In
   Rust 2024, `std::env::set_var` is `unsafe` (other threads may read the
   environment). It would be a leaf wrapper in `gl/`, called at the top
   of `main` before any thread exists. That is cheap, but it is a
   process-global knob for a per-display choice.
2. **A glutin patch** adding display attributes for the ANGLE platform
   (and lifting `not(apple)`). This is upstreamable, and it is the clean
   fix for Windows and Linux, but it does not by itself make a CALayer
   surface on macOS.
3. **Our own EGL provider** (the macOS row), used on every platform for
   ANGLE. That is one code path for D3D11, Vulkan and Metal, and glutin
   stays for WGL, GLX and CGL.

## 4. What Strobe needs from a backend, on ANGLE-Metal and ANGLE-Vulkan

Strobe's ES path asks for only these (`gl/mod.rs`, `caps.rs`, `gl/debug.rs`,
`readback.rs`).

| Need | GLES 3.0 core? | ANGLE Metal | ANGLE Vulkan |
|---|---|---|---|
| `texelFetch`, `textureSize`, integer ops | core | yes (ES 3.0 conformant) | yes (conformant) |
| Float render targets (RGBA16F, RGBA32F passes) | `EXT_color_buffer_float` (Strobe checks it: `Info::es_float_render`) | derived from format caps (`Caps.cpp`, `DetermineColorBufferFloatSupport`); Metal renders both formats; extension string present in the macOS dylib | measured: on, FLOAT passes rendered (reaction-diffusion) |
| Multiple render targets | core (4+) | `maxDrawBuffers = mtl::kMaxRenderTargets` (8; 4 on older families) | yes |
| sRGB textures | core | yes | yes |
| Timer queries (`TIME_ELAPSED`) | `EXT_disjoint_timer_query` | `disjointTimerQueryEXT = true`, 64-bit elapsed counter, timestamps 0 bits (`DisplayMtl.mm`) | yes |
| `KHR_debug` | extension | front-end, every backend (`Context.cpp`: `debugKHR = true`) | same |
| Robustness / reset status (D66) | `EXT_robustness` / `KHR_robustness` | both `true`; `createContextRobustness` in EGL | yes |
| PBO readback, `glMapBufferRange` (D162) | core | yes; `mapBufferRangeEXT = true` | yes |
| Fences (`glFenceSync`, `glClientWaitSync`) | core | yes; EGL `fenceSync = true` | yes |
| `sampler1D` | desktop only | no, already handled: `Caps::has_1d_textures` is false on ES | same |
| Desktop-only GLSL | n/a | the ES dialect plus `fixups` (D199) already run on ANGLE-D3D11, and the corpus is 90 of 90 on GLES (D268) | same |

So nothing Strobe uses is missing from the ES 3.0 feature set ANGLE-Metal
claims. The gaps are behavioural, and only a Mac can show them:

- feedback passes that read and write one texture (D3D11 under ANGLE
  returned zeros, D203 double-buffers them);
- precision of RGBA16F accumulation;
- vsync and present timing through `CAMetalLayer`;
- shader compile time (the Metal backend translates to MSL and the Metal
  compiler runs at link; the README notes long compile times and a cache
  hook).

## 5. The experiment: ANGLE-Vulkan in this container

**Setup:**

- Electron v38.2.0 `linux-x64`'s `libEGL.so` and `libGLESv2.so`, with a
  `libEGL.so.1` symlink because glutin loads that name, in a scratch folder
  on `LD_LIBRARY_PATH`.
- Xvfb for an X display, because the window path is the one that reaches
  ANGLE.
- `ANGLE_DEFAULT_PLATFORM=vulkan` with Mesa's lavapipe
  (`VK_ICD_FILENAMES=.../lvp_icd.json`), or `=swiftshader`.
- No Strobe code changed. Binaries built at 43ccdd3 (SwiftShader column:
  an earlier build, 81e7fbf, with the same renderer).

The renderer string shows the backend:
`ANGLE (Mesa, Vulkan 1.4.318 (llvmpipe (LLVM 20.1.2 256 bits) (0x00000000)), llvmpipe-25.2.8) | OpenGL ES 3.1.0 (ANGLE 2.1 git hash: a8c8a6febe63)`.
ANGLE reports ES 3.1, and Strobe asked for, and runs, 3.0.

### The library: `strobe-bench --context window --gl angle --api gles`

The run was at 640x360, with 30 warm-up and 20 timed frames. The baseline
is Mesa's own GLES 3.2 on llvmpipe, through the same window path (no
`LD_LIBRARY_PATH`, so glutin finds Mesa's `libEGL.so.1`). **All 16 scenes
compiled and rendered** on both Vulkan devices.

The table gives frame means in ms. The diff columns compare each image
with Mesa GLES: mean absolute difference over 255, then the percentage of
pixels differing by more than 8.

| Scene | Mesa GLES (llvmpipe) | ANGLE-Vulkan (lavapipe) | diff | ANGLE-Vulkan (SwiftShader) | diff |
|---|---:|---:|---|---:|---|
| 01-aurora-gradient | 2.46 | 3.13 | 0.00 / 0.00% | 4.33 | 0.01 / 0.00% |
| 02-plasma-pulse | 3.03 | 3.10 | 0.00 / 0.00% | 6.90 | 0.02 / 0.00% |
| 03-kaleido-tunnel | 1.37 | 2.58 | 0.00 / 0.00% | 5.46 | 0.01 / 0.00% |
| 04-voronoi-glass | 18.76 | 20.53 | 0.04 / 0.00% | 129.86 | 0.28 / 0.69% |
| 05-domain-warp | 14.16 | 15.23 | 0.15 / 0.01% | 55.80 | 0.33 / 0.72% |
| 06-feedback-trails | 2.89 | 3.19 | 0.00 / 0.00% | 9.80 | 1.63 / 0.00% |
| 07-reaction-diffusion | 68.27 | 103.56 | 0.76 / 0.44% | 378.75 | 6.31 / 7.19% |
| 08-bloom-orbs | 16.21 | 22.63 | 0.00 / 0.00% | 55.12 | 0.08 / 0.00% |
| 09-sdf-cathedral | 30.84 | 47.57 | 0.00 / 0.00% | 77.35 | 0.00 / 0.00% |
| 10-volume-clouds | 177.38 | 194.92 | 0.01 / 0.01% | 1559.52 | 0.02 / 0.03% |
| 11-script-orbits | 4.63 | 3.70 | 0.00 / 0.00% | 19.59 | 0.00 / 0.00% |
| 12-silicon-atlas | 51.48 | 55.22 | 0.00 / 0.00% | 329.95 | 0.00 / 0.00% |
| 13-chromatic-rise | 19.04 | 20.63 | 1.73 / 3.78% | 122.99 | 13.22 / 41.02% |
| 14-electric-bloom | 20.63 | 22.80 | 0.00 / 0.00% | 886.29 | 0.20 / 0.35% |
| 15-neon-ribbon-flow | 18.09 | 14.73 | 0.00 / 0.00% | 100.12 | 0.06 / 0.00% |
| 16-acid-mirror | 16.67 | 13.32 | 1.30 / 2.80% | 90.64 | 9.76 / 25.17% |

**Reading it:**

- **lavapipe is the fair comparison.** It is the same llvmpipe rasteriser
  under Mesa's GL and under ANGLE's Vulkan.
  - **Pictures:** identical in 12 of 16 scenes. The other 4 are feedback
    and advection scenes (07, 13, 16, and a trace in 05), where tiny
    differences accumulate. Mesa's own images were identical between its
    headless and window runs, so the differences come from ANGLE.
  - **Speed:** the sum of the 15 unscripted scenes' means is 461 ms on
    Mesa against 543 ms on ANGLE (+18%) in this run. An earlier run, with
    Mesa headless, went the other way (646 against 510). Single scenes
    swing by 30 to 50% between runs in this shared container. So the
    translation layer's cost here is somewhere between nothing and about
    a fifth, on a CPU rasteriser. Only a GPU can say what it costs in a
    show.
- **SwiftShader** is a different CPU rasteriser. Its large differences are
  all in feedback and advection scenes (13, 16, 07), where a different
  rounding grows into a different but equally valid picture. A
  side-by-side of `13-chromatic-rise` shows the same structure.

### The private corpus: `strobe-corpus --dialect gles --context window`

The owner's 90 Synesthesia scenes ran with scripts on and
`STROBE_GLSL_PATH` at the corpus's `engine/lib`, at the harness default of
3 frames at 160x90. The baseline is Mesa GLES on the headless context.
The reports stayed in the scratch folder: they hold scene error text.

- **ANGLE-Vulkan on lavapipe: 90 of 90 ok.**
  - One scene first failed with "Failed to open connection to X server"
    (Xvfb refused a client), and passed on its own rerun.
  - Scripted scenes compiled their script uniforms (46 scenes) as on Mesa.
- **Mesa GLES: 90 of 90 ok**, as D268 recorded.
- **Images:**
  - 47 of 90 final-frame checksums are identical to Mesa's. The rest
    differ in some bits, which includes every scene whose picture builds
    on itself.
  - The same 7 scenes are flagged all-black or all-white on both, so ANGLE
    adds no blank picture.
- **Times:**
  - The per-frame median ratio, ANGLE over Mesa, is 1.50 (10th to 90th
    percentile 0.91 to 2.74), and compile is 1.32.
  - At 160x90 and three frames, that mostly measures fixed per-frame and
    pipeline-creation overhead on a CPU device, not fill rate. G-05's
    GPU runs are the real measure.

### Not measured

- A real GPU's Vulkan driver. The T480 and TORNADO can run the same
  experiment with the shipped DLLs and the variable.
- Present and vsync timing, since the bench renders offscreen.
- Metal, of course.

## 6. Distribution on macOS

- **Where to get the dylibs.** ANGLE publishes no binaries.
  - Electron's `darwin-arm64` and `darwin-x64` zips carry `libEGL.dylib`
    and `libGLESv2.dylib`, the same provenance as D12's Windows DLLs, so
    `scripts/fetch-angle.ps1`'s approach (pinned version, SHA-256 of the
    zip and of each file) carries over.
  - Size: 7.1 MB for the two dylibs, plus 16.7 MB if SwiftShader's Vulkan
    is wanted (it is not, for Metal).
  - Minimum macOS 12.0 for v38.2.0's build.
- **Other sources.**
  - The `comfy-angle` Python wheels repackage the same Electron dylibs,
    arm64 only.
  - Firefox has a bug for building and packaging its own Metal ANGLE dylibs
    on macOS.
  - Building from source needs Chromium's `depot_tools` and GN, which is
    D12's reason for not doing so.
- **Licence.** ANGLE is BSD-3-Clause, and Electron's
  `LICENSES.chromium.html` covers the bundle, as on Windows today.
- **Signing.** The dylibs come signed by Electron. Inside a notarised
  Strobe `.app` with the hardened runtime, library validation requires
  them to be re-signed with Strobe's own identity
  (`codesign --force --sign`). That is part of the macOS release work, not
  a blocker.

## 7. Performance reports

- **Apple ships it.** WebKit adopted ANGLE as Safari's WebGL backend, and
  Apple engineers spent over a year on its Metal backend. WebGL 2 in
  Safari 15 (2021) runs on it ([WebKit wiki](https://trac.webkit.org/wiki/AngleforWebGL),
  [WebKit bug 219759](https://bugs.webkit.org/show_bug.cgi?id=219759)). So
  it is the GLES implementation Apple itself relies on for GLES content on
  Metal.
- **Chrome** moved macOS WebGL from ANGLE-on-GL to ANGLE-on-Metal around
  Chrome 115-117 (2023). Reports are mixed and anecdotal:
  - one developer saw much faster shader compiles and fewer glitches
    ([Defold forum](https://forum.defold.com/t/html5-screen-flickering-when-window-is-being-resized/73440));
  - one saw lower frame rates on their machine
    ([Babylon.js forum](https://forum.babylonjs.com/t/chrome-mac-extremely-slow/48492));
  - Safari 15.2 drew WebGL 2 slowdown complaints
    ([Apple forums](https://developer.apple.com/forums/thread/696821)).
- **No published numbers** compare ANGLE-Metal with Apple's GL 4.1 for
  fragment-heavy workloads like Strobe's. That is the Mac measurement the
  backlog item asks for.

## 8. Verdict for Strobe

| | ANGLE-Vulkan (Win/Linux) | ANGLE-Metal (macOS) |
|---|---|---|
| Shader front end | unchanged: the ES dialect and fixups already pass the corpus on ANGLE | same |
| Renderer code | none | none in the renderer; a new EGL provider (context, display, surface) |
| Selection | env var today; a glutin patch or own provider for a flag | must be explicit (ANGLE defaults to its GL backend on macOS) |
| Evidence | this container: library 16/16, corpus 90/90 (on lavapipe); no GPU yet | none yet; needs a Mac |
| Distribution | already in the shipped zip's source (Electron) | Electron darwin zips, re-signed |

So ANGLE gives Metal and Vulkan without a second renderer, a second shader
pipeline or a second egui painter (`egui_glow` runs on any GL ES context).
It does not give D3D12, and it adds a translation layer, which is why
native GL stays first wherever its driver works (D2). The recommendation
and its triggers are D292.

## 9. macOS's real risk: Apple's OpenGL

- **Deprecated since macOS 10.14 Mojave (2018), frozen at 4.1, still
  shipped.**
  - No removal has been announced for macOS 26 Tahoe (2025) or macOS 27
    (WWDC 2026), as far as public reporting shows.
  - The only direct answer found is a community reply on Apple's own
    forums in August 2025: "Apple made no announcement about the removal
    of OpenGL ... there is no way to know until they say something"
    ([Apple Developer Forums 796543](https://developer.apple.com/forums/thread/796543)).
  - Coverage of the 2018 deprecation made the same point: deprecated is not
    removed
    ([PC Gamer](https://www.pcgamer.com/developers-fear-for-mac-gaming-as-apple-deprecates-opengl-support/),
    [Architosh](https://architosh.com/2018/08/apple-moving-away-from-opengl-what-is-the-answer-for-developers-now/)).
- **What WWDC 2026 did announce is the end of Intel Macs.** macOS 27 runs
  on Apple silicon only, and Rosetta 2 is winding down to a games-only role
  from macOS 28
  ([TechRepublic](https://www.techrepublic.com/article/news-apple-macos-27-drops-intel-mac-support-rosetta/),
  [AppleInsider](https://appleinsider.com/articles/26/06/12/how-and-when-macos-will-finally-stop-support-for-intel-apps)).
  On Apple silicon, Apple's OpenGL 4.1 is itself a layer over Metal; the
  renderer string reads "4.1 Metal - ...".
- **So the risk is gradual, not a date.**
  - GL stays at 4.1 with no fixes.
  - Its tooling goes: Apple's current GPU tools are Metal-only.
  - A future macOS may drop it with a year's notice, as it did 32-bit apps
    and is doing Rosetta.
- **Strobe's own macOS state matters more today.** The renderer crates do
  not build on macOS yet (D55: the headless context is EGL-only), and
  there is no Mac in the test fleet. Any macOS release needs a CGL (or
  ANGLE) provider first. That is the moment to choose ANGLE-Metal rather
  than CGL, because it costs about the same and does not sit on the
  deprecated API.

## Sources

- ANGLE [README](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/README.md),
  [`Display.cpp`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/src/libANGLE/Display.cpp),
  [`gni/angle.gni`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/gni/angle.gni),
  [`DisplayMtl.mm`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/src/libANGLE/renderer/metal/DisplayMtl.mm),
  [`SurfaceMtl.mm`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/src/libANGLE/renderer/metal/SurfaceMtl.mm),
  [`Caps.cpp`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/src/libANGLE/Caps.cpp),
  [`Context.cpp`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/src/libANGLE/Context.cpp),
  [`eglext_angle.h`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/include/EGL/eglext_angle.h),
  [`LICENSE`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/LICENSE)
  (all read on main, 2026-10-15).
- EGL extensions: [`EGL_ANGLE_platform_angle`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/extensions/EGL_ANGLE_platform_angle.txt),
  [`_metal`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/extensions/EGL_ANGLE_platform_angle_metal.txt),
  [`_vulkan`](https://chromium.googlesource.com/angle/angle/+/refs/heads/main/extensions/EGL_ANGLE_platform_angle_vulkan.txt).
- Electron v38.2.0 release zips (`win32-x64`, `linux-x64`, `darwin-arm64`),
  [github.com/electron/electron/releases/tag/v38.2.0](https://github.com/electron/electron/releases/tag/v38.2.0),
  inspected with `unzip -l` and `strings`.
- glutin 0.32.3 source (`glutin/build.rs`, `src/api/egl/display.rs`), and
  [glutin `master`](https://github.com/rust-windowing/glutin) `build.rs`
  and CHANGELOG, 2026-10-15.
- GLFW's ANGLE platform-type hint:
  [GLFW commit](https://fsunuc.physics.fsu.edu/git/gwm17/glfw/commit/0dea8a4441828311af4c31de8ba798ba330c4755)
  (mirror of upstream GLFW 3.4's `GLFW_ANGLE_PLATFORM_TYPE`).
- WebKit: [ANGLE and WebGL 2.0 in WebKit](https://trac.webkit.org/wiki/AngleforWebGL),
  [bug 219759](https://bugs.webkit.org/show_bug.cgi?id=219759).
- Apple OpenGL status: see section 9's links.
- [comfy-angle](https://openapps.pro/packages/comfy-angle);
  [Firefox bug 2055532](https://bugzilla.mozilla.org/show_bug.cgi?id=2055532).
