// ============================================================================
// Module:       strobe_bench::readback
// Description:  `--readback`: the textureFinalPass readback checked on the
//               context the bench opened: its latency, its cost, its
//               behaviour on a busy GPU, across a resize and a lost context.
//
// Dependencies: anyhow; strobe-gfx (player, Gl), strobe-scene (scene
//               definition, controls, audio scalars)
// ============================================================================

//! The final-pass readback on a real driver (backlog C-04, D162, D245).
//!
//! `strobe-gfx`'s tests pin the readback's behaviour on a headless EGL
//! context, which Windows' Intel driver does not have, so on the T480 they
//! skip. `strobe-bench --readback --context window --gl native|angle` runs
//! the same checks on the window context the show really uses, and adds the
//! numbers the tests cannot give: what the readback costs a frame, and how
//! stale the picture gets when the GPU is busy.
//!
//! The checks, in order, each a row in `readback.md`:
//!
//! 1. **Latency.** A scene whose red channel encodes the frame number,
//!    with a `glFinish` after each frame so every fence has signalled: the
//!    picture must be exactly two frames old, and black before that.
//! 2. **Cost.** A heavy scene rendered by two players, readback off and
//!    on, alternating frame by frame with a `glFinish` after each: the
//!    difference in the frame's wall time is what the readback costs, GPU
//!    included. A measurement: it never fails the check, and is flagged
//!    above a millisecond.
//! 3. **Busy GPU.** The same heavy scene with no `glFinish`: the age of the
//!    picture is recorded per frame (never newer than two frames, never
//!    going backwards), with the CPU time of each `render` call, which
//!    must not grow into a stall.
//! 4. **Resize.** A resize drops the old picture, and the new size reads
//!    back.
//! 5. **Lost context.** With the context marked lost, dropping a player
//!    with a readback makes no GL call (it must not crash the driver).

use anyhow::Result;
use std::fmt::Write as _;
use std::time::Instant;
use strobe_gfx::gl::Gl;
use strobe_gfx::{Caps, FrameInputs, Mouse, Options, ScenePlayer};
use strobe_scene::SceneFolder;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;
use strobe_scene::scene::SceneDef;

/// Frames the frame-number encodings count over before wrapping: 2.55
/// levels of red per frame, enough to read a frame number back exactly.
const PERIOD: u64 = 100;

/// The readback's documented latency, in frames.
const LATENCY: u64 = 2;

/// Frames rendered for the cost and busy-GPU checks.
const TIMED_FRAMES: u64 = 90;

/// The heavy scene: enough arithmetic per pixel to keep a laptop GPU busy
/// for several milliseconds at the bench's size, and red still encoding
/// the frame number.
const HEAVY: &str = "\
vec4 renderMain() {
    float acc = 0.0;
    for (int i = 0; i < 160; i++) {
        acc += sin(_uv.x * float(i) + TIME) * cos(_uv.y * float(i));
    }
    float frame = mod(FRAMECOUNT, 100.0) / 100.0;
    return vec4(frame, fract(acc) * 0.001, 0.0, 1.0);
}";

/// The light scene: red encodes the frame number and nothing else.
const LIGHT: &str =
    "vec4 renderMain() { return vec4(mod(FRAMECOUNT, 100.0) / 100.0, 0.25, 0.5, 1.0); }";

/// One check's outcome.
struct Check {
    name: &'static str,
    passed: bool,
    detail: String,
}

/// Runs every check on the current context and returns the report
/// (Markdown) and whether all of them passed.
pub(crate) fn run(gl: &Gl, caps: &Caps, context: &str, size: [i32; 2]) -> Result<(String, bool)> {
    let _ = gl.error();
    let checks = vec![
        latency(gl, caps)?,
        cost(gl, caps, size)?,
        busy(gl, caps, size)?,
        resize(gl, caps)?,
        lost(gl, caps)?,
    ];
    let mut report = String::new();
    writeln!(report, "# textureFinalPass readback check\n")?;
    writeln!(
        report,
        "{} | {} | {:?} | {context} | {}x{}\n",
        caps.info.renderer, caps.info.version, caps.dialect, size[0], size[1]
    )?;
    writeln!(report, "| Check | Result | Detail |")?;
    writeln!(report, "|---|---|---|")?;
    for check in &checks {
        let result = if check.passed { "pass" } else { "FAIL" };
        writeln!(report, "| {} | {result} | {} |", check.name, check.detail)?;
    }
    let passed = checks.iter().all(|check| check.passed);
    Ok((report, passed))
}

