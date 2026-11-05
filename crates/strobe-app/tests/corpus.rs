// ============================================================================
// Module:       corpus (integration test)
// Description:  The corpus harness over synthetic scenes written to a temp
//               folder: one that works, one that does not compile, one whose
//               uniform only exists because its script ran, a bad
//               scene.json, a script that throws, and a blank frame. In
//               process, through worker processes, and through the CLI,
//               on a headless context in both dialects. Also the Shadertoy
//               converter's fixtures, converted and run the same way.
//
// Dependencies: anyhow; strobe-app (corpus); strobe-gfx (headless context);
//               strobe-shadertoy (the fixtures' conversion)
// ============================================================================

//! The harness end to end.
//!
//! Every scene here is written by this file; nothing comes from a real
//! corpus. Without EGL these tests report a skip and pass, as the
//! renderer's own tests do, unless `STROBE_REQUIRE_HEADLESS_GL` is set
//! (CI's Linux job).

use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};
use std::time::Duration;
use strobe_app::corpus::cli::ContextArg;
use strobe_app::corpus::driver::{self, Options};
use strobe_app::corpus::report::Report;
use strobe_app::corpus::run::run_scene;
use strobe_app::corpus::{Dialect, Record, Settings, Status, capture};
use strobe_gfx::{Api, Caps, Headless};

/// A library folder in the temp directory, removed on drop.
struct Library(PathBuf);

