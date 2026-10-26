// ============================================================================
// Module:       strobe_autocontrol (binary crate root)
// Description:  `strobe-autocontrol`: generates a control set for a scene
//               folder and writes the result as a new scene, never in place.
//
// Dependencies: clap, anyhow; strobe_autocontrol (the library half),
//               strobe-gfx (offscreen contexts)
// ============================================================================

//! `strobe-autocontrol`.
//!
//! ```text
//! strobe-autocontrol scenes/12-warm.synScene
//! strobe-autocontrol scenes/12-warm.synScene --out generated --keep 6 --report
//! strobe-autocontrol scenes/12-warm.synScene --analyze-only      (no GPU needed)
//! strobe-autocontrol scenes/12-warm.synScene --context window    (no EGL device)
//! ```
//!
//! Reads the scene, finds the literals that could be controls, measures each
//! by rendering fixed frames headless with its value nudged, keeps the ones
//! that visibly change the picture, and writes
//! `<out>/<name>-auto.synScene`: a copy of the scene with the rewritten GLSL
//! and the new controls in `scene.json`. The original is never touched.
//!
//! A scene that already has authored controls is refused unless
//! `--force-merge` is given; then the authored controls stay and only new
//! names are added. The five "Strobe" controls of a Shadertoy import do not
//! count as authored.
//!
//! Scoring needs a GL context, and `--context auto|headless|window` (with
//! `--gl native|angle|auto` for the window) says where it comes from, with
//! the same meaning as in `strobe-bench`: `headless` is an EGL device with no
//! window system (CI, Mesa llvmpipe); `window` is one hidden window owning
//! the app's own context type, for Intel on Windows, which has no EGL device;
//! `auto`, the default, is headless if a device exists and the window
//! otherwise. Both run the same job on the same `&Gl` and `&Caps`
//! ([`strobe_gfx::offscreen`], D140), so the scores and kept controls agree.
//! `--analyze-only` opens no context at all.

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use strobe_autocontrol::analyze::analyze;
use strobe_autocontrol::generate::{Options, authored_controls, generate};
use strobe_autocontrol::output::write_scene;
use strobe_autocontrol::score::ScoreOptions;
use strobe_autocontrol::source::Sources;
use strobe_gfx::offscreen::{self, ContextMode, OffscreenError, Request};
use strobe_gfx::{Api, Backend, GlChoice};
use strobe_scene::SceneFolder;

/// The command line.
#[derive(Parser, Debug)]
#[command(
    name = "strobe-autocontrol",
    about = "Generate controls for a scene by finding and measuring the literals that shape it"
)]
struct Cli {
    /// The scene folder (`*.synScene`).
    scene: PathBuf,
    /// The scene library to write `<name>-auto.synScene` into. Defaults to
    /// the folder the scene is in.
    #[arg(long)]
    out: Option<PathBuf>,
    /// The most controls to generate.
    #[arg(long, default_value_t = 8)]
    keep: usize,
    /// Print every candidate with its scores and verdict, and write the
    /// report as JSON beside the result.
    #[arg(long)]
    report: bool,
    /// Add to a scene that already has authored controls (they are kept;
    /// only new names are added).
    #[arg(long)]
    force_merge: bool,
    /// Replace an earlier result at the destination.
    #[arg(long)]
    replace: bool,
    /// Measure and report, but write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Only list the candidates found by static analysis. Needs no GPU and
    /// writes nothing.
    #[arg(long)]
    analyze_only: bool,
    /// Write the suggested audio mappings into the GLSL. Their amount
    /// controls default to 0, so the scene looks the same until raised.
    #[arg(long)]
    audio_bindings: bool,
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
    /// hidden one, for a platform that will not give a hidden window a GL
    /// surface. Hidden is tried first anyway.
    #[arg(long)]
    show_window: bool,
    /// The GL dialect to measure in (default desktop). In window mode the
    /// backend decides it, and this must agree: `gles` means ANGLE.
    #[arg(long, value_enum)]
    api: Option<ApiArg>,
    /// Scoring resolution, WIDTHxHEIGHT.
    #[arg(long, default_value = "160x90")]
    size: String,
    /// Frames rendered per probe.
    #[arg(long, default_value_t = 10)]
    frames: u32,
    /// The least mean per-channel difference (0 to 1) that counts as visible.
    #[arg(long, default_value_t = 0.003)]
    threshold: f32,
    /// A candidate under this fraction of the strongest one's score, and
    /// under 0.01, is weak and not kept.
    #[arg(long, default_value_t = 0.05)]
    relative: f32,
    /// The most candidates to measure.
    #[arg(long, default_value_t = 40)]
    max_scored: usize,
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
    /// ANGLE on Direct3D 11 only (ANGLE's default on Windows, named).
    AngleD3d11,
    /// ANGLE on Vulkan only: the GPU's Vulkan driver under GLES (D485).
    AngleVulkan,
    /// ANGLE on the desktop OpenGL driver only.
    AngleGl,
    /// ANGLE on Metal only (macOS).
    AngleMetal,
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
            Self::Angle
            | Self::AngleD3d11
            | Self::AngleVulkan
            | Self::AngleGl
            | Self::AngleMetal => GlChoice::Only(Backend::Angle),
        }
    }
}

