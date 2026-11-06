// ============================================================================
// Module:       strobe_autocontrol::generate
// Description:  The engine end to end: analyse, rewrite, compile once, score by
//               measured effect, keep the best, name them, and rewrite again.
//
// Dependencies: crate::{analyze, rewrite, score, suggest, naming, report}, strobe-gfx
// ============================================================================

//! The pipeline.
//!
//! ```text
//! analyse ─▶ pick ≤ N to try ─▶ rewrite ALL into uniforms ─▶ compile once
//!    ─▶ render baseline + two nudges per candidate ─▶ score
//!    ─▶ drop dead / broken ─▶ keep the top K ─▶ final names
//!    ─▶ rewrite only those (plus opt-in audio bindings) ─▶ verify it compiles
//! ```
//!
//! The scoring scene uses placeholder names (`auto_ctl_1`...) and plain
//! slider types so a nudged value lands on the next frame; the output scene
//! uses the final readable names and the smoothed performance types. The
//! two rewrites differ only in names, types and the chosen subset, so the
//! output is verified by compiling it once more and measuring how far its
//! default picture is from the original ([`Report::fidelity`]).
//!
//! A scene that already has authored controls is left alone unless the caller
//! sets [`Options::merge`]; then the authored ones stay and only new names are
//! added. The five "Strobe" controls every Shadertoy import carries do not
//! count as authored.
//!
//! ## One step at a time
//!
//! All of that is a [`Job`], a state machine whose [`Job::step`] does one
//! piece and returns (D558): hand a build to the driver, poll it, render one
//! probe. A browser tab has one context and no worker to run it in, so its
//! page takes a step per animation frame and keeps drawing between them.
//! [`generate`] runs the same machine to the end in one call, so the two give
//! the same controls, and dropping a [`Job`] cancels it: every GL object it
//! made belongs to a value it holds.

use crate::analyze::{Analysis, analyze};
use crate::candidate::{Candidate, Group, Kind, ValueType};
use crate::naming::{Taken, assign, assign_with_roles};
use crate::report::{CandidateReport, Report, ReportOptions, Verdict, round5};
use crate::rewrite::{self, AudioBind, PlannedControl};
use crate::score::{self, PendingRig, Probing, Rig, ScoreError, ScoreOptions};
use crate::source::Sources;
use crate::suggest;
use strobe_gfx::Caps;
use strobe_gfx::gl::Gl;
use strobe_scene::SceneFolder;
use strobe_scene::control::ControlDef;
use strobe_scene::json::RawControl;
use strobe_scene::scene::SceneDef;

/// How the engine runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// The most controls to generate.
    pub keep: usize,
    /// The most candidates to measure; the rest are reported `not scored`.
    pub max_scored: usize,
    /// How scoring renders.
    pub score: ScoreOptions,
    /// Add to a scene that already has authored controls.
    pub merge: bool,
    /// Write the suggested audio mappings into the GLSL (off by default; the
    /// new `*_audio` sliders default to 0 either way).
    pub bind_audio: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            keep: 8,
            max_scored: 40,
            score: ScoreOptions::default(),
            merge: false,
            bind_audio: false,
        }
    }
}

/// Why generation stopped.
#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    /// The scene already has controls somebody wrote.
    #[error(
        "the scene already has {count} authored control(s) ({names}); \
         ask for a merge to add to them"
    )]
    Authored {
        /// How many.
        count: usize,
        /// The first few names.
        names: String,
    },
    /// The scene as given does not compile, so nothing can be measured.
    #[error("the original scene does not compile: {0}")]
    Original(ScoreError),
    /// Measuring failed.
    #[error("{0}")]
    Score(#[from] ScoreError),
    /// A generated control did not parse as a control (a bug in this crate).
    #[error("a generated control is invalid: {0}")]
    Control(String),
    /// The finished scene did not compile (a bug in this crate).
    #[error("the generated scene does not compile: {0}")]
    Verify(ScoreError),
    /// A [`Job`] was stepped after it returned its result or its error.
    #[error("this generation has ended")]
    Ended,
}

/// What the engine made.
#[derive(Debug, Clone)]
pub struct Generated {
    /// The decisions.
    pub report: Report,
    /// The rewritten GLSL files.
    pub sources: Sources,
    /// The `CONTROLS` entries to add, one JSON object per line, in listing
    /// order.
    pub control_lines: Vec<String>,
    /// The scene as it would play: the rewritten GLSL and the new controls
    /// added to the original ones.
    pub folder: SceneFolder,
}

/// The controls a person wrote: everything except the "Strobe" group an
/// import adds.
pub fn authored_controls(def: &SceneDef) -> Vec<String> {
    def.controls
        .iter()
        .filter(|c| is_authored(c))
        .map(|c| c.name.clone())
        .collect()
}

