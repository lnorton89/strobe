// ============================================================================
// Module:       render (integration test)
// Description:  Converts every fixture, loads the result like the app would,
//               and renders it on headless GL in both dialects.
//
// Dependencies: anyhow; strobe-shadertoy; strobe-gfx (dev), strobe-scene
// ============================================================================

//! Render tests for converted Shadertoy scenes.
//!
//! Each test converts a fixture from `tests/fixtures/` (original shaders in
//! exact Shadertoy export shape), writes it with the atomic writer, loads
//! the folder with `SceneFolder::load`, and renders it on a headless EGL
//! context: Mesa's llvmpipe on a CI runner. Without one the tests report a
//! skip and pass, unless `STROBE_REQUIRE_HEADLESS_GL` is set (CI's Linux
//! job sets it), in which case a missing driver fails them.
//!
//! The dependency direction stays clean: `strobe-gfx` knows nothing of this
//! crate; this crate's tests depend on it.

use anyhow::{Context as _, Result, anyhow};
use std::path::PathBuf;
use strobe_gfx::{
    Api, AudioTextures, Caps, FloatPrecision, FrameInputs, Headless, Mouse, Options, ScenePlayer,
};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;
use strobe_shadertoy::{Converted, Shader, convert};

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

/// Every fixture, by file stem.
const FIXTURES: [&str; 9] = [
    "single_image",
    "common_image",
    "buffer_feedback",
    "chain_four_buffers",
    "gl_traps",
    "legacy",
    "music",
    "missing_texture",
    "name_collisions",
];

/// Frame size for the tests: small, and even in both directions.
const SIZE: [i32; 2] = [64, 36];

fn context(api: Api) -> Result<Option<Headless>> {
    match Headless::new(api) {
        Ok(headless) => Ok(Some(headless)),
        Err(error) if std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_none() => {
            strobe_gfx::skip::note(format_args!("no headless {api:?} context ({error})"));
            Ok(None)
        }
        Err(error) => Err(anyhow!("headless {api:?} context required: {error}")),
    }
}

fn fixture_text(name: &str) -> Result<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.json"));
    std::fs::read_to_string(&path).with_context(|| path.display().to_string())
}

/// Converts `json`, writes it under a folder private to `tag` and to the
/// running test, and loads it. Private to the test as well: tests import
/// the same fixture in parallel, and on Windows two of them swapping one
/// staging folder fail with "Access is denied" (AMD desktop, Task A1).
fn import_text(json: &str, tag: &str) -> Result<(Converted, SceneFolder)> {
    let shader = Shader::from_json(json)?;
    let converted = convert(&shader, &strobe_shadertoy::Options::default())?;
    let thread = std::thread::current();
    let test = thread.name().unwrap_or("main").replace("::", "-");
    let library = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("scenes-{tag}-{test}"));
    let dir = strobe_shadertoy::convert::write(&converted, &library, true)?;
    let folder = SceneFolder::load(&dir)?;
    Ok((converted, folder))
}

fn import(name: &str) -> Result<(Converted, SceneFolder)> {
    import_text(&fixture_text(name)?, name)
}

/// A rendered frame, top row first.
struct Frame {
    pixels: Vec<u8>,
}

impl Frame {
    fn at(&self, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * SIZE[0] as usize + x) * 4;
        let px = self.pixels.get(offset..offset + 4).unwrap_or(&[0; 4]);
        [px[0], px[1], px[2], px[3]]
    }

    fn center(&self) -> [u8; 4] {
        self.at(SIZE[0] as usize / 2, SIZE[1] as usize / 2)
    }

    /// Fraction of pixels with any colour at all.
    fn lit_fraction(&self) -> f32 {
        let lit = self
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
            .count();
        lit as f32 / (self.pixels.len() / 4).max(1) as f32
    }
}

/// What a test can change while frames render.
struct Tweaks<'a> {
    controls: &'a mut ControlBank,
    audio: &'a mut Vec<f32>,
    player: &'a mut ScenePlayer,
}

