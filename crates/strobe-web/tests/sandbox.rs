// ============================================================================
// Module:       sandbox (integration test)
// Description:  Drives the browser sandbox's engine on the headless GLES 3.0
//               context, the dialect WebGL2 speaks.
//
// Dependencies: anyhow; strobe-web (sandbox), strobe-gfx (headless context)
// ============================================================================

//! The sandbox's behaviour the /play page depends on, without a browser.
//!
//! WebGL2 is GLSL ES 3.00, so the headless GLES 3.0 context compiles what
//! the browser compiles. Where no headless EGL exists the tests report a
//! skip and pass, unless `STROBE_REQUIRE_HEADLESS_GL` is set (CI's Linux
//! job), as the renderer tests do.

#![cfg(not(target_arch = "wasm32"))]

use anyhow::{Result, anyhow};
use strobe_gfx::{Api, Headless};
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
        .load("edit", files(&[("main.glsl", good)]), Vec::new())
        .map_err(|e| anyhow!(e))?;
    let broken = "vec4 renderMain() {\n  return vec4(oops);\n}";
    let error = sandbox
        .load("edit", files(&[("main.glsl", broken)]), Vec::new())
        .err()
        .ok_or_else(|| anyhow!("a broken shader loaded"))?;
    assert!(
        error.contains("main.glsl:2"),
        "the error names the user's line: {error}"
    );
    let bad_json = sandbox.load(
        "edit",
        files(&[("main.glsl", good), ("scene.json", "{ nope")]),
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
        .load("keep", files(&scene), Vec::new())
        .map_err(|e| anyhow!(e))?;
    sandbox.set_control("level", &[0.9]);
    let edited = "vec4 renderMain() { return vec4(level * 1.0); }";
    let controls = sandbox
        .load(
            "keep",
            files(&[("main.glsl", edited), ("scene.json", SOLID)]),
            Vec::new(),
        )
        .map_err(|e| anyhow!(e))?;
    assert!(
        controls.contains("\"VALUE\":[0.9"),
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
        .load("script", files(&[("main.glsl", shader)]), extras)
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
