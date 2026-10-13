// ============================================================================
// Module:       engine (integration test)
// Description:  Runs the whole engine on headless GL in both dialects: every
//               Shadertoy fixture through convert then autocontrol, and every
//               library scene with its CONTROLS stripped.
//
// Dependencies: anyhow; strobe-autocontrol, strobe-shadertoy, strobe-gfx, strobe-scene
// ============================================================================

//! End-to-end tests for the auto-generated controls engine.
//!
//! Each test opens a headless EGL context (Mesa llvmpipe on a CI runner).
//! Without one the tests report a skip and pass, unless
//! `STROBE_REQUIRE_HEADLESS_GL` is set, which CI's Linux job does.
//!
//! What is pinned here:
//!
//! - **Compiles in both dialects.** `generate` compiles the original and the
//!   finished scene in the context's dialect, so success on a desktop 3.3
//!   core context and on a GLES 3.0 context is the proof.
//! - **Something is kept, or the reason is documented.** [`NO_CONTROLS`]
//!   lists the fixtures that have nothing tunable in them and says why.
//! - **Every kept control visibly changes the image**, by its measured score
//!   and again independently on the finished scene.
//! - **The rewrite is faithful**: with every control at its default the
//!   picture is the original's, and no line of any file moved.
//! - **Deterministic**: a second run gives an identical report, the same
//!   controls and the same GLSL.

use anyhow::{Context as _, Result, anyhow, bail};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use strobe_autocontrol::generate::{
    Choice, GenerateError, Generated, Options, authored_controls, generate, rewrite_chosen,
};
use strobe_autocontrol::output::write_scene;
use strobe_autocontrol::report::Verdict;
use strobe_autocontrol::score::{Rig, ScoreOptions, difference};
use strobe_autocontrol::source::Sources;
use strobe_gfx::{Api, Caps, Headless};
use strobe_scene::SceneFolder;
use strobe_scene::control::ControlKind;
use strobe_scene::names::{is_glsl_identifier, is_reserved};
use strobe_shadertoy::{Shader, convert};

const BOTH: [Api; 2] = [Api::DesktopCore33, Api::Gles30];

/// Fixtures (of the converter's nine) with nothing tunable in them, and why.
/// The engine is meant to find nothing here: a control for a structural
/// literal is worse than no control.
const NO_CONTROLS: [(&str, &str); 6] = [
    (
        "buffer_feedback",
        "a counter: `prev + vec4(1.0 / 255.0)` is bookkeeping, and there is no colour or scale",
    ),
    (
        "chain_four_buffers",
        "four pass-through buffers; the only literals are `1.0 / 255.0` and masks",
    ),
    (
        "gl_traps",
        "`const int MODE` feeds a `case` label (a uniform cannot), and the vec3s are axis masks",
    ),
    (
        "missing_texture",
        "`0.2 + 0.5 * texture(..)` is a gain on a texture, not a coordinate scale or colour",
    ),
    (
        "music",
        "the literals are texture row coordinates and a texel-width normalisation",
    ),
    (
        "single_image",
        "`iTime + uv.xyx + vec3(0.0, 2.0, 4.0)`: a phase offset (not in 0..1) and no time multiplier",
    ),
];

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

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch(tag: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("autocontrol-{tag}"))
}

/// Every Shadertoy fixture: the converter's nine, then this crate's five.
fn fixtures() -> Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    for dir in [
        manifest().join("../strobe-shadertoy/tests/fixtures"),
        manifest().join("tests/fixtures"),
    ] {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .with_context(|| dir.display().to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort();
        for path in files {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            found.push((stem, path));
        }
    }
    Ok(found)
}

/// Converts a fixture, writes it under `tag`, and loads the folder.
fn import(name: &str, path: &Path) -> Result<SceneFolder> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let shader = Shader::from_json(&text)?;
    let converted = convert(&shader, &strobe_shadertoy::Options::default())?;
    let library = scratch(&format!("import-{name}"));
    let dir = strobe_shadertoy::convert::write(&converted, &library, true)?;
    Ok(SceneFolder::load(&dir)?)
}

