// ============================================================================
// Module:       compositor (integration test)
// Description:  The transition compositor against a real GLSL compiler:
//               crossfade and fade-through-black at known progress, a
//               custom transition, the compile counter, and a resize that
//               keeps a feedback scene's contents.
//
// Dependencies: anyhow; strobe-gfx (headless context, player, compositor)
// ============================================================================

//! Compositor tests.
//!
//! Two solid-colour scenes stand in for the outgoing and incoming cues, so
//! the expected mix is arithmetic. Like `render.rs`, each test opens a
//! headless EGL context (Mesa llvmpipe on CI) and reports a skip when none
//! exists, unless `STROBE_REQUIRE_HEADLESS_GL` is set.

use anyhow::{Result, anyhow};
use std::path::PathBuf;
use strobe_gfx::{
    Api, Blend, Caps, Compositor, FrameInputs, Headless, Mouse, Options, ScenePlayer,
};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;
use strobe_scene::scene::SceneDef;

/// The target the tests render at.
const SIZE: [i32; 2] = [32, 18];

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

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

fn folder(json: &str, shader: &str) -> Result<SceneFolder> {
    Ok(SceneFolder {
        dir: PathBuf::from("."),
        def: SceneDef::parse(json, "test")?,
        shader: shader.to_string(),
        pass_shaders: Vec::new(),
        script: None,
    })
}

/// Renders `frames` frames of `player`.
fn render(player: &mut ScenePlayer, scene: &SceneFolder, frames: u64) -> Result<()> {
    let controls = ControlBank::new(scene.def.controls.clone())?;
    let audio = vec![0.0; SCALARS.len()];
    for frame in 0..frames {
        player.render(&FrameInputs {
            time: frame as f32 / 60.0,
            frame,
            mouse: Mouse::default(),
            audio: &audio,
            controls: &controls,
            transitions: &[],
        });
    }
    Ok(())
}

/// The pixel at (`x`, `y`) of an RGBA8 frame read back top row first.
fn pixel(pixels: &[u8], size: [i32; 2], x: i32, y: i32) -> Result<[u8; 4]> {
    let offset = ((y * size[0] + x) * 4) as usize;
    let px = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok([px[0], px[1], px[2], px[3]])
}

fn near(actual: u8, expected: u8) -> bool {
    actual.abs_diff(expected) <= 2
}

const RED: &str = "vec4 renderMain() { return vec4(1.0, 0.0, 0.0, 1.0); }";
const BLUE: &str = "vec4 renderMain() { return vec4(0.0, 0.0, 1.0, 1.0); }";

#[test]
fn a_crossfade_gives_the_outgoing_frame_the_mix_and_the_incoming_frame() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let (red, blue) = (folder("{}", RED)?, folder("{}", BLUE)?);
        let mut a = ScenePlayer::new(gl, &caps, &red, SIZE, Options::default())?;
        let mut b = ScenePlayer::new(gl, &caps, &blue, SIZE, Options::default())?;
        render(&mut a, &red, 1)?;
        render(&mut b, &blue, 1)?;
        let compositor = Compositor::new(gl, &caps, SIZE)?;

        for (progress, expected) in [(0.0, [255, 0, 0]), (0.5, [128, 0, 128]), (1.0, [0, 0, 255])] {
            compositor.render(
                a.output_texture(),
                b.output_texture(),
                Blend::Crossfade,
                progress,
            );
            let px = pixel(&compositor.read_output(), SIZE, 16, 9)?;
            assert!(
                near(px[0], expected[0]) && near(px[1], expected[1]) && near(px[2], expected[2]),
                "{api:?} progress {progress}: expected {expected:?}, got {px:?}"
            );
        }
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn a_fade_through_black_goes_dark_in_the_middle() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let (red, blue) = (folder("{}", RED)?, folder("{}", BLUE)?);
        let mut a = ScenePlayer::new(gl, &caps, &red, SIZE, Options::default())?;
        let mut b = ScenePlayer::new(gl, &caps, &blue, SIZE, Options::default())?;
        render(&mut a, &red, 1)?;
        render(&mut b, &blue, 1)?;
        let compositor = Compositor::new(gl, &caps, SIZE)?;

        for (progress, expected) in [
            (0.0, [255, 0, 0]),
            (0.25, [128, 0, 0]),
            (0.5, [0, 0, 0]),
            (0.75, [0, 0, 128]),
            (1.0, [0, 0, 255]),
        ] {
            compositor.render(
                a.output_texture(),
                b.output_texture(),
                Blend::FadeThroughBlack,
                progress,
            );
            let px = pixel(&compositor.read_output(), SIZE, 16, 9)?;
            assert!(
                near(px[0], expected[0]) && near(px[1], expected[1]) && near(px[2], expected[2]),
                "{api:?} progress {progress}: expected {expected:?}, got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_custom_transition_is_compiled_once_and_mixes_by_its_own_rule() -> Result<()> {
    // A wipe: the incoming scene is revealed from the left as progress grows.
    const WIPE: &str = "vec4 transition(vec2 uv) {\n\
        return uv.x < progress ? getToColor(uv) : getFromColor(uv);\n\
    }";
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let (red, blue) = (folder("{}", RED)?, folder("{}", BLUE)?);
        let mut a = ScenePlayer::new(gl, &caps, &red, SIZE, Options::default())?;
        let mut b = ScenePlayer::new(gl, &caps, &blue, SIZE, Options::default())?;
        render(&mut a, &red, 1)?;
        render(&mut b, &blue, 1)?;
        let mut compositor = Compositor::new(gl, &caps, SIZE)?;
        let wipe = compositor.add_custom(WIPE)?;

        let before = gl.shader_compiles();
        compositor.render(
            a.output_texture(),
            b.output_texture(),
            Blend::Custom(wipe),
            0.5,
        );
        let pixels = compositor.read_output();
        let left = pixel(&pixels, SIZE, 4, 9)?;
        let right = pixel(&pixels, SIZE, 28, 9)?;
        assert!(
            near(left[2], 255) && near(left[0], 0),
            "{api:?}: the left of the wipe shows the incoming scene, got {left:?}"
        );
        assert!(
            near(right[0], 255) && near(right[2], 0),
            "{api:?}: the right of the wipe still shows the outgoing scene, got {right:?}"
        );
        assert_eq!(
            gl.shader_compiles(),
            before,
            "{api:?}: rendering a transition must not compile a shader"
        );
    }
    Ok(())
}

