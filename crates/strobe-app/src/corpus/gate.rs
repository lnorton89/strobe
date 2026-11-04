// ============================================================================
// Module:       strobe_app::corpus::gate
// Description:  The corpus as a regression gate: a run's report and final
//               frames compared against a committed expectations file and
//               baseline images, with diffs written for review, and a bless
//               that rewrites both.
//
// Dependencies: serde, serde_json, image; crate::corpus
// ============================================================================

//! The corpus harness measures; this module judges (D534).
//!
//! `strobe-corpus --expect <file.json>` runs a library as usual, then
//! compares the run with the file:
//!
//! - **Completeness.** Every scene and dialect the file expects was
//!   measured, and nothing was measured that the file does not name. A
//!   fixture that was deleted, renamed or filtered away fails; so does a new
//!   scene nobody wrote an expectation for.
//! - **Status.** Each measured status is the expected one. A scene that
//!   newly fails fails the gate, and so does one that newly passes: the
//!   expectation is then out of date, and updating it is a reviewed change.
//!   A worker that crashed or hung is a `gl_error` or `timeout` record, so
//!   it can never read as a pass.
//! - **Settings.** The run used the frames and size the file was written
//!   for, so a baseline is compared with a frame drawn the same way.
//! - **Images.** For the scenes the file lists under `images`, the final
//!   frame in each dialect the scene is expected `ok` in is compared with
//!   `<baselines>/<scene>.<dialect>.png`. A pixel differs when any channel
//!   is more than `tolerance.channel` levels away; the frame fails when more
//!   than `tolerance.fraction` of its pixels differ. Mesa's llvmpipe draws
//!   the same frame every run, but not across every Mesa release, so the
//!   default allows a little, and a shift that a person would see fails.
//!   Every failed comparison leaves `diff/<scene>.<dialect>/` in the output
//!   folder with `baseline.png`, `candidate.png` and `diff.png` (each
//!   channel's difference, amplified), so the change is reviewable from
//!   the CI artefact.
//!
//! `--bless` writes the run back instead: the statuses into the file (its
//! image list and tolerance kept), and the listed scenes' frames into the
//! baselines. Baselines come from the environment that checks them, CI's
//! Linux runner; a blessed frame from another driver would fail there.

use super::driver::frame_name;
use super::report::Report;
use super::{Dialect, Settings, Status};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// How far a frame may drift from its baseline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tolerance {
    /// Levels (of 255) a channel may differ before its pixel counts as
    /// changed.
    pub channel: u8,
    /// The share of pixels, 0 to 1, that may change before the frame fails.
    pub fraction: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        // 16 levels absorbs a rounding or a filtering change in one tap;
        // 0.5 % of a 160x90 frame is 72 pixels, an edge's worth, not a
        // shape's.
        Self {
            channel: 16,
            fraction: 0.005,
        }
    }
}

/// The committed expectations for one library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expectations {
    /// What the run must be measured with.
    pub settings: Settings,
    /// Scene name to the status expected in each dialect.
    pub scenes: BTreeMap<String, BTreeMap<Dialect, Status>>,
    /// Scenes whose final frames are compared with baselines.
    #[serde(default)]
    pub images: Vec<String>,
    /// How far a compared frame may drift.
    #[serde(default)]
    pub tolerance: Tolerance,
}

impl Expectations {
    /// Reads an expectations file.
    ///
    /// # Errors
    ///
    /// A message naming the file when it cannot be read or parsed.
    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The file's text: pretty JSON with a final newline.
    ///
    /// # Errors
    ///
    /// Only if serialisation fails, which these types cannot cause.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self).map(|mut text| {
            text.push('\n');
            text
        })
    }
}

/// One frame compared with its baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Compared {
    /// `<scene>.<dialect>`.
    pub name: String,
    /// Pixels that differed beyond the channel tolerance.
    pub changed: usize,
    /// Pixels in the frame.
    pub pixels: usize,
}

/// What the gate found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Verdict {
    /// Every reason the gate fails, one line each. Empty means it passed.
    pub problems: Vec<String>,
    /// The frames that were compared, passed or not.
    pub compared: Vec<Compared>,
    /// How many scene-and-dialect statuses matched.
    pub matched: usize,
}

impl Verdict {
    /// Whether the run matches its expectations.
    pub fn passed(&self) -> bool {
        self.problems.is_empty()
    }

