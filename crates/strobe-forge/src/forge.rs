// ============================================================================
// Module:       strobe_forge::forge
// Description:  `strobe-forge make`'s loop: ask for a scene, check it, send
//               compile errors back for repair, then send the sheet and the
//               report back for critique rounds, keeping the best scene.
//
// Dependencies: anyhow; crate::author, crate::prompt, crate::reply
// ============================================================================

//! The loop (D562).
//!
//! 1. The model gets the skill as its system prompt and the picture.
//! 2. Each answer is parsed ([`crate::reply`]); a malformed one, or one
//!    that does not compile, goes back with the problem, up to
//!    [`MakeOptions::repairs`] times.
//! 3. A scene that compiles is checked ([`Checker`]); the report and the
//!    sheet go back with the skill's critique step, and the revision is
//!    checked the same way, for [`MakeOptions::rounds`] rounds.
//! 4. A revision that fails a hard gate, or never compiles, does not
//!    replace a scene that passed: the model is told so and the next
//!    critique is of the scene that stands.
//!
//! Every attempt is written to `<out>/rounds/<n>/` (round 0 is the first
//! answer, a repair overwrites its round) with its check in `check/`; the
//! scene that stands at the end is written to `<out>/` itself, with
//! `forge-log.md` saying what happened. The checker is a trait so the loop
//! is tested with a fake one, and the author is one so it is tested with
//! canned replies: no GL and no network in those tests.

use crate::author::{Author, Part};
use crate::prompt::{self, Skill};
use crate::reply::{self, Candidate};
use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};

/// What a check says about a candidate.
#[derive(Debug, Clone)]
pub(crate) struct Verdict {
    /// The compile log, when it does not compile in some dialect.
    pub(crate) compile_error: Option<String>,
    /// No hard gate failed.
    pub(crate) passed: bool,
    /// The report, as Markdown, to send back.
    pub(crate) report: String,
    /// The sheet, to send back.
    pub(crate) sheet: Option<PathBuf>,
}

/// Something that checks a scene folder, writing its outputs to `out`.
pub(crate) trait Checker {
    /// Checks the scene in `scene`.
    fn check(&mut self, scene: &Path, out: &Path) -> Result<Verdict>;
}

/// How many turns the loop may take.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MakeOptions {
    /// Critique rounds after the first scene.
    pub(crate) rounds: u32,
    /// Repairs per round for an answer that cannot be used or compiled.
    pub(crate) repairs: u32,
}

/// How the loop ended.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
    /// The scene written to the output folder passed every hard gate.
    pub(crate) passed: bool,
    /// The round whose scene stands, if any compiled.
    pub(crate) round: Option<u32>,
    /// What happened, one line a step.
    pub(crate) log: Vec<String>,
}

/// One round's result: the candidate it ended with and its verdict, or
/// `None` when no answer could even be parsed.
type Attempt = Option<(Candidate, Verdict)>;

/// Writes a candidate's two files into `dir`.
fn write_candidate(dir: &Path, candidate: &Candidate) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(dir.join("main.glsl"), &candidate.glsl)?;
    std::fs::write(dir.join("scene.json"), &candidate.json)?;
    Ok(())
}

/// Sends `turn`, then repairs until an answer compiles or the repairs run
/// out.
fn attempt(
    author: &mut dyn Author,
    checker: &mut dyn Checker,
    mut turn: Vec<Part>,
    dir: &Path,
    repairs: u32,
    log: &mut Vec<String>,
) -> Result<Attempt> {
    let mut last: Attempt = None;
    for repair in 0..=repairs {
        let answer = author.send(turn)?;
        let candidate = match reply::parse(&answer) {
            Ok(candidate) => candidate,
            Err(problem) => {
                log.push(format!("{}: unusable answer: {problem}", dir.display()));
                turn = prompt::repair(&problem);
                continue;
            }
        };
        write_candidate(dir, &candidate)?;
        let verdict = checker.check(dir, &dir.join("check"))?;
        match &verdict.compile_error {
            Some(error) => {
                log.push(format!(
                    "{}: does not compile (repair {repair})",
                    dir.display()
                ));
                turn = prompt::repair(&format!("The scene does not compile:\n\n{error}"));
                last = Some((candidate, verdict));
            }
            None => {
                log.push(format!(
                    "{}: compiles, {}",
                    dir.display(),
                    if verdict.passed {
                        "passes the gates"
                    } else {
                        "fails a gate"
                    }
                ));
                return Ok(Some((candidate, verdict)));
            }
        }
    }
    Ok(last)
}

