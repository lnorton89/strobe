// ============================================================================
// Module:       script_uniforms (integration test)
// Description:  A scene's script.js creates uniforms with setUniform; they
//               must be compiled in at load, reach the shader, and survive
//               the rare late creation, in both GL dialects.
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
            eprintln!("skipping: no headless {api:?} context ({error})");
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
fn a_uniform_that_appears_later_is_adopted_with_one_recompile() -> Result<()> {
    let scene = TempScene::new(
        "late",
        r#"{ "CONTROLS": [ { "NAME": "amount", "TYPE": "slider", "DEFAULT": 0.5 } ] }"#,
        "vec4 renderMain() { return vec4(amount, 0.0, 0.0, 1.0); }",
        // `late` only appears from the fourth frame, so it cannot be known
        // at load; the shader does not use it, so it compiles without.
        "function update(dt) { if (FRAMECOUNT > 3) { setUniform('late', 1.0); } }",
    )?;
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let mut audio = AudioInput::new(false);
    let mut session = Session::load(gl, &caps, &scene.0, "late", [32, 18], 0.0, &mut audio)
        .map_err(|e| anyhow!("load failed: {e}"))?;
    assert!(
        session.player.extras().is_empty(),
        "nothing is known at load: {:?}",
        session.player.extras()
    );
    for _ in 0..8 {
        session.advance(
            1.0 / 60.0,
            &mut audio,
            Mouse::default(),
            [2026.0, 9.0, 10.0, 0.0],
        );
    }
    let names: Vec<&str> = session
        .player
        .extras()
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(names, ["late"], "the late uniform was adopted");
    let px = centre(&session)?;
    assert!(
        near(px[0], 128),
        "the scene still renders after the recompile (amount 0.5): {px:?}"
    );
    Ok(())
}
