// ============================================================================
// Module:       sandbox (integration test)
// Description:  Drives the browser sandbox's engine on the headless GLES 3.0
//               context, the dialect WebGL2 speaks.
//
// Dependencies: anyhow, serde_json; strobe-web (sandbox), strobe-gfx
//               (headless context, the app's player), strobe-scene (scene
//               folders, presets); the corpus gate's fixture scenes
// ============================================================================

//! The sandbox's behaviour the /play page depends on, without a browser.
//!
//! WebGL2 is GLSL ES 3.00, so the headless GLES 3.0 context compiles what
//! the browser compiles. Where no headless EGL exists the tests report a
//! skip and pass, unless `STROBE_REQUIRE_HEADLESS_GL` is set (CI's Linux
//! job), as the renderer tests do.
//!
//! The parity tests (D554) play the corpus gate's `media-image` and
//! `include-file` fixtures both ways, the app's (the folder from disk,
//! `ScenePlayer`) and the sandbox's (the files and pictures as the page
//! holds them), and require the same bytes.
//!
//! Generate controls (D558) is compared with the app's [`generate`] the same
//! way: the same scene must get the same controls and the same files.

#![cfg(not(target_arch = "wasm32"))]

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use strobe_autocontrol::{Options as GenerateOptions, generate, rewritten_files};
use strobe_gfx::{Api, Caps, Headless};
use strobe_scene::SceneFolder;
use strobe_web::sandbox::{AudioSource, FrameRequest, Sandbox, extra_type};

fn headless() -> Result<Option<Headless>> {
    match Headless::new(Api::Gles30) {
        Ok(headless) => Ok(Some(headless)),
        Err(error) if std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_none() => {
            strobe_gfx::skip::note(format_args!("no headless GLES context ({error})"));
            Ok(None)
        }
        Err(error) => Err(anyhow!("headless GLES context required: {error}")),
    }
}

const SIZE: [i32; 2] = [32, 18];

const SOLID: &str =
    r#"{"CONTROLS":[{"NAME":"level","TYPE":"slider","MIN":0,"MAX":1,"DEFAULT":0.5,"PARAMS":1}]}"#;

fn files<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, text)| (*text).to_string())
    }
}

/// A page that holds no pictures.
fn no_assets(_: &str) -> Option<Vec<u8>> {
    None
}

fn centre_red(sandbox: &Sandbox) -> Option<u8> {
    let [w, h] = sandbox.render_size();
    let pixels = sandbox.read_output();
    let index = (((h / 2) * w + w / 2) * 4) as usize;
    pixels.get(index).copied()
}

fn draw(sandbox: &mut Sandbox, frame: u64) {
    sandbox.frame(&FrameRequest {
        time: frame as f32 / 60.0,
        dt: 1.0 / 60.0,
        frame,
        ..FrameRequest::default()
    });
}

#[test]
fn a_scene_loads_draws_and_its_controls_move_it() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    sandbox.set_audio_source(AudioSource::Silent);
    let shader = "vec4 renderMain() { return vec4(level, 0.0, 0.0, 1.0); }";
    let controls = sandbox
        .load(
            "solid",
            files(&[("main.glsl", shader), ("scene.json", SOLID)]),
            no_assets,
            Vec::new(),
        )
        .map_err(|e| anyhow!(e))?;
    assert!(
        controls.contains("\"VALUE\":[0.5]"),
        "the control and its value: {controls}"
    );
    draw(&mut sandbox, 0);
    let half = centre_red(&sandbox).ok_or_else(|| anyhow!("no pixels"))?;
    assert!(
        (120..=135).contains(&half),
        "level 0.5 draws half red: {half}"
    );

    assert!(sandbox.set_control("level", &[1.0]), "the control is found");
    draw(&mut sandbox, 1);
    let full = centre_red(&sandbox).ok_or_else(|| anyhow!("no pixels"))?;
    assert!(full >= 250, "level 1 draws full red: {full}");
    assert!(
        !sandbox.set_control("missing", &[1.0]),
        "an unknown control is refused"
    );
    Ok(())
}

