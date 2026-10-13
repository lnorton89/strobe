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
    /// The candidate's id in the engine's analysis, for a review to name it
    /// again (D219).
    #[serde(default)]
    pub candidate: usize,
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
    /// The scene folder that was measured (never written).
    #[serde(default)]
    pub source: PathBuf,
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
                    candidate: c.id,
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
                | Verdict::Weak { .. }
                | Verdict::QualityLimit { .. }
                | Verdict::Unfaithful { .. }
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
        source: args.scene.clone(),
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

/// The operator's edit of one generated control in the review (D219),
/// parallel to [`Summary::kept`].
#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    /// Whether the control stays.
    pub keep: bool,
    /// The control's name, which the operator may change (D246). Checked
    /// when applied: a name the scene cannot declare is refused there.
    pub name: String,
    /// A slider's lower bound.
    pub min: f32,
    /// A slider's upper bound.
    pub max: f32,
}

impl Review {
    /// The review of `kept` as the engine proposed it: kept, its own range.
    pub fn of(kept: &Kept) -> Self {
        Self {
            keep: true,
            name: kept.name.clone(),
            min: kept.min.first().copied().unwrap_or(0.0),
            max: kept.max.first().copied().unwrap_or(1.0),
        }
    }
}

/// What applying a review did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reviewed {
    /// Nothing was changed; the copy stands as written.
    AsWritten,
    /// The copy was written again with the kept controls and ranges.
    Rewritten,
    /// Every control was dropped, so the copy was deleted.
    Discarded,
}