impl Tweaks<'_> {
    fn control(&mut self, name: &str, value: f32) -> Result<()> {
        let index = self
            .controls
            .find(name)
            .ok_or_else(|| anyhow!("no control {name}"))?;
        self.controls.set(index, None, &[value]);
        self.controls.tick(0.0);
        Ok(())
    }

    fn scalar(&mut self, name: &str, value: f32) -> Result<()> {
        let index = SCALARS
            .iter()
            .position(|n| *n == name)
            .ok_or_else(|| anyhow!("no audio scalar {name}"))?;
        self.audio[index] = value;
        Ok(())
    }
}

/// Renders `frames` frames; `tweak` runs before each, with its index.
fn render(
    api: Api,
    scene: &SceneFolder,
    frames: u64,
    options: Options,
    mut tweak: impl FnMut(u64, &mut Tweaks<'_>) -> Result<()>,
) -> Result<Option<Frame>> {
    let Some(headless) = context(api)? else {
        return Ok(None);
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let mut controls = ControlBank::new(scene.def.controls.clone())?;
    let mut audio = vec![0.0; SCALARS.len()];
    let mut player = ScenePlayer::new(gl, &caps, scene, SIZE, options)?;
    for frame in 0..frames {
        tweak(
            frame,
            &mut Tweaks {
                controls: &mut controls,
                audio: &mut audio,
                player: &mut player,
            },
        )?;
        player.render(&FrameInputs {
            time: frame as f32 / 60.0,
            frame,
            mouse: Mouse::default(),
            audio: &audio,
            controls: &controls,
            transitions: &[],
        });
    }
    if let Some(code) = gl.error() {
        return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
    }
    Ok(Some(Frame {
        pixels: player.read_output(),
    }))
}

fn full_precision() -> Options {
    Options {
        float_precision: FloatPrecision::Full,
        ..Options::default()
    }
}

fn near(actual: u8, expected: u8) -> bool {
    actual.abs_diff(expected) <= 2
}

#[test]
fn every_fixture_compiles_and_renders_a_lit_frame_in_both_dialects() -> Result<()> {
    for name in FIXTURES {
        let (_, scene) = import(name)?;
        for api in BOTH {
            let Some(frame) = render(api, &scene, 12, Options::default(), |_, _| Ok(()))? else {
                continue;
            };
            let lit = frame.lit_fraction();
            assert!(
                lit > 0.9,
                "{name} on {api:?}: only {:.0}% of the frame is lit",
                lit * 100.0
            );
            assert!(
                frame
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|px| px[3] == 255),
                "{name} on {api:?}: the Image pass must output alpha 1"
            );
        }
    }
    Ok(())
}

#[test]
fn the_image_pass_ignores_the_alpha_the_shader_wrote() -> Result<()> {
    // single_image writes alpha 0.25.
    let (_, scene) = import("single_image")?;
    for api in BOTH {
        let Some(frame) = render(api, &scene, 1, Options::default(), |_, _| Ok(()))? else {
            continue;
        };
        assert_eq!(frame.center()[3], 255, "{api:?}: alpha is forced to 1");
    }
    Ok(())
}

#[test]
fn a_buffer_reading_itself_accumulates_exactly() -> Result<()> {
    // Frame 0 writes 0.25 + 1/255; each later frame adds 1/255 to what the
    // buffer held. After N frames the value is 0.25 + N/255.
    let (_, scene) = import("buffer_feedback")?;
    for api in BOTH {
        for (options, tolerance) in [(full_precision(), 1_u8), (Options::default(), 2)] {
            let frames = 20;
            let Some(frame) = render(api, &scene, frames, options, |_, _| Ok(()))? else {
                continue;
            };
            let expected = (0.25 * 255.0 + frames as f32).round() as u8;
            let got = frame.center()[0];
            assert!(
                got.abs_diff(expected) <= tolerance,
                "{api:?} {:?}: expected about {expected} after {frames} frames, got {got}",
                options.float_precision
            );
        }
    }
    Ok(())
}

#[test]
fn a_resize_that_wipes_the_buffers_restarts_iframe_so_they_initialise_again() -> Result<()> {
    // buffer_feedback's Buffer A writes 0.25 on `iFrame == 0` and adds 1/255
    // a frame after that. `resize` restarts the `$WIDTH` buffers from black,
    // so it must restart `iFrame` too (D91), even though the 1x1 clock pass
    // keeps its size (D287): otherwise the buffer never initialises again
    // and counts up from black.
    let (_, scene) = import("buffer_feedback")?;
    for api in BOTH {
        let frames = 20;
        let Some(frame) = render(api, &scene, frames, full_precision(), |frame, tweaks| {
            match frame {
                10 => tweaks.player.resize([SIZE[0] / 2, SIZE[1] / 2])?,
                11 => tweaks.player.resize(SIZE)?,
                _ => {}
            }
            Ok(())
        })?
        else {
            continue;
        };
        // Frames 11 to 19 ran after the last wipe: 0.25 + 9/255.
        let expected = (0.25 * 255.0 + 9.0_f32).round() as u8;
        let got = frame.center()[0];
        assert!(
            got.abs_diff(expected) <= 1,
            "{api:?}: after the resize Buffer A should start again from 0.25 \
             (expected about {expected}), got {got}"
        );
    }
    Ok(())
}

#[test]
fn reading_a_later_buffer_gives_the_previous_frame() -> Result<()> {
    // A = N/255. B reads A, which ran earlier this frame: N. D = C + 1/255.
    // C reads D, which runs *after* C, so it sees D's previous frame:
    // C = D(N-1) = N-1, and D = N. The image shows (B, C, D).
    let (_, scene) = import("chain_four_buffers")?;
    for api in BOTH {
        for frames in [1_u64, 2, 10] {
            let Some(frame) = render(api, &scene, frames, full_precision(), |_, _| Ok(()))? else {
                continue;
            };
            let n = frames as u8;
            let [r, g, b, _] = frame.center();
            assert!(
                r.abs_diff(n) <= 1 && g.abs_diff(n - 1) <= 1 && b.abs_diff(n) <= 1,
                "{api:?} after {frames} frames: expected B={n} C={} D={n}, got ({r}, {g}, {b})",
                n - 1
            );
        }
    }
    Ok(())
}

#[test]
fn buffers_run_in_shadertoy_order_whatever_the_json_order() -> Result<()> {
    let (converted, _) = import("chain_four_buffers")?;
    let json = String::from_utf8(
        converted
            .files
            .get("scene.json")
            .cloned()
            .ok_or_else(|| anyhow!("no scene.json"))?,
    )?;
    let order: Vec<usize> = ["st_clock", "st_buf_a", "st_buf_b", "st_buf_c", "st_buf_d"]
        .iter()
        .filter_map(|target| json.find(&format!("\"TARGET\": \"{target}\"")))
        .collect();
    assert_eq!(order.len(), 5, "all five passes listed");
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "listed as clock, A, B, C, D even though the export lists D first: {order:?}"
    );
    Ok(())
}

