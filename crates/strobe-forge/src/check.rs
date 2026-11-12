// ============================================================================
// Module:       strobe_forge::check
// Description:  `strobe-forge check`: compiles a scene, renders it at chosen
//               times, times it at 1080p, runs the gates and moves every
//               control to its extremes, then writes frames, a sheet, a
//               controls grid and the report.
//
// Dependencies: anyhow, image, serde, serde_json; strobe-gfx (player),
//               strobe-scene (folder, controls, synthetic audio),
//               strobe-autocontrol (image difference, script uniforms);
//               crate::gate, crate::dialect
// ============================================================================

//! The check the scene-from-image skill's step 4 runs (D562).
//!
//! Everything renders the way `strobe-bench` does: the synthetic 120 BPM
//! track drives the levels and the audio textures, `dt` is a fixed 1/60 s,
//! and a frame "at" a time is the last of [`WARM`] seconds of frames that
//! end there, so a clock pass has history and a feedback pass has settled
//! (the bench's `--start`). Frames are a pure function of the scene and the
//! options, so two runs give the same pictures.
//!
//! Outputs, in `--out`:
//!
//! - `frame-<t>s.png` at each `--times` entry, at `--size`;
//! - `sheet.png`: the reference photo (when given) beside those frames;
//! - `controls.png`: per control, its default and each probe, one row each,
//!   in the order `report.md` lists them (no font in the workspace);
//! - `report.md` and `report.json`: compile time and errors, both dialects,
//!   the 1080p timing, every gate with its numbers, every control's
//!   measured effect, warnings and hard failures.
//!
//! The hard failures are a compile error in either dialect, and the black,
//! frozen and flashing gates. A flat or blown-out picture, a control that
//! does little and an extreme that breaks the picture are warnings.

use crate::dialect::{self, DialectResult};
use crate::gate::{self, FrameSet, GateResult};
use anyhow::{Context as _, Result};
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;
use strobe_autocontrol::score;
use strobe_gfx::gl::Gl;
use strobe_gfx::{AudioTextures, Caps, FrameInputs, Mouse, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::{ControlBank, ControlDef, ControlKind};

/// Seconds between frames.
const DT: f32 = 1.0 / 60.0;
/// Seconds of frames rendered before the one captured.
pub(crate) const WARM: f32 = 1.5;
/// The date every frame renders at, so a scene that shows the time is
/// deterministic (year, month 0..11, day, seconds since midnight).
const FIXED_DATE: [f32; 4] = [2026.0, 0.0, 1.0, 43_200.0];

/// What to check and where the results go.
#[derive(Debug, Clone)]
pub(crate) struct CheckOptions {
    /// Frame size for the frames, the sheet and the control probes.
    pub(crate) size: [i32; 2],
    /// Seconds of scene time to write a frame at.
    pub(crate) times: Vec<f32>,
    /// The photo the scene recreates, for the sheet.
    pub(crate) reference: Option<PathBuf>,
    /// Output folder.
    pub(crate) out: PathBuf,
    /// Compile the other dialect too, in a child process of this binary.
    pub(crate) other_dialect: bool,
    /// Time the scene at 1920x1080.
    pub(crate) timing: bool,
}

/// One frame written.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FrameResult {
    /// Scene time, seconds.
    pub(crate) time: f32,
    /// File name in the output folder.
    pub(crate) file: String,
    /// Mean luma, `0..1`.
    pub(crate) mean: f32,
    /// Fraction of blown-out pixels.
    pub(crate) blown: f32,
}

/// The 1080p timing.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Timing {
    /// Render size.
    pub(crate) size: [i32; 2],
    /// Timed frames per batch, after [`TIMING_WARMUP`] untimed.
    pub(crate) frames: u32,
    /// The lowest batch mean, ms: other GPU work only ever adds time.
    pub(crate) mean_ms: f64,
    /// That batch's 95th percentile, ms.
    pub(crate) p95_ms: f64,
    /// Every batch's mean, ms, in order.
    pub(crate) batch_means: Vec<f64>,
    /// The slowest batch mean is more than twice the fastest: something
    /// else was using the GPU, and the numbers are suspect.
    pub(crate) unsteady: bool,
    /// The GL renderer string.
    pub(crate) renderer: String,
}

/// Frames warmed up before timing.
const TIMING_WARMUP: u32 = 30;
/// Frames timed in each batch.
const TIMING_FRAMES: u32 = 30;
/// Batches timed; the lowest mean is reported.
const TIMING_BATCHES: usize = 3;
/// Slowest over fastest batch mean past which the timing is unsteady.
const UNSTEADY: f64 = 2.0;
/// The size timing renders at: the skill's budget is stated for 1080p.
const TIMING_SIZE: [i32; 2] = [1920, 1080];
/// The skill's step 5 budget, ms a frame at [`TIMING_SIZE`] (stated for
/// LIGHTNING's RX 580). Over it is a warning, not a failure: the number
/// depends on the machine, and the skill allows a stated reason.
const BUDGET_MS: f64 = 2.5;

/// How the gates sampled the scene.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Gates {
    /// First sampled second.
    pub(crate) start: f32,
    /// Seconds sampled.
    pub(crate) seconds: f32,
    /// Samples a second.
    pub(crate) rate: f32,
    /// Sample size, pixels.
    pub(crate) size: [usize; 2],
    /// What was measured.
    pub(crate) result: GateResult,
}

