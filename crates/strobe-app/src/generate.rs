// ============================================================================
// Module:       strobe_app::generate
// Description:  Generated controls from inside the app: a child process of the
//               same executable runs strobe-autocontrol on its own context and
//               writes `<name>-auto.synScene`; the app polls it, never blocks.
//
// Dependencies: strobe-autocontrol, strobe-gfx (offscreen), serde_json;
//               strobe-diag (the data directory)
// ============================================================================

//! "Generate controls", the app's side (backlog U-03 and I-03, D202).
//!
//! `strobe-autocontrol` scores candidate controls by rendering the scene
//! many times. That cannot happen on the app's own context: the frame loop
//! would stop for seconds on an old laptop, and the output is on a
//! projector. So the app starts **itself** again as a worker,
//! `strobe --generate-controls <scene> --generate-library <dir>
//! --generate-report <file>`. The worker:
//! - opens its own offscreen context (`strobe_gfx::offscreen`: an EGL
//!   device, or one hidden window on Windows);
//! - runs the engine and writes the result as a **new** scene beside the
//!   library, `<name>-auto.synScene`, a copy whose licence and attribution
//!   files travel with it;
//! - writes a [`Summary`] as JSON and exits.
//!
//! The source scene is never written. The app polls the child once a frame
//! with [`Job::poll`] and shows the summary for review. The operator then
//! opens the new scene or discards it ([`discard`]), which deletes only a
//! folder this module wrote.
//!
//! A process boundary also means a driver crash during scoring takes down
//! the worker, not the show.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use strobe_autocontrol::report::Verdict;
use strobe_autocontrol::{Options, authored_controls, generate, write_scene};
use strobe_gfx::offscreen::{self, ContextMode, Request};
use strobe_scene::SceneFolder;

/// The flag that makes the executable a worker.
pub const WORKER_FLAG: &str = "--generate-controls";

/// The longest a worker may run before it is stopped. Scoring a heavy scene
/// on an HD 4000 takes tens of seconds; minutes means something is stuck.
const MAX_RUNTIME: Duration = Duration::from_secs(10 * 60);

/// Numbers each worker's report file within this process.
static NEXT_REPORT: AtomicU64 = AtomicU64::new(0);

/// The suffix every generated scene folder carries ([`write_scene`]).
const GENERATED_SUFFIX: &str = "-auto.synScene";

/// One generated control, as the review shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    /// The control's name.
    pub name: String,
    /// The group it is listed in.
    pub group: String,
    /// What the literal was doing (`colour`, `time scale`...).
    pub kind: String,
    /// The original value, which is the control's default.
    pub default: Vec<f32>,
    /// Lower bound.
    pub min: Vec<f32>,
    /// Upper bound.
    pub max: Vec<f32>,
    /// How much the picture moved when it was nudged, `0..1`.
    pub score: Option<f32>,
    /// The line of code it came from.
    pub context: String,
}

/// What a worker reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// The scene's title.
    pub scene: String,
    /// The new scene folder, when one was written.
    pub written: Option<PathBuf>,
    /// Where the scoring context came from.
    pub context: String,
    /// How many literals the analysis found.
    pub candidates: usize,
    /// The image difference between the original and the rewrite with every
    /// control at its default; zero when faithful.
    pub fidelity: Option<f32>,
    /// The controls made, strongest first.
    pub kept: Vec<Kept>,
    /// Suggested audio pairings, one line each, for the review.
    pub suggestions: Vec<String>,
}

/// A worker's whole answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// It ran.
    Done(Summary),
    /// It could not.
    Failed {
        /// Why, in a sentence.
        message: String,
    },
}

/// What the worker was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerArgs {
    /// The scene folder.
    pub scene: PathBuf,
    /// Where `<name>-auto.synScene` goes.
    pub library: PathBuf,
    /// Where the [`Outcome`] goes.
    pub report: PathBuf,
    /// Add to authored controls instead of refusing.
    pub merge: bool,
}

/// The worker: runs the engine and writes the outcome. Returns whether it
/// produced one (the exit status).
pub fn worker(args: &WorkerArgs) -> bool {
    let outcome = match run_worker(args) {
        Ok(summary) => Outcome::Done(summary),
        Err(message) => Outcome::Failed { message },
    };
    match write_outcome(&args.report, &outcome) {
        Ok(()) => true,
        Err(error) => {
            log::error!("could not write {}: {error}", args.report.display());
            false
        }
    }
}