#[test]
fn a_broken_edit_reports_its_line_and_keeps_the_last_scene() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let good = "vec4 renderMain() { return vec4(1.0, 0.0, 0.0, 1.0); }";
    sandbox
        .load("edit", files(&[("main.glsl", good)]), no_assets, Vec::new())
        .map_err(|e| anyhow!(e))?;
    let broken = "vec4 renderMain() {\n  return vec4(oops);\n}";
    let error = sandbox
        .load(
            "edit",
            files(&[("main.glsl", broken)]),
            no_assets,
            Vec::new(),
        )
        .err()
        .ok_or_else(|| anyhow!("a broken shader loaded"))?;
    assert!(
        error.contains("main.glsl:2"),
        "the error names the user's line: {error}"
    );
    let bad_json = sandbox.load(
        "edit",
        files(&[("main.glsl", good), ("scene.json", "{ nope")]),
        no_assets,
        Vec::new(),
    );
    assert!(bad_json.is_err(), "bad JSON is an error, not a crash");
    draw(&mut sandbox, 0);
    assert_eq!(
        centre_red(&sandbox),
        Some(255),
        "the last good scene still draws"
    );
    Ok(())
}

#[test]
fn an_edit_keeps_the_values_of_controls_it_did_not_change() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let shader = "vec4 renderMain() { return vec4(level); }";
    let scene = [("main.glsl", shader), ("scene.json", SOLID)];
    sandbox
        .load("keep", files(&scene), no_assets, Vec::new())
        .map_err(|e| anyhow!(e))?;
    sandbox.set_control("level", &[0.9]);
    let edited = "vec4 renderMain() { return vec4(level * 1.0); }";
    let controls = sandbox
        .load(
            "keep",
            files(&[("main.glsl", edited), ("scene.json", SOLID)]),
            no_assets,
            Vec::new(),
        )
        .map_err(|e| anyhow!(e))?;
    // The value is an f32 widened to f64, so it reads back as 0.8999999761…
    let parsed: serde_json::Value = serde_json::from_str(&controls)?;
    let level = parsed
        .as_array()
        .and_then(|all| all.iter().find(|c| c["NAME"] == "level"))
        .and_then(|c| c["VALUE"][0].as_f64());
    assert!(
        level.is_some_and(|v| (v - 0.9).abs() < 1e-6),
        "the value survived the edit: {controls}"
    );
    Ok(())
}

#[test]
fn script_uniforms_are_declared_at_load_and_set_each_frame() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let extras = vec![(
        "glow".to_string(),
        extra_type(3).ok_or_else(|| anyhow!("vec3"))?,
    )];
    let shader = "vec4 renderMain() { return vec4(glow, 1.0); }";
    sandbox
        .load("script", files(&[("main.glsl", shader)]), no_assets, extras)
        .map_err(|e| anyhow!(e))?;
    assert_eq!(
        sandbox.extras_json(),
        r#"[["glow",3]]"#,
        "the extras the page sees"
    );
    sandbox.set_script_uniforms(&[1.0, 0.0, 0.0, 0.0]);
    draw(&mut sandbox, 0);
    assert_eq!(
        centre_red(&sandbox),
        Some(255),
        "the script's value reached the shader"
    );
    Ok(())
}

#[test]
fn the_audio_sources_move_the_scalars() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    sandbox.set_audio_source(AudioSource::Synthetic);
    draw(&mut sandbox, 30);
    let synthetic: f32 = sandbox.audio_scalars().iter().sum();
    assert!(synthetic > 0.0, "the synthetic track is not silent");

    sandbox.set_audio_source(AudioSource::Live);
    let tone: Vec<f32> = (0..48_000)
        .map(|i| (i as f32 * 110.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5)
        .collect();
    for (frame, chunk) in tone.chunks(800).enumerate() {
        sandbox.push_samples(chunk);
        draw(&mut sandbox, frame as u64);
    }
    let live: f32 = sandbox.audio_scalars().iter().take(4).sum();
    assert!(live > 0.0, "a pushed tone is heard: {live}");
    Ok(())
}

// ---- parity with the app (D554) ------------------------------------------

/// The corpus gate's fixture scenes, which the app's own tests play.
fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../strobe-app/tests/corpus-gate/fixtures")
        .join(name)
}

/// A scene folder's text files as the page holds them, by file name.
fn folder_files(dir: &std::path::Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let text_file = path
            .extension()
            .is_some_and(|ext| ext == "glsl" || ext == "json" || ext == "js");
        if text_file && let Some(name) = path.file_name() {
            out.push((
                name.to_string_lossy().into_owned(),
                std::fs::read_to_string(&path)?,
            ));
        }
    }
    Ok(out)
}

