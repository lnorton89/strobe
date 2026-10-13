// ============================================================================
// Module:       golden (integration test)
// Description:  Pins the pure half of the engine on real converted shaders:
//               the candidates found and the exact rewritten GLSL.
//
// Dependencies: anyhow; strobe-autocontrol, strobe-shadertoy, strobe-scene
// ============================================================================

//! Golden tests for analysis and rewrite. No GPU is involved: the analysis
//! and the rewrite are pure functions of the source text, so their output is
//! pinned byte for byte under `tests/golden/`. The measured half (scores,
//! what is kept) depends on the rasteriser and is covered by invariants in
//! `engine.rs` instead.
//!
//! After an intended change, regenerate with
//! `STROBE_BLESS=1 cargo test -p strobe-autocontrol --test golden` and read
//! the diff.

use anyhow::{Context as _, Result, anyhow};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use strobe_autocontrol::analyze::analyze;
use strobe_autocontrol::rewrite::{PlannedControl, apply};
use strobe_autocontrol::source::Sources;
use strobe_scene::SceneFolder;
use strobe_shadertoy::{Shader, convert};

const FIXTURES: [&str; 5] = [
    "colour_bars",
    "march_field",
    "ring_tunnel",
    "swirl_trails",
    "voronoi_cells",
];

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn import(name: &str, tag: &str) -> Result<SceneFolder> {
    let path = manifest().join(format!("tests/fixtures/{name}.json"));
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let converted = convert(
        &Shader::from_json(&text)?,
        &strobe_shadertoy::Options::default(),
    )?;
    let library = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("golden-{tag}-{name}"));
    let dir = strobe_shadertoy::convert::write(&converted, &library, true)?;
    Ok(SceneFolder::load(&dir)?)
}

/// Compares `actual` with the golden file, or rewrites it when blessing.
fn golden(file: &str, actual: &str) -> Result<()> {
    let path: PathBuf = manifest().join("tests/golden").join(file);
    if std::env::var_os("STROBE_BLESS").is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, actual)?;
        return Ok(());
    }
    let expected = std::fs::read_to_string(&path)
        .with_context(|| format!("{} (run with STROBE_BLESS=1 to create it)", path.display()))?;
    if expected != actual {
        return Err(anyhow!(
            "{} differs from the engine's output.\n--- expected\n{expected}\n--- actual\n{actual}",
            Path::new(file).display()
        ));
    }
    Ok(())
}

#[test]
fn the_candidates_found_in_each_converted_fixture_are_pinned() -> Result<()> {
    for name in FIXTURES {
        let folder = import(name, "analysis")?;
        let analysis = analyze(&Sources::from_folder(&folder));
        let mut text = String::new();
        for c in &analysis.candidates {
            let sites: Vec<String> = c
                .sites
                .iter()
                .map(|s| format!("{}:{}:{}", s.file, s.line, s.column))
                .collect();
            let _ = writeln!(
                text,
                "#{} {} {} {:?} default={:?} range={:?}..{:?} name={}{} sites=[{}] literal={:?}",
                c.id,
                c.kind.label(),
                c.group.ui_name(),
                c.value_type,
                c.default,
                c.min,
                c.max,
                c.name,
                c.role
                    .as_deref()
                    .map_or_else(String::new, |role| format!(" role={role}")),
                sites.join(" "),
                c.literal()
            );
        }
        golden(&format!("analysis/{name}.txt"), &text)?;
    }
    Ok(())
}

#[test]
fn the_rewritten_glsl_of_a_converted_fixture_is_pinned() -> Result<()> {
    for name in ["ring_tunnel", "swirl_trails"] {
        let folder = import(name, "rewrite")?;
        let sources = Sources::from_folder(&folder);
        let analysis = analyze(&sources);
        let planned: Vec<PlannedControl> = analysis
            .candidates
            .iter()
            .map(|c| PlannedControl {
                candidate: c.id,
                name: format!("auto_{}", c.id),
                audio: None,
            })
            .collect();
        let rewritten = apply(&sources, &analysis.candidates, &planned);
        let mut text = String::new();
        for (file, body) in rewritten.iter() {
            let before = sources.text(file).unwrap_or("");
            assert_eq!(
                body.lines().count(),
                before.lines().count(),
                "{name}/{file}: no line moved"
            );
            let _ = writeln!(text, "==== {file}\n{body}");
        }
        golden(&format!("rewrite/{name}.txt"), &text)?;
    }
    Ok(())
}

#[test]
fn the_analysis_is_identical_on_every_run() -> Result<()> {
    for name in FIXTURES {
        let folder = import(name, "repeat")?;
        let sources = Sources::from_folder(&folder);
        assert_eq!(
            analyze(&sources),
            analyze(&sources),
            "{name}: the same source gives the same candidates, in the same order"
        );
    }
    Ok(())
}