#[test]
fn a_broken_custom_transition_is_an_error_not_a_panic() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut compositor = Compositor::new(gl, &caps, SIZE)?;
        let error = compositor
            .add_custom("vec4 transition(vec2 uv) { return nonsense; }")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            error.contains("transition shader") && error.contains("nonsense"),
            "{api:?}: the error should carry the driver's log: {error}"
        );
        // A handle that was never made falls back to a crossfade.
        let (red, blue) = (folder("{}", RED)?, folder("{}", BLUE)?);
        let mut a = ScenePlayer::new(gl, &caps, &red, SIZE, Options::default())?;
        let mut b = ScenePlayer::new(gl, &caps, &blue, SIZE, Options::default())?;
        render(&mut a, &red, 1)?;
        render(&mut b, &blue, 1)?;
        compositor.render(
            a.output_texture(),
            b.output_texture(),
            Blend::Custom(7),
            0.5,
        );
        let px = pixel(&compositor.read_output(), SIZE, 16, 9)?;
        assert!(
            near(px[0], 128) && near(px[2], 128),
            "{api:?}: an unknown handle should crossfade, got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn compositing_does_not_compile_and_the_counter_counts_compiles() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let start = gl.shader_compiles();
    let red = folder("{}", RED)?;
    let mut a = ScenePlayer::new(gl, &caps, &red, SIZE, Options::default())?;
    // One pass: a vertex shader and a fragment shader.
    assert_eq!(
        gl.shader_compiles() - start,
        2,
        "a one-pass scene compiles two shader stages"
    );
    let compositor = Compositor::new(gl, &caps, SIZE)?;
    let loaded = gl.shader_compiles();
    render(&mut a, &red, 1)?;
    for step in 0..100 {
        compositor.render(
            a.output_texture(),
            a.output_texture(),
            Blend::Crossfade,
            step as f32 / 100.0,
        );
    }
    assert_eq!(
        gl.shader_compiles(),
        loaded,
        "a hundred transition frames compiled something"
    );
    Ok(())
}

#[test]
fn resizing_while_keeping_carries_a_feedback_scene_over_and_resizing_does_not() -> Result<()> {
    // Each frame adds 0.1 to the accumulator: after four frames it holds 0.4.
    let json = r#"{ "PASSES": [ {"TARGET": "acc", "DOUBLE_BUFFER": true} ] }"#;
    let shader = r#"
        vec4 renderMain() {
            if (PASSINDEX == 0) { return texture(acc, _uv) + vec4(0.1, 0.0, 0.0, 0.0); }
            return vec4(texture(acc, _uv).rgb, 1.0);
        }"#;
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = folder(json, shader)?;
        let big = [32, 18];
        let small = [16, 9];

        // Warm up small, then grow keeping the contents, and draw one more
        // frame: the accumulator continues from 0.4 to 0.5.
        let mut kept = ScenePlayer::new(gl, &caps, &scene, small, Options::default())?;
        render(&mut kept, &scene, 4)?;
        kept.resize_keeping(big)?;
        render(&mut kept, &scene, 1)?;
        let px = pixel(&kept.read_output(), big, 16, 9)?;
        assert!(
            near(px[0], 128),
            "{api:?}: a kept resize should continue from the warm-up (0.5), got {px:?}"
        );

        // A plain resize restarts from black: one frame gives 0.1.
        let mut restarted = ScenePlayer::new(gl, &caps, &scene, small, Options::default())?;
        render(&mut restarted, &scene, 4)?;
        restarted.resize(big)?;
        render(&mut restarted, &scene, 1)?;
        let px = pixel(&restarted.read_output(), big, 16, 9)?;
        assert!(
            near(px[0], 26),
            "{api:?}: a plain resize restarts from black (0.1), got {px:?}"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}