/// The frame the app's own path draws for `dir` at frame 0: the folder
/// loaded from disk (includes expanded, images decoded from their files)
/// and played by `ScenePlayer`, with no audio.
fn app_frame(gl: &strobe_gfx::gl::Gl, dir: &std::path::Path) -> Result<Vec<u8>> {
    let folder = strobe_scene::load::SceneFolder::load(dir)?;
    let caps = strobe_gfx::Caps::from_info(gl.info());
    let mut player =
        strobe_gfx::ScenePlayer::new(gl, &caps, &folder, SIZE, strobe_gfx::Options::default())
            .map_err(|e| anyhow!("{e}"))?;
    let controls = strobe_scene::control::ControlBank::new(folder.def.controls.clone())?;
    let audio: Vec<f32> = strobe_scene::audio::SCALARS
        .iter()
        .map(|name| strobe_scene::audio::silent_value(name))
        .collect();
    player.render(&strobe_gfx::FrameInputs {
        time: 0.0,
        frame: 0,
        mouse: strobe_gfx::Mouse::default(),
        audio: &audio,
        controls: &controls,
        transitions: &[],
    });
    Ok(player.read_output())
}

/// The same scene through the sandbox, from its files and the pictures in
/// `assets`.
fn sandbox_frame(
    sandbox: &mut Sandbox,
    dir: &std::path::Path,
    assets: &[(&str, Vec<u8>)],
) -> Result<Vec<u8>> {
    let texts = folder_files(dir)?;
    let file = |name: &str| {
        texts
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, text)| text.clone())
    };
    let asset = |name: &str| {
        assets
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, bytes)| bytes.clone())
    };
    sandbox.set_audio_source(AudioSource::Silent);
    sandbox
        .load("fixture", file, asset, Vec::new())
        .map_err(|e| anyhow!(e))?;
    draw(sandbox, 0);
    Ok(sandbox.read_output())
}

#[test]
fn an_image_the_page_adds_draws_as_the_scene_folders_file_does() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let dir = fixture("media-image.synScene");
    let app = app_frame(headless.gl(), &dir)?;
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let png = std::fs::read(dir.join("mark.png"))?;
    let with = sandbox_frame(&mut sandbox, &dir, &[("mark.png", png)])?;
    assert_eq!(sandbox.notes_json(), "[]", "nothing is missing");
    assert!(!app.is_empty(), "the app drew a frame");
    assert_eq!(with, app, "the sandbox draws the picture as the app does");

    let without = sandbox_frame(&mut sandbox, &dir, &[])?;
    assert_ne!(without, app, "without the picture the sampler reads black");
    let notes = sandbox.notes_json();
    assert!(
        notes.contains("mark.png") && notes.contains("Add image"),
        "the missing picture is named: {notes}"
    );
    let broken = sandbox_frame(&mut sandbox, &dir, &[("mark.png", vec![1, 2, 3])])?;
    assert_eq!(broken, without, "bytes that are no picture read black too");
    assert!(
        sandbox.notes_json().contains("image `mark`"),
        "and say so: {}",
        sandbox.notes_json()
    );
    Ok(())
}

#[test]
fn an_include_resolves_against_the_scenes_own_files_as_in_the_app() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let dir = fixture("include-file.synScene");
    let app = app_frame(headless.gl(), &dir)?;
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let frame = sandbox_frame(&mut sandbox, &dir, &[])?;
    assert_eq!(frame, app, "the include expanded, the same picture");

    // Without the included file the include is left for the driver to name.
    let main = std::fs::read_to_string(dir.join("main.glsl"))?;
    let error = sandbox
        .load(
            "fixture",
            files(&[("main.glsl", &main)]),
            no_assets,
            Vec::new(),
        )
        .err()
        .ok_or_else(|| anyhow!("a scene missing its include compiled"))?;
    assert!(
        error.contains("main.glsl"),
        "the error is the user's file's: {error}"
    );
    Ok(())
}

