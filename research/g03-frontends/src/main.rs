// ============================================================================
// Module:       g03_frontends (binary crate root)
// Description:  G-03 spike: every pass of every scene in the given libraries,
//               assembled exactly as the GL path compiles it, run through
//               naga's GLSL front end and through glslang to SPIR-V, then
//               SPIR-V into naga, HLSL and MSL. One row per pass and dialect.
//
// Dependencies: anyhow, naga (glsl-in, spv-in), strobe-scene; the
//               glslangValidator and spirv-cross programs on PATH
// ============================================================================

//! `cargo run --release -- <library>... --out <dir>`
//!
//! Writes `<dir>/results.csv` (one row per scene, pass and dialect) and
//! `<dir>/summary.md` (counts, and the first lines of each kind of
//! failure). The sources are written to `<dir>/src/` so any row can be
//! reproduced by hand. See README.md for the question and the answer.

use anyhow::{Context as _, Result, bail};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use strobe_scene::SceneFolder;
use strobe_scene::assemble::{self, Dialect, PassIndex, Request};

/// One step's outcome: passed, or the first line of why not, and its time.
#[derive(Clone)]
struct Step {
    ok: bool,
    why: String,
    ms: f64,
}

impl Step {
    fn passed(ms: f64) -> Self {
        Self {
            ok: true,
            why: String::new(),
            ms,
        }
    }

    fn failed(why: impl Into<String>, ms: f64) -> Self {
        let why = why.into();
        let first = why
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("glslangValidator"))
            .unwrap_or("")
            .chars()
            .take(160)
            .collect();
        Self {
            ok: false,
            why: first,
            ms,
        }
    }

    fn skipped(why: &str) -> Self {
        Self {
            ok: false,
            why: format!("not run: {why}"),
            ms: 0.0,
        }
    }

    fn word(&self) -> &'static str {
        match (self.ok, self.why.starts_with("not run")) {
            (true, _) => "ok",
            (false, true) => "skip",
            (false, false) => "FAIL",
        }
    }
}

/// The candidates, in the column order of the report.
const COLUMNS: [&str; 8] = [
    "naga glsl-in",
    "naga glsl-in (as 450 core)",
    "naga glsl-in (450, uniforms in a block)",
    "glslang -> SPIR-V (GL)",
    "glslang -> SPIR-V (Vulkan, relaxed)",
    "SPIR-V -> naga spv-in",
    "SPIR-V -> HLSL (spirv-cross)",
    "SPIR-V -> MSL (spirv-cross)",
];

struct Row {
    scene: String,
    pass: usize,
    dialect: &'static str,
    bytes: usize,
    steps: Vec<Step>,
}

fn main() -> Result<()> {
    let mut libraries = Vec::new();
    let mut out = PathBuf::from("target/g03");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out" {
            out = PathBuf::from(args.next().context("--out needs a folder")?);
        } else {
            libraries.push(PathBuf::from(arg));
        }
    }
    if libraries.is_empty() {
        bail!("usage: g03-frontends <library>... [--out <dir>]");
    }
    std::fs::create_dir_all(out.join("src"))?;
    let mut rows = Vec::new();
    for library in &libraries {
        for dir in strobe_scene::load::scan(library)? {
            let folder = match SceneFolder::load(&dir) {
                Ok(folder) => folder,
                Err(error) => {
                    eprintln!("{}: does not load: {error}", dir.display());
                    continue;
                }
            };
            let name = strobe_scene::load::folder_title(&dir);
            // The desktop dialect only: SPIR-V needs ES 3.10 or later, so
            // the ANGLE dialect (ES 3.00) cannot take this route at all,
            // and a SPIR-V backend would compile the desktop one.
            for (dialect, label) in [(Dialect::Glsl330, "desktop")] {
                for pass in 0..folder.def.passes.len() {
                    let source = assemble::fragment(&Request {
                        scene: &folder.def,
                        user_code: folder.shader_for(pass),
                        dialect,
                        pass_index: PassIndex::Constant(pass),
                        extra: &[],
                    });
                    let stem = format!("{name}__p{pass}__{label}");
                    let file = out.join("src").join(format!("{stem}.frag"));
                    std::fs::write(&file, &source)?;
                    let steps = run_all(&source, &file, &out.join("src"), &stem);
                    println!(
                        "{name:<34} p{pass} {label:<7} {}",
                        steps.iter().map(Step::word).collect::<Vec<_>>().join(" ")
                    );
                    rows.push(Row {
                        scene: name.clone(),
                        pass,
                        dialect: label,
                        bytes: source.len(),
                        steps,
                    });
                }
            }
        }
    }
    std::fs::write(out.join("results.csv"), csv(&rows))?;
    std::fs::write(out.join("summary.md"), summary(&rows)?)?;
    println!("\nwrote {}", out.display());
    Ok(())
}

