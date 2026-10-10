# strobe-shadertoy

Shadertoy import. This crate parses a Shadertoy URL or id, reads a shader's
JSON in either of its two shapes, and converts it into an ordinary Strobe
scene folder that the player runs like any other scene. The converter is
deterministic: the same JSON and options always give byte-identical files.
It never edits a statement of the original code; it renames, deletes lines
in place (so line numbers survive), and wraps the code in a generated shim.

Stage 1 is the converter. **Stage 2** adds the ways to get a shader's JSON:
`strobe-import --id` (the REST API, with a key), a loopback listener that
the "Send to Strobe" userscript posts to (`tools/userscript/`), and a
texture downloader with an on-disk cache. The app's Import panel and file
drop use the same pipeline (`import::import_json`). None of it has run
against the live site, only against fakes ([ROADMAP.md](../../docs/ROADMAP.md),
Q19; D110-D117).

## Role in the workspace

- **Depends on:** `strobe-scene` (to validate what it writes), plus serde,
  serde_json, thiserror and clap. It has no GL dependency.
- **Used by:** the `strobe-import` binary in this crate, and `strobe-app`
  (its import worker and the listener). `strobe-gfx` only uses it as a dev-dependency, for render
  tests.
- **Does not:**
  - touch the network: acquisition is a separate, later layer;
  - render: the scene folder it writes is played by `strobe-gfx`;
  - generate controls from the shader's literals: that is the planned
    auto-generated controls engine.

The scene format gained a few keys for imports, documented in
[DECISIONS.md](../../docs/DECISIONS.md) D90:
- `PASS_SOURCES`: one GLSL file per pass;
- `SOURCE`: attribution and licence;
- per-image `WRAP`, `FILTER` and `VFLIP`;
- per-pass `FULL_PRECISION`.

## Main types and entry points

| Item | What it does |
|---|---|
| `ShaderId::parse` | Accepts a bare id, a `/view/` or `/embed/` URL (with or without a scheme), or an API URL, with query and fragment tolerated |
| `Shader::from_json` | Reads the API's `{"Shader": …}` wrapper or a bare `{info, renderpass}` export. An API `{"Error": …}` reply is a typed error |
| `convert(&Shader, &Options) -> Converted` | Plans the passes, binds the channels, rewrites the dialect, and generates the shim and `scene.json` |
| `write(&Converted, library, replace)` | Writes the scene folder into a hidden staging folder, then renames it into place in one step |
| `convert::Warning` | A lossy conversion, such as a video input bound to black or a missing texture |
| `convert::ConvertError` | Something that cannot be converted, naming the pass and channel |
| `dialect`, `glsl` | The lossless tokenizer and the token-level rewrites (D94) |

## Usage

```bash
# Convert an exported shader into the scene library (default: scenes/).
cargo run --release -p strobe-shadertoy --bin strobe-import -- shader.json

# Options:
#   --out <DIR>      scene library to write into (default: scenes)
#   --name <TITLE>   scene title (default: the shader's own name)
#   --assets <DIR>   folder of already downloaded textures, found by file name
#   --strict         fail on a missing texture instead of binding black
#   --force          replace a scene imported before
#   --id <ID|URL>    fetch through the REST API instead (needs a key:
#                    STROBE_SHADERTOY_KEY, or `shadertoy.key` in the data dir)
```

Textures are downloaded and cached unless `--offline` is given; `--data-dir`
moves the key file and the cache.

The result is `<name>-<id>.synScene/`, containing:
- `scene.json`: passes, `PASS_SOURCES`, images, the five "Strobe" controls,
  and `SOURCE`;
- `main.glsl`: the Image pass;
- `passes/*.glsl`: buffers A to D, plus a one-pixel clock pass when
  anything reads time;
- `images/`: fetched textures, a black placeholder, and
  `pending-assets.json` listing the URLs still to fetch.

Open it in the app like any other scene, or render it with `strobe-bench`.

How Shadertoy maps onto the scene:
- **Uniforms:** `iResolution`, `iTime`, `iTimeDelta`, `iFrame`,
  `iFrameRate`, `iMouse`, `iDate`, `iChannelTime`, `iChannelResolution` and
  `iSampleRate` come from the shim. `iChannel0`–`3` are `#define`d to the
  real samplers, so passing a sampler as a function parameter works.
