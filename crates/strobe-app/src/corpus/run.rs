// ============================================================================
// Module:       strobe_app::corpus::run
// Description:  Measures one scene on a current GL context: loads it the way
//               the app does (script included), draws frames, and reads the
//               last one back.
//
// Dependencies: crate::{session, audio, corpus}; strobe-gfx (Gl, Caps)
// ============================================================================

//! One scene, one dialect.
//!
//! [`run_scene`] is the harness's whole per-scene job and takes a context
//! that is already current, so a test can hand it a `Headless` and the
//! worker process can hand it whatever `offscreen::run` opened.
//!
//! ## The load is the app's
//!
//! [`Session::load`] reads the folder, starts the script, runs its first
//! frame, declares the uniforms it created, compiles every pass and seals
//! the script's uniforms (D62, D161). [`Session::advance`] then runs the
//! script's `update`, ticks the controls, uploads the synthetic audio and
//! draws. Nothing here duplicates a step of that, which is the point: a
//! scene that this reports as compiling is a scene the app compiles.
//!
//! ## Where each status comes from
//!
//! | Status | Source |
//! |---|---|
//! | `parse_error` | `Session::load` returned `LoadError::Scene` |
//! | `script_error` | a `SCRIPT` failure was reported while loading or drawing |
//! | `compile_error` | `Session::load` returned `LoadError::Player` |
//! | `gl_error` | `glGetError` after the frames was not zero |
//!
//! A script that fails to start does not fail the load, by design (a show
//! plays on without it), so its failure is read back from the failure path
//! with [`super::capture`]. Script trouble outranks a compile error that
//! follows it, because the compile error is usually the consequence (the
//! uniform the script would have created was never declared); the compile
//! error is kept in `also`.

use super::capture::{self, Captured, Source};
use super::{Dialect, Note, Record, Settings, Status, cause};
use crate::audio::AudioInput;
use crate::session::{LoadError, Session};
use std::path::Path;
use std::time::Instant;
use strobe_gfx::gl::Gl;
use strobe_gfx::{Caps, Mouse};

/// Seconds of scene time at the first frame, as `strobe-bench` uses, so a
/// scene that fades in from black is not measured while it is still black.
const START: f32 = 2.0;

/// The step between frames: one frame at 60 Hz.
const DT: f32 = 1.0 / 60.0;

/// `strobe_Date`: a fixed day, so two runs draw the same frame.
const DATE: [f32; 4] = [2026.0, 1.0, 1.0, 0.0];