/// The library scenes: `scenes/` at the workspace root.
fn library_scenes() -> Result<Vec<PathBuf>> {
    Ok(strobe_scene::load::scan(&manifest().join("../../scenes"))?)
}

/// A copy of a library scene with its CONTROLS stripped.
///
/// A scene with no `CONTROLS` still references their names in its GLSL
/// (`amount`, `tint`), so each control is baked in as a `#define` at its
/// default instead: the scene a community author would have written with
/// the numbers inline. The result has no controls and compiles.
fn strip_controls(source: &Path, name: &str) -> Result<SceneFolder> {
    let original = SceneFolder::load(source)?;
    let mut defines = String::new();
    for control in &original.def.controls {
        let at = |i: usize| control.default.get(i).copied().unwrap_or(0.0);
        let value = match control.kind {
            ControlKind::Color { .. } => format!("vec3({:?}, {:?}, {:?})", at(0), at(1), at(2)),
            ControlKind::Xy(_) => format!("vec2({:?}, {:?})", at(0), at(1)),
            ControlKind::Dropdown { .. } => {
                let index = at(0).round().max(0.0) as usize;
                format!("{:?}", control.values.get(index).copied().unwrap_or(0.0))
            }
            ControlKind::Slider(_)
            | ControlKind::Knob(_)
            | ControlKind::Toggle { .. }
            | ControlKind::Bang(_) => format!("{:?}", at(0)),
        };
        let _ = writeln!(defines, "#define {} {value}", control.name);
    }

    let target = scratch(&format!("stripped-{name}")).join(format!("{name}.synScene"));
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    copy_dir(source, &target)?;
    let json_path = target.join("scene.json");
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json_path)?)?;
    if let Some(object) = json.as_object_mut() {
        object.remove("CONTROLS");
    }
    std::fs::write(&json_path, serde_json::to_string_pretty(&json)?)?;
    let files: Vec<String> = original
        .def
        .pass_sources
        .clone()
        .unwrap_or_else(|| vec!["main.glsl".to_string()]);
    let mut seen = Vec::new();
    for file in files {
        if seen.contains(&file) {
            continue;
        }
        let path = target.join(&file);
        let text = std::fs::read_to_string(&path)?;
        std::fs::write(&path, format!("{defines}{text}"))?;
        seen.push(file);
    }
    let stripped = SceneFolder::load(&target)?;
    if !stripped.def.controls.is_empty() {
        bail!("{name}: the stripped copy still has controls");
    }
    Ok(stripped)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// One scene's outcome in one dialect, for the results table.
struct Outcome {
    scene: String,
    dialect: String,
    candidates: usize,
    kept: Vec<String>,
    top: f32,
    fidelity: f32,
}