/// Runs the loop for the picture in `picture`, writing into `out`.
pub(crate) fn make(
    author: &mut dyn Author,
    checker: &mut dyn Checker,
    skill: &Skill,
    picture: Part,
    out: &Path,
    options: MakeOptions,
) -> Result<Outcome> {
    author.start(&skill.system);
    let mut log = vec![
        format!("author: {}", author.describe()),
        format!("prompt: {}", skill.files.join(", ")),
    ];
    // The scene that stands: the last one that passed, else the last one
    // that compiled.
    let mut standing: Option<(u32, Candidate, Verdict)> = None;
    let mut turn = prompt::first(picture);
    for round in 0..=options.rounds {
        let dir = out.join("rounds").join(round.to_string());
        // A provider failure (timeout, overload, refusal, a cut-off answer)
        // ends the loop, not the run: the scene that stands is still written
        // below, with the log saying why it stopped.
        let result = match attempt(author, checker, turn, &dir, options.repairs, &mut log) {
            Ok(result) => result,
            Err(error) => {
                log.push(format!("round {round}: stopped: {error:#}"));
                break;
            }
        };
        let standing_passed = standing.as_ref().is_some_and(|(_, _, v)| v.passed);
        let note = match result {
            Some((candidate, verdict))
                if verdict.passed || (!standing_passed && verdict.compile_error.is_none()) =>
            {
                standing = Some((round, candidate, verdict));
                None
            }
            Some((_, verdict)) => Some(format!(
                "That revision {}, so the previous scene stands; the check below is of that one.",
                if verdict.compile_error.is_some() {
                    "never compiled"
                } else {
                    "fails a hard gate"
                }
            )),
            None => Some(
                "No answer in that round could be used, so the previous scene stands.".to_owned(),
            ),
        };
        if let Some(note) = &note {
            log.push(format!("round {round}: {note}"));
        }
        if round == options.rounds {
            break;
        }
        let Some((_, _, verdict)) = &standing else {
            log.push("nothing compiled; stopping".to_owned());
            break;
        };
        let sheet = match &verdict.sheet {
            Some(path) => Some(prompt::image_part(&std::fs::read(path)?)?),
            None => None,
        };
        turn = prompt::critique(&verdict.report, sheet, note.as_deref());
    }
    let outcome = match &standing {
        Some((round, candidate, verdict)) => {
            write_candidate(out, candidate)?;
            log.push(format!(
                "wrote round {round}'s scene to {} ({})",
                out.display(),
                if verdict.passed {
                    "passes"
                } else {
                    "FAILS a hard gate"
                }
            ));
            Outcome {
                passed: verdict.passed,
                round: Some(*round),
                log,
            }
        }
        None => {
            log.push("no scene compiled".to_owned());
            Outcome {
                passed: false,
                round: None,
                log,
            }
        }
    };
    std::fs::create_dir_all(out)?;
    let mut text = String::from("# Forge log\n\n");
    for line in &outcome.log {
        text.push_str("- ");
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(out.join("forge-log.md"), text)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use anyhow::{anyhow, ensure};
    use std::collections::VecDeque;

    /// Canned replies in order; records every turn it was sent.
    struct Fake {
        replies: VecDeque<String>,
        turns: Vec<Vec<Part>>,
        system: String,
    }

    impl Fake {
        fn new(replies: &[String]) -> Self {
            Self {
                replies: replies.iter().cloned().collect(),
                turns: Vec::new(),
                system: String::new(),
            }
        }

        fn text(&self, turn: usize) -> String {
            self.turns
                .get(turn)
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| match p {
                            Part::Text(t) => Some(t.as_str()),
                            Part::Image { .. } => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default()
        }
    }

    impl Author for Fake {
        fn describe(&self) -> String {
            "fake".to_owned()
        }
        fn start(&mut self, system: &str) {
            system.clone_into(&mut self.system);
        }
        fn send(&mut self, turn: Vec<Part>) -> Result<String> {
            self.turns.push(turn);
            self.replies
                .pop_front()
                .ok_or_else(|| anyhow!("the fake ran out of replies"))
        }
    }

    /// Fails to compile a shader containing `BROKEN`, fails the gates for
    /// one containing `BLACK`, passes anything else.
    struct FakeChecker;

    impl Checker for FakeChecker {
        fn check(&mut self, scene: &Path, out: &Path) -> Result<Verdict> {
            std::fs::create_dir_all(out)?;
            let glsl = std::fs::read_to_string(scene.join("main.glsl"))?;
            Ok(Verdict {
                compile_error: glsl
                    .contains("BROKEN")
                    .then(|| "0:3: 'BROKEN' undeclared".to_owned()),
                passed: !glsl.contains("BROKEN") && !glsl.contains("BLACK"),
                report: format!("report of {} bytes", glsl.len()),
                sheet: None,
            })
        }
    }

    fn skill() -> Skill {
        Skill {
            system: "the skill".to_owned(),
            files: vec!["SKILL.md".to_owned()],
        }
    }

    fn picture() -> Part {
        Part::Image {
            media_type: "image/jpeg".to_owned(),
            base64: "AA==".to_owned(),
        }
    }

    const BROKEN: &str = "vec4 renderMain(void) { return BROKEN; }\n";
    const BLACK: &str = "vec4 renderMain(void) { return vec4(0.0); } // BLACK\n";

    #[test]
    fn a_compile_error_goes_back_and_the_fix_is_written() -> Result<()> {
        let out = crate::fixture::tempdir("loop-repair")?;
        let mut author = Fake::new(&[fixture::reply(BROKEN), fixture::reply(fixture::GLSL)]);
        let outcome = make(
            &mut author,
            &mut FakeChecker,
            &skill(),
            picture(),
            &out,
            MakeOptions {
                rounds: 0,
                repairs: 3,
            },
        )?;
        ensure!(
            outcome.passed && outcome.round == Some(0),
            "passes: {outcome:?}"
        );
        ensure!(
            author.system == "the skill",
            "the skill is the system prompt"
        );
        ensure!(author.turns.len() == 2, "the request and one repair");
        ensure!(
            matches!(
                author.turns.first().and_then(|t| t.first()),
                Some(Part::Image { .. })
            ),
            "the picture goes first"
        );
        ensure!(
            author.text(1).contains("does not compile")
                && author.text(1).contains("'BROKEN' undeclared"),
            "the repair carries the log: {}",
            author.text(1)
        );
        let written = std::fs::read_to_string(out.join("main.glsl"))?;
        ensure!(
            written.trim() == fixture::GLSL.trim(),
            "the fix is the scene: {written}"
        );
        ensure!(out.join("forge-log.md").is_file(), "the log is written");
        Ok(())
    }

    #[test]
    fn a_critique_round_that_breaks_the_scene_does_not_replace_it() -> Result<()> {
        let out = crate::fixture::tempdir("loop-critique")?;
        let mut author = Fake::new(&[
            fixture::reply(fixture::GLSL),
            fixture::reply(BLACK),
            fixture::reply(BROKEN),
            fixture::reply(BROKEN),
        ]);
        let outcome = make(
            &mut author,
            &mut FakeChecker,
            &skill(),
            picture(),
            &out,
            MakeOptions {
                rounds: 2,
                repairs: 1,
            },
        )?;
        ensure!(
            outcome.passed && outcome.round == Some(0),
            "round 0 stands: {outcome:?}"
        );
        ensure!(
            author.text(1).contains("report of") && author.text(1).contains("critique.md"),
            "round 1 is a critique of the report: {}",
            author.text(1)
        );
        ensure!(
            author
                .text(2)
                .contains("fails a hard gate, so the previous scene stands"),
            "the model is told its revision was rejected: {}",
            author.text(2)
        );
        let kept = std::fs::read_to_string(out.join("main.glsl"))?;
        ensure!(
            kept.trim() == fixture::GLSL.trim(),
            "the passing scene is kept: {kept}"
        );
        let rejected = std::fs::read_to_string(out.join("rounds").join("1").join("main.glsl"))?;
        ensure!(
            rejected.contains("BLACK"),
            "the rejected round is still on disk"
        );
        ensure!(
            author.turns.len() == 4,
            "request, critique, critique, one repair: {}",
            author.turns.len()
        );
        Ok(())
    }

    #[test]
    fn a_failed_request_keeps_the_scene_that_stands() -> Result<()> {
        let out = crate::fixture::tempdir("loop-api-error")?;
        // One good reply, then the fake fails the way a timed-out request does.
        let mut author = Fake::new(&[fixture::reply(fixture::GLSL)]);
        let outcome = make(
            &mut author,
            &mut FakeChecker,
            &skill(),
            picture(),
            &out,
            MakeOptions {
                rounds: 2,
                repairs: 1,
            },
        )?;
        ensure!(
            outcome.passed && outcome.round == Some(0),
            "round 0 still stands: {outcome:?}"
        );
        let kept = std::fs::read_to_string(out.join("main.glsl"))?;
        ensure!(
            kept.trim() == fixture::GLSL.trim(),
            "the passing scene is written: {kept}"
        );
        let log = std::fs::read_to_string(out.join("forge-log.md"))?;
        ensure!(
            log.contains("stopped: the fake ran out of replies"),
            "the log says why it stopped: {log}"
        );
        Ok(())
    }

    #[test]
    fn answers_that_never_parse_end_without_a_scene() -> Result<()> {
        let out = crate::fixture::tempdir("loop-garbage")?;
        let garbage = "I cannot do that.".to_owned();
        let mut author = Fake::new(&[garbage.clone(), garbage.clone(), garbage]);
        let outcome = make(
            &mut author,
            &mut FakeChecker,
            &skill(),
            picture(),
            &out,
            MakeOptions {
                rounds: 1,
                repairs: 2,
            },
        )?;
        ensure!(
            !outcome.passed && outcome.round.is_none(),
            "no scene: {outcome:?}"
        );
        ensure!(
            author.text(1).contains("could not be used"),
            "each repair says why: {}",
            author.text(1)
        );
        ensure!(
            !out.join("main.glsl").exists(),
            "nothing written as the scene"
        );
        Ok(())
    }
}