/// A one-pass shader as an export, to test the shim against known numbers.
fn inline(code: &str) -> String {
    serde_json::json!({
        "ver": "0.1",
        "info": { "id": "Tst100", "name": "inline", "username": "strobe-tests" },
        "renderpass": [ { "type": "image", "name": "Image", "code": code,
                          "inputs": [], "outputs": [] } ]
    })
    .to_string()
}

#[test]
fn time_frame_and_delta_follow_the_scene_clock_at_default_controls() -> Result<()> {
    let json = inline(
        "void mainImage(out vec4 c, in vec2 p) {\n\
         c = vec4(iTime * 0.5, float(iFrame) / 255.0, iTimeDelta * 10.0, 1.0); }\n",
    );
    let (_, scene) = import_text(&json, "clock-default")?;
    for api in BOTH {
        // 31 frames: the last has TIME 0.5 and FRAMECOUNT 30.
        let Some(frame) = render(api, &scene, 31, full_precision(), |_, _| Ok(()))? else {
            continue;
        };
        let [r, g, b, _] = frame.center();
        assert!(
            near(r, 64) && near(g, 30) && near(b, 42),
            "{api:?}: iTime 0.5 -> 64, iFrame 30 -> 30, iTimeDelta 1/60 * 10 -> 42; got ({r}, {g}, {b})"
        );
    }
    Ok(())
}