/// Checks everything that must hold for any generated scene.
fn check(folder: &SceneFolder, generated: &Generated, options: &Options) -> Result<()> {
    let name = &folder.def.title;
    let report = &generated.report;
    assert!(
        report.kept.len() <= options.keep,
        "{name}: more controls than --keep"
    );

    // Names: legal, unique by the script API's rule, not reserved, and not
    // the Strobe group's.
    let mut seen = BTreeMap::new();
    for control in &generated.folder.def.controls {
        let key = strobe_scene::names::normalize(&control.name);
        assert!(
            seen.insert(key, control.name.clone()).is_none(),
            "{name}: duplicate control name {}",
            control.name
        );
    }
    for kept in &report.kept {
        assert!(
            is_glsl_identifier(kept),
            "{name}: `{kept}` is not an identifier"
        );
        assert!(!is_reserved(kept), "{name}: `{kept}` is reserved");
        assert!(
            !kept.starts_with("st_"),
            "{name}: `{kept}` looks like a Strobe control"
        );
    }

    // Each kept control measured above the threshold, and sits in a group.
    for row in report.candidates.iter().filter(|c| c.is_kept()) {
        let score = row
            .score
            .ok_or_else(|| anyhow!("{name}: kept without a score"))?;
        assert!(
            score >= options.score.threshold,
            "{name}: `{:?}` kept at {score}, below the threshold",
            row.name
        );
        assert!(row.name.is_some(), "{name}: a kept candidate has a name");
    }
    for row in &report.candidates {
        if let Verdict::Broken { .. } | Verdict::Dead = row.verdict {
            assert!(
                !row.is_kept(),
                "{name}: dead or broken candidates are not kept"
            );
        }
    }

    // The rewrite is faithful and moved no line.
    let fidelity = report.fidelity.unwrap_or(0.0);
    assert!(
        fidelity <= 0.002,
        "{name}: at its defaults the rewrite differs from the original by {fidelity}"
    );
    let before = Sources::from_folder(folder);
    for (file, text) in generated.sources.iter() {
        let original = before
            .text(file)
            .ok_or_else(|| anyhow!("{name}: {file} vanished"))?;
        assert_eq!(
            text.lines().count(),
            original.lines().count(),
            "{name}: {file} changed length"
        );
    }
    Ok(())
}

/// Independent of the scores in the report: render the finished scene with
/// each generated control moved and require the picture to move.
fn controls_move_the_picture(
    gl: &strobe_gfx::gl::Gl,
    caps: &Caps,
    generated: &Generated,
    options: &Options,
) -> Result<()> {
    let folder = &generated.folder;
    let first_new = folder.def.controls.len() - generated.control_lines.len();
    let mut rig = Rig::new(gl, caps, folder, options.score)?;
    let baseline = rig.render(&[])?;
    for (offset, line) in generated.control_lines.iter().enumerate() {
        let index = first_new + offset;
        let Some(control) = folder.def.controls.get(index) else {
            bail!("control {index} missing for {line}");
        };
        if control.group.as_deref() == Some("Audio") {
            continue; // an audio amount: tested separately
        }
        // Move every component of the control to each end of its range; at
        // least one end must change the picture (more loop iterations than
        // the scene can use, for example, change nothing).
        let mut change = 0.0_f32;
        for end in [&control.min, &control.max] {
            let target: Vec<f32> = end.iter().take(control.dims()).copied().collect();
            let moved = rig.render(&[(index, target)])?;
            change = change.max(difference(&baseline, &moved));
        }
        assert!(
            change > 0.0005,
            "{}: moving `{}` across its range changed nothing ({change})",
            folder.def.title,
            control.name
        );
    }
    Ok(())
}

/// Runs the engine on a scene in `api`, twice on desktop (determinism).
fn run_scene(
    folder: &SceneFolder,
    api: Api,
    options: &Options,
) -> Result<Option<(Generated, Outcome)>> {
    let Some(headless) = context(api)? else {
        return Ok(None);
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let generated = generate(gl, &caps, folder, options)
        .with_context(|| format!("{} in {api:?}", folder.def.title))?;
    check(folder, &generated, options)?;
    if api == Api::DesktopCore33 && !generated.control_lines.is_empty() {
        controls_move_the_picture(gl, &caps, &generated, options)?;
    }
    let again = generate(gl, &caps, folder, options)?;
    assert_eq!(
        generated.report, again.report,
        "{}: two runs must give an identical report ({api:?})",
        folder.def.title
    );
    assert_eq!(
        generated.control_lines, again.control_lines,
        "{}: and identical controls",
        folder.def.title
    );
    assert_eq!(
        generated.sources, again.sources,
        "{}: and identical GLSL",
        folder.def.title
    );
    let report = &generated.report;
    let top = report
        .candidates
        .iter()
        .filter(|c| c.is_kept())
        .filter_map(|c| c.score)
        .fold(0.0_f32, f32::max);
    let outcome = Outcome {
        scene: folder.def.title.clone(),
        dialect: format!("{api:?}"),
        candidates: report.candidates.len(),
        kept: report.kept.clone(),
        top,
        fidelity: report.fidelity.unwrap_or(0.0),
    };
    Ok(Some((generated, outcome)))
}

fn write_results(tag: &str, outcomes: &[Outcome]) -> Result<()> {
    let mut table = String::from(
        "| scene | dialect | candidates | kept | top score | fidelity |\n|---|---|---|---|---|---|\n",
    );
    for o in outcomes {
        let _ = writeln!(
            table,
            "| {} | {} | {} | {} ({}) | {:.4} | {:.5} |",
            o.scene,
            o.dialect,
            o.candidates,
            o.kept.len(),
            o.kept.join(", "),
            o.top,
            o.fidelity
        );
    }
    let path = scratch(tag).with_extension("md");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, table)?;
    Ok(())
}