impl Library {
    fn new(name: &str) -> Result<Self> {
        let dir =
            std::env::temp_dir().join(format!("strobe-corpus-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    /// Adds a scene folder named `<name>.synScene`.
    fn scene(&self, name: &str, json: &str, shader: &str, script: Option<&str>) -> Result<PathBuf> {
        let dir = self.0.join(format!("{name}.synScene"));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("scene.json"), json)?;
        std::fs::write(dir.join("main.glsl"), shader)?;
        if let Some(script) = script {
            std::fs::write(dir.join("script.js"), script)?;
        }
        Ok(dir)
    }

    /// The six synthetic scenes the tests share.
    fn populate(&self) -> Result<()> {
        self.scene(
            "ok",
            "{}",
            "vec4 renderMain() { return vec4(0.2, 0.4, 0.6, 1.0); }",
            None,
        )?;
        self.scene(
            "broken-shader",
            "{}",
            "vec4 renderMain() { return vec4(nothingDeclaredHere, 0.0, 0.0, 1.0); }",
            None,
        )?;
        self.scene(
            "script-uniform",
            "{}",
            "vec4 renderMain() { return vec4(tint, 1.0); }",
            Some("function update(dt) { setUniform('tint', 0.5, 0.25, 1.0); }"),
        )?;
        self.scene(
            "bad-json",
            "{ this is not json",
            "vec4 renderMain() { return vec4(1.0); }",
            None,
        )?;
        self.scene(
            "script-throws",
            "{}",
            "vec4 renderMain() { return vec4(0.1, 0.2, 0.3, 1.0); }",
            Some("function setup() { throw new Error('refusing to start'); }"),
        )?;
        self.scene(
            "blank",
            "{}",
            "vec4 renderMain() { return vec4(0.0, 0.0, 0.0, 1.0); }",
            None,
        )?;
        Ok(())
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Held by every test that runs scenes in this process. `capture` reads
/// load failures back from one process-wide logger, so two such tests at
/// once would each see the other's script errors (WSL2, 2026-11-05).
static IN_PROCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Takes [`IN_PROCESS`]; a test that panicked holding it leaves nothing to
/// repair, so a poisoned lock is taken as it is.
fn in_process() -> std::sync::MutexGuard<'static, ()> {
    IN_PROCESS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn require_gl() -> bool {
    std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_some()
}

/// Whether a headless context of `api` can be had, skipping or failing as
/// the environment says.
fn have_context(api: Api) -> Result<bool> {
    match Headless::new(api) {
        Ok(_) => Ok(true),
        Err(error) if !require_gl() => {
            strobe_gfx::skip::note(format_args!("no headless {api:?} context ({error})"));
            Ok(false)
        }
        Err(error) => Err(anyhow!("headless {api:?} context required: {error}")),
    }
}

fn find<'a>(records: &'a [Record], scene: &str, dialect: Dialect) -> Result<&'a Record> {
    records
        .iter()
        .find(|r| r.scene == scene && r.dialect == dialect)
        .ok_or_else(|| anyhow!("no record for {scene} in {}", dialect.label()))
}

fn worker_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_strobe-corpus"))
}

fn options(library: &Path, work: &Path) -> Options {
    Options {
        exe: worker_exe(),
        library: library.to_path_buf(),
        filter: None,
        settings: Settings {
            frames: 2,
            size: [32, 18],
        },
        dialects: Dialect::ALL.to_vec(),
        budget: Duration::from_secs(120),
        context: ContextArg::Auto,
        show_window: false,
        gl: None,
        work: work.to_path_buf(),
        frames: None,
    }
}

/// Everything the harness must say about the six scenes, in one dialect.
fn check_records(records: &[Record], dialect: Dialect) -> Result<()> {
    let label = dialect.label();

    let ok = find(records, "ok", dialect)?;
    assert_eq!(ok.status, Status::Ok, "{label}: ok scene: {:?}", ok.error);
    assert!(ok.compile_ms.is_some(), "{label}: compile time recorded");
    assert!(ok.frame_ms.is_some(), "{label}: frame time recorded");
    assert_eq!(ok.all_black, Some(false), "{label}: not black");
    assert_eq!(ok.all_white, Some(false), "{label}: not white");
    assert!(ok.checksum.is_some(), "{label}: checksum recorded");

    let broken = find(records, "broken-shader", dialect)?;
    assert_eq!(broken.status, Status::CompileError, "{label}: {broken:?}");
    assert_eq!(
        broken.cause.as_deref(),
        Some("undeclared:nothingDeclaredHere"),
        "{label}: the cause names the identifier ({:?})",
        broken.error
    );
    assert!(
        broken.error.as_deref().is_some_and(|e| !e.contains('\n')),
        "{label}: the error is one line"
    );

    let scripted = find(records, "script-uniform", dialect)?;
    assert_eq!(
        scripted.status,
        Status::Ok,
        "{label}: `tint` compiles only because the script ran first: {:?}",
        scripted.error
    );
    assert_eq!(scripted.script_uniforms, 1, "{label}: one uniform declared");
    assert!(scripted.has_script, "{label}: a script was found");

    let bad = find(records, "bad-json", dialect)?;
    assert_eq!(bad.status, Status::ParseError, "{label}: {bad:?}");
    assert_eq!(bad.cause.as_deref(), Some("json"), "{label}: json");

    let throws = find(records, "script-throws", dialect)?;
    assert_eq!(throws.status, Status::ScriptError, "{label}: {throws:?}");
    assert!(
        throws
            .error
            .as_deref()
            .is_some_and(|e| e.contains("refusing to start")),
        "{label}: the script's own message: {:?}",
        throws.error
    );

    let blank = find(records, "blank", dialect)?;
    assert_eq!(
        blank.status,
        Status::Ok,
        "{label}: a black frame still compiles"
    );
    assert_eq!(blank.all_black, Some(true), "{label}: and is flagged black");
    Ok(())
}

#[test]
fn six_synthetic_scenes_in_process_in_both_dialects() -> Result<()> {
    let _serial = in_process();
    let library = Library::new("inprocess")?;
    library.populate()?;
    capture::install();
    // The same folder minus its script: the control that shows the script
    // is what makes the uniform exist.
    let without_script = library.scene(
        "script-uniform-no-script",
        "{}",
        "vec4 renderMain() { return vec4(tint, 1.0); }",
        None,
    )?;
    let settings = Settings {
        frames: 2,
        size: [32, 18],
    };
    for dialect in Dialect::ALL {
        if !have_context(dialect.api())? {
            continue;
        }
        let headless =
            Headless::new(dialect.api()).map_err(|e| anyhow!("{dialect:?}: context: {e}"))?;
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut records = Vec::new();
        for entry in strobe_scene::load::scan(&library.0)? {
            records.push(run_scene(gl, &caps, &entry, dialect, &settings));
        }
        check_records(&records, dialect)?;

        let control = find(&records, "script-uniform-no-script", dialect)?;
        assert_eq!(
            control.status,
            Status::CompileError,
            "{}: without the script `tint` is undeclared",
            dialect.label()
        );
        assert_eq!(
            control.cause.as_deref(),
            Some("undeclared:tint"),
            "{}: and the report says so",
            dialect.label()
        );
        let _ = without_script;
    }
    Ok(())
}

#[test]
fn six_synthetic_scenes_through_worker_processes() -> Result<()> {
    if !have_context(Api::DesktopCore33)? || !have_context(Api::Gles30)? {
        return Ok(());
    }
    let library = Library::new("workers")?;
    library.populate()?;
    let work = library.0.join(".work");
    let outcome = driver::run_library(&options(&library.0, &work), |_| {})?;
    assert_eq!(outcome.records.len(), 12, "six scenes, two dialects");
    for dialect in Dialect::ALL {
        check_records(&outcome.records, dialect)?;
    }
    assert_eq!(outcome.drivers.len(), 2, "a driver line per dialect");
    assert!(
        outcome
            .drivers
            .iter()
            .all(|d| !d.renderer.is_empty() && d.renderer != "unavailable"),
        "the driver is recorded: {:?}",
        outcome.drivers
    );
    assert!(!work.exists(), "the work folder is removed");

    let report = Report::new(
        "synthetic",
        options(&library.0, &work).settings,
        120,
        outcome.drivers,
        outcome.records,
    );
    let markdown = report.to_markdown();
    for needle in [
        "## Totals",
        "undeclared:nothingDeclaredHere",
        "json",
        "script_error",
    ] {
        assert!(
            markdown.contains(needle),
            "report.md mentions {needle}:\n{markdown}"
        );
    }
    Ok(())
}

#[test]
fn a_worker_that_overruns_its_budget_is_a_timeout_and_the_run_goes_on() -> Result<()> {
    if !have_context(Api::DesktopCore33)? {
        return Ok(());
    }
    let library = Library::new("timeout")?;
    library.scene("a", "{}", "vec4 renderMain() { return vec4(1.0); }", None)?;
    library.scene("b", "{}", "vec4 renderMain() { return vec4(1.0); }", None)?;
    let mut opts = options(&library.0, &library.0.join(".work"));
    opts.dialects = vec![Dialect::Desktop];
    // No worker can open a GL context in a millisecond.
    opts.budget = Duration::from_millis(1);
    let outcome = driver::run_library(&opts, |_| {})?;
    assert_eq!(outcome.records.len(), 2, "both scenes got a record");
    assert!(
        outcome.records.iter().all(|r| r.status == Status::Timeout),
        "both timed out: {:?}",
        outcome.records
    );
    Ok(())
}

#[test]
fn a_worker_that_cannot_start_is_a_gl_error_not_the_end_of_the_run() -> Result<()> {
    let library = Library::new("nostart")?;
    library.scene("a", "{}", "vec4 renderMain() { return vec4(1.0); }", None)?;
    library.scene("b", "{}", "vec4 renderMain() { return vec4(1.0); }", None)?;
    let mut opts = options(&library.0, &library.0.join(".work"));
    opts.exe = library.0.join("no-such-program");
    opts.dialects = vec![Dialect::Gles];
    let outcome = driver::run_library(&opts, |_| {})?;
    assert_eq!(outcome.records.len(), 2, "both scenes got a record");
    assert!(
        outcome.records.iter().all(|r| r.status == Status::GlError),
        "{:?}",
        outcome.records
    );
    Ok(())
}

#[test]
fn an_empty_library_is_an_error_naming_the_folder() -> Result<()> {
    let library = Library::new("empty")?;
    let result = driver::run_library(&options(&library.0, &library.0.join(".work")), |_| {});
    let error = result
        .err()
        .ok_or_else(|| anyhow!("an empty library succeeded"))?;
    assert!(
        error.to_string().contains("no scene folders"),
        "the message says what is missing: {error}"
    );
    Ok(())
}

#[test]
fn the_command_line_writes_both_files_and_json_only_writes_one() -> Result<()> {
    if !have_context(Api::DesktopCore33)? {
        return Ok(());
    }
    let library = Library::new("cli")?;
    library.populate()?;
    let out = library.0.join("out");
    let run = |extra: &[&str]| -> Result<()> {
        let status = std::process::Command::new(worker_exe())
            .arg("--library")
            .arg(&library.0)
            .arg("--out")
            .arg(&out)
            .args(["--frames", "2", "--size", "32x18", "--dialect", "desktop"])
            .args(extra)
            .stdout(std::process::Stdio::null())
            .status()?;
        assert!(status.success(), "the run completes whatever the scenes do");
        Ok(())
    };

    run(&["--filter", "blank"])?;
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json"))?)?;
    let records = json
        .get("records")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("report.json has no records array"))?;
    assert_eq!(
        records.len(),
        1,
        "--filter blank keeps one scene, one dialect"
    );
    assert!(out.join("report.md").is_file(), "markdown by default");