    /// `gate.md`.
    pub fn to_markdown(&self, expectations: &Path) -> String {
        let mut out = String::from("# Corpus gate\n\n");
        let _ = writeln!(
            out,
            "Against `{}`: {} status(es) matched, {} frame(s) compared.\n",
            expectations.display(),
            self.matched,
            self.compared.len()
        );
        if self.passed() {
            out.push_str("Passed.\n");
        } else {
            let _ = writeln!(out, "**Failed**, {} problem(s):\n", self.problems.len());
            for problem in &self.problems {
                let _ = writeln!(out, "- {problem}");
            }
            out.push_str(
                "\nIf the change is meant to alter these, rerun with `--bless` on CI's Linux \
                 image (or take the frames from this run's artefact) and commit the result.\n",
            );
        }
        if !self.compared.is_empty() {
            out.push_str("\n| frame | changed pixels |\n|---|---:|\n");
            for frame in &self.compared {
                let _ = writeln!(
                    out,
                    "| {} | {} of {} |",
                    frame.name, frame.changed, frame.pixels
                );
            }
        }
        out
    }
}

/// Compares a run's statuses and settings with the expectations.
pub fn check_statuses(report: &Report, expect: &Expectations) -> Verdict {
    let mut verdict = Verdict::default();
    if report.settings != expect.settings {
        verdict.problems.push(format!(
            "the run used {} frame(s) at {}x{}, the expectations were written for {} at {}x{}",
            report.settings.frames,
            report.settings.size[0],
            report.settings.size[1],
            expect.settings.frames,
            expect.settings.size[0],
            expect.settings.size[1],
        ));
    }
    if report.records.is_empty() {
        verdict.problems.push("nothing was measured".to_string());
    }
    for (scene, dialects) in &expect.scenes {
        for (dialect, want) in dialects {
            let found = report
                .records
                .iter()
                .find(|r| &r.scene == scene && r.dialect == *dialect);
            match found {
                None => verdict.problems.push(format!(
                    "`{scene}` ({}) is expected `{}` but was not measured: a fixture was removed, \
                     renamed or filtered out",
                    dialect.label(),
                    want.label()
                )),
                Some(record) if record.status == *want => verdict.matched += 1,
                Some(record) => verdict.problems.push(format!(
                    "`{scene}` ({}) is expected `{}` and was `{}`{}",
                    dialect.label(),
                    want.label(),
                    record.status.label(),
                    record
                        .error
                        .as_deref()
                        .map_or_else(String::new, |error| format!(": {error}")),
                )),
            }
        }
    }
    for record in &report.records {
        let known = expect
            .scenes
            .get(&record.scene)
            .is_some_and(|dialects| dialects.contains_key(&record.dialect));
        if !known {
            verdict.problems.push(format!(
                "`{}` ({}) was measured (`{}`) but has no expectation: add it to the file",
                record.scene,
                record.dialect.label(),
                record.status.label()
            ));
        }
    }
    for scene in &expect.images {
        if !expect.scenes.contains_key(scene) {
            verdict.problems.push(format!(
                "`{scene}` is listed under images but not under scenes"
            ));
        }
    }
    verdict
}

/// The frames the expectations compare: each listed scene in each dialect
/// it is expected `ok` in.
fn imaged(expect: &Expectations) -> Vec<(&str, Dialect, String)> {
    let mut names = Vec::new();
    for scene in &expect.images {
        let Some(dialects) = expect.scenes.get(scene) else {
            continue;
        };
        for (dialect, status) in dialects {
            if *status == Status::Ok {
                names.push((scene.as_str(), *dialect, frame_name(scene, *dialect)));
            }
        }
    }
    names
}