#[test]
fn every_shadertoy_fixture_gets_controls_in_both_dialects_or_a_documented_reason() -> Result<()> {
    let options = Options::default();
    let mut outcomes = Vec::new();
    let mut ran_any = false;
    for (name, path) in fixtures()? {
        let folder = import(&name, &path)?;
        let mut kept_by_dialect = Vec::new();
        for api in BOTH {
            let Some((generated, outcome)) = run_scene(&folder, api, &options)? else {
                continue;
            };
            ran_any = true;
            let documented = NO_CONTROLS.iter().find(|(stem, _)| *stem == name);
            match documented {
                Some((_, why)) => assert!(
                    generated.report.kept.is_empty(),
                    "{name} ({api:?}) is documented as having nothing tunable ({why}) but got {:?}",
                    generated.report.kept
                ),
                None => assert!(
                    !generated.report.kept.is_empty(),
                    "{name} ({api:?}): expected at least one control, got none; candidates: {}",
                    generated.report.detail()
                ),
            }
            kept_by_dialect.push(generated.report.kept.clone());
            outcomes.push(outcome);
        }
        if let [desktop, gles] = &kept_by_dialect[..] {
            assert_eq!(
                desktop, gles,
                "{name}: both dialects keep the same controls"
            );
        }
    }
    if ran_any {
        write_results("results-fixtures", &outcomes)?;
    }
    Ok(())
}

#[test]
fn every_library_scene_with_its_controls_stripped_gets_controls_in_both_dialects() -> Result<()> {
    let options = Options::default();
    let mut outcomes = Vec::new();
    let mut ran_any = false;
    for dir in library_scenes()? {
        let name = strobe_scene::load::folder_title(&dir);
        let folder = strip_controls(&dir, &name)?;
        let mut kept_by_dialect = Vec::new();
        for api in BOTH {
            let Some((generated, outcome)) = run_scene(&folder, api, &options)? else {
                continue;
            };
            ran_any = true;
            assert!(
                !generated.report.kept.is_empty(),
                "{name} ({api:?}): a library scene must get at least one control; {}",
                generated.report.detail()
            );
            kept_by_dialect.push(generated.report.kept.clone());
            outcomes.push(outcome);
        }
        if let [desktop, gles] = &kept_by_dialect[..] {
            assert_eq!(
                desktop, gles,
                "{name}: both dialects keep the same controls"
            );
        }
    }
    if ran_any {
        write_results("results-library", &outcomes)?;
    }
    Ok(())
}