/// Every candidate on one source.
fn run_all(source: &str, file: &Path, dir: &Path, stem: &str) -> Vec<Step> {
    let naga_as_written = naga_glsl(source);
    // naga's front end takes only 440 and 450: the least a backend would
    // do is rewrite the version line, so measure that too.
    let as_450 = match source.split_once('\n') {
        Some((_, rest)) => format!("#version 450 core\n{rest}"),
        None => source.to_string(),
    };
    let naga_450 = naga_glsl(&as_450);
    let adapted = vulkan_style(&as_450);
    let _ = std::fs::write(dir.join(format!("{stem}.naga.frag")), &adapted);
    let naga_block = naga_glsl(&adapted);
    if std::env::var_os("G03_VERBOSE").is_some()
        && let Err(errors) = naga::front::glsl::Frontend::default().parse(
            &naga::front::glsl::Options::from(naga::ShaderStage::Fragment),
            &adapted,
        )
    {
        eprintln!("{stem}: {}", errors.emit_to_string(&adapted));
    }
    let gl_spv = dir.join(format!("{stem}.gl.spv"));
    let vk_spv = dir.join(format!("{stem}.vk.spv"));
    let glslang_gl = tool(
        "glslangValidator",
        &[
            "-G",
            "--auto-map-locations",
            "--auto-map-bindings",
            "-S",
            "frag",
            "-o",
            &gl_spv.display().to_string(),
            &file.display().to_string(),
        ],
    );
    let glslang_vk = tool(
        "glslangValidator",
        &[
            "-V",
            "-R",
            "--auto-map-locations",
            "--auto-map-bindings",
            "-S",
            "frag",
            "-o",
            &vk_spv.display().to_string(),
            &file.display().to_string(),
        ],
    );
    let (spv_in, hlsl, msl) = if glslang_vk.ok {
        let vk = vk_spv.display().to_string();
        (
            naga_spv(&vk_spv),
            tool("spirv-cross", &[&vk, "--hlsl", "--shader-model", "50"]),
            tool("spirv-cross", &[&vk, "--msl"]),
        )
    } else {
        let why = "glslang (Vulkan) failed";
        (Step::skipped(why), Step::skipped(why), Step::skipped(why))
    };
    vec![
        naga_as_written,
        naga_450,
        naga_block,
        glslang_gl,
        glslang_vk,
        spv_in,
        hlsl,
        msl,
    ]
}

/// What glslang's relaxed mode does by itself, by hand, for naga: every
/// loose non-opaque `uniform` at the start of a line moves into one block
/// with a binding, each sampler gets a binding, and the output a location.
/// A line-based rewrite, enough for a spike: a uniform declared mid-line or
/// over several lines stays where it was, and shows as a failure.
fn vulkan_style(source: &str) -> String {
    let mut block = Vec::new();
    let mut body = Vec::new();
    let mut binding = 1;
    for line in source.lines() {
        let trimmed = line.trim_start();
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        match words.as_slice() {
            ["uniform", kind, ..] if kind.starts_with("sampler") => {
                body.push(format!("layout(binding = {binding}) {trimmed}"));
                binding += 1;
            }
            ["uniform", ..] if trimmed.ends_with(';') && !trimmed.contains('=') => {
                block.push(format!(
                    "    {}",
                    trimmed.trim_start_matches("uniform").trim()
                ));
            }
            ["out", "vec4", ..] => body.push(format!("layout(location = 0) {trimmed}")),
            _ => body.push(line.to_string()),
        }
    }
    let mut lines = body.into_iter();
    let mut out = lines.next().unwrap_or_default();
    out.push('\n');
    if !block.is_empty() {
        out.push_str("layout(binding = 0) uniform StrobeUniforms {\n");
        out.push_str(&block.join("\n"));
        out.push_str("\n};\n");
    }
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// naga's GLSL front end, then its validator.
fn naga_glsl(source: &str) -> Step {
    let started = Instant::now();
    let mut frontend = naga::front::glsl::Frontend::default();
    let options = naga::front::glsl::Options::from(naga::ShaderStage::Fragment);
    let module = match frontend.parse(&options, source) {
        Ok(module) => module,
        Err(errors) => {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            return Step::failed(format!("parse: {}", errors.emit_to_string(source)), ms);
        }
    };
    validate(&module, started)
}

/// SPIR-V into naga, then its validator: the path wgpu takes for a
/// SPIR-V shader.
fn naga_spv(path: &Path) -> Step {
    let started = Instant::now();
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return Step::failed(format!("read: {error}"), 0.0),
    };
    let options = naga::front::spv::Options::default();
    match naga::front::spv::parse_u8_slice(&bytes, &options) {
        Ok(module) => validate(&module, started),
        Err(error) => Step::failed(
            format!("spv-in: {error}"),
            started.elapsed().as_secs_f64() * 1000.0,
        ),
    }
}