/// Whether a person wrote `control`, rather than an import adding it.
pub fn is_authored(control: &ControlDef) -> bool {
    !(control.name.starts_with("st_") && control.group.as_deref() == Some("Strobe"))
}

/// Which candidates to try first when there are too many to try all.
fn priority(kind: Kind) -> u8 {
    match kind {
        Kind::Colour => 0,
        Kind::TimeScale => 1,
        Kind::Define | Kind::Const => 2,
        Kind::LoopBound => 3,
        Kind::Scale => 4,
        Kind::Frequency => 5,
    }
}

/// One control a person kept when reviewing a result (D219): which
/// candidate, under the name the engine gave it, with the range they chose.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Choice {
    /// The candidate's `id`, as the report listed it.
    pub candidate: usize,
    /// The control's name: as [`generate`] named it, or as the person
    /// renamed it in the review (checked like a generated name).
    pub name: String,
    /// The candidate's `context` line when it was measured, to tell that
    /// the source has not changed since.
    pub context: String,
    /// The lower bound to use (a slider's only; a colour has none).
    pub min: f32,
    /// The upper bound to use.
    pub max: f32,
}

/// Why a review could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum ChoiceError {
    /// The scene's code changed after it was measured.
    #[error("{0} has changed since its controls were measured; generate them again")]
    Changed(String),
    /// A control's name cannot be declared in this scene: not a GLSL
    /// identifier, reserved, already used by the scene, or given twice.
    #[error("{0} cannot be a control's name here: {1}")]
    Name(String, &'static str),
    /// A generated control did not parse (a bug in this crate).
    #[error("{0}")]
    Generate(#[from] GenerateError),
}

/// `folder` rewritten with exactly `choices` as its new controls, listed by
/// group in the order given, with each slider's range as chosen. No GL and
/// no scoring: the candidates were measured already, and the analysis is a
/// pure function of the code, so it finds them again by `id`. A choice
/// whose candidate no longer reads as it did is refused, not guessed at.
/// The result's report carries no fidelity; the default of every kept
/// control is still the original literal, so the picture at defaults is the
/// one [`generate`] verified.
pub fn rewrite_chosen(folder: &SceneFolder, choices: &[Choice]) -> Result<Generated, ChoiceError> {
    let sources = Sources::from_folder(folder);
    let candidates = analyze(&sources).candidates;
    // A renamed control has to meet the rules the engine's own names meet
    // (D246): the measured source is the original, so nothing generated is
    // in it yet, and each name claimed is taken for the next.
    let mut taken = Taken::of(folder, &sources);
    for choice in choices {
        let name = choice.name.as_str();
        if !strobe_scene::names::is_glsl_identifier(name) {
            return Err(ChoiceError::Name(
                name.to_string(),
                "use letters, digits and _, not starting with a digit",
            ));
        }
        if !taken.is_free(name) {
            return Err(ChoiceError::Name(
                name.to_string(),
                "the scene, SSF or GLSL already uses it, or another control has it",
            ));
        }
        taken.claim(name);
    }
    let mut chosen: Vec<(&Candidate, &Choice)> = Vec::with_capacity(choices.len());
    for choice in choices {
        match candidates.get(choice.candidate) {
            Some(candidate) if candidate.context.trim() == choice.context.trim() => {
                chosen.push((candidate, choice));
            }
            Some(_) | None => return Err(ChoiceError::Changed(folder.def.title.clone())),
        }
    }
    chosen.sort_by_key(|(candidate, _)| candidate.group);
    let planned: Vec<PlannedControl> = chosen
        .iter()
        .map(|(candidate, choice)| PlannedControl {
            candidate: candidate.id,
            name: choice.name.clone(),
            audio: None,
        })
        .collect();
    let control_lines: Vec<String> = chosen
        .iter()
        .map(|(candidate, choice)| {
            let mut ranged = (*candidate).clone();
            if ranged.value_type != ValueType::Colour && choice.min < choice.max {
                ranged.min = vec![choice.min];
                ranged.max = vec![choice.max];
            }
            rewrite::control_line(&ranged, &choice.name, false)
        })
        .collect();
    let rewritten = rewrite::apply(&sources, &candidates, &planned);
    let final_folder = with_controls(rewritten.apply_to(folder), &control_lines)?;
    let report = Report {
        scene: folder.def.title.clone(),
        dialect: String::new(),
        options: ReportOptions {
            keep: choices.len(),
            max_scored: 0,
            score: ScoreOptions::default(),
            bind_audio: false,
        },
        fidelity: None,
        candidates: Vec::new(),
        kept: chosen
            .iter()
            .map(|(_, choice)| choice.name.clone())
            .collect(),
        suggestions: Vec::new(),
    };
    Ok(Generated {
        report,
        sources: rewritten,
        control_lines,
        folder: final_folder,
    })
}

/// `folder`'s `IMAGES` pictures from `asset`, which answers a picture by its
/// file name: by the entry's whole path, then by its last part (a page keeps
/// no folders, D554). An entry it has no picture for is left out.
pub fn images_from(
    folder: &SceneFolder,
    asset: impl Fn(&str) -> Option<Vec<u8>>,
) -> Vec<(usize, Vec<u8>)> {
    folder
        .def
        .images
        .iter()
        .enumerate()
        .filter_map(|(index, image)| {
            let base = image.path.rsplit('/').next().unwrap_or(&image.path);
            let bytes = asset(&image.path).or_else(|| asset(base))?;
            Some((index, bytes))
        })
        .collect()
}

/// Parses control lines and appends them to a copy of `base`'s controls.
fn with_controls(mut folder: SceneFolder, lines: &[String]) -> Result<SceneFolder, GenerateError> {
    for line in lines {
        let raw: RawControl = serde_json::from_str(line)
            .map_err(|e| GenerateError::Control(format!("{e}: {line}")))?;
        let def = ControlDef::from_raw(&raw)
            .map_err(|e| GenerateError::Control(format!("{e}: {line}")))?;
        folder.def.controls.push(def);
    }
    Ok(folder)
}

/// The scoring scene for the candidates `live`, and the control index of each.
fn scoring_folder(
    base: &SceneFolder,
    sources: &Sources,
    candidates: &[Candidate],
    live: &[usize],
    names: &[String],
) -> Result<(SceneFolder, Vec<usize>), GenerateError> {
    let planned: Vec<PlannedControl> = live
        .iter()
        .map(|id| PlannedControl {
            candidate: *id,
            name: names.get(*id).cloned().unwrap_or_default(),
            audio: None,
        })
        .collect();
    let lines: Vec<String> = live
        .iter()
        .filter_map(|id| {
            let candidate = candidates.get(*id)?;
            Some(rewrite::control_line(candidate, names.get(*id)?, true))
        })
        .collect();
    let rewritten = rewrite::apply(sources, candidates, &planned).apply_to(base);
    let first = base.def.controls.len();
    let folder = with_controls(rewritten, &lines)?;
    Ok((folder, (first..first + live.len()).collect()))
}

/// The first line of a compiler complaint, for the report.
fn first_line(error: &ScoreError) -> String {
    error.to_string().lines().next().unwrap_or("").to_string()
}

/// The largest mean image difference (0..1) between the original and the
/// rewrite at its defaults that still counts as the same picture (D259).
/// On llvmpipe a faithful rewrite measures exactly 0.
pub const FIDELITY_LIMIT: f32 = 0.001;

/// Runs the whole engine on `folder`.
pub fn generate(
    gl: &Gl,
    caps: &Caps,
    folder: &SceneFolder,
    options: &Options,
) -> Result<Generated, GenerateError> {
    let analysis = analyze(&Sources::from_folder(folder));
    generate_from(gl, caps, folder, &analysis, options)
}

/// The finished output for one set of kept candidates.
struct Built {
    /// `(candidate id, rank)` in listing order.
    listing: Vec<(usize, usize)>,
    /// The final name of each, in listing order.
    names: Vec<String>,
    suggestions: Vec<suggest::AudioSuggestion>,
    control_lines: Vec<String>,
    sources: Sources,
    folder: SceneFolder,
}

/// Names, suggestions, audio bindings, `CONTROLS` lines and the rewrite
/// for the candidates `kept_ids` (`(id, rank)`).
fn build(
    folder: &SceneFolder,
    sources: &Sources,
    candidates: &[Candidate],
    kept_ids: &[(usize, usize)],
    bind_audio: bool,
) -> Result<Built, GenerateError> {
    // Listing order: group, then strength.
    let mut listing = kept_ids.to_vec();
    listing.sort_by_key(|(id, rank)| (candidates.get(*id).map(|c| c.group), *rank));
    // Numbered names follow the source order (`color_1` is the one that comes
    // first in the code), whatever order they are listed in; a role tells
    // two of one name apart first (`ramp_color_low`).
    let mut in_source_order: Vec<usize> = kept_ids.iter().map(|(id, _)| *id).collect();
    in_source_order.sort_unstable();
    let bases: Vec<(String, Option<String>)> = in_source_order
        .iter()
        .filter_map(|id| {
            candidates
                .get(*id)
                .map(|c| (c.name.clone(), c.role.clone()))
        })
        .collect();
    let mut taken = Taken::of(folder, sources);
    let assigned = assign_with_roles(&bases, &mut taken);
    let names: Vec<String> = listing
        .iter()
        .map(|(id, _)| {
            in_source_order
                .iter()
                .position(|other| other == id)
                .and_then(|at| assigned.get(at))
                .cloned()
                .unwrap_or_default()
        })
        .collect();

    // Suggestions go to the strongest controls, by rank.
    let mut by_rank: Vec<(&Candidate, &str, usize)> = listing
        .iter()
        .zip(&names)
        .filter_map(|((id, rank), name)| candidates.get(*id).map(|c| (c, name.as_str(), *rank)))
        .collect();
    by_rank.sort_by_key(|(_, _, rank)| *rank);
    let ranked: Vec<(&Candidate, &str)> = by_rank.iter().map(|(c, n, _)| (*c, *n)).collect();
    let suggestions = suggest::suggest(&ranked);

    // Opt-in bindings: a unique `<name>_audio` for each expressible one.
    let mut binds: Vec<Option<AudioBind>> = vec![None; listing.len()];
    let mut audio_lines: Vec<(usize, String)> = Vec::new();
    if bind_audio {
        for suggestion in suggestions.iter().filter(|s| s.expressible) {
            let Some(position) = names.iter().position(|n| *n == suggestion.control) else {
                continue;
            };
            let amount = assign(&[format!("{}_audio", suggestion.control)], &mut taken)
                .into_iter()
                .next()
                .unwrap_or_default();
            let bind = AudioBind {
                amount,
                band: suggestion.band.clone(),
            };
            if let Some((id, _)) = listing.get(position)
                && let Some(candidate) = candidates.get(*id)
            {
                audio_lines.push((position, rewrite::audio_control_line(candidate, &bind)));
            }
            if let Some(slot) = binds.get_mut(position) {
                *slot = Some(bind);
            }
        }
    }

    let planned: Vec<PlannedControl> = listing
        .iter()
        .zip(&names)
        .zip(&binds)
        .map(|(((id, _), name), bind)| PlannedControl {
            candidate: *id,
            name: name.clone(),
            audio: bind.clone(),
        })
        .collect();
    let mut control_lines: Vec<String> = Vec::new();
    for (position, ((id, _), name)) in listing.iter().zip(&names).enumerate() {
        if let Some(candidate) = candidates.get(*id) {
            control_lines.push(rewrite::control_line(candidate, name, false));
        }
        for (at, line) in &audio_lines {
            if *at == position {
                control_lines.push(line.clone());
            }
        }
    }

    let rewritten = rewrite::apply(sources, candidates, &planned);
    let final_folder = with_controls(rewritten.apply_to(folder), &control_lines)?;
    Ok(Built {
        listing,
        names,
        suggestions,
        control_lines,
        sources: rewritten,
        folder: final_folder,
    })
}

/// What a [`Job`] is busy with, for a progress line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Compiling the scene and rendering the original.
    Preparing,
    /// Measuring candidates.
    Scoring,
    /// Compiling the result and comparing it with the original.
    Checking,
}