#[test]
fn a_scene_with_authored_controls_is_refused_unless_merging() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let dir = manifest().join("../../scenes/02-plasma-pulse.synScene");
    let folder = SceneFolder::load(&dir)?;
    assert!(
        !authored_controls(&folder.def).is_empty(),
        "plasma-pulse ships with controls"
    );

    match generate(gl, &caps, &folder, &Options::default()) {
        Err(GenerateError::Authored { count, .. }) => {
            assert_eq!(count, 4, "it names how many controls it found");
        }
        other => bail!("expected a refusal, got {:?}", other.map(|g| g.report.kept)),
    }

    let options = Options {
        merge: true,
        ..Options::default()
    };
    let merged = generate(gl, &caps, &folder, &options)?;
    let authored: Vec<&str> = folder
        .def
        .controls
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    let after: Vec<&str> = merged
        .folder
        .def
        .controls
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        &after[..authored.len()],
        &authored[..],
        "the authored controls come first and are untouched"
    );
    assert!(
        after.len() > authored.len(),
        "and at least one new control was added: {after:?}"
    );
    let mut unique = after.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), after.len(), "no name is repeated: {after:?}");
    Ok(())
}

#[test]
fn an_import_with_only_the_strobe_group_is_not_treated_as_authored() -> Result<()> {
    let (name, path) = fixtures()?
        .into_iter()
        .find(|(name, _)| name == "ring_tunnel")
        .ok_or_else(|| anyhow!("ring_tunnel fixture"))?;
    let folder = import(&name, &path)?;
    assert_eq!(folder.def.controls.len(), 5, "the five st_ controls");
    assert!(
        authored_controls(&folder.def).is_empty(),
        "the Strobe group is not authored"
    );
    Ok(())
}

#[test]
fn the_result_is_written_as_a_new_folder_and_the_source_is_never_touched() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let (name, path) = fixtures()?
        .into_iter()
        .find(|(name, _)| name == "voronoi_cells")
        .ok_or_else(|| anyhow!("voronoi_cells fixture"))?;
    let folder = import(&name, &path)?;
    let generated = generate(gl, &caps, &folder, &Options::default())?;
    assert!(
        !generated.control_lines.is_empty(),
        "controls were generated"
    );

    let snapshot = |dir: &Path| -> Result<BTreeMap<String, String>> {
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                files.insert(
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read_to_string(entry.path()).unwrap_or_default(),
                );
            }
        }
        Ok(files)
    };
    let before = snapshot(&folder.dir)?;

    let library = scratch("written");
    if library.exists() {
        std::fs::remove_dir_all(&library)?;
    }
    let written = write_scene(&folder.dir, &library, &generated, false)?;
    assert!(
        written.ends_with(format!(
            "{}-auto.synScene",
            strobe_scene::load::folder_title(&folder.dir)
        )),
        "named beside the source: {}",
        written.display()
    );
    assert_eq!(
        before,
        snapshot(&folder.dir)?,
        "the source folder is unchanged"
    );

    // The copy loads, carries the new controls after the old ones, and the
    // rest of the scene (the SOURCE block, the clock pass, images) came along.
    let loaded = SceneFolder::load(&written)?;
    assert_eq!(
        loaded.def.controls.len(),
        folder.def.controls.len() + generated.control_lines.len(),
        "authored plus generated"
    );
    assert!(
        loaded.def.source.is_some(),
        "the import's SOURCE block survived"
    );
    assert!(
        written.join("passes/st_clock.glsl").is_file(),
        "supporting files were copied"
    );
    assert!(
        std::fs::read_to_string(written.join("scene.json"))?.contains("\"SOURCE\""),
        "scene.json keeps its other keys"
    );

    // It plays: the loaded copy compiles and draws what the engine said.
    let mut rig = Rig::new(gl, &caps, &loaded, ScoreOptions::default())?;
    let drawn = rig.render(&[])?;
    let mut original = Rig::new(gl, &caps, &folder, ScoreOptions::default())?;
    let expected = original.render(&[])?;
    assert!(
        difference(&drawn, &expected) <= 0.002,
        "the written scene draws the original at its defaults"
    );

    // Refuses to overwrite without being asked, and never targets the source.
    assert!(
        write_scene(&folder.dir, &library, &generated, false).is_err(),
        "an existing result is not overwritten silently"
    );
    write_scene(&folder.dir, &library, &generated, true)?;
    let in_place = folder
        .dir
        .parent()
        .ok_or_else(|| anyhow!("fixture has a parent"))?;
    let renamed = in_place.join(format!(
        "{}-auto.synScene",
        strobe_scene::load::folder_title(&folder.dir)
    ));
    assert_ne!(
        renamed, folder.dir,
        "the destination name never equals the source"
    );
    Ok(())
}

