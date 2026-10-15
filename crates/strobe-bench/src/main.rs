// ============================================================================
// Module:       strobe_bench (binary crate root)
// Description:  Renders every scene in a library on a headless or windowed
//               context, times each, and writes frames, a contact sheet and
//               a results table.
//
// Dependencies: clap, anyhow, image; strobe-gfx (contexts, player),
//               strobe-scene (loading, controls), strobe-diag (stderr log)
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
//! ## Which context
//!
//! `--context headless` opens an EGL *device* display: no window system,
//! which is what CI and a cloud container have. Windows' Intel driver
//! exposes no EGL devices, so there `--context window` opens one hidden
//! window through the same [`strobe_gfx::WindowedContext`] the app uses
//! (`--gl native|angle|auto` picks the backend) and runs the identical
//! loop from inside the event loop. The window only owns a context: scenes
//! render into the player's offscreen targets, nothing is ever swapped, and
//! the surface's vsync is off. `--context auto`, the default, tries
//! headless first and falls back to the window. The context code is
//! [`strobe_gfx::offscreen`], shared with `strobe-autocontrol` (D140).
//!
//! `--readback` runs the `textureFinalPass` readback checks on the same
//! context instead (latency, cost, busy GPU, resize, lost context; see
//! [`readback`]), so they can run on Windows, which has no headless EGL.
//!
//! On a machine with no GPU (CI, a cloud container) the context is Mesa's
//! llvmpipe: absolute numbers are CPU rasterisation, but the *ranking* of
//! scenes and the effect of a renderer change hold.

mod bisect;
mod probe;
mod readback;