/// Where the dense gate sequence starts, seconds.
const GATE_START: f32 = 5.0;
/// How long it runs.
const GATE_SECONDS: f32 = 4.0;
/// Frames rendered per sample: 60 fps rendered, 30 sampled.
const GATE_STRIDE: u32 = 2;
/// The size each sample is reduced to before measuring.
const GATE_SIZE: [u32; 2] = [256, 144];
/// The time the control probes render at.
const PROBE_TIME: f32 = 7.0;
/// How long before the probe frame a bang is pressed. A smooth bang's
/// counter crosses its half step about 0.28 s after the press, and the
/// scene's response takes time to show, so a second and more.
const PRESS_LEAD: f32 = 1.25;
/// A tile difference under which a probe changed nothing locally either.
const LOCAL_TILE: f32 = 0.02;
/// Tiles a side for the local measure.
const TILES: usize = 16;
/// A channel change (of 255) past which a pixel counts as changed.
const PIXEL_DELTA: u8 = 26;

/// One value a control was moved to.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Probe {
    /// `min`, `max`, a dropdown label, or `press`.
    pub(crate) label: String,
    /// The values set.
    pub(crate) values: Vec<f32>,
    /// Mean absolute difference from the default render, `0..1`.
    pub(crate) difference: f32,
    /// The largest tile difference over a 16x16 grid, `0..1`: a
    /// small feature that changes a lot still shows here.
    pub(crate) tile_max: f32,
    /// Share of pixels whose largest channel change exceeds 0.1.
    pub(crate) changed_share: f32,
    /// The frame's mean luma.
    pub(crate) mean: f32,
    /// The frame's blown-out share.
    pub(crate) blown: f32,
}

/// One control's measured effect.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ControlResult {
    /// The uniform name.
    pub(crate) name: String,
    /// Its type, as `scene.json` spells the base.
    pub(crate) kind: String,
    /// Each value it was moved to; empty when skipped.
    pub(crate) probes: Vec<Probe>,
    /// Why it was skipped, if it was.
    pub(crate) skipped: Option<String>,
    /// No probe changed the picture visibly.
    pub(crate) does_little: bool,
    /// A probe blacked out or blew out the picture.
    pub(crate) breaks: bool,
}

/// Everything the check found.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Report {
    /// The scene folder.
    pub(crate) scene: String,
    /// `TITLE`.
    pub(crate) title: String,
    /// Number of controls.
    pub(crate) control_count: usize,
    /// The GL renderer and version.
    pub(crate) renderer: String,
    /// Where the context came from.
    pub(crate) context: String,
    /// Frame size.
    pub(crate) size: [i32; 2],
    /// Wall time to compile and link, ms.
    pub(crate) compile_ms: f64,
    /// Each dialect's compile, this context's first.
    pub(crate) dialects: Vec<DialectResult>,
    /// The frames written.
    pub(crate) frames: Vec<FrameResult>,
    /// The 1080p timing.
    pub(crate) timing: Option<Timing>,
    /// The gates.
    pub(crate) gates: Option<Gates>,
    /// Every control.
    pub(crate) controls: Vec<ControlResult>,
    /// Things worth fixing that do not fail the check.
    pub(crate) warnings: Vec<String>,
    /// Things that fail it.
    pub(crate) failures: Vec<String>,
    /// Files written, relative to the output folder.
    pub(crate) files: Vec<String>,
}

impl Report {
    /// No hard failure.
    pub(crate) fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    /// The compile errors, if any dialect failed.
    pub(crate) fn compile_errors(&self) -> Option<String> {
        let errors: Vec<String> = self
            .dialects
            .iter()
            .filter_map(|d| d.error.as_ref().map(|e| format!("{}: {e}", d.dialect)))
            .collect();
        (!errors.is_empty()).then(|| errors.join("\n"))
    }
}

/// The synthetic audio and the frame loop, around one player.
struct Stage<'a> {
    gl: &'a Gl,
    player: ScenePlayer,
    transitions: Vec<f32>,
    audio: Vec<f32>,
    spectrum: Vec<[f32; 4]>,
    trail: Vec<[f32; 4]>,
}

impl<'a> Stage<'a> {
    fn new(gl: &'a Gl, mut player: ScenePlayer, scene: &SceneFolder) -> Self {
        // The app's audio texture sizes (D269), as the bench uses.
        const SPECTRUM_TEXELS: usize = 1024;
        const TRAIL_TEXELS: usize = 256;
        player.set_date(FIXED_DATE);
        Self {
            gl,
            player,
            transitions: scene
                .def
                .transition_uniforms
                .iter()
                .map(|t| t.initial)
                .collect(),
            audio: vec![0.0; SCALARS.len()],
            spectrum: vec![[0.0; 4]; SPECTRUM_TEXELS],
            trail: vec![[0.0; 4]; TRAIL_TEXELS],
        }
    }

    /// Renders one frame at `time`.
    fn frame(&mut self, bank: &ControlBank, time: f32, index: u64) {
        const TRAIL_SECONDS: f32 = 4.0;
        strobe_scene::synthetic::fill(time, &mut self.audio);
        strobe_scene::synthetic::fill_textures(
            time,
            &mut self.spectrum,
            &mut self.trail,
            TRAIL_SECONDS,
        );
        self.player.upload_audio(&AudioTextures {
            spectrum: &self.spectrum,
            level_trail: &self.trail,
            shadertoy: None,
        });
        self.player.render(&FrameInputs {
            time,
            frame: index,
            mouse: Mouse::default(),
            audio: &self.audio,
            controls: bank,
            transitions: &self.transitions,
        });
    }

