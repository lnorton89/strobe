// ============================================================================
// Module:       assets (integration test)
// Description:  Cube maps and syn_Media against a real GLSL compiler: each
//               face direction returns its face colour, a missing cube map
//               reports a problem and reads black, and a still image and an
//               image sequence arrive through syn_Media, in both dialects.
//
// Dependencies: anyhow, image; strobe-gfx (headless context, player)
// ============================================================================

//! Scene asset tests.
//!
//! Scenes are written to a scratch folder with real PNG files, loaded
//! through `SceneFolder`, and rendered on a headless EGL context (Mesa
//! llvmpipe on CI). Like `render.rs` they report a skip when no context
//! exists, unless `STROBE_REQUIRE_HEADLESS_GL` is set.

use anyhow::{Result, anyhow};
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};
use strobe_gfx::{Api, Caps, FrameInputs, Headless, MediaRequest, Mouse, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

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

/// A scene folder in the temp directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("strobe-gfx-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, name: &str, text: &str) -> Result<()> {
        std::fs::write(self.0.join(name), text)?;
        Ok(())
    }

    fn picture(&self, name: &str, image: &RgbaImage) -> Result<()> {
        image.save(self.0.join(name))?;
        Ok(())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn solid(size: u32, color: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(size, size, Rgba(color))
}

fn load(scratch: &Scratch, json: &str, shader: &str) -> Result<SceneFolder> {
    scratch.write("scene.json", json)?;
    scratch.write("main.glsl", shader)?;
    Ok(SceneFolder::load(scratch.path())?)
}

/// Renders one frame at `time` and returns the output, top row first.
fn render_at(player: &mut ScenePlayer, scene: &SceneFolder, time: f32) -> Result<Vec<u8>> {
    let controls = ControlBank::new(scene.def.controls.clone())?;
    let audio = vec![0.0; SCALARS.len()];
    player.render(&FrameInputs {
        time,
        frame: 0,
        mouse: Mouse::default(),
        audio: &audio,
        controls: &controls,
        transitions: &[],
    });
    Ok(player.read_output())
}

fn pixel(pixels: &[u8], width: i32, x: i32, y: i32) -> Result<[u8; 4]> {
    let offset = ((y * width + x) * 4) as usize;
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

/// Samples the cube map along +X, -X, +Y, -Y, +Z, -Z, one sixth of the
/// frame each.
const CUBE_SHADER: &str = r#"
vec4 renderMain() {
    int i = int(floor(_uv.x * 6.0));
    vec3 d = vec3(1.0, 0.0, 0.0);
    if (i == 1) d = vec3(-1.0, 0.0, 0.0);
    if (i == 2) d = vec3(0.0, 1.0, 0.0);
    if (i == 3) d = vec3(0.0, -1.0, 0.0);
    if (i == 4) d = vec3(0.0, 0.0, 1.0);
    if (i == 5) d = vec3(0.0, 0.0, -1.0);
    return texture(env, d);
}"#;

const FACE_COLORS: [[u8; 4]; 6] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 0, 255],
    [255, 0, 255, 255],
    [0, 255, 255, 255],
];

const SIZE: [i32; 2] = [12, 2];

fn check_faces(pixels: &[u8], api: Api) -> Result<()> {
    for (face, expected) in FACE_COLORS.iter().enumerate() {
        let px = pixel(pixels, SIZE[0], face as i32 * 2, 0)?;
        assert!(
            near(px, *expected),
            "{api:?}: direction {face} should read {expected:?}, got {px:?}"
        );
    }
    Ok(())
}