/// Compares the listed scenes' frames in `frames` with `baselines`, adding
/// to `verdict`, and writes each failed comparison's images under `diffs`.
/// A frame whose scene did not draw in `report` is not looked for: its
/// status is already a problem, and a second line for the same cause would
/// bury it.
pub fn check_images(
    expect: &Expectations,
    report: &Report,
    frames: &Path,
    baselines: &Path,
    diffs: &Path,
    verdict: &mut Verdict,
) {
    for (scene, dialect, file) in imaged(expect) {
        let drew = report
            .records
            .iter()
            .any(|r| r.scene == scene && r.dialect == dialect && r.status == Status::Ok);
        if !drew {
            continue;
        }
        let name = file.trim_end_matches(".png").to_string();
        let candidate_path = frames.join(&file);
        let baseline_path = baselines.join(&file);
        let candidate = match image::open(&candidate_path) {
            Ok(image) => image.to_rgba8(),
            Err(error) => {
                verdict.problems.push(format!(
                    "`{name}`: no frame from this run ({}: {error})",
                    candidate_path.display()
                ));
                continue;
            }
        };
        let baseline = match image::open(&baseline_path) {
            Ok(image) => image.to_rgba8(),
            Err(error) => {
                verdict.problems.push(format!(
                    "`{name}`: no baseline ({}: {error}); bless it on CI's Linux image",
                    baseline_path.display()
                ));
                continue;
            }
        };
        let dir = diffs.join(&name);
        if candidate.dimensions() != baseline.dimensions() {
            verdict.problems.push(format!(
                "`{name}`: the frame is {:?} and the baseline {:?}",
                candidate.dimensions(),
                baseline.dimensions()
            ));
            keep(&dir, &baseline, &candidate, None, verdict);
            continue;
        }
        let (changed, diff) = difference(&baseline, &candidate, expect.tolerance.channel);
        let pixels = candidate.pixels().len();
        verdict.compared.push(Compared {
            name: name.clone(),
            changed,
            pixels,
        });
        let allowed = expect.tolerance.fraction * pixels as f64;
        if changed as f64 > allowed {
            verdict.problems.push(format!(
                "`{name}`: {changed} of {pixels} pixels changed by more than {} levels (allowed {:.0}); see diff/{name}/",
                expect.tolerance.channel, allowed
            ));
            keep(&dir, &baseline, &candidate, Some(&diff), verdict);
        }
    }
}

/// The pixels that differ by more than `channel` in any of red, green and
/// blue, and an image of the difference, each channel amplified four times.
fn difference(
    baseline: &image::RgbaImage,
    candidate: &image::RgbaImage,
    channel: u8,
) -> (usize, image::RgbaImage) {
    let mut diff = image::RgbaImage::new(candidate.width(), candidate.height());
    let mut changed = 0;
    for ((was, now), out) in baseline
        .pixels()
        .zip(candidate.pixels())
        .zip(diff.pixels_mut())
    {
        let mut worst = 0;
        let mut shown = [0u8, 0, 0, 255];
        for ((shown, was), now) in shown.iter_mut().zip(&was.0[..3]).zip(&now.0[..3]) {
            let delta = was.abs_diff(*now);
            worst = worst.max(delta);
            *shown = delta.saturating_mul(4);
        }
        if worst > channel {
            changed += 1;
        }
        *out = image::Rgba(shown);
    }
    (changed, diff)
}

/// Writes a failed comparison's images for review. Failing to write them is
/// itself a problem, since the reviewer then has nothing to look at.
fn keep(
    dir: &Path,
    baseline: &image::RgbaImage,
    candidate: &image::RgbaImage,
    diff: Option<&image::RgbaImage>,
    verdict: &mut Verdict,
) {
    let written = std::fs::create_dir_all(dir)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            baseline
                .save(dir.join("baseline.png"))
                .and_then(|()| candidate.save(dir.join("candidate.png")))
                .and_then(|()| diff.map_or(Ok(()), |diff| diff.save(dir.join("diff.png"))))
                .map_err(|error| error.to_string())
        });
    if let Err(error) = written {
        verdict.problems.push(format!(
            "cannot write the diff in {}: {error}",
            dir.display()
        ));
    }
}

/// The expectations rewritten from this run: its settings and statuses,
/// with the image list and tolerance kept (dropping a listed scene that is
/// gone).
pub fn blessed(report: &Report, old: &Expectations) -> Expectations {
    let mut scenes: BTreeMap<String, BTreeMap<Dialect, Status>> = BTreeMap::new();
    for record in &report.records {
        scenes
            .entry(record.scene.clone())
            .or_default()
            .insert(record.dialect, record.status);
    }
    let images = old
        .images
        .iter()
        .filter(|scene| scenes.contains_key(*scene))
        .cloned()
        .collect();
    Expectations {
        settings: report.settings,
        scenes,
        images,
        tolerance: old.tolerance,
    }
}

