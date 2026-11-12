// ============================================================================
// Module:       strobe_forge (binary crate root)
// Description:  The scene forge: `check` renders and judges a scene the way
//               the scene-from-image skill asks, `make` runs the
//               picture-to-scene LLM loop around it.
//
// Dependencies: clap, anyhow; strobe-gfx (offscreen contexts), strobe-diag
//               (stderr log); the modules below
// ============================================================================

//! `strobe-forge` (D562).
//!
//! ```text
//! cargo run --profile release-fast -p strobe-forge -- check user-scenes/<slug>.synScene --reference <photo>
//! cargo run --profile release-fast -p strobe-forge -- make <photo> --out user-scenes/<slug>.synScene
//! ```
//!
//! `check` writes frames, a sheet beside the photo, a controls grid and a
//! report to `target/forge/<slug>/` ([`check`]); it exits non-zero only on
//! a hard failure (a compile error in either dialect, black, frozen,
//! flashing). `make` sends the picture and the skill to a model and loops
//! over `check` ([`forge`]); it needs `ANTHROPIC_API_KEY` and the default
//! `llm` feature. Both open one offscreen context the way `strobe-bench`
//! does ([`strobe_gfx::offscreen`]): headless where an EGL device exists,
//! else one hidden window.

#[cfg(feature = "llm")]
mod anthropic;
mod author;
mod check;
mod dialect;
#[cfg(test)]
mod fixture;
mod forge;
mod gate;
mod prompt;
mod reply;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use strobe_gfx::gl::Gl;
use strobe_gfx::offscreen::{self, ContextMode, Request};
use strobe_gfx::{Backend, Caps, GlChoice};

/// The command line.
#[derive(Parser, Debug)]
#[command(
    name = "strobe-forge",
    about = "Check a scene, or make one from a picture"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Where the GL context comes from.
#[derive(clap::Args, Debug, Clone)]
struct ContextArgs {
    /// `headless` (an EGL device), `window` (one hidden window), or `auto`.
    #[arg(long, value_enum, default_value_t = ContextArg::Auto)]
    context: ContextArg,
    /// Window backend: native OpenGL 3.3, ANGLE's GLES 3.0, or native then
    /// ANGLE.
    #[arg(long, value_enum, default_value_t = GlArg::Auto)]
    gl: GlArg,
}