    std::fs::remove_file(out.join("report.md"))?;
    run(&["--json-only"])?;
    assert!(
        !out.join("report.md").exists(),
        "--json-only writes no markdown"
    );
    assert!(out.join("report.json").is_file(), "but the JSON");
    Ok(())
}

/// The Shadertoy converter's own fixtures, converted as an import converts
/// them and run through the harness the way a scene library is (G-01): each
/// gets a record in both dialects, so an import that compiles in the
/// converter's tests but not in the app's loader shows up here as a
/// categorised failure rather than as nothing.
#[test]
fn shadertoy_fixtures_convert_and_pass_the_harness_in_both_dialects() -> Result<()> {
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../strobe-shadertoy/tests/fixtures");
    let _serial = in_process();
    let library = Library::new("shadertoy")?;
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&fixtures)? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let shader = strobe_shadertoy::Shader::from_json(&text)
            .map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let converted = strobe_shadertoy::convert(&shader, &strobe_shadertoy::Options::default())
            .map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let dir = strobe_shadertoy::convert::write(&converted, &library.0, true)
            .map_err(|e| anyhow!("{}: {e}", path.display()))?;
        names.push(strobe_scene::load::folder_title(&dir));
    }
    assert!(
        names.len() >= 9,
        "every fixture converted and was written: {names:?}"
    );
    capture::install();
    let settings = Settings {
        frames: 2,
        size: [32, 18],
    };
    for dialect in Dialect::ALL {
        if !have_context(dialect.api())? {
            continue;
        }
        let headless =
            Headless::new(dialect.api()).map_err(|e| anyhow!("{dialect:?}: context: {e}"))?;
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let mut records = Vec::new();
        for entry in strobe_scene::load::scan(&library.0)? {
            records.push(run_scene(gl, &caps, &entry, dialect, &settings));
        }
        assert_eq!(
            records.len(),
            names.len(),
            "{}: one record per converted fixture",
            dialect.label()
        );
        for record in &records {
            assert_eq!(
                record.status,
                Status::Ok,
                "{}: {} ({:?}: {:?})",
                dialect.label(),
                record.scene,
                record.cause,
                record.error
            );
            assert!(
                record.checksum.is_some() && record.frame_ms.is_some(),
                "{}: {} has a frame and its time",
                dialect.label(),
                record.scene
            );
        }
    }
    Ok(())
}