    /// From black, renders [`WARM`] seconds of frames ending at `time` with
    /// `defs` as the controls, pressing `press` [`PRESS_LEAD`] seconds
    /// before the end, and returns the last frame.
    fn render_at(
        &mut self,
        defs: &[ControlDef],
        time: f32,
        press: Option<usize>,
    ) -> Result<Vec<u8>> {
        self.player.restart()?;
        let mut bank = ControlBank::new(defs.to_vec())?;
        let start = (time - WARM).max(0.0);
        let count = ((time - start) / DT).round() as u64 + 1;
        let press_at = count.saturating_sub((PRESS_LEAD / DT) as u64);
        for index in 0..count {
            if Some(index) == press.map(|_| press_at)
                && let Some(control) = press
            {
                bank.press(control);
            }
            self.frame(&bank, start + index as f32 * DT, index);
            bank.tick(DT);
        }
        Ok(self.player.read_output())
    }

    fn size(&self) -> [u32; 2] {
        let [w, h] = self.player.render_size();
        [w.max(1) as u32, h.max(1) as u32]
    }

    fn image(&self, pixels: Vec<u8>) -> Option<image::RgbaImage> {
        let [w, h] = self.size();
        image::RgbaImage::from_raw(w, h, pixels)
    }
}

/// Compiles `scene` on this context, timing it.
fn compile(
    gl: &Gl,
    caps: &Caps,
    scene: &SceneFolder,
    size: [i32; 2],
) -> Result<(ScenePlayer, f64), String> {
    gl.clear_errors();
    let extras = strobe_autocontrol::script::uniforms(scene);
    let start = Instant::now();
    let pending = ScenePlayer::begin(gl, caps, scene, size, Options::default(), &extras)
        .map_err(|e| e.to_string())?;
    while !pending.ready(gl) {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let player = pending.finish(gl, caps).map_err(|e| e.to_string())?;
    gl.finish();
    Ok((player, start.elapsed().as_secs_f64() * 1000.0))
}

/// The file name of the frame at `time`: `frame-2s.png`, `frame-2.5s.png`.
pub(crate) fn frame_name(time: f32) -> String {
    format!("frame-{time}s.png")
}

/// Runs the whole check on the current context and writes its outputs.
/// Fails only when the outputs cannot be written; a scene that does not
/// compile gives a report saying so.
pub(crate) fn check(
    gl: &Gl,
    caps: &Caps,
    context: &str,
    dir: &Path,
    options: &CheckOptions,
) -> Result<Report> {
    std::fs::create_dir_all(&options.out)
        .with_context(|| format!("creating {}", options.out.display()))?;
    let mut report = Report {
        scene: dir.display().to_string(),
        title: strobe_scene::load::folder_title(dir),
        control_count: 0,
        renderer: format!("{} ({})", caps.info.renderer, caps.info.version),
        context: context.to_owned(),
        size: options.size,
        compile_ms: 0.0,
        dialects: Vec::new(),
        frames: Vec::new(),
        timing: None,
        gates: None,
        controls: Vec::new(),
        warnings: Vec::new(),
        failures: Vec::new(),
        files: Vec::new(),
    };
    let here = dialect::name(caps.dialect);
    let scene = match SceneFolder::load(dir) {
        Ok(scene) => scene,
        Err(error) => {
            report
                .failures
                .push(format!("the scene does not load: {error}"));
            report.dialects.push(DialectResult::failed(
                here,
                &caps.info.renderer,
                error.to_string(),
            ));
            write(&report, &options.out)?;
            return Ok(report);
        }
    };
    report.title.clone_from(&scene.def.title);
    report.control_count = scene.def.controls.len();

    let compiled = compile(gl, caps, &scene, options.size);
    let other = if options.other_dialect {
        dialect::check_other(dir, caps.dialect, context)
    } else {
        DialectResult::not_checked(dialect::other_name(caps.dialect), "not asked for")
    };
    let player = match compiled {
        Ok((player, ms)) => {
            report.compile_ms = ms;
            report
                .dialects
                .push(DialectResult::ok(here, &caps.info.renderer));
            player
        }
        Err(error) => {
            report.failures.push(format!("{here} does not compile"));
            report
                .dialects
                .push(DialectResult::failed(here, &caps.info.renderer, error));
            report.dialects.push(other);
            write(&report, &options.out)?;
            return Ok(report);
        }
    };
    match other.status {
        dialect::Status::Failed => report
            .failures
            .push(format!("{} does not compile", other.dialect)),
        dialect::Status::NotChecked => report.warnings.push(format!(
            "{} not checked: {}",
            other.dialect,
            other.error.as_deref().unwrap_or("unavailable")
        )),
        dialect::Status::Ok => {}
    }
    report.dialects.push(other);

    let mut stage = Stage::new(gl, player, &scene);
    let defaults = scene.def.controls.clone();

    // Frames at the chosen times.
    let mut images = Vec::new();
    for &time in &options.times {
        let pixels = stage.render_at(&defaults, time, None)?;
        let (mean, blown) = gate::mean_and_blown(&pixels);
        let file = frame_name(time);
        if let Some(image) = stage.image(pixels) {
            image
                .save(options.out.join(&file))
                .with_context(|| format!("writing {file}"))?;
            images.push(image);
            report.files.push(file.clone());
        }
        if mean < gate::BLACK {
            report
                .warnings
                .push(format!("the frame at {time} s is almost black"));
        }
        if blown > gate::BLOWN_SHARE {
            report
                .warnings
                .push(format!("the frame at {time} s is blown out"));
        }
        report.frames.push(FrameResult {
            time,
            file,
            mean,
            blown,
        });
    }

    // The gates, over a dense run.
    let gates = run_gates(&mut stage, &defaults)?;
    report.failures.extend(gates.result.hard_problems());
    report.warnings.extend(gates.result.soft_problems());
    report.gates = Some(gates);

    // Every control at its extremes.
    let (controls, grid) = probe_controls(&mut stage, &defaults)?;
    for control in &controls {
        if control.does_little {
            report
                .warnings
                .push(format!("`{}` does little", control.name));
        }
        if control.breaks {
            report.warnings.push(format!(
                "`{}` breaks the picture at an extreme",
                control.name
            ));
        }
    }
    report.controls = controls;
    if let Some(grid) = grid {
        grid.save(options.out.join("controls.png"))?;
        report.files.push("controls.png".to_owned());
    }

    if let Some(code) = gl.error() {
        report
            .failures
            .push(format!("GL error 0x{code:x} while rendering"));
    }
    drop(stage);

    // The 1080p timing, on a player of its own.
    if options.timing {
        match time_scene(gl, caps, &scene) {
            Ok(timing) => {
                if !timing.unsteady && timing.mean_ms > BUDGET_MS {
                    report.warnings.push(format!(
                        "over budget: {:.2} ms a frame at {}x{} on {}, over the skill's {BUDGET_MS} ms",
                        timing.mean_ms, timing.size[0], timing.size[1], timing.renderer
                    ));
                }
                if timing.unsteady {
                    report.warnings.push(format!(
                        "timing unsteady: batch means {} ms differ by more than {UNSTEADY}x, so other GPU work was running",
                        timing
                            .batch_means
                            .iter()
                            .map(|m| format!("{m:.2}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                report.timing = Some(timing);
            }
            Err(error) => report.warnings.push(format!("not timed: {error}")),
        }
    }

    // The sheet.
    let reference = match &options.reference {
        Some(path) => match std::fs::read(path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| crate::prompt::decode_oriented(&bytes))
        {
            Ok(image) => Some(image.to_rgba8()),
            Err(error) => {
                report.warnings.push(format!(
                    "the reference {} does not open: {error}",
                    path.display()
                ));
                None
            }
        },
        None => None,
    };
    if let Some(sheet) = sheet(reference.as_ref(), &images) {
        sheet.save(options.out.join("sheet.png"))?;
        report.files.push("sheet.png".to_owned());
    }
    write(&report, &options.out)?;
    Ok(report)
}

/// Renders [`GATE_SECONDS`] from [`GATE_START`] and runs the gates over
/// every second frame, each reduced to [`GATE_SIZE`].
fn run_gates(stage: &mut Stage<'_>, defs: &[ControlDef]) -> Result<Gates> {
    stage.player.restart()?;
    let mut bank = ControlBank::new(defs.to_vec())?;
    let start = GATE_START - WARM;
    let first = (WARM / DT).round() as u64;
    let count = first + (GATE_SECONDS / DT).round() as u64;
    let mut samples = Vec::new();
    for index in 0..count {
        stage.frame(&bank, start + index as f32 * DT, index);
        bank.tick(DT);
        if index >= first && (index - first).is_multiple_of(u64::from(GATE_STRIDE)) {
            let pixels = stage.player.read_output();
            if let Some(image) = stage.image(pixels) {
                let small = image::imageops::resize(
                    &image,
                    GATE_SIZE[0],
                    GATE_SIZE[1],
                    image::imageops::FilterType::Triangle,
                );
                samples.push(small.into_raw());
            }
        }
    }
    let rate = 1.0 / (DT * GATE_STRIDE as f32);
    let result = gate::gate(&FrameSet {
        width: GATE_SIZE[0] as usize,
        height: GATE_SIZE[1] as usize,
        rate,
        frames: &samples,
    });
    Ok(Gates {
        start: GATE_START,
        seconds: GATE_SECONDS,
        rate,
        size: [GATE_SIZE[0] as usize, GATE_SIZE[1] as usize],
        result,
    })
}

/// The values to move control `def` to, labelled; `None` with a reason
/// when it is skipped. A bang is one press (empty values).
fn probes_for(def: &ControlDef) -> Result<Vec<(String, Vec<f32>)>, String> {
    let dims = def.dims();
    let take = |v: &[f32; strobe_scene::control::def::MAX_DIMENSIONS]| v[..dims].to_vec();
    match def.kind {
        ControlKind::Slider(_)
        | ControlKind::Knob(_)
        | ControlKind::Toggle { .. }
        | ControlKind::Xy(_) => Ok(vec![
            ("min".to_owned(), take(&def.min)),
            ("max".to_owned(), take(&def.max)),
        ]),
        ControlKind::Dropdown { .. } => {
            let default = def.default[0].round() as usize;
            let count = def.labels.len().max(def.values.len());
            Ok((0..count)
                .filter(|index| *index != default)
                .map(|index| {
                    let label = def
                        .labels
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("#{index}"));
                    (label, vec![index as f32])
                })
                .collect())
        }
        ControlKind::Bang(_) => Ok(vec![("press".to_owned(), Vec::new())]),
        ControlKind::Color { .. } => Err("colours are not probed".to_owned()),
    }
}

/// The base type's name, as `scene.json` spells it.
fn kind_name(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::Slider(_) => "slider",
        ControlKind::Knob(_) => "knob",
        ControlKind::Toggle { .. } => "toggle",
        ControlKind::Bang(_) => "bang",
        ControlKind::Xy(_) => "xy",
        ControlKind::Color { .. } => "color",
        ControlKind::Dropdown { .. } => "dropdown",
    }
}

/// Renders every control at each of its probes at [`PROBE_TIME`], others at
/// their defaults, and measures each against the default render. Returns
/// the results and the grid image.
fn probe_controls(
    stage: &mut Stage<'_>,
    defaults: &[ControlDef],
) -> Result<(Vec<ControlResult>, Option<image::RgbaImage>)> {
    const THUMB: [u32; 2] = [240, 135];
    const GAP: u32 = 4;
    let baseline = stage.render_at(defaults, PROBE_TIME, None)?;
    let lit = !score::is_black(&vec![baseline.clone()]);
    let base = vec![baseline];
    let threshold = score::ScoreOptions::default().threshold;
    let mut rows: Vec<Vec<image::RgbaImage>> = Vec::new();
    let thumb = |stage: &Stage<'_>, pixels: Vec<u8>| {
        stage.image(pixels).map(|image| {
            image::imageops::resize(
                &image,
                THUMB[0],
                THUMB[1],
                image::imageops::FilterType::Triangle,
            )
        })
    };
    let base_thumb = base
        .first()
        .cloned()
        .and_then(|pixels| thumb(stage, pixels));
    let mut results = Vec::new();
    for (index, def) in defaults.iter().enumerate() {
        let mut result = ControlResult {
            name: def.name.clone(),
            kind: kind_name(def.kind).to_owned(),
            probes: Vec::new(),
            skipped: None,
            does_little: false,
            breaks: false,
        };
        let probes = match probes_for(def) {
            Ok(probes) => probes,
            Err(why) => {
                result.skipped = Some(why);
                results.push(result);
                continue;
            }
        };
        let mut row: Vec<image::RgbaImage> = base_thumb.iter().cloned().collect();
        for (label, values) in probes {
            let (pixels, values) = if values.is_empty() {
                (stage.render_at(defaults, PROBE_TIME, Some(index))?, values)
            } else {
                let mut defs = defaults.to_vec();
                if let Some(changed) = defs.get_mut(index) {
                    for (slot, value) in values.iter().enumerate() {
                        changed.default[slot] = *value;
                    }
                }
                (stage.render_at(&defs, PROBE_TIME, None)?, values)
            };
            let frames = vec![pixels];
            let difference = score::difference(&base, &frames);
            let [width, height] = stage.size();
            let (tile_max, changed_share) = match (base.first(), frames.first()) {
                (Some(a), Some(b)) => local_change(a, b, width as usize, height as usize),
                (None, _) | (_, None) => (0.0, 0.0),
            };
            let (mean, blown) = frames
                .first()
                .map_or((0.0, 0.0), |f| gate::mean_and_blown(f));
            if lit && (score::is_black(&frames) || mean < gate::BLACK || blown > gate::BLOWN_SHARE)
            {
                result.breaks = true;
            }
            result.probes.push(Probe {
                label,
                values,
                difference,
                tile_max,
                changed_share,
                mean,
                blown,
            });
            if let Some(pixels) = frames.into_iter().next()
                && let Some(small) = thumb(stage, pixels)
            {
                row.push(small);
            }
        }
        result.does_little = !result.probes.is_empty()
            && result
                .probes
                .iter()
                .all(|p| p.difference < threshold && p.tile_max < LOCAL_TILE);
        rows.push(row);
        results.push(result);
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0) as u32;
    if rows.is_empty() || columns == 0 {
        return Ok((results, None));
    }
    let mut grid = image::RgbaImage::from_pixel(
        columns * (THUMB[0] + GAP) + GAP,
        rows.len() as u32 * (THUMB[1] + GAP) + GAP,
        image::Rgba([24, 24, 24, 255]),
    );
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let x = GAP + c as u32 * (THUMB[0] + GAP);
            let y = GAP + r as u32 * (THUMB[1] + GAP);
            image::imageops::overlay(&mut grid, cell, i64::from(x), i64::from(y));
        }
    }
    Ok((results, Some(grid)))
}

/// Mean and 95th percentile frame time at [`TIMING_SIZE`].
fn time_scene(gl: &Gl, caps: &Caps, scene: &SceneFolder) -> Result<Timing> {
    let (player, _) = compile(gl, caps, scene, TIMING_SIZE).map_err(anyhow::Error::msg)?;
    let mut stage = Stage::new(gl, player, scene);
    let mut bank = ControlBank::new(scene.def.controls.clone())?;
    let mut batches: Vec<Vec<f64>> = (0..TIMING_BATCHES)
        .map(|_| Vec::with_capacity(TIMING_FRAMES as usize))
        .collect();
    let total = u64::from(TIMING_WARMUP) + u64::from(TIMING_FRAMES) * TIMING_BATCHES as u64;
    for index in 0..total {
        let time = PROBE_TIME + index as f32 * DT;
        let start = Instant::now();
        stage.frame(&bank, time, index);
        stage.gl.finish();
        if let Some(timed) = index.checked_sub(u64::from(TIMING_WARMUP))
            && let Some(batch) = batches.get_mut((timed / u64::from(TIMING_FRAMES)) as usize)
        {
            batch.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        bank.tick(DT);
    }
    let summary = summarise(&batches);
    Ok(Timing {
        size: TIMING_SIZE,
        frames: TIMING_FRAMES,
        mean_ms: summary.mean_ms,
        p95_ms: summary.p95_ms,
        batch_means: summary.means,
        unsteady: summary.unsteady,
        renderer: caps.info.renderer.clone(),
    })
}

/// What timing batches add up to.
#[derive(Debug, Clone, PartialEq)]
struct Summary {
    /// The lowest batch mean.
    mean_ms: f64,
    /// That batch's 95th percentile.
    p95_ms: f64,
    /// Every batch's mean.
    means: Vec<f64>,
    /// The slowest mean is over [`UNSTEADY`] times the fastest.
    unsteady: bool,
}

/// The lowest-mean batch's mean and p95, every mean, and whether they
/// disagree by more than [`UNSTEADY`].
fn summarise(batches: &[Vec<f64>]) -> Summary {
    let mean = |times: &[f64]| times.iter().sum::<f64>() / times.len().max(1) as f64;
    let means: Vec<f64> = batches.iter().map(|b| mean(b)).collect();
    let best = means
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i);
    let mut sorted = batches.get(best).cloned().unwrap_or_default();
    sorted.sort_by(f64::total_cmp);
    let p95_ms = sorted
        .get((sorted.len() as f64 * 0.95) as usize)
        .or(sorted.last())
        .copied()
        .unwrap_or(0.0);
    let low = means.iter().copied().fold(f64::INFINITY, f64::min);
    let high = means.iter().copied().fold(0.0, f64::max);
    Summary {
        mean_ms: means.get(best).copied().unwrap_or(0.0),
        p95_ms,
        unsteady: low > 0.0 && high > low * UNSTEADY,
        means,
    }
}

/// How much changed locally between two RGBA8 frames of `width` x
/// `height`: the largest tile difference over a [`TILES`] grid
/// (mean absolute colour difference inside a tile, `0..1`) and the share
/// of pixels whose largest channel change exceeds [`PIXEL_DELTA`]. A
/// control that moves a pupil or a sparkle barely shifts the whole frame's
/// mean but lights up its own tiles.
pub(crate) fn local_change(a: &[u8], b: &[u8], width: usize, height: usize) -> (f32, f32) {
    let mut sums = vec![0_u64; TILES * TILES];
    let mut counts = vec![0_u64; TILES * TILES];
    let mut changed = 0_u64;
    let mut pixels = 0_u64;
    for (i, (pa, pb)) in a
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .enumerate()
    {
        let (x, y) = (i % width.max(1), i / width.max(1));
        let tile = (y * TILES / height.max(1)).min(TILES - 1) * TILES
            + (x * TILES / width.max(1)).min(TILES - 1);
        let deltas = [
            pa[0].abs_diff(pb[0]),
            pa[1].abs_diff(pb[1]),
            pa[2].abs_diff(pb[2]),
        ];
        sums[tile] += deltas.iter().map(|d| u64::from(*d)).sum::<u64>();
        counts[tile] += 3;
        if deltas.iter().any(|d| *d > PIXEL_DELTA) {
            changed += 1;
        }
        pixels += 1;
    }
    // The largest tile, not a percentile: a pupil at 1280x720 spans one to
    // four of the 256 tiles, under any percentile's cut. Frames are a pure
    // function of time, so an unchanged tile is exactly zero, not noise.
    let largest = sums
        .iter()
        .zip(&counts)
        .filter(|(_, count)| **count > 0)
        .map(|(sum, count)| *sum as f32 / (*count as f32 * 255.0))
        .fold(0.0, f32::max);
    (largest, changed as f32 / pixels.max(1) as f32)
}

/// The reference (when there is one) beside the frames, in a grid of
/// 640x360 cells, three across. The reference keeps its aspect, centred.
pub(crate) fn sheet(
    reference: Option<&image::RgbaImage>,
    frames: &[image::RgbaImage],
) -> Option<image::RgbaImage> {
    const CELL: [u32; 2] = [640, 360];
    const COLUMNS: u32 = 3;
    let mut cells: Vec<image::RgbaImage> = Vec::new();
    if let Some(reference) = reference {
        let (w, h) = reference.dimensions();
        let scale = (CELL[0] as f32 / w.max(1) as f32).min(CELL[1] as f32 / h.max(1) as f32);
        let size = [
            ((w as f32 * scale).round() as u32).clamp(1, CELL[0]),
            ((h as f32 * scale).round() as u32).clamp(1, CELL[1]),
        ];
        let fitted = image::imageops::resize(
            reference,
            size[0],
            size[1],
            image::imageops::FilterType::Triangle,
        );
        let mut cell = image::RgbaImage::from_pixel(CELL[0], CELL[1], image::Rgba([0, 0, 0, 255]));
        image::imageops::overlay(
            &mut cell,
            &fitted,
            i64::from((CELL[0] - size[0]) / 2),
            i64::from((CELL[1] - size[1]) / 2),
        );
        cells.push(cell);
    }
    for frame in frames {
        cells.push(image::imageops::resize(
            frame,
            CELL[0],
            CELL[1],
            image::imageops::FilterType::Triangle,
        ));
    }
    if cells.is_empty() {
        return None;
    }
    let count = cells.len() as u32;
    let columns = COLUMNS.min(count);
    let mut sheet = image::RgbaImage::from_pixel(
        CELL[0] * columns,
        CELL[1] * count.div_ceil(columns),
        image::Rgba([0, 0, 0, 255]),
    );
    for (i, cell) in cells.iter().enumerate() {
        let i = i as u32;
        image::imageops::overlay(
            &mut sheet,
            cell,
            i64::from((i % columns) * CELL[0]),
            i64::from((i / columns) * CELL[1]),
        );
    }
    Some(sheet)
}

/// Writes `report.md` and `report.json` into `out`.
fn write(report: &Report, out: &Path) -> Result<()> {
    std::fs::write(out.join("report.md"), markdown(report))?;
    std::fs::write(
        out.join("report.json"),
        serde_json::to_string_pretty(report)?,
    )?;
    Ok(())
}

/// The report as Markdown.
pub(crate) fn markdown(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Forge check: {}\n", report.title);
    let _ = writeln!(
        out,
        "- Result: **{}**\n- Scene: `{}`\n- Controls: {}\n- Renderer: {}\n- Context: {}\n- Frame size: {}x{}\n- Compile: {:.1} ms\n",
        if report.passed() { "pass" } else { "FAIL" },
        report.scene,
        report.control_count,
        report.renderer,
        report.context,
        report.size[0],
        report.size[1],
        report.compile_ms,
    );
    if !report.failures.is_empty() {
        let _ = writeln!(out, "## Failures\n");
        for failure in &report.failures {
            let _ = writeln!(out, "- {failure}");
        }
        out.push('\n');
    }
    if !report.warnings.is_empty() {
        let _ = writeln!(out, "## Warnings\n");
        for warning in &report.warnings {
            let _ = writeln!(out, "- {warning}");
        }
        out.push('\n');
    }
    let _ = writeln!(out, "## Dialects\n");
    for d in &report.dialects {
        let _ = writeln!(
            out,
            "- {}: {}{}",
            d.dialect,
            d.status.label(),
            match (&d.renderer, &d.error) {
                (_, Some(error)) => format!("\n\n```text\n{}\n```", error.trim()),
                (Some(renderer), None) => format!(" ({renderer})"),
                (None, None) => String::new(),
            }
        );
    }
    out.push('\n');
    if let Some(timing) = &report.timing {
        let _ = writeln!(
            out,
            "## Timing\n\n{}x{} on {}, {} batches of {} frames after {} warm-up. Lowest of three batch means: **{:.2} ms** (p95 {:.2} ms); every batch: {} ms{}.\n",
            timing.size[0],
            timing.size[1],
            timing.renderer,
            TIMING_BATCHES,
            timing.frames,
            TIMING_WARMUP,
            timing.mean_ms,
            timing.p95_ms,
            timing
                .batch_means
                .iter()
                .map(|m| format!("{m:.2}"))
                .collect::<Vec<_>>()
                .join(", "),
            if timing.unsteady {
                ". **Unsteady**: the batches differ by more than 2x, so other GPU work was running"
            } else {
                ""
            }
        );
    }
    if let Some(gates) = &report.gates {
        let g = &gates.result;
        let mark = |failed: bool| if failed { "fail" } else { "pass" };
        let _ = writeln!(
            out,
            "## Gates\n\n{} s from {} s at {} samples a second, each {}x{}.\n\n| Gate | Result | Measured | Limit |\n|---|---|---:|---|",
            gates.seconds, gates.start, gates.rate, gates.size[0], gates.size[1]
        );
        let _ = writeln!(
            out,
            "| black | {} | mean luma {:.4} | under {} fails |",
            mark(g.black),
            g.mean,
            gate::BLACK
        );
        let _ = writeln!(
            out,
            "| flat (warning) | {} | luma spread {:.4} | under {} |",
            mark(g.flat),
            g.spread,
            gate::FLAT
        );
        let _ = writeln!(
            out,
            "| blown (warning) | {} | blown share {:.3} | over {} |",
            mark(g.blown_out),
            g.blown,
            gate::BLOWN_SHARE
        );
        let _ = writeln!(
            out,
            "| frozen | {} | largest change {:.4} | under {} fails |",
            mark(g.frozen),
            g.change,
            gate::FROZEN
        );
        let _ = writeln!(
            out,
            "| flash | {} | {:.2} a second over a quarter | over {} fails |\n",
            mark(g.flashing),
            g.flashes_per_second,
            gate::FLASH_LIMIT
        );
    }
    if !report.frames.is_empty() {
        let _ = writeln!(
            out,
            "## Frames\n\n| Time s | File | Mean luma | Blown |\n|---:|---|---:|---:|"
        );
        for f in &report.frames {
            let _ = writeln!(
                out,
                "| {} | {} | {:.3} | {:.3} |",
                f.time, f.file, f.mean, f.blown
            );
        }
        out.push('\n');
    }
    if !report.controls.is_empty() {
        let _ = writeln!(
            out,
            "## Controls\n\nEach rendered at {PROBE_TIME} s with the others at their defaults; difference is the mean absolute change from the default render over the whole frame (0..1), tile is the largest change of any tile in a 16x16 grid, and changed is the share of pixels that moved by more than 0.1; a control does little only when its difference and its tile are both small. `controls.png` has one row per probed control in this order: the default, then each probe as listed.\n\n| Control | Type | Probes (difference) | Verdict |\n|---|---|---|---|"
        );
        for c in &report.controls {
            let probes = match &c.skipped {
                Some(why) => why.clone(),
                None => c
                    .probes
                    .iter()
                    .map(|p| {
                        format!(
                            "{} {:.4} (tile {:.3}, changed {:.3})",
                            p.label, p.difference, p.tile_max, p.changed_share
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            let verdict = match (c.skipped.is_some(), c.does_little, c.breaks) {
                (true, _, _) => "skipped",
                (false, true, true) => "does little; breaks",
                (false, true, false) => "does little",
                (false, false, true) => "breaks",
                (false, false, false) => "ok",
            };
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} |",
                c.name, c.kind, probes, verdict
            );
        }
        out.push('\n');
    }
    if !report.files.is_empty() {
        let _ = writeln!(out, "## Files\n");
        for file in &report.files {
            let _ = writeln!(out, "- `{file}`");
        }
    }
    out
}

/// The default output folder for `dir`: `target/forge/<slug>`, the slug
/// being the folder name without `.synScene`.
pub(crate) fn default_out(dir: &Path) -> PathBuf {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "scene".to_owned());
    let slug = name.strip_suffix(".synScene").unwrap_or(&name).to_owned();
    PathBuf::from("target").join("forge").join(slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{anyhow, ensure};

    #[test]
    fn frame_names_drop_a_whole_number_fraction() -> Result<()> {
        ensure!(frame_name(2.0) == "frame-2s.png", "{}", frame_name(2.0));
        ensure!(frame_name(2.5) == "frame-2.5s.png", "{}", frame_name(2.5));
        Ok(())
    }

    #[test]
    fn a_small_feature_that_changes_shows_locally() -> Result<()> {
        let (w, h) = (64, 64);
        let a = vec![0_u8; w * h * 4];
        let mut b = a.clone();
        // A 4x4 spot turns white: 0.4% of the frame.
        for y in 10..14 {
            for x in 20..24 {
                let i = (y * w + x) * 4;
                b[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let whole = score::difference(&vec![a.clone()], &vec![b.clone()]);
        let (tile, share) = local_change(&a, &b, w, h);
        ensure!(
            whole < score::ScoreOptions::default().threshold * 2.0,
            "the frame mean barely moves: {whole}"
        );
        ensure!(tile > LOCAL_TILE, "but its tile does: {tile}");
        ensure!(
            (share - 16.0 / 4096.0).abs() < 1e-6,
            "16 pixels changed: {share}"
        );
        let (still, none) = local_change(&a, &a, w, h);
        ensure!(
            still == 0.0 && none == 0.0,
            "nothing changed: {still} {none}"
        );
        Ok(())
    }

    #[test]
    fn timing_reports_the_lowest_batch_and_flags_disagreement() -> Result<()> {
        let steady = summarise(&[vec![2.0, 2.2], vec![1.8, 2.0], vec![2.1, 2.1]]);
        ensure!(
            (steady.mean_ms - 1.9).abs() < 1e-9,
            "lowest mean: {steady:?}"
        );
        ensure!(steady.p95_ms == 2.0, "that batch's p95: {steady:?}");
        ensure!(!steady.unsteady, "steady: {steady:?}");
        let busy = summarise(&[vec![2.0], vec![5.0], vec![2.1]]);
        ensure!(
            busy.unsteady && busy.mean_ms == 2.0,
            "a busy batch: {busy:?}"
        );
        Ok(())
    }

    #[test]
    fn the_slug_drops_the_extension() -> Result<()> {
        let out = default_out(Path::new("user-scenes/crab.synScene"));
        ensure!(
            out == Path::new("target").join("forge").join("crab"),
            "got {}",
            out.display()
        );
        Ok(())
    }

    #[test]
    fn the_sheet_puts_the_reference_first_in_rows_of_three() -> Result<()> {
        let reference = image::RgbaImage::from_pixel(100, 200, image::Rgba([255, 0, 0, 255]));
        let frame = image::RgbaImage::from_pixel(64, 36, image::Rgba([0, 255, 0, 255]));
        let sheet = sheet(Some(&reference), &[frame.clone(), frame.clone(), frame])
            .ok_or_else(|| anyhow!("a sheet"))?;
        ensure!(
            sheet.dimensions() == (1920, 720),
            "{:?}",
            sheet.dimensions()
        );
        let centre = sheet.get_pixel(320, 180);
        ensure!(
            centre.0[0] == 255,
            "the reference is centred in the first cell: {centre:?}"
        );
        let edge = sheet.get_pixel(5, 180);
        ensure!(edge.0 == [0, 0, 0, 255], "letterboxed: {edge:?}");
        ensure!(
            sheet.get_pixel(320, 540).0[1] == 255,
            "the third frame wraps to row two"
        );
        Ok(())
    }

    /// The template from the skill, checked for real on a headless context,
    /// skipped (and passing) where there is none.
    #[test]
    fn the_template_passes_on_a_headless_context() -> Result<()> {
        let headless = match strobe_gfx::Headless::new(strobe_gfx::Api::DesktopCore33) {
            Ok(headless) => headless,
            Err(error) if std::env::var_os("STROBE_REQUIRE_HEADLESS_GL").is_none() => {
                strobe_gfx::skip::note(format_args!("no headless context ({error})"));
                return Ok(());
            }
            Err(error) => return Err(anyhow!("headless context required: {error}")),
        };
        let gl = headless.gl();
        let caps = Caps::from_info(gl.info());
        let root = crate::fixture::tempdir("template")?;
        let scene = root.join("t.synScene");
        crate::fixture::write_scene(&scene, crate::fixture::GLSL, crate::fixture::SCENE)?;
        let options = CheckOptions {
            size: [160, 90],
            times: vec![2.0, 7.0],
            reference: None,
            out: root.join("out"),
            other_dialect: false,
            timing: false,
        };
        let report = check(gl, &caps, "headless test", &scene, &options)?;
        ensure!(report.passed(), "the fixture passes: {:?}", report.failures);
        ensure!(report.frames.len() == 2, "two frames");
        ensure!(
            options.out.join("report.json").is_file(),
            "the JSON report is written"
        );
        ensure!(
            options.out.join("sheet.png").is_file(),
            "the sheet is written"
        );
        let size = report.controls.iter().find(|c| c.name == "size");
        ensure!(
            size.is_some_and(|c| !c.does_little && c.probes.len() == 2),
            "the size slider is probed and visible: {size:?}"
        );
        Ok(())
    }
}