/// Copies the frames `expect` compares from `frames` into `baselines`.
///
/// # Errors
///
/// A message naming the frame that could not be copied.
pub fn bless_frames(
    expect: &Expectations,
    frames: &Path,
    baselines: &Path,
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(baselines)
        .map_err(|error| format!("cannot create {}: {error}", baselines.display()))?;
    let mut written = Vec::new();
    for (_, _, file) in imaged(expect) {
        let from = frames.join(&file);
        let to = baselines.join(&file);
        std::fs::copy(&from, &to).map_err(|error| {
            format!(
                "cannot copy {} to {}: {error}",
                from.display(),
                to.display()
            )
        })?;
        written.push(to);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::Record;
    use anyhow::Result;

    fn report(records: Vec<Record>) -> Report {
        Report::new("lib", Settings::default(), 60, Vec::new(), records)
    }

    fn ok(scene: &str, dialect: Dialect) -> Record {
        Record::new(scene, dialect)
    }

    fn expect(scenes: &[(&str, Status, Status)]) -> Expectations {
        Expectations {
            settings: Settings::default(),
            scenes: scenes
                .iter()
                .map(|(scene, desktop, gles)| {
                    (
                        (*scene).to_string(),
                        BTreeMap::from([(Dialect::Desktop, *desktop), (Dialect::Gles, *gles)]),
                    )
                })
                .collect(),
            images: Vec::new(),
            tolerance: Tolerance::default(),
        }
    }

    fn both_ok(scene: &str) -> Vec<Record> {
        vec![ok(scene, Dialect::Desktop), ok(scene, Dialect::Gles)]
    }

    #[test]
    fn a_run_that_matches_passes() {
        let verdict = check_statuses(
            &report(both_ok("a")),
            &expect(&[("a", Status::Ok, Status::Ok)]),
        );
        assert!(verdict.passed(), "{:?}", verdict.problems);
        assert_eq!(verdict.matched, 2, "both dialects matched");
    }

    #[test]
    fn a_removed_fixture_fails_completeness() {
        let verdict = check_statuses(
            &report(both_ok("a")),
            &expect(&[
                ("a", Status::Ok, Status::Ok),
                ("gone", Status::Ok, Status::Ok),
            ]),
        );
        assert_eq!(verdict.problems.len(), 2, "{:?}", verdict.problems);
        assert!(
            verdict
                .problems
                .iter()
                .all(|p| p.contains("`gone`") && p.contains("not measured")),
            "{:?}",
            verdict.problems
        );
    }

    #[test]
    fn a_new_scene_without_an_expectation_fails() {
        let mut records = both_ok("a");
        records.extend(both_ok("new"));
        let verdict = check_statuses(&report(records), &expect(&[("a", Status::Ok, Status::Ok)]));
        assert!(
            verdict
                .problems
                .iter()
                .any(|p| p.contains("`new`") && p.contains("no expectation")),
            "{:?}",
            verdict.problems
        );
    }

    #[test]
    fn a_broken_script_uniform_fails_the_gate() {
        // What a regression in script-created uniforms (D62) looks like: the
        // scene that compiled because its script ran no longer does.
        let records = vec![
            Record::failed(
                "script-uniform",
                Dialect::Desktop,
                Status::CompileError,
                "0:1(1): error: `tint' undeclared",
            ),
            ok("script-uniform", Dialect::Gles),
        ];
        let verdict = check_statuses(
            &report(records),
            &expect(&[("script-uniform", Status::Ok, Status::Ok)]),
        );
        assert!(
            verdict
                .problems
                .iter()
                .any(|p| p.contains("expected `ok` and was `compile_error`")
                    && p.contains("undeclared")),
            "{:?}",
            verdict.problems
        );
    }

    #[test]
    fn a_crashed_or_hung_worker_never_reads_as_a_pass() {
        let records = vec![
            Record::failed(
                "a",
                Dialect::Desktop,
                Status::GlError,
                "the worker died (exit code 139)",
            ),
            Record::failed("a", Dialect::Gles, Status::Timeout, "no result within 60 s"),
        ];
        let verdict = check_statuses(&report(records), &expect(&[("a", Status::Ok, Status::Ok)]));
        assert_eq!(verdict.problems.len(), 2, "{:?}", verdict.problems);
        assert!(!verdict.passed(), "a crash and a hang fail");
    }

    #[test]
    fn a_scene_that_newly_passes_fails_until_its_expectation_moves() {
        let verdict = check_statuses(
            &report(both_ok("fixed")),
            &expect(&[("fixed", Status::CompileError, Status::CompileError)]),
        );
        assert_eq!(verdict.problems.len(), 2, "{:?}", verdict.problems);
    }

    #[test]
    fn different_settings_fail_and_an_empty_run_fails() {
        let mut expectations = expect(&[]);
        expectations.settings.frames = 9;
        let verdict = check_statuses(&report(Vec::new()), &expectations);
        assert!(
            verdict
                .problems
                .iter()
                .any(|p| p.contains("expectations were written for 9")),
            "{:?}",
            verdict.problems
        );
        assert!(
            verdict.problems.iter().any(|p| p == "nothing was measured"),
            "{:?}",
            verdict.problems
        );
    }

    /// A temp folder, removed on drop.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Result<Self> {
            let dir = std::env::temp_dir()
                .join(format!("strobe-gate-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            // Scratch space; a leftover is harmless.
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn solid(rgb: [u8; 3]) -> image::RgbaImage {
        image::RgbaImage::from_pixel(20, 10, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
    }

    fn ran() -> Report {
        report(vec![ok("a", Dialect::Desktop)])
    }

    fn imaged_expectations() -> Expectations {
        let mut expectations = expect(&[("a", Status::Ok, Status::CompileError)]);
        expectations.images = vec!["a".to_string()];
        expectations
    }

    #[test]
    fn a_frame_within_tolerance_passes_and_a_changed_one_leaves_a_diff() -> Result<()> {
        let temp = Temp::new("images")?;
        let (frames, baselines, diffs) = (
            temp.0.join("frames"),
            temp.0.join("baselines"),
            temp.0.join("diff"),
        );
        std::fs::create_dir_all(&frames)?;
        std::fs::create_dir_all(&baselines)?;
        solid([100, 100, 100]).save(baselines.join("a.desktop.png"))?;
        solid([108, 100, 95]).save(frames.join("a.desktop.png"))?;
        let expectations = imaged_expectations();
        let ran = ran();

        let mut verdict = Verdict::default();
        check_images(
            &expectations,
            &ran,
            &frames,
            &baselines,
            &diffs,
            &mut verdict,
        );
        assert!(
            verdict.passed(),
            "8 levels is within 16: {:?}",
            verdict.problems
        );
        assert_eq!(
            verdict.compared.len(),
            1,
            "only the dialect expected ok is compared"
        );

        let mut changed = solid([100, 100, 100]);
        for x in 0..10 {
            changed.put_pixel(x, 0, image::Rgba([255, 0, 0, 255]));
        }
        changed.save(frames.join("a.desktop.png"))?;
        let mut verdict = Verdict::default();
        check_images(
            &expectations,
            &ran,
            &frames,
            &baselines,
            &diffs,
            &mut verdict,
        );
        assert!(
            verdict
                .problems
                .iter()
                .any(|p| p.contains("10 of 200 pixels")),
            "{:?}",
            verdict.problems
        );
        for file in ["baseline.png", "candidate.png", "diff.png"] {
            assert!(
                diffs.join("a.desktop").join(file).is_file(),
                "{file} kept for review"
            );
        }
        Ok(())
    }

    #[test]
    fn a_missing_baseline_fails_and_bless_writes_it() -> Result<()> {
        let temp = Temp::new("bless")?;
        let (frames, baselines) = (temp.0.join("frames"), temp.0.join("baselines"));
        std::fs::create_dir_all(&frames)?;
        solid([1, 2, 3]).save(frames.join("a.desktop.png"))?;
        let expectations = imaged_expectations();
        let ran = ran();

        let mut verdict = Verdict::default();
        check_images(
            &expectations,
            &ran,
            &frames,
            &baselines,
            &temp.0.join("diff"),
            &mut verdict,
        );
        assert!(
            verdict.problems.iter().any(|p| p.contains("no baseline")),
            "{:?}",
            verdict.problems
        );

        let written =
            bless_frames(&expectations, &frames, &baselines).map_err(anyhow::Error::msg)?;
        assert_eq!(
            written,
            vec![baselines.join("a.desktop.png")],
            "the one frame"
        );
        let mut verdict = Verdict::default();
        check_images(
            &expectations,
            &ran,
            &frames,
            &baselines,
            &temp.0.join("diff"),
            &mut verdict,
        );
        assert!(verdict.passed(), "{:?}", verdict.problems);
        Ok(())
    }

    #[test]
    fn bless_takes_the_run_and_keeps_the_image_list() -> Result<()> {
        let mut old = imaged_expectations();
        old.images.push("gone".to_string());
        let new = blessed(&report(both_ok("a")), &old);
        assert_eq!(
            new.images,
            vec!["a".to_string()],
            "a scene that is gone leaves the list"
        );
        assert_eq!(
            new.scenes.get("a").and_then(|d| d.get(&Dialect::Gles)),
            Some(&Status::Ok),
            "the run's status replaces the old one"
        );
        let back = serde_json::from_str::<Expectations>(&new.to_json()?)?;
        assert_eq!(back, new, "the file round-trips");
        assert!(
            new.to_json()?.contains("\"desktop\": \"ok\""),
            "dialects and statuses are words"
        );
        Ok(())
    }
}
