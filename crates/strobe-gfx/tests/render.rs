// ============================================================================
// Module:       render (integration test)
// Description:  Compiles and renders scenes on headless GL in both dialects:
//               desktop 3.3 core (native path) and GLES 3.0 (ANGLE path).
//
// Dependencies: anyhow; strobe-gfx (headless context, player), strobe-scene
// ============================================================================

//! Renderer tests against a real GLSL compiler.
//!
//! Each test opens a headless EGL context — Mesa's llvmpipe on a Linux CI
//! runner, no GPU and no display needed — and asserts on pixels read back
//! from the scene's output. Where no headless EGL exists (a Windows
//! runner), the tests report a skip and pass, unless
//! `STROBE_REQUIRE_HEADLESS_GL` is set, which CI's Linux job does so a
//! missing driver cannot pass silently there.

use anyhow::{Result, anyhow};
use std::path::PathBuf;
use strobe_gfx::{
    Api, AudioTextures, Caps, FrameInputs, GpuTimer, Headless, Mouse, Options, ScenePlayer,
};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;
use strobe_scene::scene::SceneDef;

/// A headless context, or `None` (after saying so) when none exists here.
fn context(api: Api) -> Result<Option<Headless>> {
    match Headless::new(api) {
        Ok(headless) => Ok(Some(headless)),
        Err(error) if std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_none() => {
            eprintln!("skipping: no headless {api:?} context ({error})");
            Ok(None)
        }
        Err(error) => Err(anyhow!("headless {api:?} context required: {error}")),
    }
}

fn folder(json: &str, shader: &str) -> Result<SceneFolder> {
    Ok(SceneFolder {
        dir: PathBuf::from("."),
        def: SceneDef::parse(json, "test")?,
        shader: shader.to_string(),
        script: None,
    })
}