/// Milliseconds since `start`.
fn since(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// Measures the scene in `dir` on the current context, with the script
/// run as the app runs it. Never panics and never returns early without a
/// record: every failure is a [`Status`].
pub fn run_scene(
    gl: &Gl,
    caps: &Caps,
    dir: &Path,
    dialect: Dialect,
    settings: &Settings,
) -> Record {
    let name = strobe_scene::load::folder_title(dir);
    let mut record = Record::new(&name, dialect);
    record.has_script = dir.join("script.js").is_file();

    // Context creation can leave an error flag set that is not the
    // scene's doing, and an earlier scene's reports are not this one's.
    let _ = gl.error();
    let _ = capture::take();
    let mut audio = AudioInput::new(false);

    let started = Instant::now();
    let loaded = Session::load(gl, caps, dir, &name, settings.size, START, &mut audio);
    gl.finish();
    let compile_ms = since(started);
    let mut captured = capture::take();

    let mut session = match loaded {
        Ok(session) => session,
        Err(LoadError::Scene(error)) => {
            let mut failed = Record::failed(&name, dialect, Status::ParseError, &error.to_string());
            failed.has_script = record.has_script;
            return failed;
        }
        Err(LoadError::Player(error)) => {
            let message = error.to_string();
            let mut failed = match first_script_problem(&captured) {
                Some(script) => {
                    let mut failed = Record::failed(&name, dialect, Status::ScriptError, &script);
                    failed.also = Some(cause::first_error_line(&message));
                    failed
                }
                None => {
                    let mut failed = Record::failed(&name, dialect, Status::CompileError, &message);
                    failed.cause = Some(cause::categorise_in(&message, &shader_text(dir)));
                    failed
                }
            };
            failed.has_script = record.has_script;
            failed.compile_ms = Some(compile_ms);
            return failed;
        }
    };
    record.compile_ms = Some(compile_ms);
    record.script_uniforms = session.player.extras().len();

    let mut frame_times = Vec::new();
    for _ in 0..settings.frames.max(1) {
        let frame = Instant::now();
        session.advance(DT, &mut audio, Mouse::default(), DATE);
        gl.finish();
        frame_times.push(since(frame));
    }
    record.first_frame_ms = frame_times.first().copied();
    record.frame_ms = mean_after_first(&frame_times);
    captured.extend(capture::take());

    let gl_error = gl.error();
    read_back(&session, &mut record);

    let script = first_script_problem(&captured);
    let gl_message = gl_error.map(|code| format!("GL error 0x{code:x}"));
    match (script, gl_message) {
        (Some(script), gl_message) => {
            record.status = Status::ScriptError;
            record.cause = Some(cause::categorise(&script));
            record.error = Some(cause::first_error_line(&script));
            record.also = gl_message;
        }
        (None, Some(message)) => {
            record.status = Status::GlError;
            record.cause = Some(cause::categorise(&message));
            record.error = Some(message);
        }
        (None, None) => {}
    }
    record.notes = notes(&captured, &shader_text(dir));
    record
}

/// The mean of every time after the first; the first when it is alone.
fn mean_after_first(times: &[f64]) -> Option<f64> {
    let rest = times.get(1..).filter(|rest| !rest.is_empty());
    let slice = rest.unwrap_or(times);
    (!slice.is_empty()).then(|| slice.iter().sum::<f64>() / slice.len() as f64)
}

/// The first script failure among the captured reports.
fn first_script_problem(captured: &[Captured]) -> Option<String> {
    captured
        .iter()
        .find(|c| c.source == Source::ScriptFailure)
        .map(|c| c.message.clone())
}

/// Everything captured that did not stop the scene.
fn notes(captured: &[Captured], source: &str) -> Vec<Note> {
    captured
        .iter()
        .filter(|c| c.source != Source::ScriptFailure)
        .map(|c| Note {
            message: cause::first_error_line(&c.message),
            cause: cause::categorise_in(&c.message, source),
        })
        .collect()
}

/// Reads the final frame back and fills in the content check.
fn read_back(session: &Session, record: &mut Record) {
    let pixels = session.player.read_output();
    let [width, height] = session.player.render_size();
    let expected = width.max(0) as usize * height.max(0) as usize * 4;
    if pixels.len() != expected || pixels.is_empty() {
        return;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let (mut black, mut white) = (true, true);
    for texel in pixels.as_chunks::<4>().0 {
        for &channel in &texel[..3] {
            black &= channel == 0;
            white &= channel == 255;
            hash ^= u64::from(channel);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    record.all_black = Some(black);
    record.all_white = Some(white);
    record.checksum = Some(format!("{hash:016x}"));
}

/// The text of every `.glsl` file in the scene folder (and its `passes/`),
/// for [`cause::categorise_in`]. Empty when unreadable.
fn shader_text(dir: &Path) -> String {
    let mut text = String::new();
    for folder in [dir.to_path_buf(), dir.join("passes")] {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "glsl")
                && let Ok(source) = std::fs::read_to_string(&path)
            {
                text.push_str(&source);
                text.push('\n');
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mean_leaves_out_the_first_frame_unless_it_is_alone() {
        assert_eq!(mean_after_first(&[]), None, "no frames");
        assert_eq!(mean_after_first(&[7.0]), Some(7.0), "one frame");
        assert_eq!(
            mean_after_first(&[100.0, 2.0, 4.0]),
            Some(3.0),
            "the first frame carries the driver's deferred compile"
        );
    }
}