fn run_worker(args: &WorkerArgs) -> Result<Summary, String> {
    let folder = SceneFolder::load(&args.scene)
        .map_err(|e| format!("could not read {}: {e}", args.scene.display()))?;
    let authored = authored_controls(&folder.def);
    if !authored.is_empty() && !args.merge {
        return Err(format!(
            "{} already has {} control(s) of its own; choose \"Add controls\" to keep them and add more",
            folder.def.title,
            authored.len()
        ));
    }
    let options = Options {
        merge: args.merge,
        ..Options::default()
    };
    let request = Request {
        mode: ContextMode::Auto,
        ..Request::default()
    };
    let (generated, context) = offscreen::run(
        &request,
        |why| log::info!("generate controls: {why}"),
        |gl, caps, context| {
            let line = format!("{context} | {}", caps.info.renderer);
            generate(gl, caps, &folder, &options).map(|g| (g, line))
        },
    )
    .map_err(|e| format!("no GL context to measure with: {e}"))?
    .map_err(|e| e.to_string())?;

    let written = if generated.control_lines.is_empty() {
        None
    } else {
        Some(
            write_scene(&args.scene, &args.library, &generated, true)
                .map_err(|e| format!("could not write the new scene: {e}"))?,
        )
    };
    let report = &generated.report;
    let mut kept: Vec<(usize, Kept)> = report
        .candidates
        .iter()
        .filter_map(|c| match (&c.verdict, &c.name) {
            (Verdict::Kept { rank }, Some(name)) => Some((
                *rank,
                Kept {
                    name: name.clone(),
                    group: c.group.ui_name().to_owned(),
                    kind: c.kind.label().to_owned(),
                    default: c.default.clone(),
                    min: c.min.clone(),
                    max: c.max.clone(),
                    score: c.score,
                    context: c.context.trim().to_owned(),
                },
            )),
            (
                Verdict::Kept { .. }
                | Verdict::Cut { .. }
                | Verdict::Dead
                | Verdict::Broken { .. }
                | Verdict::DoesNotCompile { .. }
                | Verdict::NotScored,
                _,
            ) => None,
        })
        .collect();
    kept.sort_by_key(|(rank, _)| *rank);
    Ok(Summary {
        scene: report.scene.clone(),
        written,
        context,
        candidates: report.candidates.len(),
        fidelity: report.fidelity,
        kept: kept.into_iter().map(|(_, k)| k).collect(),
        suggestions: report
            .suggestions
            .iter()
            .map(|s| format!("{} with {}: {}", s.control, s.band, s.reason))
            .collect(),
    })
}

/// Writes `outcome` beside its final name and renames it into place, so the
/// app never reads half a file.
fn write_outcome(path: &Path, outcome: &Outcome) -> io::Result<()> {
    let text = serde_json::to_string_pretty(outcome).map_err(io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

/// A running worker.
#[derive(Debug)]
pub struct Job {
    child: Child,
    report: PathBuf,
    started: Instant,
    /// The scene being worked on, for the progress line.
    pub scene: String,
}

impl Job {
    /// Starts a worker on `scene`, writing into `library`. `exe` is this
    /// executable.
    ///
    /// # Errors
    ///
    /// When the process cannot be started.
    pub fn spawn(exe: &Path, scene: &Path, library: &Path, merge: bool) -> io::Result<Self> {
        let dir = strobe_diag::paths::base_dir().join("generate");
        std::fs::create_dir_all(&dir)?;
        let serial = NEXT_REPORT.fetch_add(1, Ordering::Relaxed);
        let report = dir.join(format!("{}-{serial}.json", std::process::id()));
        let _ = std::fs::remove_file(&report);
        let mut command = Command::new(exe);
        command
            .arg(WORKER_FLAG)
            .arg(scene)
            .arg("--generate-library")
            .arg(library)
            .arg("--generate-report")
            .arg(&report)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if merge {
            command.arg("--generate-merge");
        }
        let child = command.spawn()?;
        Ok(Self {
            child,
            report,
            started: Instant::now(),
            scene: strobe_scene::load::folder_title(scene),
        })
    }

    /// How long it has run.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// The outcome, once the worker has ended; `None` while it runs. Never
    /// blocks.
    pub fn poll(&mut self) -> Option<Outcome> {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => status,
            Ok(None) if self.started.elapsed() > MAX_RUNTIME => {
                self.stop();
                return Some(Outcome::Failed {
                    message: format!(
                        "stopped after {} minutes with no result",
                        MAX_RUNTIME.as_secs() / 60
                    ),
                });
            }
            Ok(None) => return None,
            Err(error) => {
                return Some(Outcome::Failed {
                    message: format!("lost track of the worker: {error}"),
                });
            }
        };
        let read = std::fs::read_to_string(&self.report)
            .ok()
            .and_then(|text| serde_json::from_str::<Outcome>(&text).ok());
        let _ = std::fs::remove_file(&self.report);
        Some(read.unwrap_or_else(|| Outcome::Failed {
            message: format!(
                "the worker ended ({status}) without a result; the session log has its last words"
            ),
        }))
    }

    /// Stops the worker and waits for it.
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.report);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // A worker must not outlive the app that started it.
        if matches!(self.child.try_wait(), Ok(None)) {
            self.stop();
        }
    }
}

