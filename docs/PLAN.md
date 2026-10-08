# Strobe — Rust Rebuild of a Synesthesia-Style VJ Engine

Working name: **Strobe**. Personal tool. Goals, in priority order:

1. Run Synesthesia-format scenes (`.synScene`) with the **JavaScript control API intact** (`setup`/`update`, `setUniform`, `setControl*`, event handlers).
2. **One-click Shadertoy import** into a playable, audio-reactive scene.
3. **Fast playlist authoring**: long shows built in minutes, not per-item hand entry.
4. Fullscreen output to a projector/LED wall; MIDI/OSC later.

Out of scope for v1: marketplace, licensing/DRM, video media library, Syphon/Spout, ISF import, sound-output shaders.

---

## 1. Research Summary: What Synesthesia Actually Is

### 1.1 Scene format (SSF)

- A scene is a folder ending in `.synScene` containing `main.glsl` (fragment shader), `scene.json` (config), a thumbnail, plus optional `script.js` and an `images/` directory [1].
- `main.glsl` is a GLSL fragment shader; SSF adds built-in uniforms, helper functions, and audio uniforms on top of standard GLSL [1].
- Entry point is `vec4 renderMain(void)`, called once per pass; passes are distinguished with the `PASSINDEX` uniform [2][5].

### 1.2 `scene.json` keys

Documented top-level keys: `TITLE`, `CREDIT`, `DESCRIPTION`, `IMAGE_PATH`, `WIDTH`, `HEIGHT`, `CONTROLS`, `PASSES`, `HARD_TRANSITIONS`, `SMOOTH_TRANSITIONS`, `IMAGES`; the same page also documents `MEDIA` and `CUBEMAPS` [2].

**Controls** — seven base types (`slider`, `knob`, `toggle`, `bang`, `xy`, `color`, `dropdown`) with modifiers `smooth`, `speed`, `counter` [2]. Per-control keys: `NAME` (becomes the uniform name), `DEFAULT`, `MIN`/`MAX` (arrays for multi-dimensional controls), `UI_GROUP`, `DESCRIPTION`, `PARAMS` (smoothing; roughly `1/PARAMS` frames to converge, default `0.01`), `LABELS`/`VALUES` for dropdowns, and `DEF_COLOR` for color defaults [2].

