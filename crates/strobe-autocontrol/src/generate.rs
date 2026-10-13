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

use crate::analyze::{Analysis, analyze};
use crate::candidate::{Candidate, Group, Kind, ValueType};
use crate::naming::{Taken, assign, assign_with_roles};
use crate::report::{CandidateReport, Report, ReportOptions, Verdict, round5};
use crate::rewrite::{self, AudioBind, PlannedControl};
use crate::score::{self, Rig, ScoreError, ScoreOptions};
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
#[derive(Debug, Clone, PartialEq)]
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

/// How far `folder`, at its defaults, draws from `original`: it must
/// compile, and the result is the mean image difference.
fn distance_from(
    gl: &Gl,
    caps: &Caps,
    folder: &SceneFolder,
    options: &Options,
    original: &score::Frames,
) -> Result<f32, GenerateError> {
    let mut check = Rig::new(gl, caps, folder, options.score).map_err(GenerateError::Verify)?;
    let frames = check.render(&[]).map_err(GenerateError::Verify)?;
    Ok(score::difference(original, &frames))
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

    let sources = Sources::from_folder(folder);
    let candidates = &analysis.candidates;
    let mut rows: Vec<CandidateReport> = candidates
        .iter()
        .map(|c| CandidateReport::new(c, Verdict::NotScored))
        .collect();
    let mut report = Report {
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
    let unchanged = |report: Report, rows: Vec<CandidateReport>| Generated {
        report: Report {
            candidates: rows,
            ..report
        },
        sources: sources.clone(),
        control_lines: Vec::new(),
        folder: folder.clone(),
    };

    // The original, always, even with nothing to measure: proof that it
    // compiles (a scene whose own GLSL is broken is an error, not "0
    // candidates", D259), the picture the rewrite must reproduce, and how
    // far two renders of it differ (a scene that is not a pure function of
    // time, such as one playing a video, has a noise floor).
    let (original_frames, noise) = {
        let mut rig = Rig::new(gl, caps, folder, options.score).map_err(GenerateError::Original)?;
        let first = rig.render(&[]).map_err(GenerateError::Original)?;
        if candidates.is_empty() {
            return Ok(unchanged(report, rows));
        }
        let second = rig.render(&[]).map_err(GenerateError::Original)?;
        let noise = score::difference(&first, &second);
        (first, noise)
    };

    // Which candidates to try, and the placeholder names they are tried under.
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|id| {
        (
            candidates.get(*id).map_or(u8::MAX, |c| priority(c.kind)),
            *id,
        )
    });
    order.truncate(options.max_scored);
    order.sort_unstable();
    let mut scratch = Taken::of(folder, &sources);
    let placeholders = assign(
        &vec!["auto_ctl".to_string(); candidates.len()],
        &mut scratch,
    );

    // Compile once with everything; if that fails, find out who broke it.
    let mut live = order.clone();
    let mut tried = scoring_folder(folder, &sources, candidates, &live, &placeholders)?;
    let mut rig = match Rig::new(gl, caps, &tried.0, options.score) {
        Ok(rig) => rig,
        Err(ScoreError::Player(_)) => {
            let mut survivors = Vec::new();
            for id in &live {
                let (single, _) =
                    scoring_folder(folder, &sources, candidates, &[*id], &placeholders)?;
                match Rig::new(gl, caps, &single, options.score) {
                    Ok(_) => survivors.push(*id),
                    Err(error) => {
                        if let Some(row) = rows.get_mut(*id) {
                            row.verdict = Verdict::DoesNotCompile {
                                message: first_line(&error),
                            };
                        }
                    }
                }
            }
            live = survivors;
            tried = scoring_folder(folder, &sources, candidates, &live, &placeholders)?;
            Rig::new(gl, caps, &tried.0, options.score)?
        }
        Err(other) => return Err(other.into()),
    };

    let probes: Vec<(&Candidate, usize)> = live
        .iter()
        .zip(&tried.1)
        .filter_map(|(id, control)| candidates.get(*id).map(|c| (c, *control)))
        .collect();
    let (measures, _) = score::measure(&mut rig, &probes)?;
    drop(rig);

    // Verdicts: dead, broken, or effective and ranked.
    let mut effective: Vec<(usize, f32)> = Vec::new();
    for (id, measure) in live.iter().zip(&measures) {
        let Some(row) = rows.get_mut(*id) else {
            continue;
        };
        row.measured(measure);
        match measure.broken {
            Some(reason) => row.verdict = Verdict::Broken { reason },
            None if measure.score < options.score.threshold => row.verdict = Verdict::Dead,
            None => effective.push((*id, round5(measure.score))),
        }
    }
    effective.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    // Kept: the strongest, within a fraction of the best, one Quality
    // control at most, `keep` in all (D259).
    let best = effective.first().map_or(0.0, |(_, score)| *score);
    let floor = score::weak_floor(best, &options.score);
    let mut quality_kept = false;
    let mut kept_ids: Vec<(usize, usize)> = Vec::new(); // (id, rank)
    for (index, (id, score)) in effective.iter().enumerate() {
        let rank = index + 1;
        let quality = candidates
            .get(*id)
            .is_some_and(|c| c.group == Group::Quality);
        let verdict = if *score < floor {
            Verdict::Weak { rank }
        } else if quality && quality_kept {
            Verdict::QualityLimit { rank }
        } else if kept_ids.len() >= options.keep {
            Verdict::Cut { rank }
        } else {
            quality_kept |= quality;
            kept_ids.push((*id, rank));
            Verdict::Kept { rank }
        };
        if let Some(row) = rows.get_mut(*id) {
            row.verdict = verdict;
        }
    }
    if kept_ids.is_empty() {
        return Ok(unchanged(report, rows));
    }

    // Verify: it compiles, and at its defaults it draws what the original
    // drew. If it does not, find the controls that change the default
    // picture and drop them: an unfaithful scene is never written (D259).
    let limit = FIDELITY_LIMIT + noise;
    let mut built = build(folder, &sources, candidates, &kept_ids, options.bind_audio)?;
    let mut fidelity = distance_from(gl, caps, &built.folder, options, &original_frames)?;
    if fidelity > limit {
        let mut offenders: Vec<(usize, f32)> = Vec::new();
        for kept in &kept_ids {
            let alone = build(folder, &sources, candidates, &[*kept], false)?;
            let difference = distance_from(gl, caps, &alone.folder, options, &original_frames)?;
            if difference > limit {
                offenders.push((kept.0, round5(difference)));
            }
        }
        if offenders.is_empty() {
            // Faithful one by one, not together: nothing can be kept.
            offenders = kept_ids
                .iter()
                .map(|(id, _)| (*id, round5(fidelity)))
                .collect();
        }
        for (id, difference) in &offenders {
            if let Some(row) = rows.get_mut(*id) {
                row.verdict = Verdict::Unfaithful {
                    difference: *difference,
                };
            }
        }
        kept_ids.retain(|(id, _)| !offenders.iter().any(|(bad, _)| bad == id));
        log::warn!(
            "{}: {} generated control(s) changed the default picture and were dropped",
            folder.def.title,
            offenders.len()
        );
        if kept_ids.is_empty() {
            return Ok(unchanged(report, rows));
        }
        built = build(folder, &sources, candidates, &kept_ids, options.bind_audio)?;
        fidelity = distance_from(gl, caps, &built.folder, options, &original_frames)?;
        if fidelity > limit {
            for (id, _) in &kept_ids {
                if let Some(row) = rows.get_mut(*id) {
                    row.verdict = Verdict::Unfaithful {
                        difference: round5(fidelity),
                    };
                }
            }
            return Ok(unchanged(report, rows));
        }
    }
    report.fidelity = Some(round5(fidelity));

    for ((id, _), name) in built.listing.iter().zip(&built.names) {
        if let Some(row) = rows.get_mut(*id) {
            row.name = Some(name.clone());
        }
    }
    report.kept = built.names.clone();
    report.suggestions = built.suggestions;
    report.candidates = rows;
    Ok(Generated {
        report,
        sources: built.sources,
        control_lines: built.control_lines,
        folder: built.folder,
    })
}