/// What `check` and `make` share.
#[derive(clap::Args, Debug, Clone)]
struct CheckArgs {
    /// Frame size, WIDTHxHEIGHT.
    #[arg(long, default_value = "1280x720")]
    size: String,
    /// Seconds to write frames at, comma-separated.
    #[arg(long, default_value = "2,7,15,40,300")]
    times: String,
    /// Skip compiling the other GLSL dialect.
    #[arg(long)]
    no_dialect: bool,
    /// Skip the 1080p timing.
    #[arg(long)]
    no_timing: bool,
    #[command(flatten)]
    context: ContextArgs,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Render and judge one scene folder.
    Check {
        /// The `.synScene` folder.
        scene: PathBuf,
        /// The photo it recreates, for the sheet.
        #[arg(long)]
        reference: Option<PathBuf>,
        /// Output folder (default `target/forge/<slug>`).
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        args: CheckArgs,
    },
    /// Make a scene from a picture with a model, checking each answer.
    Make {
        /// The picture.
        image: PathBuf,
        /// The scene folder to write.
        #[arg(long)]
        out: PathBuf,
        /// The model.
        #[arg(long, default_value = author::DEFAULT_MODEL)]
        model: String,
        /// Critique rounds after the first scene.
        #[arg(long, default_value_t = 2)]
        rounds: u32,
        /// The skill folder whose files are the prompt.
        #[arg(long, default_value = ".claude/skills/scene-from-image")]
        skill: PathBuf,
        #[command(flatten)]
        args: CheckArgs,
    },
    /// Compile a scene in one dialect and print the result as JSON (what
    /// `check` runs in a child process).
    #[command(hide = true)]
    Dialect {
        scene: PathBuf,
        #[arg(long, value_enum)]
        api: ApiArg,
        #[arg(long, value_enum)]
        context: ContextArg,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ApiArg {
    Desktop,
    Gles,
}

impl ContextArgs {
    fn request(&self) -> Request {
        Request {
            mode: match self.context {
                ContextArg::Auto => ContextMode::Auto,
                ContextArg::Headless => ContextMode::Headless,
                ContextArg::Window => ContextMode::Window,
            },
            api: None,
            gl: match self.gl {
                GlArg::Auto => GlChoice::Auto,
                GlArg::Native => GlChoice::Only(Backend::Native),
                GlArg::Angle => GlChoice::Only(Backend::Angle),
            },
            tiny: false,
            window_size: None,
        }
    }
}

impl CheckArgs {
    fn options(&self, reference: Option<PathBuf>, out: PathBuf) -> Result<check::CheckOptions> {
        let (w, h) = self
            .size
            .split_once(['x', 'X'])
            .ok_or_else(|| anyhow!("--size must be WIDTHxHEIGHT, got {}", self.size))?;
        let times = self
            .times
            .split(',')
            .map(|t| {
                t.trim()
                    .parse::<f32>()
                    .map_err(|e| anyhow!("--times: {t}: {e}"))
            })
            .collect::<Result<Vec<_>>>()?;
        if times.iter().any(|t| !t.is_finite() || *t < 0.0) {
            return Err(anyhow!("--times must be seconds, zero or more"));
        }
        let size: [i32; 2] = [w.trim().parse()?, h.trim().parse()?];
        if size.iter().any(|side| *side < 1) {
            return Err(anyhow!(
                "--size must be at least 1x1 pixels, got {}",
                self.size
            ));
        }
        Ok(check::CheckOptions {
            size,
            times,
            reference,
            out,
            other_dialect: !self.no_dialect,
            timing: !self.no_timing,
        })
    }
}

fn main() -> Result<()> {
    let _ = strobe_gfx::gl::angle::choose_from_args(std::env::args_os());
    let cli = Cli::parse();
    let _ = strobe_diag::logger::init_with(strobe_diag::logger::LoggerConfig {
        filter: strobe_diag::filter::Filter::from_env(),
        dir: None,
        stderr: true,
        max_file_bytes: strobe_diag::logger::MAX_FILE_BYTES,
    });
    match cli.command {
        Command::Check {
            scene,
            reference,
            out,
            args,
        } => {
            let out = out.unwrap_or_else(|| check::default_out(&scene));
            let options = args.options(reference, out)?;
            let report = offscreen::run(
                &args.context.request(),
                |why| eprintln!("{why}"),
                |gl, caps, context| check::check(gl, caps, context, &scene, &options),
            )??;
            print_summary(&report, &options.out);
            if report.passed() {
                Ok(())
            } else {
                Err(anyhow!("hard failure: {}", report.failures.join("; ")))
            }
        }
        Command::Make {
            image,
            out,
            model,
            rounds,
            skill,
            args,
        } => {
            // Refuse before opening anything when there is no key.
            let mut author = author::from_env(&model)?;
            let skill = prompt::load_skill(&skill)?;
            let picture = prompt::image_part(&std::fs::read(&image)?)?;
            let options = args.options(Some(image.clone()), PathBuf::new())?;
            let outcome = offscreen::run(
                &args.context.request(),
                |why| eprintln!("{why}"),
                |gl, caps, context| {
                    let mut checker = GlChecker {
                        gl,
                        caps,
                        context,
                        options: &options,
                    };
                    forge::make(
                        author.as_mut(),
                        &mut checker,
                        &skill,
                        picture,
                        &out,
                        forge::MakeOptions { rounds, repairs: 3 },
                    )
                },
            )??;
            for line in &outcome.log {
                println!("{line}");
            }
            if let Some(round) = outcome.round {
                println!("round {round}'s scene stands in {}", out.display());
            }
            if outcome.passed {
                Ok(())
            } else {
                Err(anyhow!(
                    "no scene passed; see {}",
                    out.join("forge-log.md").display()
                ))
            }
        }
        Command::Dialect {
            scene,
            api,
            context,
        } => dialect::child(&scene, api == ApiArg::Gles, context == ContextArg::Headless),
    }
}

/// The real checker: [`check::check`] on the loop's one context.
struct GlChecker<'a> {
    gl: &'a Gl,
    caps: &'a Caps,
    context: &'a str,
    options: &'a check::CheckOptions,
}

impl forge::Checker for GlChecker<'_> {
    fn check(&mut self, scene: &Path, out: &Path) -> Result<forge::Verdict> {
        let mut options = self.options.clone();
        out.clone_into(&mut options.out);
        let report = check::check(self.gl, self.caps, self.context, scene, &options)?;
        let sheet = out.join("sheet.png");
        Ok(forge::Verdict {
            compile_error: report.compile_errors(),
            passed: report.passed(),
            report: check::markdown(&report),
            sheet: sheet.is_file().then_some(sheet),
        })
    }
}

/// What `check` prints: the verdict and where the outputs are.
fn print_summary(report: &check::Report, out: &Path) {
    println!("{} | {}", report.renderer, report.context);
    println!(
        "{}: {} controls, compile {:.1} ms",
        report.title, report.control_count, report.compile_ms
    );
    for d in &report.dialects {
        println!("  {}: {}", d.dialect, d.status.label());
    }
    if let Some(timing) = &report.timing {
        println!(
            "  timing {}x{}: lowest of three batch means {:.2} ms (p95 {:.2} ms; batches {})",
            timing.size[0],
            timing.size[1],
            timing.mean_ms,
            timing.p95_ms,
            timing
                .batch_means
                .iter()
                .map(|m| format!("{m:.2}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if let Some(gates) = &report.gates {
        let g = &gates.result;
        println!(
            "  gates: mean {:.3}, spread {:.3}, blown {:.3}, change {:.4}, flashes {:.2}/s",
            g.mean, g.spread, g.blown, g.change, g.flashes_per_second
        );
    }
    for warning in &report.warnings {
        println!("  warning: {warning}");
    }
    for failure in &report.failures {
        println!("  FAIL: {failure}");
    }
    println!(
        "{} -> {}",
        if report.passed() { "pass" } else { "FAIL" },
        out.display()
    );
}
