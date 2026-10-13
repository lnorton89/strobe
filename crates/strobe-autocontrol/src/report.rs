// ============================================================================
// Module:       strobe_autocontrol::report
// Description:  The report: every candidate with its measured score and
//               verdict, the controls kept, and the audio suggestions.
//
// Dependencies: crate::candidate, crate::score, crate::suggest, serde
// ============================================================================

//! The report.
//!
//! One value that says everything the engine decided and why, so a person
//! (or a golden test) can see why a literal was or was not offered. It
//! serialises to JSON and prints as text. Scores are rounded to five
//! decimals so a report is byte-stable: two runs on one machine are equal.

use crate::candidate::{Candidate, Group, Kind, Site, ValueType};
use crate::score::{Broken, Measure, ScoreOptions};
use crate::suggest::AudioSuggestion;
use serde::Serialize;
use std::fmt::Write as _;

/// What became of a candidate.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "verdict")]
pub enum Verdict {
    /// It is a generated control. `rank` counts from 1, strongest first.
    Kept {
        /// Position by measured effect.
        rank: usize,
    },
    /// It was scored and ranked below the `keep` limit.
    Cut {
        /// Position by measured effect.
        rank: usize,
    },
    /// Visible, but under [`crate::score::weak_floor`]: a small fraction of
    /// the strongest candidate's score and under one percent of full scale.
    /// Next to the strongest it is noise on the panel (D259).
    Weak {
        /// Position by measured effect.
        rank: usize,
    },
    /// A loop count or draw distance when a stronger one was already kept:
    /// a scene gets one Quality control (D259).
    QualityLimit {
        /// Position by measured effect.
        rank: usize,
    },
    /// With it at its default the rewritten scene did not draw the
    /// original, so it was dropped rather than ship an unfaithful scene
    /// (D259).
    Unfaithful {
        /// The mean image difference from the original, `0..1`.
        difference: f32,
    },
    /// Its effect on the picture was below the visibility threshold.
    Dead,
    /// A nudge turned a lit picture black.
    Broken {
        /// How.
        reason: Broken,
    },
    /// The rewritten scene did not compile with it.
    DoesNotCompile {
        /// The first line of the compiler's complaint.
        message: String,
    },
    /// There were more candidates than the scoring budget; this one was not
    /// tried.
    NotScored,
}

/// One candidate and its fate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CandidateReport {
    /// Position in the analysis order.
    pub id: usize,
    /// What the literal was doing.
    pub kind: Kind,
    /// The group its control is listed in.
    pub group: Group,
    /// What the value is.
    pub value_type: ValueType,
    /// The control's name, when it was kept.
    pub name: Option<String>,
    /// The base name the analysis suggested.
    pub suggested_name: String,
    /// The original value.
    pub default: Vec<f32>,
    /// The control's lower bound.
    pub min: Vec<f32>,
    /// The control's upper bound.
    pub max: Vec<f32>,
    /// The line of code, for people.
    pub context: String,
    /// Every place it is applied.
    pub sites: Vec<Site>,
    /// Mean per-channel image difference when nudged, `0..1`.
    pub score: Option<f32>,
    /// The difference when nudged down.
    pub low: Option<f32>,
    /// The difference when nudged up.
    pub high: Option<f32>,
    /// What was decided.
    pub verdict: Verdict,
}

/// The settings a report was made with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportOptions {
    /// How many controls at most.
    pub keep: usize,
    /// How many candidates at most were scored.
    pub max_scored: usize,
    /// The measuring setup.
    pub score: ScoreOptions,
    /// Whether audio mappings were written into the scene.
    pub bind_audio: bool,
}

/// Everything the engine decided about one scene.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The scene's title.
    pub scene: String,
    /// The GLSL dialect the scene was measured in.
    pub dialect: String,
    /// The settings.
    pub options: ReportOptions,
    /// The mean image difference between the original scene and the
    /// rewritten one with every control at its default; zero when the
    /// rewrite is faithful.
    pub fidelity: Option<f32>,
    /// Every candidate found, in analysis order.
    pub candidates: Vec<CandidateReport>,
    /// The names of the controls written, in the order they are listed.
    pub kept: Vec<String>,
    /// Reactive mappings proposed for the strongest controls.
    pub suggestions: Vec<AudioSuggestion>,
}

/// Rounds a score to five decimals, so equal runs print equal numbers.
pub fn round5(value: f32) -> f32 {
    (value * 100_000.0).round() / 100_000.0
}

impl CandidateReport {
    /// A report row for `candidate` before anything has been decided.
    pub fn new(candidate: &Candidate, verdict: Verdict) -> Self {
        Self {
            id: candidate.id,
            kind: candidate.kind,
            group: candidate.group,
            value_type: candidate.value_type,
            name: None,
            suggested_name: candidate.name.clone(),
            default: candidate.default.clone(),
            min: candidate.min.clone(),
            max: candidate.max.clone(),
            context: candidate.context.clone(),
            sites: candidate.sites.clone(),
            score: None,
            low: None,
            high: None,
            verdict,
        }
    }

    /// Records what was measured.
    pub fn measured(&mut self, measure: &Measure) {
        self.score = Some(round5(measure.score));
        self.low = Some(round5(measure.low));
        self.high = Some(round5(measure.high));
    }

    /// Whether this candidate became a control.
    pub fn is_kept(&self) -> bool {
        matches!(self.verdict, Verdict::Kept { .. })
    }
}