/// Deletes a scene this module wrote. Refuses anything that is not a
/// `*-auto.synScene` folder directly inside `library`, so a bad path can
/// never delete a source scene.
///
/// # Errors
///
/// When the folder is not one this module writes, or cannot be removed.
pub fn discard(library: &Path, written: &Path) -> io::Result<()> {
    let name = written
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let inside = written
        .parent()
        .zip(std::fs::canonicalize(library).ok())
        .and_then(|(parent, library)| {
            std::fs::canonicalize(parent)
                .ok()
                .map(|parent| parent == library)
        })
        .unwrap_or(false);
    if !name.ends_with(GENERATED_SUFFIX) || !inside {
        return Err(io::Error::other(format!(
            "{} is not a generated scene in the library",
            written.display()
        )));
    }
    std::fs::remove_dir_all(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn an_outcome_round_trips_through_json() -> Result<()> {
        let outcome = Outcome::Done(Summary {
            scene: "Plasma".to_owned(),
            written: Some(PathBuf::from("lib/plasma-auto.synScene")),
            context: "headless EGL device | llvmpipe".to_owned(),
            candidates: 12,
            fidelity: Some(0.0),
            kept: vec![Kept {
                name: "speed".to_owned(),
                group: "Motion".to_owned(),
                kind: "time scale".to_owned(),
                default: vec![0.5],
                min: vec![0.0],
                max: vec![2.0],
                score: Some(0.12),
                context: "float t = TIME * 0.5;".to_owned(),
            }],
            suggestions: vec!["speed with syn_BassLevel: bass drives motion".to_owned()],
        });
        let text = serde_json::to_string(&outcome)?;
        assert_eq!(
            serde_json::from_str::<Outcome>(&text)?,
            outcome,
            "the app reads what the worker wrote"
        );
        let failed = Outcome::Failed {
            message: "nope".to_owned(),
        };
        assert!(
            serde_json::to_string(&failed)?.contains("\"outcome\":\"failed\""),
            "tagged"
        );
        Ok(())
    }

    #[test]
    fn discard_only_removes_generated_scenes_inside_the_library() -> Result<()> {
        let root = std::env::temp_dir().join(format!("strobe-discard-{}", std::process::id()));
        let library = root.join("lib");
        let source = library.join("plasma.synScene");
        let generated = library.join("plasma-auto.synScene");
        let elsewhere = root.join("other-auto.synScene");
        for dir in [&source, &generated, &elsewhere] {
            std::fs::create_dir_all(dir)?;
        }
        assert!(
            discard(&library, &source).is_err(),
            "a source scene is refused"
        );
        assert!(
            discard(&library, &elsewhere).is_err(),
            "a generated-looking folder outside the library is refused"
        );
        assert!(source.is_dir() && elsewhere.is_dir(), "neither was touched");
        discard(&library, &generated)?;
        assert!(!generated.exists(), "the generated scene is gone");
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }
}