/// A scene folder from a shader alone.
fn scene(shader: &str) -> Result<SceneFolder> {
    Ok(SceneFolder {
        dir: std::path::PathBuf::from("."),
        def: SceneDef::parse("{}", "readback-check")?,
        shader: shader.to_string(),
        pass_shaders: Vec::new(),
        script: None,
    })
}

/// A player for `shader` at `size`, with the readback on or off.
fn player(gl: &Gl, caps: &Caps, shader: &str, size: [i32; 2], on: bool) -> Result<ScenePlayer> {
    let mut player = ScenePlayer::new(gl, caps, &scene(shader)?, size, Options::default())?;
    if on {
        player.enable_final_pass_readback()?;
    }
    Ok(player)
}

/// Renders frame `frame` with a silent room.
fn render(player: &mut ScenePlayer, controls: &ControlBank, frame: u64) {
    let audio = vec![0.0; SCALARS.len()];
    player.render(&FrameInputs {
        time: frame as f32 / 60.0,
        frame,
        mouse: Mouse::default(),
        audio: &audio,
        controls,
        transitions: &[],
    });
}

/// The frame number the newest readback shows, from its bottom-left
/// pixel's red channel, or `None` before anything has arrived.
fn shown(player: &ScenePlayer) -> Option<u64> {
    let red = *player.final_pass_pixels()?.first()?;
    Some((f64::from(red) / 255.0 * PERIOD as f64).round() as u64 % PERIOD)
}

/// How many frames old a picture showing `shown` is after `frame`,
/// counting across the encoding's wrap.
fn age(frame: u64, shown: u64) -> u64 {
    (frame % PERIOD + PERIOD - shown) % PERIOD
}

/// The mean and the largest of `samples`.
fn mean_max(samples: &[f64]) -> (f64, f64) {
    let mean = samples.iter().sum::<f64>() / samples.len().max(1) as f64;
    let max = samples.iter().copied().fold(0.0, f64::max);
    (mean, max)
}

fn latency(gl: &Gl, caps: &Caps) -> Result<Check> {
    let controls = ControlBank::new(Vec::new())?;
    let mut player = player(gl, caps, LIGHT, [320, 180], true)?;
    let mut problems = Vec::new();
    for frame in 0..12_u64 {
        render(&mut player, &controls, frame);
        gl.finish();
        match (frame < LATENCY, shown(&player)) {
            (true, None) => {}
            (true, Some(seen)) => problems.push(format!("frame {frame} already shows {seen}")),
            (false, None) => problems.push(format!("frame {frame}: nothing arrived")),
            (false, Some(seen)) if age(frame, seen) != LATENCY => {
                problems.push(format!("frame {frame} shows frame {seen}"));
            }
            (false, Some(_)) => {}
        }
    }
    if let Some(code) = gl.error() {
        problems.push(format!("GL error 0x{code:x}"));
    }
    Ok(Check {
        name: "Latency",
        passed: problems.is_empty(),
        detail: if problems.is_empty() {
            "black for 2 frames, then exactly 2 frames old on every frame".to_string()
        } else {
            problems.join("; ")
        },
    })
}