#[test]
fn the_speed_control_scales_itime() -> Result<()> {
    let json = inline(
        "void mainImage(out vec4 c, in vec2 p) { c = vec4(iTime * 0.25, 0.0, 0.0, 1.0); }\n",
    );
    let (_, scene) = import_text(&json, "clock-speed")?;
    for api in BOTH {
        for (speed, expected) in [(1.0_f32, 64_u8), (2.0, 128), (0.0, 0)] {
            // Frame 0 starts the clock at TIME 0; 60 more frames at 1/60 s.
            let Some(frame) = render(api, &scene, 61, full_precision(), |frame, tweaks| {
                if frame == 0 {
                    tweaks.control("st_speed", speed)?;
                }
                Ok(())
            })?
            else {
                continue;
            };
            let got = frame.center()[0];
            assert!(
                near(got, expected),
                "{api:?} speed {speed}: iTime*0.25 after one second should be {expected}, got {got}"
            );
        }
    }
    Ok(())
}

#[test]
fn zoom_hue_and_bass_flash_act_on_the_image_pass() -> Result<()> {
    let json = inline(
        "void mainImage(out vec4 c, in vec2 p) { c = vec4(p.x / iResolution.x, 0.0, 0.0, 1.0); }\n",
    );
    let (_, scene) = import_text(&json, "controls")?;
    let quarter = SIZE[0] as usize / 4;
    for api in BOTH {
        let plain = render(api, &scene, 1, Options::default(), |_, _| Ok(()))?;
        let Some(plain) = plain else { continue };
        let x = plain.at(quarter, 10)[0];
        assert!(near(x, 66), "{api:?}: red tracks x, 0.258 -> 66; got {x}");

        let zoomed = render(api, &scene, 1, Options::default(), |_, t| {
            t.control("st_zoom", 2.0)
        })?
        .ok_or_else(|| anyhow!("context vanished"))?;
        let zx = zoomed.at(quarter, 10)[0];
        // Zooming 2x around the centre pulls a quarter-way pixel to 0.379.
        assert!(near(zx, 97), "{api:?}: zoom 2 -> 0.379 -> 97; got {zx}");

        let hued = render(api, &scene, 1, Options::default(), |_, t| {
            t.control("st_hue", 120.0)
        })?
        .ok_or_else(|| anyhow!("context vanished"))?;
        let [hr, hg, hb, _] = hued.at(SIZE[0] as usize - 4, 10);
        assert!(
            hg > hr && hg > hb,
            "{api:?}: a 120 degree turn takes red toward green; got ({hr}, {hg}, {hb})"
        );

        let flashed = render(api, &scene, 1, Options::default(), |_, t| {
            t.control("st_bass_flash", 0.5)?;
            t.scalar("syn_BassHits", 1.0)
        })?
        .ok_or_else(|| anyhow!("context vanished"))?;
        let fx = flashed.at(quarter, 10)[0];
        assert!(
            near(fx, 99),
            "{api:?}: bass flash 0.5 on a hit multiplies by 1.5 -> 99; got {fx}"
        );
    }
    Ok(())
}

#[test]
fn the_music_input_reads_the_live_audio_texture() -> Result<()> {
    let (_, scene) = import("music")?;
    for api in BOTH {
        // Raw spectrum 0.75 everywhere and a waveform of 0.25.
        let texels = vec![[0.75, 0.0, 0.0, 0.25]; 256];
        let Some(frame) = render(api, &scene, 1, Options::default(), |_, t| {
            t.player.upload_audio(&AudioTextures {
                spectrum: &texels,
                level_trail: &texels,
                shadertoy: None,
            });
            Ok(())
        })?
        else {
            continue;
        };
        let [r, g, b, _] = frame.center();
        // (0.75 * 80 - 6.02) / 70 = 0.771 -> 197; waveform 0.25 -> 64;
        // iChannelResolution[0].x is 512 -> 0.1 + 0.2 -> 77.
        assert!(
            near(r, 197) && near(g, 64) && near(b, 77),
            "{api:?}: spectrum 197, waveform 64, width 512 -> 77; got ({r}, {g}, {b})"
        );
    }
    Ok(())
}