#[test]
fn a_script_that_reads_the_picture_gets_the_asynchronous_readback() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let red = "vec4 renderMain() { return vec4(1.0, 0.0, 0.0, 1.0); }";
    let script = "function update() { setUniform('seen', textureFinalPass(0.5, 0.5)); }";
    sandbox
        .load(
            "readback",
            files(&[("main.glsl", red), ("script.js", script)]),
            no_assets,
            Vec::new(),
        )
        .map_err(|e| anyhow!(e))?;
    assert!(
        sandbox.reads_final_pass(),
        "the script asks for the picture"
    );
    let mut landed = None;
    for frame in 0..30 {
        draw(&mut sandbox, frame);
        if let Some(pixels) = sandbox.final_pass() {
            landed = Some(pixels.to_vec());
            break;
        }
    }
    let pixels = landed.ok_or_else(|| anyhow!("no readback landed in 30 frames"))?;
    assert_eq!(pixels.len(), 64 * 36 * 3, "64x36 RGB, as the app's");
    assert!(
        pixels.chunks(3).all(|rgb| rgb == [255, 0, 0]),
        "the picture, read back: {:?}",
        pixels.get(..6)
    );

    sandbox
        .load("plain", files(&[("main.glsl", red)]), no_assets, Vec::new())
        .map_err(|e| anyhow!(e))?;
    for frame in 0..3 {
        draw(&mut sandbox, frame);
    }
    assert!(
        !sandbox.reads_final_pass() && sandbox.final_pass().is_none(),
        "a scene whose script never asks pays nothing"
    );
    Ok(())
}

#[test]
fn a_preset_captures_the_controls_and_brings_them_back() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let shader = "vec4 renderMain() { return vec4(level, 0.0, 0.0, 1.0); }";
    sandbox
        .load(
            "presets",
            files(&[("main.glsl", shader), ("scene.json", SOLID)]),
            no_assets,
            Vec::new(),
        )
        .map_err(|e| anyhow!(e))?;
    sandbox.set_control("level", &[0.25]);
    let text = sandbox.capture_preset("quiet").map_err(|e| anyhow!(e))?;
    let saved = strobe_scene::preset::Preset::from_text(&text)?;
    assert_eq!(saved.name, "quiet", "named: {text}");
    assert!(
        saved.scene.contains_key("level") && !saved.channels.meta,
        "the scene's control, no meta channel: {text}"
    );

    sandbox.set_control("level", &[1.0]);
    let problems = sandbox.apply_preset(&text).map_err(|e| anyhow!(e))?;
    assert!(problems.is_empty(), "everything applied: {problems:?}");
    let controls: serde_json::Value = serde_json::from_str(&sandbox.controls_json())?;
    let level = controls[0]["VALUE"][0].as_f64();
    assert!(
        level.is_some_and(|v| (v - 0.25).abs() < 1e-6),
        "the preset's value is the target again: {controls}"
    );

    let foreign = r#"{"name":"x","scene":{"gone":1.0},"meta":{"brightness":0.2}}"#;
    let problems = sandbox.apply_preset(foreign).map_err(|e| anyhow!(e))?;
    assert_eq!(
        problems.len(),
        1,
        "a control the scene lacks is reported; meta is not the sandbox's: {problems:?}"
    );
    assert!(
        sandbox.capture_preset("../escape").is_err(),
        "a name that is no file name is refused"
    );
    assert!(
        sandbox.apply_preset("{ nope").is_err(),
        "bad JSON is an error"
    );
    Ok(())
}

/// A scene with numbers worth a control and none yet.
const BARS: &str = "vec4 renderMain() {\n    \
    float w = sin(_uv.x * 12.0 + TIME * 2.0);\n    \
    vec3 c = mix(vec3(0.9, 0.2, 0.1), vec3(0.1, 0.3, 0.9), 0.5 + 0.5 * w);\n    \
    return vec4(c, 1.0);\n}\n";

/// Runs a generation to its end, a step at a time as the page does.
fn generate_steps(
    sandbox: &mut Sandbox,
    pairs: &[(&str, &str)],
    between: &mut dyn FnMut(&mut Sandbox),
) -> Result<Value> {
    sandbox
        .generate_start("scene", files(pairs), no_assets)
        .map_err(|e| anyhow!(e))?;
    for _ in 0..50_000_000_u64 {
        let answer: Value =
            serde_json::from_str(&sandbox.generate_step().map_err(|e| anyhow!(e))?)?;
        if answer["state"] == "done" {
            return Ok(answer);
        }
        between(sandbox);
    }
    bail!("the generation never finished")
}