#[test]
fn audio_bindings_are_generated_off_and_do_nothing_until_raised() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let (name, path) = fixtures()?
        .into_iter()
        .find(|(name, _)| name == "voronoi_cells")
        .ok_or_else(|| anyhow!("voronoi_cells fixture"))?;
    let folder = import(&name, &path)?;

    let plain = generate(gl, &caps, &folder, &Options::default())?;
    let bound = generate(
        gl,
        &caps,
        &folder,
        &Options {
            bind_audio: true,
            ..Options::default()
        },
    )?;
    let amounts: Vec<usize> = bound
        .folder
        .def
        .controls
        .iter()
        .enumerate()
        .filter(|(_, c)| c.group.as_deref() == Some("Audio"))
        .map(|(i, _)| i)
        .collect();
    assert!(
        !amounts.is_empty(),
        "bindings were written: {:?}",
        bound.report.suggestions
    );
    for index in &amounts {
        let control = &bound.folder.def.controls[*index];
        assert_eq!(
            control.default[0], 0.0,
            "`{}` defaults to off",
            control.name
        );
    }
    assert!(
        bound.report.suggestions.iter().any(|s| s.expressible),
        "at least one suggestion could be written into the GLSL"
    );
    assert_eq!(
        plain.report.kept, bound.report.kept,
        "the audio amounts are extra controls, not different ones"
    );

    // At 0 the bound scene draws exactly what the plain one does.
    let mut plain_rig = Rig::new(gl, &caps, &plain.folder, ScoreOptions::default())?;
    let mut bound_rig = Rig::new(gl, &caps, &bound.folder, ScoreOptions::default())?;
    let off = bound_rig.render(&[])?;
    assert!(
        difference(&plain_rig.render(&[])?, &off) <= 0.0005,
        "audio amount 0 is the same picture"
    );
    // Raised, the music moves it.
    let raised = bound_rig.render(&[(amounts[0], vec![1.0])])?;
    assert!(
        difference(&off, &raised) > 0.0005,
        "raising `{}` makes the picture follow the band",
        bound.folder.def.controls[amounts[0]].name
    );
    Ok(())
}

#[test]
fn candidates_that_break_the_compile_are_dropped_not_fatal() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    // `W` sizes an array through another macro, which the scan cannot see; a
    // uniform there does not compile, so the engine must drop it and still
    // offer the colour.
    let target = scratch("breaks").join("breaks.synScene");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::create_dir_all(&target)?;
    std::fs::write(
        target.join("main.glsl"),
        "#define W 4.0\n#define SIZE (W + 4.0)\nfloat table[int(SIZE)];\n\
         vec4 renderMain() {\n    table[0] = W * 0.1;\n    vec3 c = vec3(0.9, 0.4, 0.1);\n    return vec4(c * (0.5 + 0.5 * sin(TIME)) + table[0], 1.0);\n}\n",
    )?;
    std::fs::write(target.join("scene.json"), "{}")?;
    let folder = SceneFolder::load(&target)?;
    let generated = generate(gl, &caps, &folder, &Options::default())?;
    let dropped = generated
        .report
        .candidates
        .iter()
        .filter(|c| matches!(c.verdict, Verdict::DoesNotCompile { .. }))
        .count();
    assert!(dropped >= 1, "{}", generated.report.detail());
    assert!(
        generated.report.kept.iter().any(|n| n.contains("color")),
        "the colour survives: {:?}",
        generated.report.kept
    );
    Ok(())
}