#[test]
fn idate_is_the_date_the_host_sets() -> Result<()> {
    let json = inline(
        "void mainImage(out vec4 c, in vec2 p) {\n\
         c = vec4(iDate.x / 4096.0, iDate.y / 16.0, iDate.z / 32.0, 1.0); }\n",
    );
    let (_, scene) = import_text(&json, "date")?;
    for api in BOTH {
        let Some(frame) = render(api, &scene, 1, Options::default(), |_, t| {
            t.player.set_date([2048.0, 8.0, 16.0, 0.0]);
            Ok(())
        })?
        else {
            continue;
        };
        let [r, g, b, _] = frame.center();
        assert!(
            near(r, 128) && near(g, 128) && near(b, 128),
            "{api:?}: year, month, day each half scale; got ({r}, {g}, {b})"
        );
    }
    Ok(())
}

#[test]
fn colliding_helper_names_in_two_passes_each_get_their_own() -> Result<()> {
    // Buffer A: palette -> (K, 0, 0) with K = 0.25. Image: palette ->
    // (0, K, 0) with K = 0.5, plus 0.5 * 8 / 16 in blue.
    let (_, scene) = import("name_collisions")?;
    for api in BOTH {
        let Some(frame) = render(api, &scene, 1, Options::default(), |_, _| Ok(()))? else {
            continue;
        };
        let [r, g, b, _] = frame.center();
        assert!(
            near(r, 64) && near(g, 128) && near(b, 64),
            "{api:?}: expected (64, 128, 64); got ({r}, {g}, {b})"
        );
    }
    Ok(())
}

#[test]
fn a_missing_texture_reads_black_and_says_so() -> Result<()> {
    let (converted, scene) = import("missing_texture")?;
    assert_eq!(converted.pending_assets.len(), 1, "one texture to fetch");
    let asset = &converted.pending_assets[0];
    assert!(
        asset
            .url
            .starts_with("https://www.shadertoy.com/media/a/0123456789abcdef"),
        "the URL to download from: {}",
        asset.url
    );
    assert!(
        converted.files.contains_key("images/pending-assets.json"),
        "the fetch list is written beside the images"
    );
    for api in BOTH {
        let Some(frame) = render(api, &scene, 1, Options::default(), |_, _| Ok(()))? else {
            continue;
        };
        assert!(
            near(frame.center()[0], 51),
            "{api:?}: black texture -> 0.2 -> 51; got {:?}",
            frame.center()
        );
    }
    Ok(())
}

#[test]
fn a_compile_error_in_a_buffer_names_its_file_and_maps_to_the_tab_line() -> Result<()> {
    let text = fixture_text("buffer_feedback")?;
    // Break the second line of Buffer A's code.
    let broken = text.replace(
        "vec4 prev = (iFrame == 0)",
        "vec4 prev = undefinedThing + (iFrame == 0)",
    );
    assert_ne!(broken, text, "the fixture changed");
    let (converted, scene) = import_text(&broken, "broken")?;
    let json = String::from_utf8(
        converted
            .files
            .get("scene.json")
            .cloned()
            .ok_or_else(|| anyhow!("no scene.json"))?,
    )?;
    // SOURCE.passes carries where each tab's line 1 sits in its file.
    let code_line = json
        .split("\"name\": \"Buffer A\"")
        .nth(1)
        .and_then(|rest| rest.split("\"code_line\": ").nth(1))
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("SOURCE.passes has no code_line for Buffer A"))?;
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let error = ScenePlayer::new(gl, &caps, &scene, SIZE, Options::default())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    // The broken statement is on tab line 3 (the `{` is line 2).
    let expected = format!("passes/buffer_a.glsl:{}", code_line + 2);
    assert!(
        error.contains("pass 1") && error.contains(&expected),
        "expected pass 1 and {expected}, got: {error}"
    );
    Ok(())
}