/// Applies `reviews` (one per [`Summary::kept`]) to the copy the worker
/// wrote: written again with only the kept controls and their ranges
/// (`strobe_autocontrol::rewrite_chosen`, no measuring), or deleted when
/// none is kept. The measured source is read, never written.
///
/// # Errors
///
/// When the source changed since it was measured, or a file cannot be
/// read or written; the copy is then left as it was.
pub fn review(summary: &Summary, reviews: &[Review], library: &Path) -> Result<Reviewed, String> {
    let Some(written) = summary.written.as_ref() else {
        return Ok(Reviewed::AsWritten);
    };
    let proposed = summary.kept.iter().map(Review::of);
    let unchanged =
        reviews.len() == summary.kept.len() && proposed.zip(reviews).all(|(a, b)| a == *b);
    if unchanged {
        return Ok(Reviewed::AsWritten);
    }
    let choices: Vec<strobe_autocontrol::Choice> = summary
        .kept
        .iter()
        .zip(reviews)
        .filter(|(_, review)| review.keep)
        .map(|(kept, review)| strobe_autocontrol::Choice {
            candidate: kept.candidate,
            name: review.name.trim().to_string(),
            context: kept.context.clone(),
            min: review.min,
            max: review.max,
        })
        .collect();
    if choices.is_empty() {
        discard(library, written).map_err(|e| e.to_string())?;
        return Ok(Reviewed::Discarded);
    }
    let folder = SceneFolder::load(&summary.source)
        .map_err(|e| format!("could not read {}: {e}", summary.source.display()))?;
    let generated =
        strobe_autocontrol::rewrite_chosen(&folder, &choices).map_err(|e| e.to_string())?;
    let parent = written.parent().unwrap_or(library);
    write_scene(&summary.source, parent, &generated, true)
        .map_err(|e| format!("could not write the copy again: {e}"))?;
    Ok(Reviewed::Rewritten)
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
            source: PathBuf::from("lib/plasma.synScene"),
            written: Some(PathBuf::from("lib/plasma-auto.synScene")),
            context: "headless EGL device | llvmpipe".to_owned(),
            candidates: 12,
            fidelity: Some(0.0),
            kept: vec![Kept {
                candidate: 0,
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

    #[test]
    fn a_review_rewrites_the_copy_drops_it_or_leaves_it() -> Result<()> {
        let root = std::env::temp_dir().join(format!("strobe-review-{}", std::process::id()));
        let library = root.join("lib");
        let source = library.join("pulse.synScene");
        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("scene.json"), "{}\n")?;
        std::fs::write(
            source.join("main.glsl"),
            "vec4 renderMain() {\n\
             \x20   float t = TIME * 0.5;\n\
             \x20   float rings = 12.0;\n\
             \x20   float d = length(_uv - 0.5);\n\
             \x20   vec3 tint = vec3(0.9, 0.4, 0.2);\n\
             \x20   return vec4(tint * (0.5 + 0.5 * sin(d * rings - t * 3.0)), 1.0);\n\
             }\n",
        )?;
        let folder = SceneFolder::load(&source)?;
        let candidates =
            strobe_autocontrol::analyze(&strobe_autocontrol::source::Sources::from_folder(&folder))
                .candidates;
        let kept: Vec<Kept> = candidates
            .iter()
            .take(2)
            .enumerate()
            .map(|(i, c)| Kept {
                candidate: c.id,
                name: format!("knob_{i}"),
                group: c.group.ui_name().to_owned(),
                kind: c.kind.label().to_owned(),
                default: c.default.clone(),
                min: c.min.clone(),
                max: c.max.clone(),
                score: Some(0.1),
                context: c.context.trim().to_owned(),
            })
            .collect();
        anyhow::ensure!(
            kept.len() == 2,
            "the scene has a speed and a colour: {candidates:?}"
        );
        // What the worker would have written.
        let choices: Vec<strobe_autocontrol::Choice> = kept
            .iter()
            .map(|k| strobe_autocontrol::Choice {
                candidate: k.candidate,
                name: k.name.clone(),
                context: k.context.clone(),
                min: k.min.first().copied().unwrap_or(0.0),
                max: k.max.first().copied().unwrap_or(1.0),
            })
            .collect();
        let generated = strobe_autocontrol::rewrite_chosen(&folder, &choices)?;
        let written = write_scene(&source, &library, &generated, false)?;
        let summary = Summary {
            scene: "Pulse".to_owned(),
            source: source.clone(),
            written: Some(written.clone()),
            context: String::new(),
            candidates: candidates.len(),
            fidelity: Some(0.0),
            kept: kept.clone(),
            suggestions: Vec::new(),
        };
        let proposed: Vec<Review> = kept.iter().map(Review::of).collect();
        assert_eq!(
            review(&summary, &proposed, &library).map_err(anyhow::Error::msg)?,
            Reviewed::AsWritten,
            "nothing edited, nothing written"
        );

        let mut edited = proposed.clone();
        edited[1].keep = false;
        edited[0].max = edited[0].min + 42.0;
        assert_eq!(
            review(&summary, &edited, &library).map_err(anyhow::Error::msg)?,
            Reviewed::Rewritten,
            "edited, so written again"
        );
        let copy = SceneFolder::load(&written)?;
        let names: Vec<&str> = copy.def.controls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["knob_0"], "only the kept control is left");
        let text = std::fs::read_to_string(written.join("scene.json"))?;
        assert!(
            text.contains(&format!("{:?}", edited[0].max)),
            "with its new range: {text}"
        );
        assert!(
            std::fs::read_to_string(source.join("main.glsl"))?.contains("TIME * 0.5"),
            "the source is untouched"
        );

        // A rename is written under the new name (D246); one the scene
        // cannot declare is refused with the reason, and the copy stays.
        edited[0].name = " pulse_rate ".to_owned();
        assert_eq!(
            review(&summary, &edited, &library).map_err(anyhow::Error::msg)?,
            Reviewed::Rewritten,
            "renamed, so written again"
        );
        let copy = SceneFolder::load(&written)?;
        let names: Vec<&str> = copy.def.controls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["pulse_rate"],
            "the control has its new name, trimmed"
        );
        edited[0].name = "vec2".to_owned();
        let refused = review(&summary, &edited, &library);
        assert!(
            refused
                .as_ref()
                .is_err_and(|why| why.contains("cannot be a control's name")),
            "a GLSL type is refused: {refused:?}"
        );
        assert!(written.is_dir(), "a refused rename leaves the copy");
        edited[0].name = kept[0].name.clone();

        let none: Vec<Review> = proposed
            .iter()
            .map(|r| Review {
                keep: false,
                ..r.clone()
            })
            .collect();
        assert_eq!(
            review(&summary, &none, &library).map_err(anyhow::Error::msg)?,
            Reviewed::Discarded,
            "nothing kept, so the copy goes"
        );
        assert!(!written.exists(), "deleted");
        assert!(source.is_dir(), "and the source stays");
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }
}