use anyhow::{Context as _, Result, anyhow};
use clap::{Parser, ValueEnum};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;
use strobe_gfx::gl::Gl;
use strobe_gfx::offscreen::{self, ContextMode, Request};
use strobe_gfx::{
    Api, Backend, Caps, FloatPrecision, FrameInputs, GlChoice, Mouse, Options, ScenePlayer,
};
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
    /// Where the GL context comes from: an EGL device with no window
    /// system (`headless`), one hidden window (`window`), or headless
    /// when a device exists and the window otherwise (`auto`).
    #[arg(long, value_enum, default_value_t = ContextArg::Auto)]
    context: ContextArg,
    /// GL backend for `--context window`: the platform's OpenGL 3.3
    /// core (`native`), ANGLE's OpenGL ES 3.0 (`angle`), or native then
    /// ANGLE (`auto`).
    #[arg(long, value_enum, default_value_t = GlArg::Auto)]
    gl: GlArg,
    /// Use a tiny undecorated window parked off-screen instead of a
    /// hidden one. Hidden is tried first anyway, and a tiny window is the
    /// fallback when a platform will not give a hidden one a GL surface.
    #[arg(long)]
    show_window: bool,
    /// Window contexts only: the stand-in window's client size,
    /// WIDTHxHEIGHT (default 16x16). The render never depends on it; use
    /// it to see whether a driver that draws black above some size limits
    /// work to the window (D296).
    #[arg(long)]
    window_size: Option<String>,
    /// Context dialect for headless (default desktop). In window mode the
    /// backend decides it, and this must agree: `gles` means ANGLE.
    #[arg(long, value_enum)]
    api: Option<ApiArg>,
    /// RGBA32F instead of the default RGBA16F for FLOAT passes.
    #[arg(long)]
    full: bool,
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
    /// Check the `textureFinalPass` readback on this context (latency,
    /// cost, busy GPU, resize, lost context) instead of benchmarking the
    /// library; writes `readback.md` (C-04).
    #[arg(long)]
    readback: bool,
    /// Probe the driver at a ladder of sizes (plus `--size`) instead of
    /// benchmarking: does it clear, draw into and read back a target, and
    /// what viewport does it keep. Writes `probe.md`. For a machine whose
    /// frames come out black (D296).
    #[arg(long)]
    probe: bool,
    /// D296 experiment (throwaway): bisect why scene draws into targets
    /// over 16384 pixels never land while the probe's do. Uses the first
    /// scene `--filter` matches in `--library` (05 when no filter) and
    /// `--size` as the failing size. Writes `bisect.md`.
    #[arg(long)]
    bisect: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ApiArg {
    Desktop,
    Gles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ContextArg {
    Auto,
    Headless,
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum GlArg {
    Auto,
    Native,
    Angle,
}

impl ApiArg {
    fn api(self) -> Api {
        match self {
            Self::Desktop => Api::DesktopCore33,
            Self::Gles => Api::Gles30,
        }
    }
}

impl ContextArg {
    fn mode(self) -> ContextMode {
        match self {
            Self::Auto => ContextMode::Auto,
            Self::Headless => ContextMode::Headless,
            Self::Window => ContextMode::Window,
        }
    }
}

impl GlArg {
    fn choice(self) -> GlChoice {
        match self {
            Self::Auto => GlChoice::Auto,
            Self::Native => GlChoice::Only(Backend::Native),
            Self::Angle => GlChoice::Only(Backend::Angle),
        }
    }
}

/// One scene's results.
struct Row {
    name: String,
    /// The scene has a `script.js`, which the bench does not run (C-05).
    script: bool,
    compile_ms: f64,
    mean_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    image: Option<image::RgbaImage>,
    error: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // The renderer says things worth seeing in a benchmark run (a pass
    // double-buffered because it reads itself, D203; an asset that would
    // not load). To stderr only, filtered by `STROBE_LOG`; no session file.
    let _ = strobe_diag::logger::init_with(strobe_diag::logger::LoggerConfig {
        filter: strobe_diag::filter::Filter::from_env(),
        dir: None,
        stderr: true,
    });
    let size = parse_size(&cli.size)?;
    let window_size = cli
        .window_size
        .as_deref()
        .map(parse_size)
        .transpose()?
        .map(|[w, h]| [w.max(1) as u32, h.max(1) as u32]);
    let request = Request {
        mode: cli.context.mode(),
        api: cli.api.map(ApiArg::api),
        gl: cli.gl.choice(),
        tiny: cli.show_window,
        window_size,
    };
    offscreen::run(
        &request,
        |why| println!("{why}"),
        |gl, caps, context| run(gl, caps, context, size, &cli),
    )?
}

/// The benchmark proper, on whichever context is current. `context`
/// says where the context came from, for the report.
fn run(gl: &Gl, caps: &Caps, context: &str, size: [i32; 2], cli: &Cli) -> Result<()> {
    let options = Options {
        specialise_passes: !cli.shared,
        float_precision: if cli.full {
            FloatPrecision::Full
        } else {
            FloatPrecision::Half
        },
    };
    println!(
        "{} | {} | {:?} | {context} | {}x{} | {} | float {:?}",
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

    // Context creation can leave an error flag set that is not the
    // scenes' doing.
    gl.clear_errors();
    std::fs::create_dir_all(&cli.out).with_context(|| format!("creating {}", cli.out.display()))?;
    if cli.bisect {
        let filter = cli.filter.as_deref().unwrap_or("05");
        let scene_dir = strobe_scene::load::scan(&cli.library)?
            .into_iter()
            .find(|dir| strobe_scene::load::folder_title(dir).contains(filter))
            .ok_or_else(|| anyhow!("no scene in {} matches {filter}", cli.library.display()))?;
        let report = bisect::run(gl, caps, context, size, &scene_dir, &cli.out)?;
        println!("{report}");
        std::fs::write(cli.out.join("bisect.md"), &report)?;
        return Ok(());
    }
    if cli.probe {
        let (report, passed) = probe::run(gl, caps, context, size)?;
        println!("{report}");
        std::fs::write(cli.out.join("probe.md"), &report)?;
        let trace = gl.trace_summary();
        if !trace.is_empty() {
            println!("GL trace (STROBE_GL_TRACE):\n{trace}");
        }
        return if passed {
            Ok(())
        } else {
            Err(anyhow!(
                "the driver dropped work at some size; see probe.md"
            ))
        };
    }
    if cli.readback {
        let (report, passed) = readback::run(gl, caps, context, size)?;
        println!("{report}");
        std::fs::write(cli.out.join("readback.md"), &report)?;
        return if passed {
            Ok(())
        } else {
            Err(anyhow!("the readback check failed; see readback.md"))
        };
    }
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
        let row = bench_scene(gl, caps, &dir, &name, size, options, cli);
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
            if is_black(image) {
                let clear = image.pixels().filter(|px| px.0[3] == 0).count();
                println!(
                    "{name:<28} warning: the frame is all black ({clear} of {} pixels have alpha 0)",
                    image.pixels().len()
                );
            }
            image.save(cli.out.join(format!("{name}.png")))?;
        }
        rows.push(row);
    }

    let trace = gl.trace_summary();
    if !trace.is_empty() {
        println!("\nGL trace (STROBE_GL_TRACE):\n{trace}");
    }
    write_contact_sheet(&rows, &cli.out.join("contact-sheet.png"))?;
    let report = report(&rows, caps, context, size, options, cli);
    std::fs::write(cli.out.join("results.md"), &report)?;
    println!("\nwrote {}", cli.out.display());
    // Every frame and the report are written first, so a failing run still
    // leaves its evidence; then the exit status says it failed, which is
    // what lets CI's bench step catch a scene that no longer compiles (D56).
    let failed: Vec<&str> = rows
        .iter()
        .filter(|row| row.error.is_some())
        .map(|row| row.name.as_str())
        .collect();
    // A context that draws nothing still times: on the projector mini-PC,
    // a hidden window in a Windows service's session 0 gave all-black
    // frames at a tenth of the real frame time, and the run looked fine.
    // Every frame black means no number in the report is real.
    let images: Vec<&image::RgbaImage> = rows.iter().filter_map(|row| row.image.as_ref()).collect();
    if !images.is_empty() && images.iter().all(|image| is_black(image)) {
        return Err(anyhow!(
            "every frame is all black, so the context drew nothing and the \
             timings are not real (a window in a session nobody sees, such as \
             a Windows service's? A driver that drops work above some size? \
             `--probe` finds the size, `STROBE_GL_TRACE=1` shows where the \
             renderer drew)"
        ));
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "{} of {} scenes failed: {}",
            failed.len(),
            rows.len(),
            failed.join(", ")
        ))
    }
}