/// What the review sends when a person changes nothing.
fn all_kept(review: &Value) -> Value {
    let kept = review["kept"].as_array().cloned().unwrap_or_default();
    Value::Array(
        kept.iter()
            .map(|control| {
                json!({
                    "candidate": control["candidate"],
                    "name": control["name"],
                    "context": control["context"],
                    "min": control["min"][0],
                    "max": control["max"][0],
                })
            })
            .collect(),
    )
}

fn names(review: &Value) -> Vec<String> {
    review["kept"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|control| control["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn the_sandbox_proposes_and_writes_what_the_apps_engine_does() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let pairs = [
        ("main.glsl", BARS),
        ("scene.json", "{ \"TITLE\": \"Bars\" }\n"),
    ];
    let folder = SceneFolder::from_files("scene", files(&pairs))?;
    let app = generate(gl, &caps, &folder, &GenerateOptions::default())?;
    assert!(!app.report.kept.is_empty(), "this scene gets controls");
    let wanted = rewritten_files(&app, Some("{ \"TITLE\": \"Bars\" }\n"))?;

    let mut sandbox = Sandbox::new(gl.clone(), SIZE, 48_000);
    let review = generate_steps(&mut sandbox, &pairs, &mut |_| {})?;
    let mut listed = names(&review);
    listed.sort();
    let mut kept = app.report.kept.clone();
    kept.sort();
    assert_eq!(listed, kept, "the same controls");
    assert!(
        review["fidelity"].as_f64().is_some_and(|f| f < 0.001),
        "it says how faithful they are: {review}"
    );

    let text = sandbox
        .generate_apply("scene", files(&pairs), &all_kept(&review).to_string())
        .map_err(|e| anyhow!(e))?;
    let written: Value = serde_json::from_str(&text)?;
    for (name, content) in &wanted {
        assert_eq!(
            written[name.as_str()].as_str(),
            Some(content.as_str()),
            "{name} is byte for byte what the app's copy holds"
        );
    }
    Ok(())
}

#[test]
fn a_scene_folders_picture_and_the_pages_are_measured_alike() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let dir = fixture("media-image.synScene");
    let app = generate(
        gl,
        &caps,
        &SceneFolder::load(&dir)?,
        &GenerateOptions::default(),
    )?;
    let main = std::fs::read_to_string(dir.join("main.glsl"))?;
    let json = std::fs::read_to_string(dir.join("scene.json"))?;
    let mark = std::fs::read(dir.join("mark.png"))?;
    let pairs = [("main.glsl", main.as_str()), ("scene.json", json.as_str())];

    let mut sandbox = Sandbox::new(gl.clone(), SIZE, 48_000);
    sandbox
        .generate_start("media-image", files(&pairs), |name: &str| {
            (name == "mark.png").then(|| mark.clone())
        })
        .map_err(|e| anyhow!(e))?;
    let review = loop {
        let answer: Value =
            serde_json::from_str(&sandbox.generate_step().map_err(|e| anyhow!(e))?)?;
        if answer["state"] == "done" {
            break answer;
        }
    };
    let mut listed = names(&review);
    listed.sort();
    let mut kept = app.report.kept.clone();
    kept.sort();
    assert_eq!(listed, kept, "the picture was there for both: {review}");
    Ok(())
}

#[test]
fn generating_between_frames_leaves_the_live_scene_as_it_would_have_been() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let gl = headless.gl();
    // Feedback: a live pass whose targets or bindings were disturbed would
    // carry the damage from frame to frame.
    let dir = fixture("feedback.synScene");
    let main = std::fs::read_to_string(dir.join("main.glsl"))?;
    let json = std::fs::read_to_string(dir.join("scene.json"))?;
    let live = [("main.glsl", main.as_str()), ("scene.json", json.as_str())];

    let play = |generating: bool| -> Result<Vec<u8>> {
        let mut sandbox = Sandbox::new(gl.clone(), SIZE, 48_000);
        sandbox
            .load("feedback", files(&live), no_assets, Vec::new())
            .map_err(|e| anyhow!(e))?;
        if generating {
            sandbox
                .generate_start("bars", files(&[("main.glsl", BARS)]), no_assets)
                .map_err(|e| anyhow!(e))?;
        }
        for frame in 0..40 {
            draw(&mut sandbox, frame);
            if sandbox.generate_running() {
                sandbox.generate_step().map_err(|e| anyhow!(e))?;
            }
        }
        Ok(sandbox.read_output())
    };
    let alone = play(false)?;
    let beside = play(true)?;
    assert!(alone.iter().any(|byte| *byte != 0), "the scene draws");
    assert_eq!(alone, beside, "the same picture, byte for byte");
    Ok(())
}