fn values(values: &[f32]) -> String {
    let parts: Vec<String> = values.iter().map(|v| format!("{v}")).collect();
    parts.join(",")
}

impl Verdict {
    fn label(&self) -> String {
        match self {
            Self::Kept { rank } => format!("KEPT #{rank}"),
            Self::Cut { rank } => format!("cut (rank {rank})"),
            Self::Weak { rank } => format!("weak (rank {rank})"),
            Self::QualityLimit { rank } => format!("one quality control already (rank {rank})"),
            Self::Unfaithful { difference } => format!("unfaithful ({difference:.5})"),
            Self::Dead => "dead".to_string(),
            Self::Broken { reason } => format!("broken ({reason:?})"),
            Self::DoesNotCompile { .. } => "does not compile".to_string(),
            Self::NotScored => "not scored".to_string(),
        }
    }
}

impl Report {
    /// The report as pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// The controls that were kept, in listing order.
    pub fn kept_candidates(&self) -> Vec<&CandidateReport> {
        let mut kept: Vec<&CandidateReport> =
            self.candidates.iter().filter(|c| c.is_kept()).collect();
        kept.sort_by_key(|c| (c.group, c.id));
        kept
    }

    /// A short summary: counts, and each kept control on one line.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let count =
            |f: &dyn Fn(&Verdict) -> bool| self.candidates.iter().filter(|c| f(&c.verdict)).count();
        let _ = writeln!(
            out,
            "{}: {} candidates, {} kept ({} dead, {} broken, {} cut, {} not compiled)",
            self.scene,
            self.candidates.len(),
            count(&|v| matches!(v, Verdict::Kept { .. })),
            count(&|v| matches!(v, Verdict::Dead)),
            count(&|v| matches!(v, Verdict::Broken { .. })),
            count(&|v| matches!(v, Verdict::Cut { .. })),
            count(&|v| matches!(v, Verdict::DoesNotCompile { .. })),
        );
        let weak = count(&|v| matches!(v, Verdict::Weak { .. }));
        let limited = count(&|v| matches!(v, Verdict::QualityLimit { .. }));
        let unfaithful = count(&|v| matches!(v, Verdict::Unfaithful { .. }));
        if weak + limited + unfaithful > 0 {
            let _ = writeln!(
                out,
                "  also not kept: {weak} weak next to the strongest, {limited} over the one-quality-control limit, {unfaithful} unfaithful at defaults"
            );
        }
        for c in self.kept_candidates() {
            let _ = writeln!(
                out,
                "  {:<8} {:<20} {:>8}  {}",
                c.group.ui_name(),
                c.name.as_deref().unwrap_or("?"),
                c.score.map_or_else(String::new, |s| format!("{s:.4}")),
                c.context
            );
        }
        for s in &self.suggestions {
            let _ = writeln!(
                out,
                "  suggest: {} <- {} ({:.0}%){}",
                s.control,
                s.band,
                s.amount * 100.0,
                if s.expressible { "" } else { " [data only]" }
            );
        }
        out
    }

    /// Every candidate, one block each, with scores and verdicts.
    pub fn detail(&self) -> String {
        let mut out = self.summary();
        let _ = writeln!(
            out,
            "\n{} candidates ({} dialect, {}x{}, {} frames, threshold {})",
            self.candidates.len(),
            self.dialect,
            self.options.score.size[0],
            self.options.score.size[1],
            self.options.score.frames,
            self.options.score.threshold
        );
        if let Some(fidelity) = self.fidelity {
            let _ = writeln!(out, "fidelity of the rewrite at defaults: {fidelity:.5}");
        }
        for c in &self.candidates {
            let place = c.sites.first().map_or_else(String::new, |s| {
                format!("{}:{}:{}", s.file, s.line, s.column)
            });
            let more = c.sites.len().saturating_sub(1);
            let _ = writeln!(
                out,
                "#{:<3} {:<10} {:<8} {:<18} default {} range {}..{}  {}{}",
                c.id,
                c.kind.label(),
                c.group.ui_name(),
                c.name.as_deref().unwrap_or(&c.suggested_name),
                values(&c.default),
                values(&c.min),
                values(&c.max),
                place,
                if more > 0 {
                    format!(" (+{more})")
                } else {
                    String::new()
                }
            );
            let _ = writeln!(
                out,
                "       score {}  (down {}, up {})  {}  | {}",
                c.score
                    .map_or_else(|| "-".to_string(), |s| format!("{s:.5}")),
                c.low.map_or_else(|| "-".to_string(), |s| format!("{s:.5}")),
                c.high
                    .map_or_else(|| "-".to_string(), |s| format!("{s:.5}")),
                c.verdict.label(),
                c.context
            );
            if let Verdict::DoesNotCompile { message } = &c.verdict {
                let _ = writeln!(out, "       {message}");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_round_to_five_decimals() {
        assert_eq!(round5(0.123_456_7), 0.123_46, "rounded");
        assert_eq!(round5(0.0), 0.0, "zero");
    }

    #[test]
    fn a_verdict_serialises_with_its_tag() {
        let json = serde_json::to_string(&Verdict::Kept { rank: 2 });
        assert_eq!(
            json.as_deref().unwrap_or(""),
            r#"{"verdict":"kept","rank":2}"#,
            "an internally tagged verdict"
        );
    }
}