/// Whether every pixel of `image` is black (alpha ignored).
fn is_black(image: &image::RgbaImage) -> bool {
    image
        .pixels()
        .all(|pixel| pixel.0[..3].iter().all(|&c| c == 0))
}

fn parse_size(text: &str) -> Result<[i32; 2]> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow!("size must be WIDTHxHEIGHT, got {text}"))?;
    Ok([w.trim().parse()?, h.trim().parse()?])
}

fn bench_scene(
    gl: &Gl,
    caps: &Caps,
    dir: &Path,
    name: &str,
    size: [i32; 2],
    options: Options,
    cli: &Cli,
) -> Row {
    let script = dir.join("script.js").is_file();
    let failed = |error: String| Row {
        name: name.to_string(),
        script,
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
    // D296 experiment (throwaway): read the output back after every frame.
    let read_each = std::env::var_os("STROBE_D296_READ_EACH_FRAME").is_some();
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
        if read_each {
            drop(player.read_output());
        }
        if frame >= u64::from(cli.warmup) {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    if let Some(code) = gl.error() {
        return failed(format!("GL error 0x{code:x}"));
    }

    let pixels = player.read_output();
    // A scene that pins its final pass (D280) renders at its own size.
    let [width, height] = player.render_size();
    let image = image::RgbaImage::from_raw(width as u32, height as u32, pixels);
    times.sort_by(f64::total_cmp);
    let mean_ms = times.iter().sum::<f64>() / times.len().max(1) as f64;
    let p95_ms = times
        .get(((times.len() as f64) * 0.95) as usize)
        .or(times.last())
        .copied()
        .unwrap_or(0.0);
    Row {
        name: name.to_string(),
        script,
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
fn report(
    rows: &[Row],
    caps: &Caps,
    context: &str,
    size: [i32; 2],
    options: Options,
    cli: &Cli,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Strobe benchmark\n");
    let _ = writeln!(
        out,
        "- Renderer: {} ({})\n- Context: {}\n- Dialect: {:?}\n- Resolution: {}x{}\n- Programs: {}\n- Float passes: {:?}\n- Frames: {} timed after {} warm-up\n",
        caps.info.renderer,
        caps.info.version,
        context,
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
    let scripted = rows.iter().filter(|row| row.script).count();
    if scripted > 0 {
        let _ = writeln!(
            out,
            "**Scripts are not run.** The {scripted} scene(s) marked * have a `script.js`; \
             here they render with their uniforms' defaults, so their times are not what \
             they cost in the app. `cargo xtask corpus` runs scripts and reports their \
             frame time.\n"
        );
    }
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
                    "| {}{} | failed: {} | | | | |",
                    row.name,
                    if row.script { " *" } else { "" },
                    error.replace('\n', " ")
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "| {}{} | {:.1} | {:.2} | {:.2} | {:.2} | {:.1} |",
                    row.name,
                    if row.script { " *" } else { "" },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_frame_with_no_colour_at_all_is_black() -> Result<()> {
        let black = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255]));
        assert!(is_black(&black), "all zero is black");
        let mut one = black.clone();
        one.put_pixel(3, 3, image::Rgba([0, 0, 1, 255]));
        assert!(!is_black(&one), "one lit pixel is not black");
        Ok(())
    }
}