#[test]
fn asking_for_a_shared_program_falls_back_for_an_import() -> Result<()> {
    let (_, scene) = import("chain_four_buffers")?;
    let options = Options {
        specialise_passes: false,
        float_precision: FloatPrecision::Full,
    };
    let Some(frame) = render(Api::DesktopCore33, &scene, 3, options, |_, _| Ok(()))? else {
        return Ok(());
    };
    let [r, g, b, _] = frame.center();
    assert!(
        r.abs_diff(3) <= 1 && g.abs_diff(2) <= 1 && b.abs_diff(3) <= 1,
        "per-pass sources still render correctly; got ({r}, {g}, {b})"
    );
    Ok(())
}

#[test]
fn a_title_author_or_tab_name_cannot_break_the_generated_comments() -> Result<()> {
    // The shader's name, author and tab names are copied into comment lines
    // above the author's code. A carriage return ends a GLSL line comment as
    // a newline does, so these would turn the rest of the line into code;
    // non-ASCII text in a comment is what the dialect stage replaces for
    // drivers that reject it (D94).
    let text = serde_json::json!({
        "ver": "0.1",
        "info": { "id": "Tst200", "name": "Caf\u{e9} \u{2606}\rvoid broken",
                  "username": "\u{5c71}\u{7530}\r#error injected" },
        "renderpass": [
            { "type": "buffer", "name": "Buffer A\n#error tab", "code":
              "void mainImage(out vec4 c, in vec2 p) { c = vec4(0.5); }",
              "inputs": [], "outputs": [ { "id": 257, "channel": 0 } ] },
            { "type": "image", "name": "Image", "code":
              "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel0, p / iResolution.xy); }",
              "inputs": [ { "id": 257, "channel": 0, "type": "buffer" } ], "outputs": [] }
        ]
    })
    .to_string();
    let (converted, scene) = import_text(&text, "hostile-names")?;
    for api in BOTH {
        let Some(frame) = render(api, &scene, 2, Options::default(), |_, _| Ok(()))? else {
            continue;
        };
        let [r, _, _, _] = frame.center();
        assert!(
            near(r, 128),
            "{api:?}: the buffer's 0.5 reaches the image, got {r}"
        );
    }
    for (path, bytes) in &converted.files {
        if path.ends_with(".glsl") {
            assert!(
                bytes.iter().all(|b| b.is_ascii() && *b != b'\r'),
                "{path} must be plain ASCII with \\n line ends"
            );
        }
    }
    Ok(())
}

#[test]
fn a_helper_named_like_an_ssf_library_function_is_the_shaders_own() -> Result<()> {
    // Every pass is compiled after the SSF function library, which defines
    // `_noise(vec2)`, `_rand(vec2)` and `_rotate(vec2, float)`, and the
    // macro `_loadUserImage`. A Shadertoy shader knows nothing of it, so the
    // same names are its own helpers.
    let code = "float _noise(vec2 p) { return 0.25; }\n\
                float _rand(vec2 p) { return 0.25; }\n\
                vec2 _rotate(vec2 v, float a) { return v; }\n\
                float _loadUserImage(vec2 p) { return 0.0; }\n\
                void mainImage(out vec4 c, in vec2 p) {\n\
                \x20   c = vec4(_noise(p) + _rand(p) + _rotate(vec2(0.0), 1.0).x\n\
                \x20       + _loadUserImage(p), 0.0, 0.0, 1.0);\n\
                }";
    let (_, scene) = import_text(&inline(code), "ssf-names")?;
    for api in BOTH {
        let Some(frame) = render(api, &scene, 1, Options::default(), |_, _| Ok(()))? else {
            continue;
        };
        let [r, _, _, _] = frame.center();
        assert!(
            near(r, 128),
            "{api:?}: the shader's own helpers run, got {r}"
        );
    }
    Ok(())
}