#[test]
fn a_scene_that_does_not_compile_is_reported_not_measured() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let target = scratch("broken").join("broken.synScene");
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::create_dir_all(&target)?;
    std::fs::write(
        target.join("main.glsl"),
        "vec4 renderMain() { vec3 c = vec3(0.9, 0.4, 0.1); return vec4(c * nope, 1.0); }\n",
    )?;
    std::fs::write(target.join("scene.json"), "{}")?;
    let folder = SceneFolder::load(&target)?;
    match generate(gl, &caps, &folder, &Options::default()) {
        Err(GenerateError::Original(_)) => Ok(()),
        other => bail!(
            "expected the original's compile error, got {:?}",
            other.map(|g| g.report.kept)
        ),
    }
}

#[test]
fn a_review_drops_and_re_ranges_controls_without_measuring_again() -> Result<()> {
    let Some(headless) = context(Api::DesktopCore33)? else {
        return Ok(());
    };
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let (name, path) = fixtures()?
        .into_iter()
        .find(|(name, _)| name == "voronoi_cells")
        .ok_or_else(|| anyhow!("voronoi_cells fixture"))?;
    let folder = import(&name, &path)?;
    let generated = generate(gl, &caps, &folder, &Options::default())?;
    let mut choices: Vec<Choice> = generated
        .report
        .candidates
        .iter()
        .filter(|c| matches!(c.verdict, Verdict::Kept { .. }))
        .filter_map(|c| {
            Some(Choice {
                candidate: c.id,
                name: c.name.clone()?,
                context: c.context.clone(),
                min: c.min.first().copied().unwrap_or(0.0),
                max: c.max.first().copied().unwrap_or(1.0),
            })
        })
        .collect();
    if choices.len() < 2 {
        bail!(
            "voronoi_cells kept {} control(s); the test needs two",
            choices.len()
        );
    }
    let dropped = choices.remove(0);
    let slider = choices
        .iter_mut()
        .find(|c| {
            generated.report.candidates.iter().any(|r| {
                r.id == c.candidate && r.value_type != strobe_autocontrol::ValueType::Colour
            })
        })
        .ok_or_else(|| anyhow!("no slider left to re-range"))?;
    slider.min = -3.0;
    slider.max = 7.5;
    let ranged = slider.name.clone();

    let reviewed = rewrite_chosen(&folder, &choices)?;
    assert_eq!(
        reviewed.control_lines.len(),
        choices.len(),
        "one control per choice"
    );
    assert!(
        !reviewed
            .control_lines
            .iter()
            .any(|line| line.contains(&format!("\"{}\"", dropped.name))),
        "the dropped control is gone"
    );
    let line = reviewed
        .control_lines
        .iter()
        .find(|line| line.contains(&format!("\"{ranged}\"")))
        .ok_or_else(|| anyhow!("the re-ranged control is missing"))?;
    assert!(
        line.contains("\"MIN\": -3.0") && line.contains("\"MAX\": 7.5"),
        "the chosen range is written: {line}"
    );

    // It compiles and, at its defaults, still draws the original.
    let mut rig = Rig::new(gl, &caps, &reviewed.folder, ScoreOptions::default())?;
    let drawn = rig.render(&[])?;
    let mut original = Rig::new(gl, &caps, &folder, ScoreOptions::default())?;
    let expected = original.render(&[])?;
    assert!(
        difference(&drawn, &expected) <= 0.002,
        "the reviewed scene draws the original at its defaults"
    );

    // A choice that no longer matches the code is refused.
    let mut stale = choices.clone();
    if let Some(first) = stale.first_mut() {
        first.context = "float nothing = 0.0;".to_string();
    }
    assert!(
        matches!(
            rewrite_chosen(&folder, &stale),
            Err(strobe_autocontrol::ChoiceError::Changed(_))
        ),
        "a changed source is refused, not guessed at"
    );
    Ok(())
}
