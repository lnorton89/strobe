// ============================================================================
// Module:       script_uniforms (integration test)
// Description:  A scene's script.js creates uniforms with setUniform; they
//               must be compiled in at load and reach the shader; one
//               created late is refused and never compiles a shader, in
//               both GL dialects.
//
// Dependencies: anyhow; strobe-app (Session, AudioInput); strobe-gfx
//               (headless context)
// ============================================================================

//! Script-created uniforms, end to end on a headless context.
//!
//! Without EGL these tests report a skip and pass, as the renderer's own
//! tests do, unless `STROBE_REQUIRE_HEADLESS_GL` is set (CI's Linux job).

use anyhow::{Result, anyhow};
use std::path::PathBuf;
use strobe_app::audio::AudioInput;
use strobe_app::session::Session;
use strobe_gfx::{Api, Caps, Headless, Mouse};

/// A scene folder in the temp directory, removed on drop.
struct TempScene(PathBuf);

impl TempScene {
    fn new(name: &str, json: &str, shader: &str, script: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "strobe-app-test-{}-{name}.synScene",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("scene.json"), json)?;
        std::fs::write(dir.join("main.glsl"), shader)?;
        std::fs::write(dir.join("script.js"), script)?;
        Ok(Self(dir))
    }
}

impl Drop for TempScene {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

/// The centre pixel of the scene's output.
fn centre(session: &Session) -> Result<[u8; 3]> {
    let [w, h] = session.player.render_size();
    let pixels = session.player.read_output();
    let offset = ((h / 2 * w + w / 2) * 4) as usize;
    let px = pixels
        .get(offset..offset + 3)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok([px[0], px[1], px[2]])
}

fn near(actual: u8, expected: u8) -> bool {
    actual.abs_diff(expected) <= 2
}

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

const DATE: [f32; 4] = [2026.0, 9.0, 10.0, 0.0];

#[test]
fn a_uniform_made_in_update_is_declared_at_load_and_reaches_the_shader() -> Result<()> {
    let scene = TempScene::new(
        "update",
        "{}",
        "vec4 renderMain() { return vec4(tint, 1.0); }",
        "function update(dt) { setUniform('tint', 0.5, 0.25, 1.0); }",
    )?;
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut audio = AudioInput::new(false);
        // The shader cannot compile without `tint`, so a successful load is
        // itself the proof that it was declared before the first compile.
        let mut session = Session::load(gl, &caps, &scene.0, "update", [32, 18], 0.0, &mut audio)
            .map_err(|e| anyhow!("{api:?}: load failed: {e}"))?;
        assert_eq!(
            session.player.extras().len(),
            1,
            "{api:?}: one script uniform compiled in"
        );
        session.advance(
            1.0 / 60.0,
            &mut audio,
            Mouse::default(),
            [2026.0, 9.0, 10.0, 0.0],
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
        let px = centre(&session)?;
        assert!(
            near(px[0], 128) && near(px[1], 64) && near(px[2], 255),
            "{api:?}: tint (0.5, 0.25, 1.0) should be on screen, got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn a_uniform_made_in_setup_is_also_declared_at_load() -> Result<()> {
    let scene = TempScene::new(
        "setup",
        "{}",
        "vec4 renderMain() { return vec4(base, 0.0, 0.0, 1.0); }",
        "function setup() { setUniform('base', 0.75); }",
    )?;
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let mut audio = AudioInput::new(false);
    let mut session = Session::load(gl, &caps, &scene.0, "setup", [32, 18], 0.0, &mut audio)
        .map_err(|e| anyhow!("load failed: {e}"))?;
    session.advance(
        1.0 / 60.0,
        &mut audio,
        Mouse::default(),
        [2026.0, 9.0, 10.0, 0.0],
    );
    let px = centre(&session)?;
    assert!(near(px[0], 191), "0.75 is 191 of 255, got {px:?}");
    Ok(())
}

#[test]
fn a_first_update_slower_than_a_frame_still_declares_its_uniforms() -> Result<()> {
    // The load-time frame is paid while the scene loads, not on screen. A
    // first update that does its set-up work (here 20 ms of it, as a heavy
    // one might take on a 2012 laptop) must not be cut at the 4 ms frame
    // budget: its uniforms would then be discarded, never declared, and the
    // shader that reads them would not compile, on slow machines only.
    let scene = TempScene::new(
        "slow-first",
        "{}",
        "vec4 renderMain() { return vec4(ready, 0.0, 0.0, 1.0); }",
        "function update(dt) {\n\
           if (FRAMECOUNT === 0) { var until = Date.now() + 20; while (Date.now() < until) {} }\n\
           setUniform('ready', 1.0);\n\
         }",
    )?;
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let mut audio = AudioInput::new(false);
    let mut session = Session::load(gl, &caps, &scene.0, "slow-first", [32, 18], 0.0, &mut audio)
        .map_err(|e| anyhow!("load failed: {e}"))?;
    session.advance(1.0 / 60.0, &mut audio, Mouse::default(), DATE);
    let px = centre(&session)?;
    assert!(near(px[0], 255), "`ready` reaches the shader, got {px:?}");
    Ok(())
}

#[test]
fn a_uniform_that_appears_later_is_refused_and_nothing_compiles_after_load() -> Result<()> {
    let scene = TempScene::new(
        "late",
        r#"{ "CONTROLS": [ { "NAME": "amount", "TYPE": "slider", "DEFAULT": 0.5 } ] }"#,
        "vec4 renderMain() { return vec4(amount, 0.0, 0.0, 1.0); }",
        // `known` is declared in the first update; `late` only appears from
        // the fourth frame, so it cannot be known at load. The shader uses
        // neither, so it compiles regardless.
        "function update(dt) { setUniform('known', 1.0); \
         if (FRAMECOUNT > 3) { setUniform('late', 1.0); setUniform('known', 2.0); } }",
    )?;
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut audio = AudioInput::new(false);
        let mut session = Session::load(gl, &caps, &scene.0, "late", [32, 18], 0.0, &mut audio)
            .map_err(|e| anyhow!("{api:?}: load failed: {e}"))?;
        let names = |session: &Session| -> Vec<String> {
            session
                .player
                .extras()
                .iter()
                .map(|(name, _)| name.clone())
                .collect()
        };
        assert_eq!(names(&session), ["known"], "{api:?}: only `known` at load");
        let compiled_at_load = gl.shader_compiles();
        assert!(
            compiled_at_load > 0,
            "{api:?}: the counter sees the load's compiles"
        );
        for _ in 0..12 {
            session.advance(1.0 / 60.0, &mut audio, Mouse::default(), DATE);
        }
        assert_eq!(
            gl.shader_compiles(),
            compiled_at_load,
            "{api:?}: no shader was compiled while the scene played, even though the script \
             created `late` on frame 4"
        );
        assert_eq!(
            names(&session),
            ["known"],
            "{api:?}: the late uniform was refused, not adopted"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
        let px = centre(&session)?;
        assert!(
            near(px[0], 128),
            "{api:?}: the scene still renders (amount 0.5): {px:?}"
        );
    }
    Ok(())
}

#[test]
fn texture_final_pass_reads_the_picture_a_few_frames_late() -> Result<()> {
    // The picture is red 0.8 on every frame; green is whatever the script
    // saw of it. The script can only see it once the readback ring has
    // filled, so green is 0 for the first frames and 0.8 afterwards.
    let scene = TempScene::new(
        "final-pass",
        "{}",
        "vec4 renderMain() { return vec4(0.8, seen.r, 0.0, 1.0); }",
        "function update(dt) { setUniform('seen', textureFinalPass(0.5, 0.5)); }",
    )?;
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut audio = AudioInput::new(false);
        let mut session =
            Session::load(gl, &caps, &scene.0, "final-pass", [64, 36], 0.0, &mut audio)
                .map_err(|e| anyhow!("{api:?}: load failed: {e}"))?;
        for step in 1..=3 {
            session.advance(1.0 / 60.0, &mut audio, Mouse::default(), DATE);
            let px = centre(&session)?;
            assert!(
                near(px[0], 204) && px[1] == 0,
                "{api:?}: advance {step}: the script still sees black; got {px:?}"
            );
        }
        for _ in 0..40 {
            session.advance(1.0 / 60.0, &mut audio, Mouse::default(), DATE);
            gl.finish();
        }
        let px = centre(&session)?;
        assert!(
            near(px[0], 204) && near(px[1], 204),
            "{api:?}: the script saw the red the scene drew; got {px:?}"
        );
        assert_eq!(
            strobe_gfx::readback::RGB_BYTES,
            strobe_script::FINAL_PASS_BYTES,
            "the gfx and script sides agree on the downsample size"
        );
    }
    Ok(())
}

#[test]
fn a_script_that_never_names_texture_final_pass_pays_for_no_readback() -> Result<()> {
    let scene = TempScene::new(
        "no-final-pass",
        "{}",
        "vec4 renderMain() { return vec4(0.8, 0.0, 0.0, 1.0); }",
        "function update(dt) { }",
    )?;
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let mut audio = AudioInput::new(false);
    let mut session = Session::load(
        gl,
        &caps,
        &scene.0,
        "no-final-pass",
        [64, 36],
        0.0,
        &mut audio,
    )
    .map_err(|e| anyhow!("load failed: {e}"))?;
    for _ in 0..6 {
        session.advance(1.0 / 60.0, &mut audio, Mouse::default(), DATE);
        gl.finish();
    }
    assert!(
        session.player.final_pass_pixels().is_none(),
        "no script call, no readback ring"
    );
    Ok(())
}

#[test]
fn uniforms_reach_the_right_slots_when_a_name_is_dropped_and_after_a_restart() -> Result<()> {
    // `fade` clashes with a transition uniform, so the shaders are built without it
    // while the script still sets it first: the script's order and the
    // shaders' differ, and the values are bound to the shaders' names once,
    // at the seal and again at a restart, not looked up by name each frame.
    let scene = TempScene::new(
        "bound",
        r#"{ "HARD_TRANSITIONS": [ { "UNIFORM": "fade", "VALUES": [0.0] } ] }"#,
        "vec4 renderMain() { return vec4(first, second, fade, 1.0); }",
        "function update(dt) { setUniform('fade', 0.9); setUniform('second', 0.25); \
         setUniform('first', 0.5); }",
    )?;
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut audio = AudioInput::new(false);
        let mut session = Session::load(gl, &caps, &scene.0, "bound", [32, 18], 0.0, &mut audio)
            .map_err(|e| anyhow!("{api:?}: load failed: {e}"))?;
        let names: Vec<&str> = session
            .player
            .extras()
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            names,
            ["second", "first"],
            "{api:?}: the clashing name is left out"
        );
        for restarted in [false, true] {
            if restarted {
                session.restart_script(&mut audio, 0.0);
            }
            session.advance(1.0 / 60.0, &mut audio, Mouse::default(), DATE);
            if let Some(code) = gl.error() {
                return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
            }
            let px = centre(&session)?;
            assert!(
                near(px[0], 128) && near(px[1], 64) && near(px[2], 0),
                "{api:?} restarted={restarted}: first 0.5 and second 0.25 reach their own \
                 uniforms, and the transition stays at 0: {px:?}"
            );
        }
    }
    Ok(())
}
