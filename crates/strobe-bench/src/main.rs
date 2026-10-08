// ============================================================================
// Module:       strobe_bench (binary crate root)
// Description:  Renders every scene in a library on a headless context, times
//               each, and writes frames, a contact sheet and a results table.
//
// Dependencies: clap, anyhow, image; strobe-gfx (headless context, player),
//               strobe-scene (loading, controls)
// ============================================================================

//! `strobe-bench`: the performance baseline.
//!
//! ```text
//! cargo run --release -p strobe-bench -- --library scenes --size 640x360
//! ```
//!
//! For each scene: load and compile it (timed), render a warm-up so
//! feedback and simulation scenes have history, then render `--frames`
//! frames with a `glFinish` after each, so every sample is the GPU's real
//! time for that frame rather than the time to queue it. Output goes to
//! `--out`: one PNG per scene, `contact-sheet.png`, and `results.md`.
//!
//! Frames are a pure function of time (fixed `dt`, synthetic audio), so
//! two runs on one machine render identical images — which is what makes
//! the PNGs usable as a visual regression check, and the timings
//! comparable across a change.
//!
//! On a machine with no GPU (CI, a cloud container) the context is Mesa's
//! llvmpipe: absolute numbers are CPU rasterisation, but the *ranking* of
//! scenes and the effect of a renderer change hold.

use anyhow::{Context as _, Result, anyhow};
use clap::{Parser, ValueEnum};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;
use strobe_gfx::{Api, Caps, FloatPrecision, FrameInputs, Headless, Mouse, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;

/// The command line.
#[derive(Parser, Debug)]
#[command(
    name = "strobe-bench",
    about = "Render and time every scene in a library"
)]
struct Cli {
    /// Folder of scene folders.
    #[arg(long, default_value = "scenes")]
    library: PathBuf,
    /// Render resolution, WIDTHxHEIGHT.
    #[arg(long, default_value = "640x360")]
    size: String,
    /// Timed frames per scene.
    #[arg(long, default_value_t = 60)]
    frames: u32,
    /// Untimed frames first, so feedback scenes have history.
    #[arg(long, default_value_t = 90)]
    warmup: u32,
    /// Seconds of scene time at the first frame.
    #[arg(long, default_value_t = 2.0)]
    start: f32,
    /// Context dialect.
    #[arg(long, value_enum, default_value_t = ApiArg::Desktop)]
    api: ApiArg,
    /// RGBA16F instead of RGBA32F for FLOAT passes.
    #[arg(long)]
    half: bool,
    /// One shared program per scene with a PASSINDEX uniform, instead of
    /// one specialised program per pass.
    #[arg(long)]
    shared: bool,
    /// Only scenes whose folder name contains this.
    #[arg(long)]
    filter: Option<String>,
    /// Output folder.
    #[arg(long, default_value = "target/bench")]
    out: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ApiArg {
    Desktop,
    Gles,
}

/// One scene's results.
struct Row {
    name: String,
    compile_ms: f64,
    mean_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    image: Option<image::RgbaImage>,
    error: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let size = parse_size(&cli.size)?;
    let api = match cli.api {
        ApiArg::Desktop => Api::DesktopCore33,
        ApiArg::Gles => Api::Gles30,
    };
    let headless = Headless::new(api).map_err(|e| anyhow!("headless context: {e}"))?;
    let gl = headless.gl();
    let caps = Caps::from_info(gl.info());
    let options = Options {
        specialise_passes: !cli.shared,
        float_precision: if cli.half {
            FloatPrecision::Half
        } else {
            FloatPrecision::Full
        },
    };
    println!(
        "{} | {} | {:?} | {}x{} | {} | float {:?}",
        caps.info.renderer,
        caps.info.version,
        caps.dialect,
        size[0],
        size[1],
        if options.specialise_passes {
            "per-pass programs"
        } else {
            "shared program"
        },
        options.float_precision,
    );

    std::fs::create_dir_all(&cli.out).with_context(|| format!("creating {}", cli.out.display()))?;
    let mut rows = Vec::new();
    for dir in strobe_scene::load::scan(&cli.library)? {
        let name = strobe_scene::load::folder_title(&dir);
        if cli
            .filter
            .as_ref()
            .is_some_and(|f| !name.contains(f.as_str()))
        {
            continue;
        }
        let row = bench_scene(gl, &caps, &dir, &name, size, options, &cli);
        match &row.error {
            Some(error) => println!("{name:<28} FAILED: {error}"),
            None => println!(
                "{name:<28} compile {:>7.1} ms | frame mean {:>8.2} ms  p95 {:>8.2} ms  ({:>6.1} fps)",
                row.compile_ms,
                row.mean_ms,
                row.p95_ms,
                1000.0 / row.mean_ms.max(0.001)
            ),
        }
        if let Some(image) = &row.image {
            image.save(cli.out.join(format!("{name}.png")))?;
        }
        rows.push(row);
    }

    write_contact_sheet(&rows, &cli.out.join("contact-sheet.png"))?;
    let report = report(&rows, &caps, size, options, &cli);
    std::fs::write(cli.out.join("results.md"), &report)?;
    println!("\nwrote {}", cli.out.display());
    Ok(())
}

fn parse_size(text: &str) -> Result<[i32; 2]> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow!("size must be WIDTHxHEIGHT, got {text}"))?;
    Ok([w.trim().parse()?, h.trim().parse()?])
}