#[test]
fn cancelling_leaves_the_scene_as_it_was_and_frees_what_was_made() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let gl = headless.gl();
    let mut sandbox = Sandbox::new(gl.clone(), SIZE, 48_000);
    let pairs = [("main.glsl", BARS)];
    sandbox
        .load("bars", files(&pairs), no_assets, Vec::new())
        .map_err(|e| anyhow!(e))?;
    draw(&mut sandbox, 0);
    let controls = sandbox.controls_json();
    let objects = gl.live_objects();

    sandbox
        .generate_start("bars", files(&pairs), no_assets)
        .map_err(|e| anyhow!(e))?;
    for _ in 0..12 {
        sandbox.generate_step().map_err(|e| anyhow!(e))?;
    }
    assert!(
        sandbox.generate_running(),
        "still going after a dozen steps"
    );
    assert!(
        gl.live_objects() > objects,
        "the run holds a compiled scene while it goes"
    );
    sandbox.generate_cancel();
    assert!(!sandbox.generate_running(), "stopped");
    assert_eq!(gl.live_objects(), objects, "everything it made is gone");
    assert_eq!(
        sandbox.controls_json(),
        controls,
        "the controls are as they were"
    );
    assert!(
        sandbox.generate_step().is_err(),
        "a step with nothing running is refused, not a panic"
    );
    draw(&mut sandbox, 1);
    assert!(
        sandbox.read_output().iter().any(|byte| *byte != 0),
        "the scene plays on"
    );
    Ok(())
}

#[test]
fn a_scene_with_controls_keeps_them_and_gets_new_ones_beside_them() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let shader = "vec4 renderMain() {\n    \
        float w = sin(_uv.x * 12.0 + TIME * 2.0);\n    \
        return vec4(vec3(0.9, 0.2, 0.1) * level * (0.5 + 0.5 * w), 1.0);\n}\n";
    let pairs = [("main.glsl", shader), ("scene.json", SOLID)];
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    sandbox
        .load("mine", files(&pairs), no_assets, Vec::new())
        .map_err(|e| anyhow!(e))?;
    assert_eq!(sandbox.authored_controls(), 1, "a person wrote this one");

    let review = generate_steps(&mut sandbox, &pairs, &mut |_| {})?;
    assert!(!names(&review).is_empty(), "new controls are proposed");
    assert!(
        !names(&review).contains(&"level".to_string()),
        "under new names: {review}"
    );
    let text = sandbox
        .generate_apply("mine", files(&pairs), &all_kept(&review).to_string())
        .map_err(|e| anyhow!(e))?;
    let written: Value = serde_json::from_str(&text)?;
    let json = written["scene.json"].as_str().unwrap_or("");
    let main = written["main.glsl"].as_str().unwrap_or("");
    assert!(
        json.contains("\"NAME\":\"level\""),
        "the old one stays: {json}"
    );
    let controls = sandbox
        .load(
            "mine",
            files(&[("main.glsl", main), ("scene.json", json)]),
            no_assets,
            Vec::new(),
        )
        .map_err(|e| anyhow!(e))?;
    assert!(
        controls.contains("level"),
        "it compiles with both: {controls}"
    );
    for name in names(&review) {
        assert!(
            controls.contains(&name),
            "{name} is in the panel: {controls}"
        );
    }
    Ok(())
}