/// Renders `frames` frames of a scene and returns the centre pixel.
fn render_center(
    api: Api,
    options: Options,
    json: &str,
    shader: &str,
    frames: u64,
) -> Result<Option<[u8; 4]>> {
    let Some(headless) = context(api)? else {
        return Ok(None);
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let scene = folder(json, shader)?;
    let controls = ControlBank::new(scene.def.controls.clone())?;
    let transitions: Vec<f32> = scene
        .def
        .transition_uniforms
        .iter()
        .map(|t| t.initial)
        .collect();
    let audio = vec![0.0; SCALARS.len()];
    let mut player = ScenePlayer::new(gl, &caps, &scene, [32, 18], options)?;
    for frame in 0..frames {
        player.render(&FrameInputs {
            time: frame as f32 / 60.0,
            frame,
            mouse: Mouse::default(),
            audio: &audio,
            controls: &controls,
            transitions: &transitions,
        });
    }
    if let Some(code) = gl.error() {
        return Err(anyhow!("GL error 0x{code:x}"));
    }
    let pixels = player.read_output();
    let offset = (9 * 32 + 16) * 4;
    let px = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok(Some([px[0], px[1], px[2], px[3]]))
}

fn near(actual: u8, expected: u8) -> bool {
    actual.abs_diff(expected) <= 2
}

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

#[test]
fn a_control_value_reaches_the_shader() -> Result<()> {
    for api in BOTH {
        let Some(px) = render_center(
            api,
            Options::default(),
            r#"{ "CONTROLS": [ {"NAME": "amount", "TYPE": "slider", "DEFAULT": 0.5} ] }"#,
            "vec4 renderMain() { return vec4(amount, 0.0, 0.0, 1.0); }",
            1,
        )?
        else {
            continue;
        };
        assert!(near(px[0], 128) && px[1] == 0, "{api:?}: got {px:?}");
    }
    Ok(())
}

#[test]
fn the_ssf_library_compiles_in_both_dialects() -> Result<()> {
    // Touch a function from every family, so the driver cannot skip
    // compiling any of them.
    let shader = r#"
        vec4 renderMain() {
            vec2 p = _uvc;
            float n = _noise(p * 3.0) + _fbm(vec3(p, TIME), 4.0) + _noise(TIME);
            vec3 c = _hsv2rgb(vec3(n, 0.8, 0.9));
            c = _hueRotate(_contrast(_saturation(c, 1.2), 1.1), 30.0);
            c = _palette(n, vec3(0.5), vec3(0.5), vec3(1.0), vec3(0.0, 0.33, 0.67)) * c;
            vec2 q = _rotate(_toRect(_toPolarTrue(p)), 45.0) + _hash22(p) * 0.0;
            float m = _mix3(0.0, 0.5, 1.0, _nsin(TIME)) * _pulse(_uv.x, 0.5, 0.5) + _smin(q.x, q.y, 8.0) * 0.0;
            vec4 e = _edgeDetectSobel(syn_FinalPass) * 0.0 + _hBlurTiltShift(syn_FinalPass, 2.0, 0.5) * 0.0;
            vec4 media = _isMediaActive() ? _loadMedia() : vec4(0.0);
            return vec4(_gamma(c, 1.0) * m + e.rgb + media.rgb + _hash44(vec4(1.0)).rgb * 0.0, 1.0);
        }"#;
    for api in BOTH {
        render_center(api, Options::default(), "{}", shader, 2)?;
    }
    Ok(())
}

#[test]
fn a_buffer_pass_feeds_the_final_pass() -> Result<()> {
    let json = r#"{ "PASSES": [ {"TARGET": "buf"} ] }"#;
    let shader = r#"
        vec4 renderMain() {
            if (PASSINDEX == 0) { return vec4(0.25, 0.5, 0.75, 1.0); }
            return texture(buf, _uv);
        }"#;
    for api in BOTH {
        for specialise_passes in [true, false] {
            let options = Options {
                specialise_passes,
                ..Options::default()
            };
            let Some(px) = render_center(api, options, json, shader, 1)? else {
                continue;
            };
            assert!(
                near(px[0], 64) && near(px[1], 128) && near(px[2], 191),
                "{api:?} specialised={specialise_passes}: got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_double_buffered_pass_reads_its_previous_frame() -> Result<()> {
    let json = r#"{ "PASSES": [ {"TARGET": "acc", "DOUBLE_BUFFER": true} ] }"#;
    let shader = r#"
        vec4 renderMain() {
            if (PASSINDEX == 0) { return texture(acc, _uv) + vec4(0.1, 0.0, 0.0, 0.0); }
            return vec4(texture(acc, _uv).rgb, 1.0);
        }"#;
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), json, shader, 3)? else {
            continue;
        };
        assert!(near(px[0], 77), "{api:?}: three frames of +0.1, got {px:?}");
    }
    Ok(())
}

#[test]
fn syn_final_pass_is_last_frames_output() -> Result<()> {
    let shader = "vec4 renderMain() { return vec4(texture(syn_FinalPass, _uv).rgb + 0.1, 1.0); }";
    for api in BOTH {
        let Some(px) = render_center(api, Options::default(), "{}", shader, 4)? else {
            continue;
        };
        assert!(near(px[0], 102), "{api:?}: four frames of +0.1, got {px:?}");
    }
    Ok(())
}

#[test]
fn real_scene_idioms_compile_on_desktop_gl() -> Result<()> {
    // Real scenes compare PASSINDEX with a float literal, rely on implicit
    // int-to-float conversion, and use texture2D. Desktop GLSL accepts all
    // three, in either PASSINDEX mode.
    let json = r#"{ "PASSES": [ {"TARGET": "firstBuffer"} ] }"#;
    let shader = r#"
        vec4 renderMain(void) {
            float scale = 2;
            if (PASSINDEX == 0.0) { return vec4(0.5 * scale * 0.5); }
            else if (PASSINDEX == 1.0) { return texture2D(firstBuffer, _uv); }
        }"#;
    for specialise_passes in [true, false] {
        let options = Options {
            specialise_passes,
            ..Options::default()
        };
        let Some(px) = render_center(Api::DesktopCore33, options, json, shader, 1)? else {
            continue;
        };
        assert!(
            near(px[0], 128),
            "specialised={specialise_passes}: got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn a_compile_error_points_at_main_glsl() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let scene = folder("{}", "vec4 renderMain() {\n    return undefinedThing;\n}\n")?;
    let error = ScenePlayer::new(gl, &caps, &scene, [8, 8], Options::default())
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        error.contains("main.glsl:2"),
        "expected a main.glsl:2 location, got: {error}"
    );
    Ok(())
}

/// Reads both audio textures at the middle of the frame: spectrum in red,
/// level trail in green. Written for both dialects with one `#ifdef`,
/// because GLES has no `sampler1D`.
const AUDIO_PROBE: &str = r#"
    #ifdef GL_ES
    #define SPEC(x) texture(syn_Spectrum, vec2(x, 0.5))
    #define TRAIL(x) texture(syn_LevelTrail, vec2(x, 0.5))
    #else
    #define SPEC(x) texture(syn_Spectrum, x)
    #define TRAIL(x) texture(syn_LevelTrail, x)
    #endif
    vec4 renderMain() { return vec4(SPEC(0.5).r, TRAIL(0.5).g, SPEC(0.5).a, 1.0); }
"#;

#[test]
fn uploaded_audio_texels_reach_the_spectrum_and_trail_samplers() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder("{}", AUDIO_PROBE)?;
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let audio = vec![0.0; SCALARS.len()];
        let mut player = ScenePlayer::new(gl, &caps, &scene, [32, 18], Options::default())?;
        // A width other than the 256 the player starts with, to cover the
        // reallocation path as well.
        for width in [256_usize, 64] {
            let spectrum = vec![[0.5, 0.0, 0.0, 1.0]; width];
            let trail = vec![[0.0, 0.25, 0.0, 0.0]; width];
            player.upload_audio(&AudioTextures {
                spectrum: &spectrum,
                level_trail: &trail,
            });
            player.render(&FrameInputs {
                time: 0.0,
                frame: 0,
                mouse: Mouse::default(),
                audio: &audio,
                controls: &controls,
                transitions: &[],
            });
            if let Some(code) = gl.error() {
                return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
            }
            let pixels = player.read_output();
            let offset = (9 * 32 + 16) * 4;
            let px = pixels
                .get(offset..offset + 3)
                .ok_or_else(|| anyhow!("readback too short"))?;
            assert!(
                near(px[0], 128) && near(px[1], 64) && near(px[2], 255),
                "{api:?} width {width}: spectrum r 0.5, trail g 0.25, wave a 1.0; got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_scene_that_ignores_the_audio_textures_never_uploads_them() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let scene = folder("{}", "vec4 renderMain() { return vec4(1.0); }")?;
    let mut player = ScenePlayer::new(gl, &caps, &scene, [32, 18], Options::default())?;
    // Nothing to assert on pixels: the point is that a mismatched width
    // (which would reallocate) is harmless and raises no GL error.
    player.upload_audio(&AudioTextures {
        spectrum: &[[1.0; 4]; 7],
        level_trail: &[[1.0; 4]; 7],
    });
    assert!(gl.error().is_none(), "an unused upload must be a no-op");
    Ok(())
}

#[test]
fn the_gpu_timer_reports_results_without_ever_waiting() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let Some(mut timer) = GpuTimer::new(gl, &caps) else {
            eprintln!("skipping: {api:?} has no GPU timer queries");
            continue;
        };
        let scene = folder("{}", "vec4 renderMain() { return vec4(TIME); }")?;
        let controls = ControlBank::new(scene.def.controls.clone())?;
        let audio = vec![0.0; SCALARS.len()];
        let mut player = ScenePlayer::new(gl, &caps, &scene, [64, 36], Options::default())?;
        let mut samples = Vec::new();
        for frame in 0..12_u64 {
            timer.begin();
            player.render(&FrameInputs {
                time: frame as f32 / 60.0,
                frame,
                mouse: Mouse::default(),
                audio: &audio,
                controls: &controls,
                transitions: &[],
            });
            timer.end();
            timer.poll(|ms| samples.push(ms));
            assert!(
                timer.in_flight() <= strobe_gfx::timing::SLOTS,
                "{api:?}: never more than the ring holds"
            );
        }
        // Let the GPU finish, then every outstanding result is readable.
        gl.finish();
        timer.poll(|ms| samples.push(ms));
        assert!(
            !samples.is_empty(),
            "{api:?}: at least one measurement must come back"
        );
        assert!(
            samples.iter().all(|ms| ms.is_finite() && *ms >= 0.0),
            "{api:?}: samples are finite and non-negative: {samples:?}"
        );
        assert_eq!(timer.in_flight(), 0, "{api:?}: everything was drained");
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn a_healthy_context_reports_no_reset_and_a_lost_one_is_never_asked() -> Result<()> {
    for api in [Api::DesktopCore33, Api::Gles30] {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        assert!(!gl.is_lost(), "{api:?}: a new context is not lost");
        let status = gl.reset_status();
        assert!(
            !status.is_reset(),
            "{api:?}: a fresh context has not been reset, got {status}"
        );
        let clone = gl.clone();
        gl.mark_lost();
        assert!(clone.is_lost(), "{api:?}: the flag is shared by clones");
        assert_eq!(
            clone.reset_status(),
            strobe_gfx::gl::ResetStatus::Unknown,
            "{api:?}: a lost context is reported without calling the driver"
        );
        // Dropping an object on a lost context skips the delete (a smoke
        // test: the skip itself cannot be observed from outside, but the
        // drop must be harmless and leave no GL error behind).
        let texture = gl
            .texture_2d(
                4,
                4,
                strobe_gfx::gl::Format::Rgba8,
                strobe_gfx::gl::Sampling::LINEAR_CLAMP,
                None,
            )
            .map_err(|e| anyhow!("{api:?}: texture: {e}"))?;
        drop(texture);
        let error = gl.error();
        assert!(error.is_none(), "{api:?}: unexpected GL error {error:?}");
    }
    Ok(())
}