#[test]
fn six_face_files_answer_each_direction_with_their_colour() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("cube6")?;
        let names = ["px", "nx", "py", "ny", "pz", "nz"];
        for (name, color) in names.iter().zip(FACE_COLORS) {
            scratch.picture(&format!("{name}.png"), &solid(8, color))?;
        }
        let paths: Vec<String> = names.iter().map(|n| format!("\"{n}.png\"")).collect();
        let json = format!(
            r#"{{ "CUBEMAPS": [ {{ "NAME": "env", "PATHS": [{}] }} ] }}"#,
            paths.join(", ")
        );
        let scene = load(&scratch, &json, CUBE_SHADER)?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, SIZE, Options::default())?;
        assert!(
            player.take_asset_problems().is_empty(),
            "{api:?}: a good cube map reports nothing"
        );
        let pixels = render_at(&mut player, &scene, 0.0)?;
        check_faces(&pixels, api)?;
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn a_cross_picture_gives_the_same_six_faces() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("cross")?;
        // A 4:3 cross of 4-texel cells.
        let mut cross = RgbaImage::from_pixel(16, 12, Rgba([0, 0, 0, 255]));
        let cells = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)];
        for (color, (column, row)) in FACE_COLORS.iter().zip(cells) {
            for x in 0..4 {
                for y in 0..4 {
                    cross.put_pixel(column * 4 + x, row * 4 + y, Rgba(*color));
                }
            }
        }
        scratch.picture("cross.png", &cross)?;
        let scene = load(
            &scratch,
            r#"{ "CUBEMAPS": [ { "NAME": "env", "PATHS": ["cross.png"] } ] }"#,
            CUBE_SHADER,
        )?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, SIZE, Options::default())?;
        check_faces(&render_at(&mut player, &scene, 0.0)?, api)?;
    }
    Ok(())
}

#[test]
fn a_missing_image_reports_a_problem_and_reads_black() -> Result<()> {
    // TORNADO T6: a real scene names `x.png` while its folder holds
    // `x-01.png`. Synesthesia plays it; the picture is simply missing.
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("image-missing")?;
        let scene = load(
            &scratch,
            r#"{ "IMAGES": [ { "NAME": "pic", "PATH": "images/gone.png" } ] }"#,
            "vec4 renderMain() { return vec4(texture(pic, _uv).rgb + vec3(0.0, 0.5, 0.0), 1.0); }",
        )?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, SIZE, Options::default())?;
        let problems = player.take_asset_problems();
        assert_eq!(problems.len(), 1, "{api:?}: one problem, got {problems:?}");
        assert_eq!(problems[0].asset, "pic", "{api:?}: names the sampler");
        assert_eq!(problems[0].path, "images/gone.png", "{api:?}: and the file");
        let pixels = render_at(&mut player, &scene, 0.0)?;
        assert!(
            near(pixel(&pixels, SIZE[0], 0, 0)?, [0, 128, 0, 255]),
            "{api:?}: the scene runs and the picture reads black"
        );
    }
    Ok(())
}

#[test]
fn a_missing_cube_map_reports_a_problem_and_reads_black() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("cube-missing")?;
        let scene = load(
            &scratch,
            r#"{ "CUBEMAPS": [ { "NAME": "env",
                 "PATHS": ["a.png", "b.png", "c.png", "d.png", "e.png", "f.png"] } ] }"#,
            CUBE_SHADER,
        )?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, SIZE, Options::default())?;
        let problems = player.take_asset_problems();
        assert_eq!(problems.len(), 1, "{api:?}: one problem, got {problems:?}");
        assert_eq!(problems[0].asset, "env", "{api:?}: names the sampler");
        assert!(
            problems[0].message.contains("a.png"),
            "{api:?}: names the file: {}",
            problems[0].message
        );
        assert!(
            player.take_asset_problems().is_empty(),
            "{api:?}: reported once"
        );
        let pixels = render_at(&mut player, &scene, 0.0)?;
        for face in 0..6 {
            assert!(
                near(pixel(&pixels, SIZE[0], face * 2, 0)?, [0, 0, 0, 255]),
                "{api:?}: the scene keeps running with a black cube, direction {face}"
            );
        }
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn an_invalid_cube_map_is_a_problem_too() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("cube-invalid")?;
        scratch.picture("small.png", &solid(4, [9, 9, 9, 255]))?;
        scratch.picture("big.png", &solid(8, [9, 9, 9, 255]))?;
        let scene = load(
            &scratch,
            r#"{ "CUBEMAPS": [ { "NAME": "env",
                 "PATHS": ["small.png", "small.png", "small.png", "small.png", "small.png", "big.png"] } ] }"#,
            CUBE_SHADER,
        )?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, SIZE, Options::default())?;
        let problems = player.take_asset_problems();
        assert!(
            problems.len() == 1 && problems[0].message.contains("face 5"),
            "{api:?}: mismatched faces: {problems:?}"
        );
    }
    Ok(())
}