/// How far a [`Job`] has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Progress {
    /// What it is doing.
    pub phase: Phase,
    /// Candidates measured so far; `0` outside [`Phase::Scoring`].
    pub done: usize,
    /// Candidates being measured; `0` outside [`Phase::Scoring`].
    pub total: usize,
}

/// A verification of a built result: the three times the finished scene, or
/// a part of it, is compared with the original.
enum Check {
    /// The result as first built.
    Whole,
    /// It failed: each kept control on its own, `at` being the one under
    /// test, to find the ones that change the default picture.
    Alone {
        at: usize,
        offenders: Vec<(usize, f32)>,
        whole: f32,
    },
    /// The result again, without the offenders.
    Again,
}

/// Where a [`Job`] is. Holding its GL objects here is what makes dropping
/// the job a cancel.
enum Stage {
    /// Nothing done yet.
    Start,
    /// The original is compiling.
    OriginalCompiling(PendingRig),
    /// The original's first render, proof it draws and the picture to match.
    OriginalFirst(Rig),
    /// Its second, to learn the scene's own noise.
    OriginalSecond(Rig, score::Frames),
    /// The scene with every candidate as a control is compiling.
    ScoringCompiling {
        pending: PendingRig,
        live: Vec<usize>,
        controls: Vec<usize>,
        /// Whether a failure should find out which candidate caused it;
        /// `false` for the survivors of that search.
        search: bool,
    },
    /// The next candidate to try on its own.
    Lone {
        live: Vec<usize>,
        at: usize,
        survivors: Vec<usize>,
    },
    /// That candidate compiling.
    LoneCompiling {
        pending: PendingRig,
        live: Vec<usize>,
        at: usize,
        survivors: Vec<usize>,
    },
    /// Rendering probes.
    Scoring {
        rig: Rig,
        live: Vec<usize>,
        probing: Probing,
    },
    /// A result compiling, to be compared with the original.
    CheckCompiling {
        pending: PendingRig,
        built: Box<Built>,
        check: Check,
    },
    /// A result's picture.
    CheckRendering {
        rig: Rig,
        built: Box<Built>,
        check: Check,
    },
    /// It has returned its result or its error.
    Ended,
}

