// ============================================================================
// Module:       generate (integration test)
// Description:  "Generate controls" end to end: the real `strobe` executable
//               started as a worker on a scene with no controls, writing a
//               new scene and a result the app reads, with the source left
//               byte for byte as it was.
//
// Dependencies: anyhow; strobe-app (generate); strobe-gfx (headless context)
// ============================================================================

//! The worker the app starts (D202), run the way the app runs it.
//!
//! Without EGL this reports a skip and passes, as the renderer's own tests
//! do, unless `STROBE_REQUIRE_HEADLESS_GL` is set (CI's Linux job).

use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use strobe_app::generate::{Job, Outcome, discard};
use strobe_gfx::{Api, Headless};

/// A shader whose character is all literal numbers: a speed, a colour, a
/// frequency and a scale.
const SHADER: &str = "vec4 renderMain() {\n\
    vec2 uv = _uv * 3.0;\n\
    float t = TIME * 0.5;\n\
    vec3 tint = vec3(0.8, 0.3, 0.1);\n\
    float wave = 0.5 + 0.5 * sin(uv.x * 6.0 + t);\n\
    return vec4(tint * wave, 1.0);\n\
}\n";

/// A library folder in the temp directory, removed on drop.
struct Library(PathBuf);

impl Library {
    fn new(name: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "strobe-generate-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Whether a headless desktop context can be had, skipping or failing as the
/// environment says.
fn have_context() -> Result<bool> {
    match Headless::new(Api::DesktopCore33) {
        Ok(_) => Ok(true),
        Err(error) if std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_none() => {
            strobe_gfx::skip::note(format_args!("no headless context ({error})"));
            Ok(false)
        }
        Err(error) => Err(anyhow!("headless context required: {error}")),
    }
}

/// Waits for `job` the way the app does, by polling.
fn finish(mut job: Job) -> Result<Outcome> {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if let Some(outcome) = job.poll() {
            return Ok(outcome);
        }
        if Instant::now() > deadline {
            job.stop();
            bail!("the worker took more than five minutes");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn snapshot(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        files.push((name, std::fs::read(&path)?));
    }
    files.sort();
    Ok(files)
}

#[test]
fn the_worker_writes_a_new_scene_and_leaves_the_source_alone() -> Result<()> {
    if !have_context()? {
        return Ok(());
    }
    let library = Library::new("literal")?;
    let source = library.0.join("literal.synScene");
    std::fs::create_dir_all(&source)?;
    std::fs::write(source.join("scene.json"), "{ \"TITLE\": \"Literal\" }")?;
    std::fs::write(source.join("main.glsl"), SHADER)?;
    std::fs::write(source.join("LICENSE.txt"), "the scene's own licence")?;
    let before = snapshot(&source)?;

    let exe = PathBuf::from(env!("CARGO_BIN_EXE_strobe"));
    let job = Job::spawn(&exe, &source, &library.0, false)?;
    let Outcome::Done(summary) = finish(job)? else {
        bail!("the worker failed");
    };

    assert_eq!(
        snapshot(&source)?,
        before,
        "the source scene is byte for byte as it was"
    );
    assert!(
        !summary.kept.is_empty(),
        "the literals became controls; found {} candidates, kept none",
        summary.candidates
    );
    assert!(
        summary.fidelity.is_some_and(|f| f < 0.01),
        "the defaults reproduce the original picture; fidelity {:?}",
        summary.fidelity
    );
    let written = summary
        .written
        .clone()
        .ok_or_else(|| anyhow!("no scene was written"))?;
    assert_eq!(
        written.file_name().and_then(|n| n.to_str()),
        Some("literal-auto.synScene"),
        "named beside the original"
    );
    let json = std::fs::read_to_string(written.join("scene.json"))?;
    for kept in &summary.kept {
        assert!(
            json.contains(&format!("\"{}\"", kept.name)),
            "{} is in the new scene.json",
            kept.name
        );
    }
    assert!(
        written.join("LICENSE.txt").is_file(),
        "the licence travels with the copy"
    );

    discard(&library.0, &written)?;
    assert!(
        !written.exists() && source.is_dir(),
        "discarding removes the copy only"
    );
    Ok(())
}

#[test]
fn a_scene_with_its_own_controls_is_refused_unless_adding() -> Result<()> {
    if !have_context()? {
        return Ok(());
    }
    let library = Library::new("authored")?;
    let source = library.0.join("authored.synScene");
    std::fs::create_dir_all(&source)?;
    std::fs::write(
        source.join("scene.json"),
        r#"{ "CONTROLS": [ { "NAME": "level", "TYPE": "slider", "DEFAULT": 0.5 } ] }"#,
    )?;
    std::fs::write(
        source.join("main.glsl"),
        SHADER.replace("vec4(tint * wave, 1.0)", "vec4(tint * wave * level, 1.0)"),
    )?;
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_strobe"));
    let refused = finish(Job::spawn(&exe, &source, &library.0, false)?)?;
    let Outcome::Failed { message } = refused else {
        bail!("a scene with authored controls was rewritten without asking");
    };
    assert!(
        message.contains("control(s) of its own"),
        "the refusal says why: {message}"
    );
    let added = finish(Job::spawn(&exe, &source, &library.0, true)?)?;
    let Outcome::Done(summary) = added else {
        bail!("adding to authored controls failed");
    };
    assert!(
        summary.kept.iter().all(|k| k.name != "level"),
        "the authored control is kept, not regenerated"
    );
    Ok(())
}