fn validate(module: &naga::Module, started: Instant) -> Step {
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    let result = validator.validate(module);
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    match result {
        Ok(_) => Step::passed(ms),
        Err(error) => Step::failed(format!("validate: {}", error.into_inner()), ms),
    }
}

/// An external program; its stdout and stderr together are the reason.
fn tool(program: &str, args: &[&str]) -> Step {
    let started = Instant::now();
    match Command::new(program).args(args).output() {
        Ok(output) => {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if output.status.success() {
                Step::passed(ms)
            } else {
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                // glslang prints the file name first, then "ERROR: ...".
                let why = text
                    .lines()
                    .find(|line| line.contains("ERROR") || line.contains("error"))
                    .unwrap_or(&text)
                    .to_string();
                Step::failed(why, ms)
            }
        }
        Err(error) => Step::failed(format!("{program}: {error}"), 0.0),
    }
}

fn csv(rows: &[Row]) -> String {
    let mut out = String::from("scene,pass,dialect,bytes");
    for column in COLUMNS {
        let _ = write!(out, ",\"{column}\",\"{column} ms\",\"{column} why\"");
    }
    out.push('\n');
    for row in rows {
        let _ = write!(
            out,
            "{},{},{},{}",
            row.scene, row.pass, row.dialect, row.bytes
        );
        for step in &row.steps {
            let _ = write!(
                out,
                ",{},{:.2},\"{}\"",
                step.word(),
                step.ms,
                step.why.replace('"', "'")
            );
        }
        out.push('\n');
    }
    out
}

fn summary(rows: &[Row]) -> Result<String> {
    let mut out = String::new();
    let scenes: std::collections::BTreeSet<&str> = rows.iter().map(|r| r.scene.as_str()).collect();
    writeln!(
        out,
        "# G-03 front ends: {} scenes, {} pass sources\n",
        scenes.len(),
        rows.len()
    )?;
    for dialect in ["desktop"] {
        let these: Vec<&Row> = rows.iter().filter(|r| r.dialect == dialect).collect();
        writeln!(out, "## {dialect} ({} pass sources)\n", these.len())?;
        writeln!(
            out,
            "| Candidate | ok | failed | skipped | scenes failing | median ms |"
        )?;
        writeln!(out, "|---|---|---|---|---|---|")?;
        for (column, name) in COLUMNS.iter().enumerate() {
            let steps: Vec<(&Row, &Step)> = these
                .iter()
                .filter_map(|r| r.steps.get(column).map(|s| (*r, s)))
                .collect();
            let ok = steps.iter().filter(|(_, s)| s.word() == "ok").count();
            let failed = steps.iter().filter(|(_, s)| s.word() == "FAIL").count();
            let skipped = steps.len() - ok - failed;
            let failing: std::collections::BTreeSet<&str> = steps
                .iter()
                .filter(|(_, s)| s.word() == "FAIL")
                .map(|(r, _)| r.scene.as_str())
                .collect();
            let mut times: Vec<f64> = steps
                .iter()
                .filter(|(_, s)| s.word() == "ok")
                .map(|(_, s)| s.ms)
                .collect();
            times.sort_by(f64::total_cmp);
            let median = times.get(times.len() / 2).copied().unwrap_or(0.0);
            writeln!(
                out,
                "| {name} | {ok} | {failed} | {skipped} | {} | {median:.1} |",
                failing.len()
            )?;
        }
        writeln!(out)?;
        for (column, name) in COLUMNS.iter().enumerate() {
            let mut kinds: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for row in &these {
                if let Some(step) = row.steps.get(column)
                    && step.word() == "FAIL"
                {
                    kinds
                        .entry(normalise(&step.why))
                        .or_default()
                        .push(format!("{} p{}", row.scene, row.pass));
                }
            }
            if kinds.is_empty() {
                continue;
            }
            writeln!(out, "### {name}: why\n")?;
            for (why, wheres) in &kinds {
                writeln!(
                    out,
                    "- {} x `{}` (e.g. {})",
                    wheres.len(),
                    why,
                    wheres
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )?;
            }
            writeln!(out)?;
        }
    }
    Ok(out)
}

/// A failure's first line with paths, line numbers and quoted names
/// blurred, so that failures of one kind group together.
fn normalise(why: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    for c in why.chars() {
        match c {
            '\'' | '`' | '"' => {
                quoted = !quoted;
                out.push('…');
            }
            _ if quoted => {}
            '0'..='9' => {
                if !out.ends_with('#') {
                    out.push('#');
                }
            }
            _ => out.push(c),
        }
    }
    let out = match out.find(".frag") {
        Some(at) => out[at + 5..]
            .trim_start_matches([':', ' ', '#'])
            .to_string(),
        None => out,
    };
    out.chars().take(110).collect()
}
