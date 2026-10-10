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

use crate::analyze::analyze;
use crate::candidate::{Candidate, Kind};
use crate::naming::{Taken, assign};
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
        .filter(|c| !(c.name.starts_with("st_") && c.group.as_deref() == Some("Strobe")))
        .map(|c| c.name.clone())
        .collect()
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

/// Runs the whole engine on `folder`.
pub fn generate(
    gl: &Gl,
    caps: &Caps,
    folder: &SceneFolder,
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
    let analysis = analyze(&sources);
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
    if candidates.is_empty() {
        report.candidates = rows;
        return Ok(Generated {
            report,
            sources: sources.clone(),
            control_lines: Vec::new(),
            folder: folder.clone(),
        });
    }

    // The original, once: proof that it compiles, and the picture the
    // rewrite must reproduce.
    let original_frames = {
        let mut rig = Rig::new(gl, caps, folder, options.score).map_err(GenerateError::Original)?;
        rig.render(&[]).map_err(GenerateError::Original)?
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
    let mut kept_ids: Vec<(usize, usize)> = Vec::new(); // (id, rank)
    for (index, (id, _)) in effective.iter().enumerate() {
        let rank = index + 1;
        let kept = rank <= options.keep;
        if let Some(row) = rows.get_mut(*id) {
            row.verdict = if kept {
                Verdict::Kept { rank }
            } else {
                Verdict::Cut { rank }
            };
        }
        if kept {
            kept_ids.push((*id, rank));
        }
    }
    if kept_ids.is_empty() {
        report.candidates = rows;
        return Ok(Generated {
            report,
            sources: sources.clone(),
            control_lines: Vec::new(),
            folder: folder.clone(),
        });
    }

    // Listing order: group, then strength.
    let mut listing = kept_ids.clone();
    listing.sort_by_key(|(id, rank)| (candidates.get(*id).map(|c| c.group), *rank));
    // Numbered names follow the source order (`color_1` is the one that comes
    // first in the code), whatever order they are listed in.
    let mut in_source_order: Vec<usize> = kept_ids.iter().map(|(id, _)| *id).collect();
    in_source_order.sort_unstable();
    let bases: Vec<String> = in_source_order
        .iter()
        .filter_map(|id| candidates.get(*id).map(|c| c.name.clone()))
        .collect();
    let mut taken = Taken::of(folder, &sources);
    let assigned = assign(&bases, &mut taken);
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
    if options.bind_audio {
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

    let rewritten = rewrite::apply(&sources, candidates, &planned);
    let final_folder = with_controls(rewritten.apply_to(folder), &control_lines)?;

    // Verify: it compiles, and at its defaults it draws what the original drew.
    let mut check =
        Rig::new(gl, caps, &final_folder, options.score).map_err(GenerateError::Verify)?;
    let final_frames = check.render(&[]).map_err(GenerateError::Verify)?;
    drop(check);
    report.fidelity = Some(round5(score::difference(&original_frames, &final_frames)));

    for ((id, _), name) in listing.iter().zip(&names) {
        if let Some(row) = rows.get_mut(*id) {
            row.name = Some(name.clone());
        }
    }
    report.kept = names.clone();
    report.suggestions = suggestions;
    report.candidates = rows;
    Ok(Generated {
        report,
        sources: rewritten,
        control_lines,
        folder: final_folder,
    })
}
