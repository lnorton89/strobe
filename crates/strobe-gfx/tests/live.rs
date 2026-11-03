// ============================================================================
// Module:       live (integration test)
// Description:  A camera behind syn_Media against a real GLSL compiler: its
//               pictures show upright with syn_MediaType 3 in one reused
//               texture, a failure reads black and is said once, and the
//               frames ask the driver nothing.
//
// Dependencies: anyhow; strobe-gfx (headless context, player, live),
//               strobe-scene
// ============================================================================

//! Live-source tests (backlog M-04, D528).
//!
//! A fake [`LiveSources`] hands out one mailbox per camera name, and the
//! test plays the capture thread by putting pictures in it. Its own test
//! binary, because the sources are installed process-wide. Like
//! `render.rs` these report a skip when no headless context exists, unless
//! `STROBE_REQUIRE_HEADLESS_GL` is set.

use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use strobe_gfx::live::{LiveFrame, LiveSources, Mailbox};
use strobe_gfx::{Api, Caps, FrameInputs, Headless, MediaRequest, Mouse, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;
use strobe_scene::scene::SceneDef;

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];
const SIZE: i32 = 4;

/// Reads `syn_Media` and puts `syn_MediaType / 4` in blue (3 reads 191).
const SHADER: &str = r#"
vec4 renderMain() {
    vec4 m = texture(syn_Media, _uv);
    return vec4(m.rg, syn_MediaType / 4.0, 1.0);
}"#;

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

/// One mailbox per camera name, as the app's registry would keep.
#[derive(Default)]
struct Fake(Mutex<HashMap<String, Mailbox>>);

impl LiveSources for Fake {
    fn camera(&self, name: &str) -> Mailbox {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(name.to_string())
            .or_default()
            .clone()
    }
}

/// The capture side of camera `name`.
fn camera(name: &str) -> Mailbox {
    static SOURCES: std::sync::OnceLock<Arc<Fake>> = std::sync::OnceLock::new();
    let sources = SOURCES.get_or_init(|| {
        let fake = Arc::new(Fake::default());
        strobe_gfx::live::install(fake.clone());
        fake
    });
    sources.camera(name)
}

fn scene() -> Result<SceneFolder> {
    Ok(SceneFolder {
        dir: PathBuf::from("."),
        def: SceneDef::parse("{}", "test")?,
        shader: SHADER.to_string(),
        pass_shaders: Vec::new(),
        script: None,
    })
}

/// A picture, bottom row first: `bottom` in the lower half, `top` above.
fn picture(bottom: [u8; 4], top: [u8; 4]) -> LiveFrame {
    let side = SIZE as usize;
    let mut rgba = Vec::with_capacity(side * side * 4);
    for row in 0..side {
        let color = if row < side / 2 { bottom } else { top };
        for _ in 0..side {
            rgba.extend_from_slice(&color);
        }
    }
    LiveFrame {
        width: SIZE as u32,
        height: SIZE as u32,
        rgba,
        captured: None,
    }
}

fn render(player: &mut ScenePlayer, scene: &SceneFolder, frame: u64) -> Result<Vec<u8>> {
    let controls = ControlBank::new(scene.def.controls.clone())?;
    let audio = vec![0.0; SCALARS.len()];
    player.render(&FrameInputs {
        time: frame as f32 / 60.0,
        frame,
        mouse: Mouse::default(),
        audio: &audio,
        controls: &controls,
        transitions: &[],
    });
    Ok(player.read_output())
}

/// The pixel at `(x, y)`, `y` counted from the top (readback order).
fn pixel(pixels: &[u8], x: i32, y: i32) -> Result<[u8; 4]> {
    let offset = ((y * SIZE + x) * 4) as usize;
    let px = pixels
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("readback too short"))?;
    Ok([px[0], px[1], px[2], px[3]])
}

fn near(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual
        .iter()
        .zip(&expected)
        .all(|(a, e)| a.abs_diff(*e) <= 2)
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

#[test]
fn a_camera_shows_upright_as_type_3_in_one_texture() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = scene()?;
        let name = format!("Shows {api:?}");
        let capture = camera(&name);
        let mut player = ScenePlayer::new(gl, &caps, &scene, [SIZE, SIZE], Options::default())?;
        player.choose_media(Some(MediaRequest::for_path(
            strobe_scene::camera::path_for(&name),
        )));
        assert!(
            player.media_loading(),
            "{api:?}: waiting for a first picture"
        );
        let px = pixel(&render(&mut player, &scene, 0)?, 1, 1)?;
        assert!(near(px, [0, 0, 0, 255]), "{api:?}: black before it: {px:?}");

        capture.put(picture(RED, GREEN));
        let pixels = render(&mut player, &scene, 1)?;
        let (bottom, top) = (pixel(&pixels, 1, 3)?, pixel(&pixels, 1, 0)?);
        assert!(
            near(bottom, [255, 0, 191, 255]) && near(top, [0, 255, 191, 255]),
            "{api:?}: upright, type 3: bottom {bottom:?}, top {top:?}"
        );
        assert!(!player.media_loading(), "{api:?}: no longer loading");

        // Later pictures go into the same texture, asking the driver nothing.
        let queries = gl.state_queries();
        for frame in 2..12 {
            let (bottom, top) = if frame % 2 == 0 {
                (GREEN, RED)
            } else {
                (RED, GREEN)
            };
            capture.put(picture(bottom, top));
            render(&mut player, &scene, frame)?;
        }
        let pixels = render(&mut player, &scene, 12)?;
        assert!(
            near(pixel(&pixels, 1, 3)?, [255, 0, 191, 255]),
            "{api:?}: the newest picture shows"
        );
        assert_eq!(
            gl.state_queries(),
            queries,
            "{api:?}: camera frames queried the driver (D497)"
        );
        let stats = capture.stats();
        assert_eq!(stats.taken, 11, "{api:?}: one picture a frame: {stats:?}");
        assert!(
            capture.buffer().capacity() > 0,
            "{api:?}: buffers come back"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn a_failing_camera_reads_black_says_so_once_and_comes_back() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scene = scene()?;
        let name = format!("Fails {api:?}");
        let capture = camera(&name);
        let mut player = ScenePlayer::new(gl, &caps, &scene, [SIZE, SIZE], Options::default())?;
        player.choose_media(Some(MediaRequest::for_path(
            strobe_scene::camera::path_for(&name),
        )));
        capture.put(picture(RED, RED));
        render(&mut player, &scene, 0)?;
        capture.fail("unplugged".to_string(), "plug it back in".to_string());
        let px = pixel(&render(&mut player, &scene, 1)?, 1, 1)?;
        assert!(near(px, [0, 0, 0, 255]), "{api:?}: black, type 0: {px:?}");
        let problems = player.take_asset_problems();
        assert!(
            problems.len() == 1
                && problems[0].asset == "syn_Media"
                && problems[0].message == "unplugged"
                && problems[0].action.as_deref() == Some("plug it back in")
                && problems[0].path.starts_with("camera:"),
            "{api:?}: said once, with its action: {problems:?}"
        );
        render(&mut player, &scene, 2)?;
        assert!(
            player.take_asset_problems().is_empty(),
            "{api:?}: and not again"
        );
        capture.put(picture(GREEN, GREEN));
        let px = pixel(&render(&mut player, &scene, 3)?, 1, 1)?;
        assert!(near(px, [0, 255, 191, 255]), "{api:?}: back: {px:?}");
    }
    Ok(())
}