/// Reads `syn_Media` upright and encodes `syn_MediaType` in blue.
const MEDIA_SHADER: &str = r#"
vec4 renderMain() {
    vec4 m = texture(syn_Media, _uv);
    return vec4(m.rg, syn_MediaType * 0.5, 1.0);
}"#;

#[test]
fn a_still_image_is_read_through_syn_media_upright() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("still")?;
        // Red in the picture's top-left, green everywhere else.
        let mut picture = RgbaImage::from_pixel(8, 8, Rgba([0, 255, 0, 255]));
        for x in 0..4 {
            for y in 0..4 {
                picture.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        scratch.picture("pic.png", &picture)?;
        let scene = load(&scratch, r#"{ "MEDIA": "pic.png" }"#, MEDIA_SHADER)?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [8, 8], Options::default())?;
        // Before the picture lands there is nothing: black and type 0.
        player.finish_media_loading();
        assert!(player.media_ready(), "{api:?}: the picture arrived");
        assert!(
            player.take_asset_problems().is_empty(),
            "{api:?}: no problems"
        );
        let pixels = render_at(&mut player, &scene, 0.0)?;
        let top_left = pixel(&pixels, 8, 1, 1)?;
        let bottom_right = pixel(&pixels, 8, 6, 6)?;
        assert!(
            near(top_left, [255, 0, 128, 255]),
            "{api:?}: the picture's top-left shows at the top-left, type 1: {top_left:?}"
        );
        assert!(
            near(bottom_right, [0, 255, 128, 255]),
            "{api:?}: and green elsewhere: {bottom_right:?}"
        );
        if let Some(code) = gl.error() {
            return Err(anyhow!("{api:?}: GL error 0x{code:x}"));
        }
    }
    Ok(())
}

#[test]
fn no_media_is_black_and_type_zero() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("nomedia")?;
        let scene = load(&scratch, "{}", MEDIA_SHADER)?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [4, 4], Options::default())?;
        let pixels = render_at(&mut player, &scene, 0.0)?;
        assert!(
            near(pixel(&pixels, 4, 1, 1)?, [0, 0, 0, 255]),
            "{api:?}: black, type 0"
        );
    }
    Ok(())
}

