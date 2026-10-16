// ============================================================================
// Module:       script_meta (integration test)
// Description:  A scene's script.js writes meta/ controls; the session's
//               meta bank must receive them, ease them, and leave the
//               scene's own controls alone; the operator's meta settings come
//               back after a rebuilt GL context.
//
// Dependencies: anyhow; strobe-app (Session, AudioInput); strobe-gfx
//               (headless context)
// ============================================================================

//! Script `meta/` writes, end to end on a headless context.
//!
//! The meta bank is held by the session and nothing in the renderer reads
//! it yet (D160), so these tests assert on the bank, never on pixels: a
//! test that expected a brighter picture would be testing an effect that
//! does not exist.
//!
//! Without EGL they report a skip and pass, unless
//! `STROBE_REQUIRE_HEADLESS_GL` is set (CI's Linux job).

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
            "strobe-app-meta-{}-{name}.synScene",
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

/// A meta control's output, first component.
fn meta_output(session: &Session, name: &str) -> Result<f32> {
    let index = session
        .meta
        .find(name)
        .ok_or_else(|| anyhow!("no meta control `{name}`"))?;
    session
        .meta
        .output(index)
        .map(|values| values[0])
        .ok_or_else(|| anyhow!("no output for `{name}`"))
}

const DATE: [f32; 4] = [2026.0, 9.0, 10.0, 0.0];

#[test]
fn a_script_write_to_a_meta_control_reaches_the_sessions_meta_bank() -> Result<()> {
    let scene = TempScene::new(
        "write",
        r#"{ "CONTROLS": [ { "NAME": "brightness", "TYPE": "slider", "DEFAULT": 0.5 } ] }"#,
        "vec4 renderMain() { return vec4(brightness, 0.0, 0.0, 1.0); }",
        "function update(dt) { setControl('meta/brightness', -0.5); setControl('meta/Hue', 0.25); }",
    )?;
    for api in [Api::DesktopCore33, Api::Gles30] {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut audio = AudioInput::new(false);
        let mut session = Session::load(gl, &caps, &scene.0, "write", [32, 18], 0.0, &mut audio)
            .map_err(|e| anyhow!("{api:?}: load failed: {e}"))?;
        session.advance(1.0 / 60.0, &mut audio, Mouse::default(), DATE);
        assert_eq!(
            meta_output(&session, "brightness")?,
            -0.5,
            "{api:?}: meta/brightness arrived"
        );
        assert_eq!(meta_output(&session, "hue")?, 0.25, "{api:?}: meta/Hue");
        let scene_brightness = session
            .controls
            .find("brightness")
            .and_then(|index| session.controls.output(index))
            .map(|values| values[0]);
        assert_eq!(
            scene_brightness,
            Some(0.5),
            "{api:?}: the scene's own `brightness` control is untouched"
        );
    }
    Ok(())
}

#[test]
fn a_rebuilt_context_gives_the_operators_meta_settings_back() -> Result<()> {
    // A lost GL context reloads the scene into a fresh session (D66) and
    // puts back what the operator had set. Master at 0 is a blackout: the
    // reloaded scene must not come back at full brightness.
    let scene = TempScene::new(
        "rebuild",
        r#"{ "CONTROLS": [ { "NAME": "amount", "TYPE": "slider", "DEFAULT": 0.5 } ] }"#,
        "vec4 renderMain() { return vec4(amount, 0.0, 0.0, 1.0); }",
        "function update(dt) {}",
    )?;
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let mut audio = AudioInput::new(false);
    let mut before = Session::load(gl, &caps, &scene.0, "rebuild", [32, 18], 0.0, &mut audio)
        .map_err(|e| anyhow!("load failed: {e}"))?;
    let master = before
        .meta
        .find("master")
        .ok_or_else(|| anyhow!("no master"))?;
    let hue = before.meta.find("hue").ok_or_else(|| anyhow!("no hue"))?;
    before.meta.set(master, None, &[0.0]);
    before.meta.set(hue, None, &[0.25]);
    before.controls.set(0, None, &[0.75]);
    let carried = before.control_targets();
    drop(before);

    let mut after = Session::load(gl, &caps, &scene.0, "rebuild", [32, 18], 0.0, &mut audio)
        .map_err(|e| anyhow!("reload failed: {e}"))?;
    after.restore_control_targets(&carried);
    after.advance(1.0 / 60.0, &mut audio, Mouse::default(), DATE);
    assert_eq!(meta_output(&after, "master")?, 0.0, "still blacked out");
    assert_eq!(meta_output(&after, "hue")?, 0.25, "the hue is kept");
    assert_eq!(
        after.controls.output(0).map(|values| values[0]),
        Some(0.75),
        "and the scene's own control"
    );
    Ok(())
}