**Passes** — each pass has `TARGET` (exposed as a `sampler2D`), `WIDTH`/`HEIGHT`, `FLOAT` (RGBA32F vs RGBA8, default float), `DOUBLE_BUFFER` (read the pass's own previous frame), `WRAP` (`repeat`/`clamp`/`mirror`), `FILTER` (`linear`/`nearest`/`mipmap`); all passes initialise to `vec4(0)`, and the previous final frame is always available as `syn_FinalPass` [2][5].

**Transitions** — `HARD_TRANSITIONS` pick a new value from a list on detected song change; `SMOOTH_TRANSITIONS` crossfade a one-hot "family" of uniforms over `DURATION` seconds [2].

### 1.3 Standard uniforms

`RENDERSIZE` (current pass size), `TIME`, `FRAMECOUNT`, `_xy`, `_uv`, `_uvc` (aspect-corrected, centered), `PI`, mouse (`_mouse` emulating Shadertoy `iMouse`, `_click`, `_muv`, `_muvc`), `PASSINDEX`, `syn_FinalPass`, `syn_Media`, `syn_MediaType` [5].

### 1.4 Audio uniforms

FFT split into four bands (Bass, Mid, MidHigh, High) [4]. Families: `syn_*Level`, `syn_*Hits`, `syn_*Time` (+ `syn_CurvedTime`), `syn_*Presence`; beat uniforms `syn_OnBeat`, `syn_ToggleOnBeat`, `syn_RandomOnBeat`, `syn_BeatTime`; BPM uniforms `syn_BPM` (50–220), `syn_BPMConfidence`, `syn_BPMTwitcher`, `syn_BPMSin{,2,4,8}`, `syn_BPMTri{,2,4,8}`; long-form `syn_FadeInOut`, `syn_Intensity` [4]. Two data textures: `syn_Spectrum` (r raw FFT, g "juiced" FFT, b smooth FFT, a waveform) and `syn_LevelTrail` (whole/bass/mid/high level history) [4].

### 1.5 JavaScript API (the part that must survive)

From the official scripting reference [3]:

| Surface | Behaviour |
|---|---|
| `setup()` | Runs once at scene load; used to init state and register event handlers |
| `update(dt)` | Runs every frame **before** `main.glsl`; `dt` = seconds since previous frame |
| Globals | Every scene uniform (standard, audio, control) is a JS global; vectors are objects with `xyzw`. Per-pixel/per-pass uniforms (`_uvc`, `PASSINDEX`, `RENDERSIZE`) are excluded |
| `setUniform(name, …)` | 1–4 floats, an array, or `{x,y,z,w}` / `{r,g,b,a}`; type inferred from arity; creates the uniform |
| `setControl(name, values, {dimension, bank})` | Raw-range values; `"meta/"` prefix or `bank:"meta"` targets meta controls |
| `setControlNormalized(...)` | Same, 0–1 normalized |
| `randomizeControl`, `defaultControl`, `randomizeGroup`, `defaultGroup` | By control or `UI_GROUP` name |
| Event registration | `onChange`, `onOffToOn`, `onOnToOff`, `whileOn`, `whileOff` — `(targetName, "callbackName")`; callbacks get `(value, previousValue)`; 0.5 is the on/off threshold |
| Name matching | Control/target names are case-insensitive and ignore spacing characters such as `_` and `-` |
| `textureFinalPass(x, y)` | Returns RGB of the final pass at normalized coords |
| `print()` / `console.log()` / `printGlobalKeys()` | Console output |
| Deprecated | `inputs` and `uniforms` global objects, still supported |
| Persistence | JS objects persist across frames (timers, classes, physics) |

### 1.6 Meta controls, presets, playlists

- Meta controls apply to the whole output: Master, Invert, Brightness/Gamma/Contrast/Hue/Saturation, vertical/horizontal mirror, Reactivity, Audio Speed, Limit Colors (two-color remap), Alpha Channel [6].
- A preset stores all scene controls, meta controls, and active media, with three toggleable channels (Scene/Meta/Media) [7].
- Playlist items are scene+preset pairs; each item gets its own HH:MM:SS duration; options are an audio-driven Auto-Transition (breakdown/drop) and Shuffle [7]. That per-item duration model is the authoring bottleneck this rebuild removes (Section 7).

### 1.7 Shadertoy facts that drive the importer

- Entry point `mainImage(out vec4 fragColor, in vec2 fragCoord)`; inputs `iResolution`, `iTime`, `iTimeDelta`, `iFrame`, `iChannelTime[4]`, `iMouse`, `iDate`, `iSampleRate`, `iChannelResolution[4]`, `iChannel0..3` [8]. The official page lists `iFrame` as `float` [8]; current tooling declares it `int` alongside `iFrameRate` [9][17] — emit `int`, matching WebGL2-era shaders.
- WebGL2 / GLSL ES 3.00 (`#version 300 es`) is the dialect [19][20].
- REST API: `GET /api/v1/shaders/{id}?key=…`, query/search endpoints, assets at `/media/a/{hash.ext}`; **only shaders marked "Public + API" are returned** [8][9][10].
- JSON shape: `ver`, `info{id,name,username,description,tags,…}`, `renderpass[]{inputs[], outputs[], code, name, description, type}`; inputs carry `id`, `src`, `ctype`, `channel`, `sampler{filter, wrap, vflip, srgb, internal}` [11].
- Audio input is a 512×2 texture: row 0 spectrum, row 1 waveform [12][13][14].
- Fetching non-API public shaders server-side runs into Cloudflare; `shadertoy-cli` solves it by driving a real browser (Camoufox) [15]. Browser-side export already exists in the "Shadertoy unofficial plugin" (single-JSON export) [16].
- Each shader's license must be respected; the API terms require it [8], and commercial use needs author permission per the TouchDesigner importer's notice [9]. **This is a work tool: check each imported shader's license before showing it in a commercial setting.**

---

## 2. Core Technical Decisions

### D1 — Renderer: OpenGL via `glow`, not `wgpu`

**Reason: GLSL compatibility.** Synesthesia scenes and Shadertoy shaders are GLSL; `wgpu` ingests GLSL through naga's GLSL frontend, which has gaps that break real Shadertoy code:

- Samplers passed as function parameters are unsupported (gfx-rs/wgpu#5256); a Shadertoy-in-Rust author reports "a lot of shaders" fail on this, and a wgpu maintainer states the glsl-in frontend is not on the team's critical path [18].
- Combined image/samplers are unsupported in naga (#4342, #4323) [21]; GLSL→SPIR-V→naga also hits failures [22][23].
- `const int` switch cases from a Shadertoy example cause a downstream panic (#4644) [24].

Native GL hands GLSL to the driver compiler — the same model Shadertoy (WebGL) and `shadertoy-cli` (OpenGL/EGL renderer) use [15].

**GLSL version strategy:**

| Platform | Context | Shadertoy code | SSF code |
|---|---|---|---|
| Windows / Linux | GL 4.3+ core | Compile **as-is** with `#version 300 es`: `ARB_ES3_compatibility` adds GLSL ES 3.00 and is core in 4.3 [25][26] | `#version 330 core` |
| macOS | GL 4.1 core (deprecated since 10.14, capped at 4.1) [28][29] | Rewrite header to `#version 410 core` + shim (keep `precision` statements, which desktop GLSL accepts from 1.30 [27]; map ES-only differences found by the fixture suite) | `#version 410 core` |

Keep a `Renderer` trait boundary so a wgpu backend can be added later without touching scene/JS/playlist code.

### D2 — Windowing and UI: `winit` + `glutin` + `egui_glow`

- `eframe` defaults to wgpu; glow is opt-in [30]. Use `egui_glow::EguiGlow` directly (the `pure_glow` integration) for full control of the GL context [31][32].
- One GL context, two `glutin` window surfaces (control window, output window), `make_current(&surface)` per window — the pattern `egui-multiwin` uses [33].
- Preview in the control UI: draw the engine's final texture inside egui via `Shape::Callback` (glow paint callback) [30][34].
- Theme: dark, single amber accent, no stock egui palette.

### D3 — JavaScript engine: `rquickjs`

- Safe high-level bindings to QuickJS(-NG); Rust closures become global JS functions via `Function::new` [35].
- `Runtime::set_interrupt_handler` aborts runaway scripts with an uncatchable exception; `set_memory_limit` and `set_max_stack_size` cap resources [35]. A `while(true){}` in a live scene must never freeze the show — this is the deciding feature.
- Alternative: Boa (pure-Rust JS engine) if eliminating the C dependency matters more than speed [36]. Decision: rquickjs for v1.

### D4 — Audio: `cpal` + `realfft`, in-house analysis

- `cpal` input stream from mic/line-in; on Windows, building an input stream on a **render** device sets `AUDCLNT_STREAMFLAGS_LOOPBACK` (system-audio capture) [37].
- `realfft` for real-input FFT on top of RustFFT [38].
- Beat/BPM: implement spectral-flux onsets + autocorrelation tempo in-house. `aubio-rs` provides onset/tempo tracking but is **GPL-3.0** [39] — acceptable for a private tool, kept behind an off-by-default cargo feature.

### D5 — Control I/O

- MIDI: `midir` (ALSA, WinMM/WinRT, CoreMIDI, JACK) [40].
- OSC: `rosc` (OSC 1.0, pure Rust) [41].

### D6 — Video out

- v1: borderless fullscreen output window on a chosen monitor.
- v2: NDI via `grafton-ndi` (NDI 6 SDK bindings, SDK installed separately) [42] or `ndi-sdk-sys` [43]; requires GPU→CPU readback via PBO ring.

### D7 — Packaging

Native binary. Docker Compose is not applicable: the app needs direct GPU, display, and audio-device access.

---

## 3. Architecture

### 3.1 Workspace layout

```
strobe/
├── Cargo.toml                 # [workspace], resolver = "2"
├── crates/
│   ├── strobe-app/            # winit event loop, windows, egui UI, wiring
│   ├── strobe-gfx/            # glow renderer: programs, FBOs, passes, compositor, post
│   ├── strobe-scene/          # SSF model: scene.json serde, controls, presets, shader assembly
│   ├── strobe-script/         # rquickjs host: Synesthesia JS API
│   ├── strobe-audio/          # cpal capture, FFT, bands, beats, BPM, textures
│   ├── strobe-shadertoy/      # import: fetch/accept JSON, convert to scene, assets
│   ├── strobe-playlist/       # playlist model, TOML I/O, scheduler, generator
│   ├── strobe-io/             # MIDI (midir), OSC (rosc), hotkeys, import HTTP listener
│   └── strobe-ssf-lib/        # GLSL: SSF function library + preambles (include_str!)
├── userscript/
│   └── send-to-strobe.user.js # Shadertoy "Send to Strobe" button
└── tests/
    ├── scenes/                # fixture .synScene folders
    ├── shadertoy/             # fixture Shadertoy JSON
    └── js/                    # JS API conformance scripts
```

### 3.2 Threads

| Thread | Work | Communication |
|---|---|---|
| Main / render | winit loop, GL, egui, JS (`update`), playlist clock | owns all GL + JS state |
| Audio callback | copy samples into lock-free ring | `rtrb`/SPSC ring → analysis |
| Audio analysis | FFT, bands, onsets, BPM at ~ 100 Hz | publishes `AudioFrame` via `arc-swap` |
| I/O | MIDI/OSC/HTTP listener | `crossbeam-channel` → main |
| Import worker | HTTP fetch, asset download, file writes | channel → main (GL compile happens on main) |

All fallible paths return `Result<_, StrobeError>` (`thiserror`); no `unwrap`/`expect` outside tests; `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` in every crate.

### 3.3 Frame order

```
1. drain I/O events (MIDI, OSC, hotkeys, UI)          → control writes
2. snapshot AudioFrame; upload syn_Spectrum / syn_LevelTrail / ST audio 512x2
3. playlist tick: advance cue? start/finish transition?
4. for each ACTIVE scene (1, or 2 during a transition):
     a. fire JS event handlers (onChange/… against last-frame values)
     b. call JS update(dt)
     c. apply control smoothing / speed integration
     d. render passes 0..N into FBOs (ping-pong for DOUBLE_BUFFER)
5. composite: transition shader mixes scene A/B finals
6. meta post: brightness/gamma/contrast/hue/sat/invert/mirror/limit-colors/master
7. present: output window (full res), control window (egui + preview)
8. async readback (PBO) for textureFinalPass + thumbnails + NDI
```

Order decision for 4a/4b: events first, then `update`, so `update` sees the effects of handlers. `setControl` writes take effect in the same frame.

---

## 4. Scene Engine (SSF Compatibility)

### 4.1 Rust model

```rust
pub struct SceneDef {
    pub title: String,
    pub credit: Option<String>,
    pub description: Option<String>,
    pub image_path: Option<PathBuf>,
    pub width: u32,           // default 1920
    pub height: u32,          // default 1080
    pub controls: Vec<ControlDef>,
    pub passes: Vec<PassDef>,
    pub hard_transitions: Vec<HardTransition>,
    pub smooth_transitions: Vec<SmoothTransition>,
    pub images: Vec<ImageDef>,
    pub cubemaps: Vec<CubemapDef>,
    pub media: Option<MediaSampling>,
    pub x_strobe: Option<StrobeExt>, // our extension block; see 4.5
}

pub enum ControlKind {
    Slider { mode: Mode }, Knob { mode: Mode },
    Toggle { smooth: bool },
    Bang { variant: BangVariant },   // Plain | Smooth | Counter | CounterSmooth
    Xy { mode: Mode }, Color { smooth: bool },
    Dropdown { smooth: bool, labels: Vec<String>, values: Vec<f32> },
}
pub enum Mode { Plain, Smooth, Speed }
```

Parse `TYPE` strings (`"slider smooth"`, `"bang counter smooth"`, …) with a strict tokenizer; unknown tokens → `SceneError::UnknownControlType { name, ty }`, shown in UI, scene not loaded.

### 4.2 Control runtime semantics

- **Smooth**: Synesthesia converges in ≈ `1/PARAMS` frames [2]. Make it frame-rate independent: `alpha = 1 - (1 - PARAMS).powf(dt * 60.0)`, `value += (target - value) * alpha`.
- **Speed**: output integrates `value * dt` each frame [2].
- **Bang**: `MAX` for one frame; `bang smooth` → attack/decay envelope sized by `PARAMS`; `bang counter` → +1 per press [2].
- **Dropdown**: index → `VALUES[i]`; values default to indices when only `LABELS` is present; labels default to "option N" when only `VALUES` is present [2].
- Name lookup: normalize (`lowercase`, strip whitespace, `_`, `-`) into a `HashMap<String, ControlId>`; collisions between normalized names → load error listing both.

### 4.3 Shader assembly

Compile `main.glsl` **once per pass with `PASSINDEX` as a uniform** (spec-faithful), cached by `(source hash, pass index, platform)`.

```glsl
#version 330 core                // 410 core on macOS
// ---- strobe preamble (generated) ----
uniform vec2  RENDERSIZE;
uniform float TIME;
uniform float FRAMECOUNT;
uniform int   PASSINDEX;
uniform vec4  _mouse;  uniform vec3 _click;  uniform vec2 _muv;  uniform vec2 _muvc;
uniform sampler2D syn_FinalPass;
uniform sampler2D syn_Media;  uniform float syn_MediaType;
// audio scalars (generated from AudioUniform enum)
uniform float syn_Level, syn_BassLevel, /* … every name in §1.4 … */ syn_Intensity;
uniform sampler1D syn_Spectrum;   // docs call these 1D samplers [4]; verify against shipped scenes
uniform sampler1D syn_LevelTrail;
// controls (generated): float / vec2 / vec3 per ControlKind
// pass targets (generated): uniform sampler2D <TARGET>;
// images / cubemaps (generated): sampler2D / samplerCube
// JS-created uniforms (generated on first setUniform, triggers recompile once)
const float PI = 3.14159265359;
vec2 _xy; vec2 _uv; vec2 _uvc;
out vec4 strobe_FragColor;
#define texture2D texture        // legacy-source safety
#include <ssf_lib.glsl>          // §4.4
#line 1
// ---- user main.glsl ----
...
// ---- strobe epilogue ----
void main() {
    _xy  = gl_FragCoord.xy;
    _uv  = _xy / RENDERSIZE;
    _uvc = (_uv - 0.5) * vec2(RENDERSIZE.x / RENDERSIZE.y, 1.0);
    strobe_FragColor = renderMain();
}
```

`_uvc` formula is a reconstruction from the description [5]; validate visually against a reference scene before Phase 1 sign-off. `#line 1` keeps driver error line numbers aligned with the user file; the UI maps errors back to `main.glsl`.

**JS-created uniforms**: `setUniform` in `setup()` defines the uniform set before first compile. A new name appearing in `update()` triggers one recompile and a console warning recommending `setup()` initialisation (the docs recommend this practice [3]).

### 4.4 SSF function library

Re-implement every documented function from signatures and descriptions [5b]:

- Math: `_scale`, `_map` (float–vec4), `_smin`, `_rand`, `_pulse`, `_sqPulse`, `_inRange`, `_triWave`, `_pixelate`, `_mix3/4/5`, `_nsin`, `_ncos`, `_nclamp`
- Noise: `_noise` (1–3D), `_fbm` (+octave), `_statelessContinuousChaotic`, `_hash{out}{in}`
- Coordinates: `_uv2uvc`, `_uvc2uv` (inout), `_rotate` (degrees), `_mirror`, `_toPolar`, `_toPolarTrue`, `_toRect`
- Color: `_rgb2hsv`, `_hsv2rgb`, `_normalizeRGB`, `_palette` (IQ cosine), `_brightness`, `_gamma`, `_contrast`, `_saturation`, `_hueRotate`, `_invert`, `_luminance`, `_grayscale`, `_dichrome`
- Media: `_loadMedia`, `_textureMedia`, `_loadMediaAsMask`, `_textureMediaAsMask`, `_correctMediaCoords`, `_isMediaActive`, `_edgeDetectMedia`, `_edgeDetectSobelMedia`
- Multipass: `_edgeDetect`, `_edgeDetectSobel`, `_hBlurTiltShift`, `_vBlurTiltShift`

Exact numeric output will differ from Synesthesia's internal implementation; the goal is behavioural equivalence. Golden-image tests lock our own output.

### 4.5 Extension block (`"X_STROBE"`)

Unknown-key tolerance in Synesthesia is undocumented, so keep extensions in one namespaced key and never alter documented keys:

```json
"X_STROBE": {
  "SOURCE": { "kind": "shadertoy", "id": "XsXXDn", "url": "...", "author": "...", "license_note": "..." },
  "PASS_SOURCES": ["passes/buffer_a.glsl", "passes/image.glsl"],
  "PASS_DIALECT": "glsl_es_300",
  "TAGS": ["tunnel", "warm"],
  "AUDIO_DRIVE": { "time_mode": "blend", "default_reactivity": 0.5 }
}
```

`PASS_SOURCES` lets imported Shadertoy scenes keep **one source file per pass** instead of merging into a `PASSINDEX` switch (avoids name collisions between buffers).

### 4.6 Features deferred

`HARD_TRANSITIONS`/`SMOOTH_TRANSITIONS` need song-change detection → Phase 7. `MEDIA`/`syn_Media` (video) → Phase 7; bind a 1×1 black texture and `syn_MediaType = 0` until then.

---

## 5. JavaScript Host (`strobe-script`)

### 5.1 Lifecycle

- One `rquickjs::Runtime` per **scene instance** (memory limit 64 MiB, stack 1 MiB) so a crashed scene cannot poison another; two instances exist only during transitions.
- Load: evaluate `script.js` → call `setup()` if defined → register handlers.
- Per frame: write globals → dispatch events → `update(dt)` → harvest uniforms.
- Interrupt handler: deadline-based (e.g. 4 ms budget per call, configurable). On interrupt or exception: log to console with file:line, keep rendering with last good uniforms, mark scene "script faulted" in the UI. Three consecutive faults → script disabled until reload.

### 5.2 Globals bridge

| Uniform kind | JS value |
|---|---|
| float | `number` |
| vec2/3/4 | object `{x,y,z,w}` (also `r,g,b,a` aliases for color) |
| Excluded | `_xy`, `_uv`, `_uvc`, `PASSINDEX`, `RENDERSIZE` [3] |

Update strategy: pre-create global property objects once; per frame only write numeric fields (avoid allocation churn). Deprecated `inputs` (getter proxy over the same values) and `uniforms` (object scanned after `update`) are provided for old scenes [3].

### 5.3 Native functions (all registered via `Function::new`)

```text
setUniform(name, ...)                       // 1–4 numbers | array | {x,y,z,w} | {r,g,b,a}
setControl(name, value|array, {dimension?, bank?})
setControlNormalized(name, value|array, {dimension?, bank?})
randomizeControl(name)   defaultControl(name)
randomizeGroup(group)    defaultGroup(group)
onChange(target, "cb")   onOffToOn(target, "cb")   onOnToOff(target, "cb")
whileOn(target, "cb")    whileOff(target, "cb")
textureFinalPass(x, y) -> [r, g, b]
print(...)  console.log(...)  printGlobalKeys()
```

- Name resolution: `"meta/brightness"` prefix or `bank: "meta"`; normalized names as §4.2 [3].
- Event targets accept any input global (control, audio, standard); thresholds at 0.5 [3]. Callback names must match exactly; a missing callback → one console error at registration, not per frame.
- `textureFinalPass`: return values from the previous frame's async readback of a 64×36 downsample of the final pass (no GPU stall). Documented as one-frame latent.
- Errors from native functions become JS exceptions (`rquickjs` converts `Err` into a thrown exception [35]).

### 5.4 Conformance tests

One JS fixture per API surface, run headless (offscreen GL via EGL on Linux CI): uniform arity inference; vector/color object forms; control set/normalized/dimension/bank; randomize/default; each event type with synthetic input sequences; interrupt kill of `while(true){}`; memory-limit breach; deprecated `inputs`/`uniforms`. Assertions on the harvested uniform map, not pixels.

---

## 6. Shadertoy One-Click Import (`strobe-shadertoy`)

### 6.1 Acquisition paths

| Path | Covers | One-click? | Notes |
|---|---|---|---|
| **A. Userscript button** "Send to Strobe" on `shadertoy.com/view/*` | All public shaders you can view | Yes | Runs in your logged-in browser, so Cloudflare is not a factor. POSTs JSON to `http://127.0.0.1:47811/import` via Tampermonkey `GM_xmlhttpRequest` with `@connect 127.0.0.1` (bypasses page CORS) |
| **B. Paste URL / ID** in Strobe | "Public + API" shaders only [8][10] | Yes | `GET /api/v1/shaders/{id}?key=…`; key from `shadertoy.com/myapps` [10]; stored in OS keychain (`keyring` crate) |
| **C. Drop / watched folder** | Anything exported by the Shadertoy unofficial plugin's single-JSON export [16] | Drag or auto | Watch `~/Downloads/*.json` with `notify`; accept files whose root parses as Shadertoy JSON |
| D. Batch: paste N URLs | B's coverage | Yes | Creates N scenes + optional playlist (§7.5) |

Path A implementation detail to confirm in Phase 0: the shader JSON source inside the page. Preferred method: the userscript issues the same same-origin request the page makes when loading a shader (inspect the DevTools Network tab on a `/view/` page), so it receives the API-shaped JSON without scraping DOM state. Fallback: path C.

Local listener: bound to `127.0.0.1` only, per-install random token in the userscript header (`X-Strobe-Token`), 2 MiB body cap, JSON schema validation before anything touches disk.

### 6.2 Conversion

**Pass mapping**

| Shadertoy `type` | Strobe |
|---|---|
| `common` | Prepended to every pass source |
| `buffer` (A–D) | `PASSES[]` entry, `FLOAT: true`, `DOUBLE_BUFFER: true` (lets a buffer sample its own previous frame for feedback), size = output |
| `image` | Final pass |
| `cubemap`, `sound` | Rejected in v1 with explicit reason; scene goes to "Needs work" bin |

Execution rule: Buffer A → B → C → D → Image; a read of a pass that has not yet run this frame returns that pass's previous frame. Confirm against multipass fixtures in Phase 0.

**Input (`ctype`) mapping**

| ctype | Binding |
|---|---|
| `texture` | Download `/media/a/{hash}` once into `images/`; honour `sampler.filter/wrap/vflip` [11] |
| `buffer` | Bind the referenced pass target (current or previous frame per order rule) |
| `keyboard` | 256×3 texture (row 0 held, row 1 one-frame press pulse [14]), zeros until keyboard input lands in Phase 7 |
| `music`, `musicstream`, `mic` | **Live audio**: 512×2 R8, row 0 spectrum, row 1 waveform [12][14] — makes audio-reactive Shadertoys react to the room immediately |
| `video`, `webcam` | 1×1 black + warning (Phase 7) |
| `cubemap`, `volume` | Rejected v1 |

**Uniform shim** (generated preamble per pass, dialect `#version 300 es` on Win/Linux):

```glsl
#version 300 es
precision highp float; precision highp int; precision highp sampler2D;
uniform vec3  iResolution;
uniform float iTime, iTimeDelta, iFrameRate, iSampleRate;
uniform int   iFrame;
uniform float iChannelTime[4];
uniform vec3  iChannelResolution[4];
uniform vec4  iMouse, iDate;
uniform sampler2D iChannel0, iChannel1, iChannel2, iChannel3;
// strobe audio-drive extras (also available to edited shaders)
uniform float syn_BassLevel, syn_Level, syn_BassHits, syn_BPMTwitcher;
uniform float strobe_Reactivity;
out vec4 strobe_FragColor;
// <common>
#line 1
// <pass code>
void main() { vec4 c = vec4(0.0); mainImage(c, gl_FragCoord.xy); strobe_FragColor = vec4(c.rgb, 1.0); }
```

Alpha forced to 1.0 on the Image pass because Shadertoy ignores it [8].

### 6.3 Making imports playable without editing

Every imported scene gets an auto-generated control group **"Strobe"**:

| Control | Type | Effect |
|---|---|---|
| `speed` | slider 0–3, default 1 | `iTime = st_clock` where clock advances by `dt * speed` |
| `audio_time` | slider 0–1 | Blends clock advance toward a bass-weighted rate (`dt * speed * mix(1, 0.3 + 2.0*syn_BassLevel, audio_time)`) — motion surges with the kick |
| `bass_flash` | slider 0–1 | Post-multiply brightness by `1 + bass_flash * syn_BassHits` |
| `zoom` | slider 0.5–2 | Scales `fragCoord` around center before `mainImage` |
| `hue` | slider -180–180 | Post hue rotate |

Implemented host-side (clock) and in the epilogue (`fragCoord` transform, post color), so the original code stays untouched.

**Promote-to-control**: the importer scans top-level `#define NAME <float literal>` and `const float NAME = <literal>;`. The UI lists them; one click rewrites the line to a `uniform float NAME;` and adds a slider with range `[0, 2×default]` (or `[-2|d|, 2|d|]` for negatives). Edit is applied to the local copy only.

### 6.4 Validation on import

1. Schema-validate JSON → 2. download assets (timeout 10 s each, SHA-256 cache) → 3. write scene folder atomically (temp dir + rename) → 4. compile all passes on main thread → 5. render 120 frames offscreen at 480×270 with silent audio, grab frame 120 as thumbnail, reject all-black/NaN output as "suspect" → 6. add to library with tags from `info.tags`.

Failures land in **Needs work** with the driver log, mapped line numbers, and the offending pass name.

### 6.5 Userscript

```js
// ==UserScript==
// @name         Send to Strobe
// @match        https://www.shadertoy.com/view/*
// @grant        GM_xmlhttpRequest
// @connect      127.0.0.1
// ==/UserScript==
```

Body: adds a button next to the shader title; on click, obtains the shader JSON (method confirmed in Phase 0), POSTs `{ shader, pageUrl }` with the token header, shows the response (`imported`, `needs work: <reason>`, `already in library`).

---

## 7. Playlists (`strobe-playlist`)

### 7.1 Model: inheritance instead of per-item entry

Synesthesia makes every item carry its own duration [7]. Strobe resolves every field through **playlist defaults → section defaults → cue overrides**; a cue that only names a scene is complete.

```rust
pub struct Playlist {
    pub name: String,
    pub defaults: CueSettings,        // all fields Some(..) at this level
    pub sections: Vec<Section>,
    pub anchor: Option<WallClockAnchor>, // e.g. start at 09:00 local
    pub loop_mode: LoopMode,          // Stop | Loop | LoopSection
    pub shuffle: Option<ShuffleSpec>, // seed + no-repeat window
}
pub struct Section { pub name: String, pub defaults: CueSettings, pub cues: Vec<Cue> }
pub struct Cue {
    pub scene: SceneRef,              // library id or path
    pub preset: Option<String>,
    pub settings: CueSettings,        // all Option<..>; None = inherit
    pub overrides: BTreeMap<String, ControlValue>, // control name → value
    pub enabled: bool,
}
pub struct CueSettings {
    pub duration: Option<DurationSpec>,
    pub advance: Option<AdvanceMode>,
    pub transition: Option<TransitionSpec>,
    pub meta: Option<MetaOverrides>,
}
pub enum DurationSpec {
    Seconds(f32), Bars(u32), Beats(u32),
    Random { min: Box<DurationSpec>, max: Box<DurationSpec> },
}
pub enum AdvanceMode { Timer, Manual, AudioEvent(AudioEventKind), FirstOf(Vec<AdvanceMode>) }
pub struct TransitionSpec { pub kind: TransitionKind, pub duration: DurationSpec }
pub enum TransitionKind { Cut, Crossfade, FadeThroughBlack, Shader(PathBuf) }
```

### 7.2 File format (TOML, hand-editable, hot-reloaded)

```toml
name = "Office lobby — weekday"
loop_mode = "loop"

[defaults]
duration   = "3m"
advance    = "timer"
transition = { kind = "crossfade", duration = "4s" }

[shuffle]
seed = 1337
no_repeat_window = 10

[[sections]]
name = "Morning"
defaults = { duration = "rand(2m, 4m)" }
cues = [
  { scene = "warm-tunnel" },
  { scene = "voronoi-geode", preset = "slow" },
  { scene = "ink-drift", duration = "32 bars", transition = { kind = "cut" } },
]

[[sections]]
name = "Afternoon"
defaults = { duration = "90s", transition = { kind = "fade-through-black", duration = "2s" } }
cues = [
  { scene = "lattice", overrides = { speed = 0.6 } },
  { scene = "ST-XsXXDn" },
]
```

Duration grammar: `90s`, `3m`, `1h30m`, `1:30`, `32 bars`, `64 beats`, `rand(a, b)`. Parser returns precise errors (`line 14: "3mn" is not a duration`); invalid files keep the previous playlist loaded.

### 7.3 Authoring UX (the time savers)

| Action | Result |
|---|---|
| Drag scenes (or a whole tag/folder) from library | One cue each, all fields inherited |
| Multi-select cues → edit field | Sets override on all selected; "Reset to inherited" per field |
| **Generate…** dialog | Inputs: target length (e.g. 4 h), pool (tags/folders/favorites), duration rule, transition rule, no-repeat window, seed → fills a section; preview shows total runtime |
| Paste list of Shadertoy URLs | Batch-import (§6.1 D) and append cues in pasted order |
| Duplicate section / cue block | Copies with inheritance intact |
| Text mode | Opens the TOML in the system editor; file watcher reloads on save |
| Timeline view | Computed start time for every cue (random durations resolved by seed), total runtime, wall-clock times when anchored |

### 7.4 Playback engine

- **Precompile at load**: compile every distinct scene in the playlist on load (progress bar); cues whose scene fails are marked and auto-skipped at runtime with a console entry. Zero shader compiles during playback.
- **Warm-up**: 2 s before a transition, start the incoming scene rendering at quarter resolution off-screen so feedback/`DOUBLE_BUFFER` scenes are not black on entry; switch to full resolution at transition start.
- **Transitions**: both scenes render during the transition; compositor mixes `A`, `B`, `progress`. Custom transition = GLSL fragment with `sampler2D from, to; float progress;`.
- **Beat-locked durations**: `Bars`/`Beats` use `syn_BPM` when `syn_BPMConfidence` exceeds a threshold, otherwise fall back to a configured BPM; cue ends on the next downbeat after the computed time.
- **Live controls** (UI, hotkeys, MIDI, OSC): next, previous, hold (freeze clock on current cue), extend +30 s, restart cue, jump to cue, skip section, blackout.
- **Resume**: playback position persisted every 5 s; restart resumes at the same cue/offset.
- **Wall-clock anchor**: schedule computed from the anchor; late start jumps to the cue that should be playing now.

### 7.5 Audio-event advance (Phase 7)

`AudioEvent(Drop | Breakdown)`: drop = sharp rise in `syn_Presence` after a sustained low; breakdown = sustained bass-presence collapse. Mirrors the Auto-Transition concept [7]; thresholds tunable per playlist.

---

## 8. Audio Analysis (`strobe-audio`)

| Stage | Spec |
|---|---|
| Capture | `cpal`, f32, mono downmix, 48 kHz preferred; device + loopback selection in settings [37] |
| FFT | 2048-point Hann, hop 512, `realfft` [38] |
| Bands | Bass 20–150 Hz, Mid 150–2 kHz, MidHigh 2–6 kHz, High 6–16 kHz (four-band split per [4]; edges are our choice) |
| Level | RMS per band → adaptive normalization (running max with slow decay) → 0–1, asymmetric smoothing |
| Hits | Positive spectral flux per band, normalized, fast decay |
| Time clocks | `syn_*Time += dt * f(level)`; `syn_CurvedTime` uses a convex `f` |
| Presence | Long-window (~4 s) level average |
| Beat | Onset peak-picking on bass flux; `syn_OnBeat` impulse; `syn_ToggleOnBeat`, `syn_RandomOnBeat` with logistic smoothing; `syn_BeatTime` counter |
| BPM | Autocorrelation of onset envelope over 8 s, clamped 50–220 [4]; confidence = peak sharpness; phase-locked oscillators drive `syn_BPMSin*`, `syn_BPMTri*`, `syn_BPMTwitcher` |
| Long-form | `syn_FadeInOut`, `syn_Intensity`: very slow followers of overall level |
| Textures | `syn_Spectrum` 1D RGBA32F (raw/juiced/smooth/wave) and `syn_LevelTrail` 1D RGBA32F history [4]; Shadertoy 512×2 R8 [12] |
| Manual | Tap tempo, BPM nudge, global Reactivity and Audio Speed meta controls [6] |

Values will not match Synesthesia numerically; scenes are tuned by ear, so behaviour (ranges, shapes) is what matters. Calibration harness: offline WAV in → CSV of all uniforms out, plotted for tuning.

---

## 9. Meta Controls and Post

Single post shader after compositing, implementing the documented set [6]: Master (not saved in presets), Invert, Brightness, Gamma, Contrast, Hue, Saturation, Vert/Hor Mirror, Limit Colors (low/high color remap), Reactivity (scales audio uniforms before upload), Audio Speed (scales audio clocks), Alpha Channel (preserve alpha for NDI). Addressable from JS as `meta/<name>` [3].

---

## 10. Presets and Storage

```
<data_dir>/strobe/
├── config.toml                 # devices, output monitor, ports, theme, API-key ref
├── library/
│   ├── warm-tunnel.synScene/   # SSF folder (+ presets/)
│   │   └── presets/default.json, slow.json
│   └── ST-XsXXDn.synScene/     # imported (X_STROBE.SOURCE)
├── needs-work/                 # failed imports with logs
├── playlists/*.toml
├── cache/programs/             # compile cache keyed by source hash
└── state/playback.json         # resume point
```

Preset JSON: `{ "name", "channels": {"scene":true,"meta":true}, "scene": {ctrl: value}, "meta": {ctrl: value} }` — channel semantics as documented [7]. Startup-default preset per scene supported.

Synesthesia scene compatibility: point Strobe at your own Synesthesia scene folders read-only. Check Synesthesia's EULA before loading marketplace/built-in scenes in another engine.

---

## 11. Phased Build Plan

Each phase ends with its acceptance tests passing in CI (Linux, EGL headless) and a manual check on the work machine.

### Phase 0 — Spikes (de-risk)

| Spike | Pass criteria |
|---|---|
| GL: one context, two window surfaces, egui in one, fullscreen quad in other | 60 fps both windows, no context errors, on Windows + work OS |
| Shadertoy dialect | 50 hand-picked Shadertoy JSON fixtures (incl. sampler-as-parameter, `const int` switch, multipass) compile with `#version 300 es` on Win/Linux; record failures; macOS `410` rewrite success rate recorded |
| rquickjs | `while(true){}` interrupted within budget; 10 k `setUniform` calls/frame < 1 ms |
| Userscript JSON acquisition | Confirmed request shape; JSON validates against `renderpass` schema |
| Loopback audio | System audio captured on the work machine |

### Phase 1 — Core renderer + SSF single/multi-pass + controls UI

Scene loading, shader assembly (§4.3), SSF lib (§4.4), passes with ping-pong, images/cubemaps, `syn_FinalPass`, control widgets for all types, smoothing/speed, output window, error console.
Accept: 10 fixture scenes render; golden images within tolerance; malformed `scene.json` produces UI error, no panic.

### Phase 2 — JavaScript API

Full §5. Accept: JS conformance suite green; faulted scripts never stall a frame beyond budget.

### Phase 3 — Audio

Full §8 with tap tempo. Accept: offline WAV harness produces stable BPM (±2) on 10 reference tracks; live latency audio→uniform < 30 ms.

### Phase 4 — Shadertoy import

Paths A, B, C, D; conversion; audio channel mapping; auto "Strobe" control group; promote-to-control; thumbnails; Needs-work bin.
Accept: 80 %+ of Phase 0 fixture set imports and plays without edits on Win/Linux; every failure carries an actionable message.

### Phase 5 — Playlists

Model, TOML I/O, inheritance, generator, timeline, precompile, warm-up, transitions (cut/crossfade/fade-black/shader), beat-locked durations, live controls, resume, wall-clock anchor.
Accept: a 4-hour, 80-cue playlist authored from tags in under 2 minutes; 4-hour soak with zero dropped transitions and zero runtime compiles.

### Phase 6 — Presets, MIDI, OSC

Preset save/recall with channels, fav slots, MIDI learn (`midir`), OSC address map (`rosc`) for controls and playlist actions.
Accept: every playlist action and control reachable from MIDI and OSC.

### Phase 7 — Extras

NDI out, media/webcam (`syn_Media`), song-change detection → `HARD/SMOOTH_TRANSITIONS` + audio-event advance, keyboard texture input, wgpu backend evaluation.

---

## 12. Dependencies

Versions verified during research where noted; pin the rest with `cargo add` at Phase 0.

| Crate | Purpose | Note |
|---|---|---|
| `glow` | OpenGL bindings | renderer core |
| `glutin`, `glutin-winit`, `winit` | context + windows | one context, multiple surfaces [33] |
| `egui`, `egui_glow`, `egui-winit` | UI | `EguiGlow` integration [31] |
| `rquickjs` | JS | interrupt/memory limits [35] |
| `cpal` | audio capture | WASAPI loopback [37] |
| `realfft` 3.5.0 | FFT | [38] |
| `midir` 0.11.0 | MIDI | [40] |
| `rosc` 0.11.4 | OSC | [41] |
| `serde`, `serde_json`, `toml` | config, scenes, playlists | |
| `thiserror` | error types | no panics policy |
| `notify` | watched folders, hot reload | |
| `ureq` or `reqwest` (blocking, rustls) | Shadertoy API + assets | import worker thread |
| `tiny_http` | localhost import listener | 127.0.0.1 only |
| `keyring` | API key storage | |
| `image` | textures, thumbnails | |
| `crossbeam-channel`, `arc-swap`, `rtrb` | threading | |
| `sha2` | asset/program cache keys | |
| `aubio-rs` (optional feature) | onset/tempo | GPL-3.0 [39] |
| `grafton-ndi` (Phase 7) | NDI out | NDI SDK required [42] |

---

## 13. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| macOS GL 4.1 cap and deprecation [28][29] | `#version 300 es` path unavailable; future removal | Header rewrite shim; `Renderer` trait keeps a wgpu/Metal path open |
| Driver-specific GLSL quirks (AMD vs NVIDIA strictness) | Shader works on one GPU, fails on another | Fixture suite run on the work GPU; per-scene "known good on" metadata |
| Userscript acquisition method changes when Shadertoy updates its site | Path A breaks | Paths B and C remain; userscript is ~50 lines to patch |
| Shadertoy API covers only Public+API shaders [8] | Paste-URL import misses many shaders | Path A/C cover the rest |
| Audio analysis differs from Synesthesia | Existing scenes feel different | Calibration harness; per-band gain trims in settings |
| JS scene stalls | Frozen output | Interrupt handler budget, fault counter [35] |
| Licensing of imported shaders in a workplace | Legal exposure | License note captured per scene; filter "cleared for work" tag in generator pools [8][9] |
| Transition GPU cost (two scenes live) | Frame drops on heavy raymarchers | Outgoing scene drops to half resolution during transition; per-scene cost estimate from Phase 1 profiler |

---

## 14. Items to Verify During Implementation

1. Exact in-page/same-origin request used to load shader JSON on `/view/` pages (Path A).
2. `_uvc` formula and `syn_Spectrum`/`syn_LevelTrail` sampler dimensionality against shipped Synesthesia scenes.
3. Whether Synesthesia ignores unknown `scene.json` keys (only matters if Strobe-edited scenes go back into Synesthesia).
4. Synesthesia EULA terms for running its scenes outside the app.
5. Work machine OS and GPU; decides whether the `#version 300 es` path is available.
6. Shadertoy's exact pass execution order and same-frame vs previous-frame buffer reads (§6.2 rule).

---

## Sources

1. Synesthesia Docs — Synesthesia Shader Format (SSF): https://app.synesthesia.live/docs/ssf/ssf.html
2. Synesthesia Docs — JSON Configuration: https://app.synesthesia.live/docs/ssf/json.html
3. Synesthesia Docs — JavaScript Scripting: https://app.synesthesia.live/docs/ssf/script.html
4. Synesthesia Docs — SSF Audio Uniforms: https://app.synesthesia.live/docs/ssf/audio_uniforms.html
5. Synesthesia Docs — SSF Standard Uniforms: https://app.synesthesia.live/docs/ssf/standard_uniforms.html
5b. Synesthesia Docs — SSF Functions: https://app.synesthesia.live/docs/ssf/functions.html
6. Synesthesia Docs — Controls & Interface: https://app.synesthesia.live/docs/manual/controls.html
7. Synesthesia Docs — Presets, FavSlots & Playlists: https://app.synesthesia.live/docs/manual/presets.html
8. Shadertoy — How To (shader inputs, API): https://www.shadertoy.com/howto
9. matthewwachter/td-shadertoy (API-only availability, commercial-use notice): https://www.github.com/matthewwachter/td-shadertoy
10. shadertoy-api (Python) on PyPI (Public+API only, key at /myapps): https://pypi.org/project/shadertoy-api/
11. `shadertoy` crate types (renderpass/input/sampler JSON schema): https://docs.rs/shadertoy/latest/i686-apple-darwin/src/shadertoy/types.rs.html
12. soulthreads — Some details on Shadertoy FFT (512×2 audio texture): https://gist.github.com/soulthreads/2efe50da4be1fb5f7ab60ff14ca434b8
13. polyfloyd/shady — audio loader 512×2 layout: https://pkg.go.dev/github.com/polyfloyd/shady
14. shadertoy-cli channels doc (music texture layout): https://docs.rs/crate/shadertoy-cli/latest/source/assets/docs/channels.md
15. shadertoy-cli 2.5.1 (Camoufox import, GL/EGL renderer): https://docs.rs/crate/shadertoy-cli/latest
16. Shadertoy unofficial plugin (JSON export): https://chrome.google.com/webstore/detail/ohicbclhdmkhoabobgppffepcopomhgl
17. GameDev.net ShaderLab — Hello, Fragment Shader (iFrame int, iFrameRate): https://gamedev.net/shaderlab/2-hello-fragment-shader/
18. r/rust — wgpu v24 AMA thread (naga sampler-parameter gap, #5256, maintainer bandwidth): https://redlib.hbubli.cc/r/rust/comments/1i28o61/wgpu_v2400_released/m7df9nl/?context=3
19. Wikipedia — OpenGL Shading Language (GLSL ES 3.00 ↔ WebGL 2.0, `#version 300 es`): https://en.wikipedia.org/wiki/OpenGL_Shading_Language
20. esshader config.h (Shadertoy GLSL 300 es preamble): https://github.com/chrismcfee/esshader/blob/master/config.h
21. gfx-rs/wgpu Discussion #5440 (no combined image/samplers in naga): https://www.Github.com/gfx-rs/wgpu/discussions/5440
22. gfx-rs/wgpu Issue #5994 (glslc SPIR-V rejected by naga): https://github.com/gfx-rs/wgpu/issues/5994
23. spirv-webgpu-transform README (SPIR-V patterns naga/tint reject): https://docs.rs/crate/spirv-webgpu-transform/0.1.6/source/README.md
24. gfx-rs/wgpu Issue #4644 (Shadertoy `const int` switch panic): https://github.com/gfx-rs/wgpu/issues/4644
25. GL_ARB_ES3_compatibility specification (adds GLSL ES 3.00): https://developer.download.nvidia.com/opengl/specs/GL_ARB_ES3_compatibility.txt
26. LWJGL ARBES3Compatibility (promoted to core in OpenGL 4.3): https://cs.unh.edu/~cs770/lwjgl-javadoc/lwjgl-opengl/org/lwjgl/opengl/ARBES3Compatibility.html
27. Khronos community — precision qualifiers unsupported only before GLSL 1.30: https://community.khronos.org/t/problem-with-amd-gpu-and-gl-arb-es3-1-compatibility/75243
28. Apple OpenGL Programming Guide (OpenGL deprecated in macOS 10.14): https://developer.apple.com/library/mac/documentation/graphicsimaging/Conceptual/OpenGL-MacProgGuide/opengl_pixelformats/opengl_pixelformats.html
29. Geeks3D — macOS 10.14 deprecates OpenGL; macOS supports only 4.1: https://geeks3d.com/20180605/apple-announces-that-opengl-and-opencl-will-be-deprecated-in-macos-10-14-mojave
30. egui / eframe README (wgpu default, glow opt-in, Shape::Callback): https://github.com/emilk/egui/blob/main/crates/eframe/README.md
31. egui_glow `EguiGlow` winit integration: https://github.com/emilk/egui/blob/main/crates/egui_glow/src/winit.rs
32. egui_glow `pure_glow` example: https://github.com/emilk/egui/blob/main/crates/egui_glow/examples/pure_glow.rs
33. egui-multiwin tracked_window.rs (glutin context + per-window surfaces): https://docs.rs/crate/egui-multiwin/latest/source/src/tracked_window.rs
34. egui custom_3d_glow example (glow paint callback): https://github.com/emilk/egui/blob/main/examples/custom_3d_glow/src/main.rs
35. rquickjs docs — Runtime (interrupt handler, memory/stack limits) and Function: https://github.com/delskayn/rquickjs/blob/master/_autodocs/api-reference/runtime.md
36. Boa JavaScript engine: https://github.com/boa-dev/boa
37. cpal WASAPI device.rs (loopback flag on render devices): https://github.com/rustaudio/cpal/blob/master/src/host/wasapi/device.rs
38. realfft docs: https://docs.rs/realfft
39. aubio-rs (onset/tempo, GPL-3.0): https://docs.rs/crate/aubio-rs
40. midir 0.11.0: https://docs.rs/crate/midir/
41. rosc 0.11.4: https://docs.rs/crate/rosc/latest/source/src/lib.rs
42. grafton-ndi: https://docs.rs/grafton-ndi
43. ndi-sdk-sys: https://docs.rs/crate/ndi-sdk-sys/latest