fn bench_scene(
    gl: &strobe_gfx::gl::Gl,
    caps: &Caps,
    dir: &Path,
    name: &str,
    size: [i32; 2],
    options: Options,
    cli: &Cli,
) -> Row {
    let failed = |error: String| Row {
        name: name.to_string(),
        compile_ms: 0.0,
        mean_ms: 0.0,
        p95_ms: 0.0,
        max_ms: 0.0,
        image: None,
        error: Some(error),
    };
    let scene = match SceneFolder::load(dir) {
        Ok(scene) => scene,
        Err(error) => return failed(error.to_string()),
    };
    let mut controls = match ControlBank::new(scene.def.controls.clone()) {
        Ok(controls) => controls,
        Err(error) => return failed(error.to_string()),
    };
    let transitions: Vec<f32> = scene
        .def
        .transition_uniforms
        .iter()
        .map(|t| t.initial)
        .collect();

    let compile_start = Instant::now();
    let mut player = match ScenePlayer::new(gl, caps, &scene, size, options) {
        Ok(player) => player,
        Err(error) => return failed(error.to_string()),
    };
    gl.finish();
    let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;

    let mut audio = vec![0.0; SCALARS.len()];
    let dt = 1.0 / 60.0;
    let mut times = Vec::with_capacity(cli.frames as usize);
    for frame in 0..u64::from(cli.warmup + cli.frames) {
        let time = cli.start + frame as f32 * dt;
        strobe_scene::synthetic::fill(time, &mut audio);
        controls.tick(dt);
        let start = Instant::now();
        player.render(&FrameInputs {
            time,
            frame,
            mouse: Mouse::default(),
            audio: &audio,
            controls: &controls,
            transitions: &transitions,
        });
        gl.finish();
        if frame >= u64::from(cli.warmup) {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    if let Some(code) = gl.error() {
        return failed(format!("GL error 0x{code:x}"));
    }

    let pixels = player.read_output();
    let image = image::RgbaImage::from_raw(size[0] as u32, size[1] as u32, pixels);
    times.sort_by(f64::total_cmp);
    let mean_ms = times.iter().sum::<f64>() / times.len().max(1) as f64;
    let p95_ms = times
        .get(((times.len() as f64) * 0.95) as usize)
        .or(times.last())
        .copied()
        .unwrap_or(0.0);
    Row {
        name: name.to_string(),
        compile_ms,
        mean_ms,
        p95_ms,
        max_ms: times.last().copied().unwrap_or(0.0),
        image,
        error: None,
    }
}

/// Thumbnails of every rendered scene in a grid, five across.
fn write_contact_sheet(rows: &[Row], path: &Path) -> Result<()> {
    const THUMB: [u32; 2] = [320, 180];
    const COLUMNS: u32 = 5;
    let images: Vec<&image::RgbaImage> = rows.iter().filter_map(|row| row.image.as_ref()).collect();
    if images.is_empty() {
        return Ok(());
    }
    let count = images.len() as u32;
    let rows_needed = count.div_ceil(COLUMNS);
    let mut sheet = image::RgbaImage::from_pixel(
        THUMB[0] * COLUMNS.min(count),
        THUMB[1] * rows_needed,
        image::Rgba([0, 0, 0, 255]),
    );
    for (i, img) in images.into_iter().enumerate() {
        let thumb = image::imageops::resize(
            img,
            THUMB[0],
            THUMB[1],
            image::imageops::FilterType::Triangle,
        );
        let (x, y) = (
            (i as u32 % COLUMNS) * THUMB[0],
            (i as u32 / COLUMNS) * THUMB[1],
        );
        image::imageops::overlay(&mut sheet, &thumb, i64::from(x), i64::from(y));
    }
    sheet.save(path)?;
    Ok(())
}

/// The results as a Markdown table.
fn report(rows: &[Row], caps: &Caps, size: [i32; 2], options: Options, cli: &Cli) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Strobe benchmark\n");
    let _ = writeln!(
        out,
        "- Renderer: {} ({})\n- Dialect: {:?}\n- Resolution: {}x{}\n- Programs: {}\n- Float passes: {:?}\n- Frames: {} timed after {} warm-up\n",
        caps.info.renderer,
        caps.info.version,
        caps.dialect,
        size[0],
        size[1],
        if options.specialise_passes {
            "one per pass (PASSINDEX constant)"
        } else {
            "shared (PASSINDEX uniform)"
        },
        options.float_precision,
        cli.frames,
        cli.warmup,
    );
    let _ = writeln!(
        out,
        "| Scene | Compile ms | Mean ms | p95 ms | Max ms | FPS |"
    );
    let _ = writeln!(out, "|---|---:|---:|---:|---:|---:|");
    for row in rows {
        match &row.error {
            Some(error) => {
                let _ = writeln!(
                    out,
                    "| {} | failed: {} | | | | |",
                    row.name,
                    error.replace('\n', " ")
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "| {} | {:.1} | {:.2} | {:.2} | {:.2} | {:.1} |",
                    row.name,
                    row.compile_ms,
                    row.mean_ms,
                    row.p95_ms,
                    row.max_ms,
                    1000.0 / row.mean_ms.max(0.001)
                );
            }
        }
    }
    out
}