- **Buffers:** buffers run A to D whatever order the JSON lists them in.
  Each is a float, double-buffered pass, and reading a later buffer gives
  its previous frame, as on Shadertoy.
- **Audio:** `music`, `musicstream` and `mic` inputs sample a live 512x2
  audio texture with Shadertoy's layout (`strobe_ShadertoyAudio`).
- **Controls:** every import gets a "Strobe" group: `st_speed`,
  `st_audio_time`, `st_bass_flash`, `st_zoom` and `st_hue` (D92).

## Testing

```bash
cargo test -p strobe-shadertoy
STROBE_REQUIRE_HEADLESS_GL=1 cargo test -p strobe-shadertoy   # fail instead of skipping without EGL
```

- **Unit tests** beside each module cover id parsing, both JSON shapes,
  the tokenizer and the rewrites.
- **`tests/convert.rs`** covers:
  - golden output files under `tests/golden/`;
  - determinism;
  - error cases and warnings;
  - assets and the atomic writer.
- **`tests/render.rs`** converts, writes, loads and renders every fixture
  on a headless context, in both GL 3.3 core and GLES 3.0. It needs Mesa's
  EGL (`apt-get install -y libegl1 libegl-mesa0`) and skips without it,
  unless `STROBE_REQUIRE_HEADLESS_GL=1`. It also checks numbers:
  - feedback accumulates exactly;
  - a later buffer reads its previous frame;
  - the clock, Speed, zoom, hue and bass flash act as specified;
  - compile errors map back to `passes/<file>.glsl:N`.
- **Fixtures** (`tests/fixtures/`) are original work in exact export
  shape, MIT like the repo:
  - single image, Common plus image, buffer feedback, and a four-buffer
    chain listed out of order;
  - sampler-as-parameter and `const int` switch traps, and a legacy
    `texture2D`/`gl_FragColor` shader;
  - a music input, a missing texture, and helper-name collisions between
    buffers.
  
  Shadertoy itself is unreachable from CI (Cloudflare), and unlicensed
  third-party shaders must not be committed.

## Design notes

- **One GLSL file per pass (D90).** Shadertoy buffers often reuse helper
  names, so each pass compiles from its own file. A request for a shared
  program, the bench's `--shared`, is overridden with a warning.
- **The shim is generated around untouched code (D91).** Time comes from a
  one-pixel full-precision clock pass, so Speed needs no host-side state.
- **Control names (D92).** The "Strobe" controls are `st_`-prefixed to
  avoid clashing with common Shadertoy identifiers, and act on the Image
  pass only.
- **Existing player paths (D93).** The audio texture and `iDate` ride paths
  the player already has, so the frame loop learns nothing new.
- **Token-level dialect rewriting (D94).** It renames the `texture2D`
  family and strips `precision`, `#version`, `#extension` and uniform
  redeclarations. It aliases user-defined built-ins under `GL_ES`. It
  stops where types begin: implicit int-to-float is not rewritten (Q8).
- **Output rules (D95).** The output is byte-identical for the same input,
  and the write is atomic.

## Status and known gaps

- **Not built:** fetching by id (path B, the REST API with a key), the
  userscript with its localhost listener (path A), the asset downloader,
  the app's Import UI, thumbnails, the Needs-work bin, and
  promote-to-control (now part of the planned auto-generated controls
  engine).
- **Unsupported inputs:** `cubemap` and `volume` inputs, and `sound` and
  `cubemap` passes, are errors. `keyboard`, `video` and `webcam` read
  black, with a warning. `srgb` textures are not decoded (Q16).
- **Audio:** the audio texture is resampled from the 256-texel spectrum,
  not the browser-matched `AudioFrame::shadertoy` (Q18).
- **`iDate`:** it falls back to the UTC wall clock until the app feeds
  local time (Q15).
- **Resize:** a resize restarts `iTime` at `TIME` and `iFrame` at 0, which
  is a visible jump at a non-default Speed.
- **Verified so far:** all nine fixtures render on llvmpipe in both
  dialects. Whether strict desktop drivers need the `GL_ES` built-in alias
  is open (Q17); run the `legacy` fixture on the T480.