#[test]
fn a_scene_that_does_not_compile_is_said_not_to_and_nothing_runs_on() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let pairs = [(
        "main.glsl",
        "vec4 renderMain() {\n    return vec4(notDeclared * 2.0, 1.0);\n}\n",
    )];
    sandbox
        .generate_start("broken", files(&pairs), no_assets)
        .map_err(|e| anyhow!(e))?;
    let mut failure = None;
    for _ in 0..50_000_000_u64 {
        match sandbox.generate_step() {
            Ok(_) => {}
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    let failure = failure.ok_or_else(|| anyhow!("it should have failed"))?;
    assert!(
        failure.contains("does not compile") && failure.contains("main.glsl:2"),
        "it says so, on the person's line: {failure}"
    );
    assert!(!sandbox.generate_running(), "the run is over");

    assert!(
        sandbox
            .generate_start(
                "bad",
                files(&[("main.glsl", BARS), ("scene.json", "{ nope")]),
                no_assets
            )
            .is_err(),
        "a scene.json that does not parse cannot start"
    );
    Ok(())
}

#[test]
fn a_scene_where_nothing_matters_proposes_nothing() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    // The number is real and never reaches the picture.
    let shader = "vec4 renderMain() {\n    \
        float unused = sin(_uv.x * 7.0);\n    \
        return vec4(0.0, 0.0, 0.0, 1.0 + unused * 0.0);\n}\n";
    let review = generate_steps(&mut sandbox, &[("main.glsl", shader)], &mut |_| {})?;
    assert!(names(&review).is_empty(), "nothing to offer: {review}");
    assert!(
        review["kept"].as_array().is_some_and(Vec::is_empty),
        "an empty list, not a missing one: {review}"
    );
    Ok(())
}

#[test]
fn a_script_scene_is_measured_with_its_uniforms_declared() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let shader = "vec4 renderMain() {\n    \
        float w = sin(_uv.x * 12.0 + pulse);\n    \
        return vec4(vec3(0.9, 0.2, 0.1) * (0.5 + 0.5 * w), 1.0);\n}\n";
    let script = "function update(dt) {\n  setUniform(\"pulse\", TIME);\n}\n";
    let pairs = [("main.glsl", shader), ("script.js", script)];
    let review = generate_steps(&mut sandbox, &pairs, &mut |_| {})?;
    assert!(
        !names(&review).is_empty(),
        "the shader compiled without its script running: {review}"
    );
    Ok(())
}

#[test]
fn a_review_drops_renames_and_re_ranges_and_refuses_a_changed_scene() -> Result<()> {
    let Some(headless) = headless()? else {
        return Ok(());
    };
    let mut sandbox = Sandbox::new(headless.gl().clone(), SIZE, 48_000);
    let pairs = [("main.glsl", BARS)];
    let review = generate_steps(&mut sandbox, &pairs, &mut |_| {})?;
    let kept = review["kept"].as_array().cloned().unwrap_or_default();
    let slider = kept
        .iter()
        .find(|control| control["default"].as_array().is_some_and(|d| d.len() == 1))
        .ok_or_else(|| anyhow!("a slider among {review}"))?;
    let choice = json!([{
        "candidate": slider["candidate"],
        "name": "wave_amount",
        "context": slider["context"],
        "min": 0.5,
        "max": 40.0,
    }]);
    let text = sandbox
        .generate_apply("scene", files(&pairs), &choice.to_string())
        .map_err(|e| anyhow!(e))?;
    let written: Value = serde_json::from_str(&text)?;
    let json = written["scene.json"].as_str().unwrap_or("");
    let main = written["main.glsl"].as_str().unwrap_or("");
    assert!(json.contains("wave_amount"), "the new name: {json}");
    assert!(json.contains("40"), "the range chosen: {json}");
    assert_eq!(
        json.matches("\"NAME\"").count(),
        1,
        "only the one kept: {json}"
    );
    assert!(main.contains("wave_amount"), "the shader reads it: {main}");
    assert_eq!(main.lines().count(), BARS.lines().count(), "no line moved");
    sandbox
        .load(
            "scene",
            files(&[("main.glsl", main), ("scene.json", json)]),
            no_assets,
            Vec::new(),
        )
        .map_err(|e| anyhow!(e))?;

    let bad_name = json!([{
        "candidate": slider["candidate"],
        "name": "not a name",
        "context": slider["context"],
        "min": 0.0,
        "max": 1.0,
    }]);
    assert!(
        sandbox
            .generate_apply("scene", files(&pairs), &bad_name.to_string())
            .is_err_and(|e| e.contains("cannot be a control's name")),
        "a name the scene cannot declare is refused"
    );
    let changed = "vec4 renderMain() {\n    return vec4(1.0);\n}\n";
    assert!(
        sandbox
            .generate_apply(
                "scene",
                files(&[("main.glsl", changed)]),
                &choice.to_string()
            )
            .is_err_and(|e| e.contains("generate them again")),
        "a scene changed since it was measured is refused"
    );
    Ok(())
}