fn parse_size(text: &str) -> Result<[i32; 2]> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow!("size must be WIDTHxHEIGHT, got {text}"))?;
    Ok([w.trim().parse()?, h.trim().parse()?])
}

fn main() -> Result<()> {
    // First, before anything can start a thread: `--gl angle-*` sets ANGLE's
    // backend through the environment (D485).
    let _ = strobe_gfx::gl::angle::choose_from_args(std::env::args_os());
    let cli = Cli::parse();
    let folder = SceneFolder::load(&cli.scene)
        .with_context(|| format!("loading {}", cli.scene.display()))?;

    if cli.analyze_only {
        let analysis = analyze(&Sources::from_folder(&folder));
        println!(
            "{}: {} candidates",
            folder.def.title,
            analysis.candidates.len()
        );
        for c in &analysis.candidates {
            let place = c.primary().map_or_else(String::new, |s| {
                format!("{}:{}:{}", s.file, s.line, s.column)
            });
            println!(
                "#{:<3} {:<10} {:<8} {:<18} default {:?}  {}{}  | {}",
                c.id,
                c.kind.label(),
                c.group.ui_name(),
                c.name,
                c.default,
                place,
                if c.sites.len() > 1 {
                    format!(" (+{})", c.sites.len() - 1)
                } else {
                    String::new()
                },
                c.context
            );
        }
        return Ok(());
    }

    let authored = authored_controls(&folder.def);
    if !authored.is_empty() && !cli.force_merge {
        bail!(
            "{} already has {} authored control(s) ({}); pass --force-merge to add to them",
            cli.scene.display(),
            authored.len(),
            authored
                .iter()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let options = Options {
        keep: cli.keep,
        max_scored: cli.max_scored,
        score: ScoreOptions {
            size: parse_size(&cli.size)?,
            frames: cli.frames,
            threshold: cli.threshold,
            relative: cli.relative,
            ..ScoreOptions::default()
        },
        merge: cli.force_merge,
        bind_audio: cli.audio_bindings,
    };
    let request = Request {
        mode: cli.context.mode(),
        api: cli.api.map(ApiArg::api),
        gl: cli.gl.choice(),
        tiny: cli.show_window,
        window_size: None,
    };
    let generated = offscreen::run(
        &request,
        |why| println!("{why}"),
        |gl, caps, context| {
            println!(
                "context: {context} | {} | {}",
                caps.info.renderer, caps.info.version
            );
            generate(gl, caps, &folder, &options)
        },
    )
    .map_err(|e| match e {
        OffscreenError::Conflict(_) => anyhow!("{e}"),
        OffscreenError::Headless(_) | OffscreenError::Window(_) => anyhow!(
            "{e}\nscoring needs a GL context: --context window uses a hidden window where \
             there is no EGL device, and --analyze-only needs none"
        ),
    })??;

    if cli.report {
        print!("{}", generated.report.detail());
    } else {
        print!("{}", generated.report.summary());
    }
    if generated.control_lines.is_empty() {
        println!("no controls generated; nothing written");
        return Ok(());
    }
    if cli.dry_run {
        println!("dry run: nothing written");
        return Ok(());
    }

    let library = match &cli.out {
        Some(dir) => dir.clone(),
        None => cli
            .scene
            .parent()
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let written = write_scene(&cli.scene, &library, &generated, cli.replace)?;
    println!("wrote {}", written.display());
    if cli.report {
        let path = written.with_extension("report.json");
        std::fs::write(&path, generated.report.to_json())
            .with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