/// What one step leads to.
#[expect(
    clippy::large_enum_variant,
    reason = "it lives for one return, moved straight into the job"
)]
enum Next {
    Stage(Stage),
    Done(Box<Generated>),
}

/// The whole engine on one scene, a piece at a time. [`Self::step`] does one
/// piece; between steps the context is free for anything else, a live scene
/// included. Dropping the job cancels it and frees what it made.
pub struct Job {
    gl: Gl,
    caps: Caps,
    folder: SceneFolder,
    sources: Sources,
    candidates: Vec<Candidate>,
    options: Options,
    /// Pictures for `IMAGES` entries, by index (D554).
    images: Vec<(usize, Vec<u8>)>,
    rows: Vec<CandidateReport>,
    report: Report,
    /// The original's picture, once rendered.
    original: score::Frames,
    /// How far two renders of the original differ.
    noise: f32,
    placeholders: Vec<String>,
    /// `(id, rank)` of the controls being kept.
    kept_ids: Vec<(usize, usize)>,
    stage: Stage,
}

impl Job {
    /// Starts the engine on `folder`; no GL work happens until a step.
    ///
    /// # Errors
    ///
    /// [`GenerateError::Authored`] for a scene with authored controls unless
    /// [`Options::merge`] is set.
    pub fn new(
        gl: &Gl,
        caps: &Caps,
        folder: &SceneFolder,
        options: &Options,
    ) -> Result<Self, GenerateError> {
        let analysis = analyze(&Sources::from_folder(folder));
        Self::from_analysis(gl, caps, folder, &analysis, options)
    }