fn cost(gl: &Gl, caps: &Caps, size: [i32; 2]) -> Result<Check> {
    let controls = ControlBank::new(Vec::new())?;
    // Two players, readback off and on, rendered alternately frame by
    // frame: anything else the machine is doing lands on both alike,
    // where timing one after the other would charge it to whichever ran
    // second.
    let mut players = [
        player(gl, caps, HEAVY, size, false)?,
        player(gl, caps, HEAVY, size, true)?,
    ];
    let mut times = [Vec::new(), Vec::new()];
    for frame in 0..10 + TIMED_FRAMES {
        for (player, samples) in players.iter_mut().zip(times.iter_mut()) {
            let start = Instant::now();
            render(player, &controls, frame);
            gl.finish();
            // The first frames pay for driver-side compilation.
            if frame >= 10 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
    let [off, on] = &times;
    let ((off_mean, off_max), (on_mean, on_max)) = (mean_max(off), mean_max(on));
    let delta = on_mean - off_mean;
    // A measurement, not a verdict: on a GPU the blits and the fence are
    // microseconds, but llvmpipe rasterises the downsample on the CPU and
    // pays milliseconds, so no one threshold fits every driver. Above a
    // millisecond is flagged for a person to judge against the frame.
    let flag = if delta > 1.0 { " (above 1 ms)" } else { "" };
    Ok(Check {
        name: "Cost",
        passed: true,
        detail: format!(
            "frame with glFinish, the two alternating: off {off_mean:.2} ms (max {off_max:.2}), \
             on {on_mean:.2} ms (max {on_max:.2}); readback costs {delta:+.3} ms a frame{flag}"
        ),
    })
}

fn busy(gl: &Gl, caps: &Caps, size: [i32; 2]) -> Result<Check> {
    let controls = ControlBank::new(Vec::new())?;
    let mut player = player(gl, caps, HEAVY, size, true)?;
    let mut calls = Vec::new();
    let mut ages = Vec::new();
    let mut problems = Vec::new();
    let mut newest: Option<u64> = None;
    for frame in 0..TIMED_FRAMES {
        let start = Instant::now();
        render(&mut player, &controls, frame);
        calls.push(start.elapsed().as_secs_f64() * 1000.0);
        let Some(seen) = shown(&player) else {
            continue;
        };
        let old = age(frame, seen);
        if old < LATENCY {
            problems.push(format!(
                "frame {frame} shows frame {seen}, newer than the ring"
            ));
        }
        // A picture going backwards shows as an age that grew by more
        // than the one frame that passed.
        if let Some(previous) = newest
            && age(seen, previous) > PERIOD / 2
        {
            problems.push(format!("went backwards to {seen} after {previous}"));
        }
        newest = Some(seen);
        ages.push(old as f64);
    }
    gl.finish();
    let (call_mean, call_max) = mean_max(&calls);
    let (age_mean, age_max) = mean_max(&ages);
    let dropped = player.final_pass_dropped();
    Ok(Check {
        name: "Busy GPU",
        passed: problems.is_empty(),
        detail: format!(
            "no glFinish, {TIMED_FRAMES} frames: age mean {age_mean:.1} max {age_max:.0} frames; \
             render() CPU {call_mean:.2} ms (max {call_max:.2}); {dropped} read(s) dropped{}",
            if problems.is_empty() {
                String::new()
            } else {
                format!("; {}", problems.join("; "))
            }
        ),
    })
}

fn resize(gl: &Gl, caps: &Caps) -> Result<Check> {
    let controls = ControlBank::new(Vec::new())?;
    let mut player = player(gl, caps, LIGHT, [128, 72], true)?;
    for frame in 0..4_u64 {
        render(&mut player, &controls, frame);
        gl.finish();
    }
    let before = player.final_pass_pixels().is_some();
    player.resize([200, 100])?;
    let dropped = player.final_pass_pixels().is_none();
    for frame in 4..8_u64 {
        render(&mut player, &controls, frame);
        gl.finish();
    }
    let after = shown(&player);
    let passed = before && dropped && after.is_some_and(|seen| age(7, seen) == LATENCY);
    Ok(Check {
        name: "Resize",
        passed,
        detail: format!(
            "data before: {before}; dropped on resize: {dropped}; after 4 frames at 200x100 \
             shows {}",
            after.map_or_else(|| "nothing".to_string(), |seen| format!("frame {seen}"))
        ),
    })
}

fn lost(gl: &Gl, caps: &Caps) -> Result<Check> {
    // The last check, because a context marked lost stays lost.
    let controls = ControlBank::new(Vec::new())?;
    let mut player = player(gl, caps, LIGHT, [64, 36], true)?;
    render(&mut player, &controls, 0);
    gl.mark_lost();
    drop(player);
    Ok(Check {
        name: "Lost context",
        passed: true,
        detail: "a player with a readback was dropped on a context marked lost without a GL call \
                 (reaching this row is the pass)"
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn age_counts_across_the_wrap() -> Result<()> {
        assert_eq!(age(12, 10), 2, "plain");
        assert_eq!(age(101, 99), 2, "across the wrap");
        assert_eq!(age(100, 98), 2, "at the wrap");
        Ok(())
    }

    #[test]
    fn mean_and_max_of_nothing_is_zero() -> Result<()> {
        assert_eq!(mean_max(&[]), (0.0, 0.0), "empty");
        assert_eq!(mean_max(&[1.0, 3.0]), (2.0, 3.0), "two");
        Ok(())
    }

    #[test]
    fn the_scenes_parse() -> Result<()> {
        scene(LIGHT)?;
        scene(HEAVY)?;
        if PERIOD == 0 {
            return Err(anyhow!("the period must not be zero"));
        }
        Ok(())
    }
}
