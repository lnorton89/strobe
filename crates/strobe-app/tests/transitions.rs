// ============================================================================
// Module:       transitions (integration test)
// Description:  A new song changes a scene's HARD_TRANSITIONS at once and
//               crossfades its SMOOTH_TRANSITIONS, and the shader sees both,
//               in both GL dialects.
//
// Dependencies: anyhow; strobe-app (Session, AudioInput); strobe-gfx
//               (headless context)
// ============================================================================

//! Scene transitions, end to end on a headless context (D489).
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
    fn new(name: &str, json: &str, shader: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "strobe-transitions-test-{}-{name}.synScene",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("scene.json"), json)?;
        std::fs::write(dir.join("main.glsl"), shader)?;
        Ok(Self(dir))
    }
}

impl Drop for TempScene {
    fn drop(&mut self) {
        // A temp folder left behind is harmless; the test's result stands.
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

fn near(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 3)
}

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

const DATE: [f32; 4] = [2026.0, 9.0, 10.0, 0.0];

const DT: f32 = 1.0 / 60.0;

#[test]
fn a_new_song_jumps_hard_transitions_and_crossfades_smooth_ones() -> Result<()> {
    let scene = TempScene::new(
        "song",
        r#"{ "HARD_TRANSITIONS": [ { "UNIFORM": "dir", "VALUES": [0.0, 1.0] } ],
             "SMOOTH_TRANSITIONS": [ { "UNIFORMS": ["a", "b"], "DURATION": 1 } ] }"#,
        "vec4 renderMain() { return vec4(dir, a, b, 1.0); }",
    )?;
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut audio = AudioInput::new(false);
        let mut session = Session::load(gl, &caps, &scene.0, "song", [32, 18], 0.0, &mut audio)
            .map_err(|e| anyhow!("{api:?}: load failed: {e}"))?;
        assert!(session.has_transitions(), "{api:?}: it has some");
        session.advance(DT, &mut audio, Mouse::default(), DATE);
        let before = centre(&session)?;
        assert!(
            near(before, [0, 255, 0]),
            "{api:?}: first value, first member: {before:?}"
        );

        assert!(session.song_change(), "{api:?}: something changes");
        // 30 frames of 1/60 s: half of the one-second crossfade.
        for _ in 0..30 {
            session.advance(DT, &mut audio, Mouse::default(), DATE);
        }
        let halfway = centre(&session)?;
        assert!(
            near(halfway, [255, 128, 128]),
            "{api:?}: dir jumped, a and b halfway: {halfway:?}"
        );
        for _ in 0..40 {
            session.advance(DT, &mut audio, Mouse::default(), DATE);
        }
        let after = centre(&session)?;
        assert!(
            near(after, [255, 0, 255]),
            "{api:?}: the crossfade is done: {after:?}"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn a_scene_without_transitions_ignores_a_new_song() -> Result<()> {
    let scene = TempScene::new(
        "plain",
        "{}",
        "vec4 renderMain() { return vec4(0.2, 0.4, 0.6, 1.0); }",
    )?;
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let mut audio = AudioInput::new(false);
    let mut session = Session::load(gl, &caps, &scene.0, "plain", [32, 18], 0.0, &mut audio)
        .map_err(|e| anyhow!("load failed: {e}"))?;
    assert!(!session.has_transitions(), "none declared");
    assert!(!session.song_change(), "nothing to change");
    Ok(())
}