    /// [`Self::new`] with the candidates given rather than found.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn from_analysis(
        gl: &Gl,
        caps: &Caps,
        folder: &SceneFolder,
        analysis: &Analysis,
        options: &Options,
    ) -> Result<Self, GenerateError> {
        let authored = authored_controls(&folder.def);
        if !authored.is_empty() && !options.merge {
            return Err(GenerateError::Authored {
                count: authored.len(),
                names: authored
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        let candidates = analysis.candidates.clone();
        let rows = candidates
            .iter()
            .map(|c| CandidateReport::new(c, Verdict::NotScored))
            .collect();
        let report = Report {
            scene: folder.def.title.clone(),
            dialect: format!("{:?}", caps.dialect),
            options: ReportOptions {
                keep: options.keep,
                max_scored: options.max_scored,
                score: options.score,
                bind_audio: options.bind_audio,
            },
            fidelity: None,
            candidates: Vec::new(),
            kept: Vec::new(),
            suggestions: Vec::new(),
        };
        Ok(Self {
            gl: gl.clone(),
            caps: caps.clone(),
            folder: folder.clone(),
            sources: Sources::from_folder(folder),
            candidates,
            options: *options,
            images: Vec::new(),
            rows,
            report,
            original: Vec::new(),
            noise: 0.0,
            placeholders: Vec::new(),
            kept_ids: Vec::new(),
            stage: Stage::Start,
        })
    }

    /// Gives the job pictures for the scene's `IMAGES`, for a host with no
    /// scene folder behind it (see [`images_from`]). Every render then reads
    /// what the scene reads when played.
    #[must_use]
    pub fn with_images(mut self, images: Vec<(usize, Vec<u8>)>) -> Self {
        self.images = images;
        self
    }

    /// How far it has got.
    pub fn progress(&self) -> Progress {
        match &self.stage {
            Stage::Start
            | Stage::OriginalCompiling(_)
            | Stage::OriginalFirst(_)
            | Stage::OriginalSecond(..)
            | Stage::ScoringCompiling { .. }
            | Stage::Lone { .. }
            | Stage::LoneCompiling { .. } => Progress {
                phase: Phase::Preparing,
                done: 0,
                total: 0,
            },
            Stage::Scoring { probing, .. } => Progress {
                phase: Phase::Scoring,
                done: probing.done(),
                total: probing.total(),
            },
            Stage::CheckCompiling { .. } | Stage::CheckRendering { .. } | Stage::Ended => {
                Progress {
                    phase: Phase::Checking,
                    done: 0,
                    total: 0,
                }
            }
        }
    }

    /// Does one piece of the work: hands a build to the driver or asks
    /// whether it is done, renders one probe, or compares a result. Returns
    /// the result when there is one, and `None` while there is more to do.
    ///
    /// The live scene's frames may fall between steps, so a GL error flag
    /// left by one is cleared first and not blamed on the measurement.
    ///
    /// # Errors
    ///
    /// As [`generate`]; the job is over then, and a further step says so.
    pub fn step(&mut self) -> Result<Option<Generated>, GenerateError> {
        self.gl.clear_errors();
        self.advance(false)
    }

    /// Runs the rest of the job in one call.
    fn run(mut self) -> Result<Generated, GenerateError> {
        loop {
            if let Some(generated) = self.advance(true)? {
                return Ok(generated);
            }
        }
    }

    /// One transition. `wait` skips the poll of a build in progress and
    /// waits for it in `finish`, for a caller with nothing else to do.
    fn advance(&mut self, wait: bool) -> Result<Option<Generated>, GenerateError> {
        let stage = std::mem::replace(&mut self.stage, Stage::Ended);
        match self.transition(stage, wait)? {
            Next::Stage(stage) => {
                self.stage = stage;
                Ok(None)
            }
            Next::Done(generated) => Ok(Some(*generated)),
        }
    }

    fn transition(&mut self, stage: Stage, wait: bool) -> Result<Next, GenerateError> {
        Ok(Next::Stage(match stage {
            Stage::Start => {
                let pending = Rig::begin(&self.gl, &self.caps, &self.folder, self.options.score)
                    .map_err(GenerateError::Original)?;
                Stage::OriginalCompiling(pending)
            }
            Stage::OriginalCompiling(pending) => {
                if !wait && !pending.ready() {
                    return Ok(Next::Stage(Stage::OriginalCompiling(pending)));
                }
                let rig = self.finish_rig(pending).map_err(GenerateError::Original)?;
                Stage::OriginalFirst(rig)
            }
            // The original, always, even with nothing to measure: proof that
            // it compiles (a scene whose own GLSL is broken is an error, not
            // "0 candidates", D259), the picture the rewrite must reproduce,
            // and how far two renders of it differ (a scene that is not a
            // pure function of time, such as one playing a video, has a
            // noise floor).
            Stage::OriginalFirst(mut rig) => {
                let first = rig.render(&[]).map_err(GenerateError::Original)?;
                if self.candidates.is_empty() {
                    return Ok(self.unchanged());
                }
                Stage::OriginalSecond(rig, first)
            }
            Stage::OriginalSecond(mut rig, first) => {
                let second = rig.render(&[]).map_err(GenerateError::Original)?;
                self.noise = score::difference(&first, &second);
                self.original = first;
                drop(rig);
                self.start_scoring()?
            }
            Stage::ScoringCompiling {
                pending,
                live,
                controls,
                search,
            } => {
                if !wait && !pending.ready() {
                    return Ok(Next::Stage(Stage::ScoringCompiling {
                        pending,
                        live,
                        controls,
                        search,
                    }));
                }
                match self.finish_rig(pending) {
                    Ok(rig) => self.start_probing(rig, live, &controls),
                    Err(error) => return self.scoring_failed(error, live, search).map(Next::Stage),
                }
            }
            Stage::Lone {
                live,
                at,
                survivors,
            } => match live.get(at).copied() {
                Some(id) => self.start_lone(id, live, at, survivors)?,
                None => self.compile_scoring(survivors, false)?,
            },
            Stage::LoneCompiling {
                pending,
                live,
                at,
                mut survivors,
            } => {
                if !wait && !pending.ready() {
                    return Ok(Next::Stage(Stage::LoneCompiling {
                        pending,
                        live,
                        at,
                        survivors,
                    }));
                }
                if let Some(id) = live.get(at).copied() {
                    match self.finish_rig(pending) {
                        Ok(_) => survivors.push(id),
                        Err(error) => self.does_not_compile(id, &error),
                    }
                }
                Stage::Lone {
                    live,
                    at: at + 1,
                    survivors,
                }
            }
            Stage::Scoring {
                mut rig,
                live,
                mut probing,
            } => {
                if !probing.step(&mut rig)? {
                    return Ok(Next::Stage(Stage::Scoring { rig, live, probing }));
                }
                drop(rig);
                let (measures, _) = probing.finish();
                self.decide(&live, &measures);
                if self.kept_ids.is_empty() {
                    return Ok(self.unchanged());
                }
                let built = self.built(&self.kept_ids, self.options.bind_audio)?;
                self.start_check(built, Check::Whole)?
            }
            Stage::CheckCompiling {
                pending,
                built,
                check,
            } => {
                if !wait && !pending.ready() {
                    return Ok(Next::Stage(Stage::CheckCompiling {
                        pending,
                        built,
                        check,
                    }));
                }
                let rig = self.finish_rig(pending).map_err(GenerateError::Verify)?;
                Stage::CheckRendering { rig, built, check }
            }
            Stage::CheckRendering {
                mut rig,
                built,
                check,
            } => {
                let frames = rig.render(&[]).map_err(GenerateError::Verify)?;
                drop(rig);
                let distance = score::difference(&self.original, &frames);
                return self.checked(*built, check, distance);
            }
            Stage::Ended => return Err(GenerateError::Ended),
        }))
    }

    /// A finished build, with the job's pictures uploaded.
    fn finish_rig(&self, pending: PendingRig) -> Result<Rig, ScoreError> {
        let mut rig = pending.finish(&self.caps)?;
        for (index, bytes) in &self.images {
            rig.replace_image(*index, bytes);
        }
        Ok(rig)
    }

    /// The scene as it came in, with the verdicts so far.
    fn unchanged(&mut self) -> Next {
        let mut report = self.report.clone();
        report.candidates = std::mem::take(&mut self.rows);
        Next::Done(Box::new(Generated {
            report,
            sources: self.sources.clone(),
            control_lines: Vec::new(),
            folder: self.folder.clone(),
        }))
    }

    /// Which candidates to try, and the placeholder names they are tried
    /// under, then the build with all of them.
    fn start_scoring(&mut self) -> Result<Stage, GenerateError> {
        let mut order: Vec<usize> = (0..self.candidates.len()).collect();
        let candidates = &self.candidates;
        order.sort_by_key(|id| {
            (
                candidates.get(*id).map_or(u8::MAX, |c| priority(c.kind)),
                *id,
            )
        });
        order.truncate(self.options.max_scored);
        order.sort_unstable();
        let mut scratch = Taken::of(&self.folder, &self.sources);
        self.placeholders = assign(
            &vec!["auto_ctl".to_string(); self.candidates.len()],
            &mut scratch,
        );
        // Compile once with everything; if that fails, find out who broke it.
        self.compile_scoring(order, true)
    }

    fn compile_scoring(&mut self, live: Vec<usize>, search: bool) -> Result<Stage, GenerateError> {
        let (folder, controls) = scoring_folder(
            &self.folder,
            &self.sources,
            &self.candidates,
            &live,
            &self.placeholders,
        )?;
        match Rig::begin(&self.gl, &self.caps, &folder, self.options.score) {
            Ok(pending) => Ok(Stage::ScoringCompiling {
                pending,
                live,
                controls,
                search,
            }),
            Err(error) => self.scoring_failed(error, live, search),
        }
    }

    /// A compile with `live` failed: a shader error starts the search for
    /// the candidates that caused it, once.
    fn scoring_failed(
        &self,
        error: ScoreError,
        live: Vec<usize>,
        search: bool,
    ) -> Result<Stage, GenerateError> {
        match error {
            ScoreError::Player(_) if search => Ok(Stage::Lone {
                live,
                at: 0,
                survivors: Vec::new(),
            }),
            error @ (ScoreError::Player(_) | ScoreError::Scene(_) | ScoreError::Gl(_)) => {
                Err(error.into())
            }
        }
    }

    fn start_lone(
        &mut self,
        id: usize,
        live: Vec<usize>,
        at: usize,
        survivors: Vec<usize>,
    ) -> Result<Stage, GenerateError> {
        let (single, _) = scoring_folder(
            &self.folder,
            &self.sources,
            &self.candidates,
            &[id],
            &self.placeholders,
        )?;
        Ok(
            match Rig::begin(&self.gl, &self.caps, &single, self.options.score) {
                Ok(pending) => Stage::LoneCompiling {
                    pending,
                    live,
                    at,
                    survivors,
                },
                Err(error) => {
                    self.does_not_compile(id, &error);
                    Stage::Lone {
                        live,
                        at: at + 1,
                        survivors,
                    }
                }
            },
        )
    }

    fn does_not_compile(&mut self, id: usize, error: &ScoreError) {
        if let Some(row) = self.rows.get_mut(id) {
            row.verdict = Verdict::DoesNotCompile {
                message: first_line(error),
            };
        }
    }

    fn start_probing(&self, rig: Rig, live: Vec<usize>, controls: &[usize]) -> Stage {
        let probes: Vec<(&Candidate, usize)> = live
            .iter()
            .zip(controls)
            .filter_map(|(id, control)| self.candidates.get(*id).map(|c| (c, *control)))
            .collect();
        let probing = Probing::new(&probes);
        Stage::Scoring { rig, live, probing }
    }

    /// Verdicts: dead, broken, or effective and ranked; then which to keep.
    fn decide(&mut self, live: &[usize], measures: &[score::Measure]) {
        let mut effective: Vec<(usize, f32)> = Vec::new();
        for (id, measure) in live.iter().zip(measures) {
            let Some(row) = self.rows.get_mut(*id) else {
                continue;
            };
            row.measured(measure);
            match measure.broken {
                Some(reason) => row.verdict = Verdict::Broken { reason },
                None if measure.score < self.options.score.threshold => {
                    row.verdict = Verdict::Dead;
                }
                None => effective.push((*id, round5(measure.score))),
            }
        }
        effective.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        // Kept: the strongest, within a fraction of the best, one Quality
        // control at most, `keep` in all (D259).
        let best = effective.first().map_or(0.0, |(_, score)| *score);
        let floor = score::weak_floor(best, &self.options.score);
        let mut quality_kept = false;
        for (index, (id, score)) in effective.iter().enumerate() {
            let rank = index + 1;
            let quality = self
                .candidates
                .get(*id)
                .is_some_and(|c| c.group == Group::Quality);
            let verdict = if *score < floor {
                Verdict::Weak { rank }
            } else if quality && quality_kept {
                Verdict::QualityLimit { rank }
            } else if self.kept_ids.len() >= self.options.keep {
                Verdict::Cut { rank }
            } else {
                quality_kept |= quality;
                self.kept_ids.push((*id, rank));
                Verdict::Kept { rank }
            };
            if let Some(row) = self.rows.get_mut(*id) {
                row.verdict = verdict;
            }
        }
    }

    fn built(&self, kept: &[(usize, usize)], bind_audio: bool) -> Result<Built, GenerateError> {
        build(
            &self.folder,
            &self.sources,
            &self.candidates,
            kept,
            bind_audio,
        )
    }

    fn start_check(&self, built: Built, check: Check) -> Result<Stage, GenerateError> {
        let pending = Rig::begin(&self.gl, &self.caps, &built.folder, self.options.score)
            .map_err(GenerateError::Verify)?;
        Ok(Stage::CheckCompiling {
            pending,
            built: Box::new(built),
            check,
        })
    }

    /// Verify: it compiles, and at its defaults it draws what the original
    /// drew. If it does not, find the controls that change the default
    /// picture and drop them: an unfaithful scene is never written (D259).
    fn checked(
        &mut self,
        built: Built,
        check: Check,
        distance: f32,
    ) -> Result<Next, GenerateError> {
        let limit = FIDELITY_LIMIT + self.noise;
        match check {
            Check::Whole if distance > limit => self.check_alone(0, Vec::new(), distance),
            Check::Alone {
                at,
                mut offenders,
                whole,
            } => {
                if distance > limit
                    && let Some((id, _)) = self.kept_ids.get(at)
                {
                    offenders.push((*id, round5(distance)));
                }
                self.check_alone(at + 1, offenders, whole)
            }
            Check::Again if distance > limit => {
                for (id, _) in &self.kept_ids {
                    if let Some(row) = self.rows.get_mut(*id) {
                        row.verdict = Verdict::Unfaithful {
                            difference: round5(distance),
                        };
                    }
                }
                Ok(self.unchanged())
            }
            Check::Whole | Check::Again => Ok(self.result(built, distance)),
        }
    }

    /// Tests kept control `at` on its own, or, past the last, drops the ones
    /// found and checks what is left.
    fn check_alone(
        &mut self,
        at: usize,
        mut offenders: Vec<(usize, f32)>,
        whole: f32,
    ) -> Result<Next, GenerateError> {
        if let Some(kept) = self.kept_ids.get(at).copied() {
            let alone = self.built(&[kept], false)?;
            let check = Check::Alone {
                at,
                offenders,
                whole,
            };
            return self.start_check(alone, check).map(Next::Stage);
        }
        if offenders.is_empty() {
            // Faithful one by one, not together: nothing can be kept.
            offenders = self
                .kept_ids
                .iter()
                .map(|(id, _)| (*id, round5(whole)))
                .collect();
        }
        for (id, difference) in &offenders {
            if let Some(row) = self.rows.get_mut(*id) {
                row.verdict = Verdict::Unfaithful {
                    difference: *difference,
                };
            }
        }
        self.kept_ids
            .retain(|(id, _)| !offenders.iter().any(|(bad, _)| bad == id));
        log::warn!(
            "{}: {} generated control(s) changed the default picture and were dropped",
            self.folder.def.title,
            offenders.len()
        );
        if self.kept_ids.is_empty() {
            return Ok(self.unchanged());
        }
        let built = self.built(&self.kept_ids, self.options.bind_audio)?;
        self.start_check(built, Check::Again).map(Next::Stage)
    }

    /// The finished result.
    fn result(&mut self, built: Built, fidelity: f32) -> Next {
        for ((id, _), name) in built.listing.iter().zip(&built.names) {
            if let Some(row) = self.rows.get_mut(*id) {
                row.name = Some(name.clone());
            }
        }
        let mut report = self.report.clone();
        report.fidelity = Some(round5(fidelity));
        report.kept.clone_from(&built.names);
        report.suggestions = built.suggestions;
        report.candidates = std::mem::take(&mut self.rows);
        Next::Done(Box::new(Generated {
            report,
            sources: built.sources,
            control_lines: built.control_lines,
            folder: built.folder,
        }))
    }
}

/// [`generate`] with the candidates given rather than found: the analysis
/// is a pure function of the code, so this is `generate` for an unchanged
/// `analysis`. Tests use it to hand the engine a candidate it must catch.
pub fn generate_from(
    gl: &Gl,
    caps: &Caps,
    folder: &SceneFolder,
    analysis: &Analysis,
    options: &Options,
) -> Result<Generated, GenerateError> {
    Job::from_analysis(gl, caps, folder, analysis, options)?.run()
}