#[test]
fn an_image_sequence_plays_at_its_rate_and_loops() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("sequence")?;
        std::fs::create_dir_all(scratch.path().join("frames"))?;
        scratch.picture("frames/f1.png", &solid(4, [255, 0, 0, 255]))?;
        scratch.picture("frames/f2.png", &solid(4, [0, 255, 0, 255]))?;
        scratch.picture("frames/f3.png", &solid(4, [0, 0, 255, 255]))?;
        let scene = load(
            &scratch,
            r#"{ "MEDIA": { "PATH": "frames", "FPS": 1 } }"#,
            MEDIA_SHADER,
        )?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [4, 4], Options::default())?;
        player.finish_media_loading();
        for (time, expected) in [
            (0.0, [255, 0, 128, 255]),
            (1.2, [0, 255, 128, 255]),
            (2.5, [0, 0, 128, 255]),
            (3.1, [255, 0, 128, 255]),
        ] {
            let pixels = render_at(&mut player, &scene, time)?;
            let px = pixel(&pixels, 4, 2, 2)?;
            assert!(
                near(px, expected),
                "{api:?}: at {time}s expected {expected:?}, got {px:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_user_chosen_file_replaces_the_scenes_media_and_a_bad_one_reports() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("choose")?;
        scratch.picture("own.png", &solid(4, [255, 0, 0, 255]))?;
        scratch.picture("chosen.png", &solid(4, [0, 255, 0, 255]))?;
        let scene = load(&scratch, r#"{ "MEDIA": "own.png" }"#, MEDIA_SHADER)?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [4, 4], Options::default())?;
        player.finish_media_loading();
        player.set_media(MediaRequest::for_path(scratch.path().join("chosen.png")));
        player.finish_media_loading();
        let px = pixel(&render_at(&mut player, &scene, 0.0)?, 4, 2, 2)?;
        assert!(
            near(px, [0, 255, 128, 255]),
            "{api:?}: the chosen picture shows: {px:?}"
        );
        player.set_media(MediaRequest::for_path(scratch.path().join("missing.png")));
        player.finish_media_loading();
        let problems = player.take_asset_problems();
        assert!(
            problems.len() == 1 && problems[0].asset == "syn_Media",
            "{api:?}: the failure is reported: {problems:?}"
        );
        let px = pixel(&render_at(&mut player, &scene, 0.0)?, 4, 2, 2)?;
        assert!(
            near(px, [0, 255, 128, 255]),
            "{api:?}: and the picture that was showing stays: {px:?}"
        );
    }
    Ok(())
}

#[test]
fn choosing_none_goes_back_to_the_scenes_own_media_or_to_nothing() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("restore")?;
        scratch.picture("own.png", &solid(4, [255, 0, 0, 255]))?;
        scratch.picture("chosen.png", &solid(4, [0, 255, 0, 255]))?;
        let chosen = MediaRequest::for_path(scratch.path().join("chosen.png"));

        let scene = load(&scratch, r#"{ "MEDIA": "own.png" }"#, MEDIA_SHADER)?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [4, 4], Options::default())?;
        player.finish_media_loading();
        assert!(player.media_is_scenes_own(), "{api:?}: starts on its own");
        player.choose_media(Some(chosen.clone()));
        player.finish_media_loading();
        assert!(
            !player.media_is_scenes_own(),
            "{api:?}: chosen in its place"
        );
        assert_eq!(
            player.media_request(),
            Some(&chosen),
            "{api:?}: the chosen source is the one showing"
        );
        player.choose_media(Some(chosen.clone()));
        assert!(
            !player.media_loading(),
            "{api:?}: asking again for what shows starts nothing"
        );
        player.choose_media(None);
        player.finish_media_loading();
        let px = pixel(&render_at(&mut player, &scene, 0.0)?, 4, 2, 2)?;
        assert!(
            near(px, [255, 0, 128, 255]),
            "{api:?}: the scene's own picture is back: {px:?}"
        );

        let bare = Scratch::new("restore-bare")?;
        let plain = load(&bare, "{}", MEDIA_SHADER)?;
        let mut player = ScenePlayer::new(gl, &caps, &plain, [4, 4], Options::default())?;
        player.choose_media(Some(chosen.clone()));
        player.finish_media_loading();
        assert!(
            player.media_ready(),
            "{api:?}: a scene without media takes one"
        );
        player.choose_media(None);
        let px = pixel(&render_at(&mut player, &plain, 0.0)?, 4, 2, 2)?;
        assert!(
            near(px, [0, 0, 0, 255]),
            "{api:?}: and goes back to black, type 0: {px:?}"
        );
    }
    Ok(())
}

#[test]
fn a_media_folder_with_nothing_in_it_is_a_problem_not_a_crash() -> Result<()> {
    for api in BOTH {
        let Some(headless) = context(api)? else {
            continue;
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let scratch = Scratch::new("emptymedia")?;
        std::fs::create_dir_all(scratch.path().join("none"))?;
        let scene = load(&scratch, r#"{ "MEDIA": "none" }"#, MEDIA_SHADER)?;
        let mut player = ScenePlayer::new(gl, &caps, &scene, [4, 4], Options::default())?;
        player.finish_media_loading();
        let problems = player.take_asset_problems();
        assert!(
            problems.len() == 1 && problems[0].message.contains("no .png"),
            "{api:?}: {problems:?}"
        );
        assert!(!player.media_ready(), "{api:?}: nothing to show");
    }
    Ok(())
}
