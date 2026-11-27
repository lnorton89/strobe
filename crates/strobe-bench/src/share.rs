// ============================================================================
// Module:       strobe_bench::share
// Description:  `--share`: what preparing the picture for Spout and NDI costs
//               a frame on the context the bench opened, against the first
//               cut's synchronous readback.
//
// Dependencies: anyhow; strobe-gfx (share stage, Gl)
// ============================================================================

//! The sharing paths timed on a real driver (backlog M-06, D573).
//!
//! Each row prepares a `--size` picture the way the app does for one kind of
//! sender, [`FRAMES`] times, and reports two numbers:
//!
//! - **CPU**: how long the call itself took. This is what the frame thread
//!   pays, and the number that must stay small: the NDI path is meant never
//!   to wait for the GPU.
//! - **Wall**: the call plus a `glFinish`, so the GPU's share of the work is
//!   counted too. An upper bound on what the frame costs in total.
//!
//! The rows:
//!
//! 1. Spout at the picture's size: the picture's own texture, no draw.
//! 2. Spout at a fixed 1920x1080: one scaled draw.
//! 3. Blackout: one clear.
//! 4. NDI: the UYVY pack and its asynchronous readback, with the frames
//!    until the first picture arrives.
//! 5. For comparison, the first cut's NDI path: a synchronous RGBA
//!    `glReadPixels` into a new buffer, turned top first into another.
//!
//! The picture is a still gradient, so the numbers isolate the sharing
//! work from any scene's cost.

use anyhow::{Result, anyhow};
use std::fmt::Write as _;
use std::time::Instant;
use strobe_gfx::Caps;
use strobe_gfx::gl::{Format, Gl, Sampling};
use strobe_gfx::share::{ShareStage, Want};

/// Timed frames per row.
const FRAMES: u32 = 120;

/// Untimed frames before each row, so the ring is full and caches warm.
const WARMUP: u32 = 10;

/// One row's numbers.
struct Timing {
    cpu_ms: f64,
    wall_ms: f64,
    cpu_max_ms: f64,
}

/// Runs the rows and returns the report, and whether every row ran.
pub(crate) fn run(gl: &Gl, caps: &Caps, context: &str, size: [i32; 2]) -> Result<(String, bool)> {
    let [width, height] = size;
    let pixels: Vec<u8> = (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                [
                    (x * 255 / width.max(1)) as u8,
                    (y * 255 / height.max(1)) as u8,
                    128,
                    255,
                ]
            })
        })
        .collect();
    let source = gl
        .texture_2d(
            width,
            height,
            Format::Rgba8,
            Sampling::LINEAR_CLAMP,
            Some(&pixels),
        )
        .map_err(|e| anyhow!("source texture: {e}"))?;
    let fbo = gl
        .framebuffer(&source)
        .map_err(|e| anyhow!("source framebuffer: {e}"))?;
    let mut stage = ShareStage::new(gl, caps).map_err(|e| anyhow!("share stage: {e}"))?;

    let base = Want {
        size: None,
        black: false,
        texture: true,
        pixels: false,
    };
    let mut rows: Vec<(String, Timing)> = Vec::new();
    let time = |label: String, want: Want, stage: &mut ShareStage| -> Result<Timing> {
        for _ in 0..WARMUP {
            stage
                .prepare(&source, want)
                .map_err(|e| anyhow!("{label}: {e}"))?;
            // Only the collection is timed; the frame itself goes nowhere.
            let _ = stage.take_pixels();
        }
        gl.finish();
        let (mut cpu, mut wall, mut cpu_max) = (0.0, 0.0, 0.0_f64);
        for _ in 0..FRAMES {
            let start = Instant::now();
            stage
                .prepare(&source, want)
                .map_err(|e| anyhow!("{label}: {e}"))?;
            // Only the collection is timed; the frame itself goes nowhere.
            let _ = stage.take_pixels();
            let called = start.elapsed().as_secs_f64() * 1000.0;
            gl.finish();
            cpu += called;
            cpu_max = cpu_max.max(called);
            wall += start.elapsed().as_secs_f64() * 1000.0;
        }
        Ok(Timing {
            cpu_ms: cpu / f64::from(FRAMES),
            wall_ms: wall / f64::from(FRAMES),
            cpu_max_ms: cpu_max,
        })
    };
    rows.push((
        "Spout, the picture's size (shared as it is)".to_string(),
        time(String::from("spout"), base, &mut stage)?,
    ));
    // A fixed size other than the picture's, so the row really draws.
    let fixed = if size == [1280, 720] {
        [1920, 1080]
    } else {
        [1280, 720]
    };
    rows.push((
        format!("Spout, fixed {}x{} (one scaled draw)", fixed[0], fixed[1]),
        time(
            String::from("spout fixed"),
            Want {
                size: Some(fixed),
                ..base
            },
            &mut stage,
        )?,
    ));
    rows.push((
        "Blackout (one clear)".to_string(),
        time(
            String::from("black"),
            Want {
                black: true,
                ..base
            },
            &mut stage,
        )?,
    ));
    let ndi = Want {
        texture: false,
        pixels: true,
        ..base
    };
    rows.push((
        "NDI: UYVY pack + asynchronous readback".to_string(),
        time(String::from("ndi"), ndi, &mut stage)?,
    ));

    // The first cut's path: a synchronous readback and a flip, two buffers
    // allocated every frame.
    let row_bytes = (width.max(0) as usize) * 4;
    gl.finish();
    let (mut cpu, mut cpu_max) = (0.0, 0.0_f64);
    for _ in 0..FRAMES {
        let start = Instant::now();
        let bottom_up = gl.read_rgba8(&fbo, width, height);
        let mut top_down = Vec::with_capacity(bottom_up.len());
        for chunk in bottom_up.chunks_exact(row_bytes.max(1)).rev() {
            top_down.extend_from_slice(chunk);
        }
        let called = start.elapsed().as_secs_f64() * 1000.0;
        cpu += called;
        cpu_max = cpu_max.max(called);
        std::hint::black_box(top_down);
    }
    let sync = cpu / f64::from(FRAMES);
    rows.push((
        "First cut's NDI path: synchronous RGBA readback + flip".to_string(),
        Timing {
            cpu_ms: sync,
            wall_ms: sync,
            cpu_max_ms: cpu_max,
        },
    ));

    // Frames until the first NDI picture arrives, and how many arrive, with
    // no glFinish and frames paced at 60 Hz as the show paces them.
    const PACE: std::time::Duration = std::time::Duration::from_micros(16_667);
    const PACED: u32 = 60;
    let mut fresh = ShareStage::new(gl, caps).map_err(|e| anyhow!("share stage: {e}"))?;
    let mut latency = None;
    let mut arrived = 0_u32;
    for frame in 1..=PACED {
        let start = Instant::now();
        fresh
            .prepare(&source, ndi)
            .map_err(|e| anyhow!("latency: {e}"))?;
        if fresh.take_pixels().is_some() {
            arrived += 1;
            latency.get_or_insert(frame);
        }
        std::thread::sleep(PACE.saturating_sub(start.elapsed()));
    }

    let error = gl.error();
    let mut report = String::new();
    writeln!(report, "# Sharing cost (`strobe-bench --share`, D573)\n")?;
    writeln!(
        report,
        "{} | {} | {:?} | {context} | picture {width}x{height} | {FRAMES} frames a row\n",
        caps.info.renderer, caps.info.version, caps.dialect
    )?;
    writeln!(
        report,
        "| Path | CPU mean (ms) | CPU max (ms) | Wall with glFinish (ms) |"
    )?;
    writeln!(report, "|---|---:|---:|---:|")?;
    for (label, timing) in &rows {
        writeln!(
            report,
            "| {label} | {:.3} | {:.3} | {:.3} |",
            timing.cpu_ms, timing.cpu_max_ms, timing.wall_ms
        )?;
    }
    writeln!(
        report,
        "\nPaced at 60 Hz with no glFinish, as the show runs: the first NDI picture \
         arrived on frame {}, {arrived} of {PACED} frames delivered one, and {} reads were \
         dropped because the GPU was late. Each picture is {} frames old when it arrives.",
        latency.map_or_else(|| "none".to_string(), |f| f.to_string()),
        fresh.dropped(),
        strobe_gfx::share::RING - 1,
    )?;
    if let Some(code) = error {
        writeln!(report, "\nGL error 0x{code:x} after the run.")?;
    }
    Ok((report, error.is_none() && latency.is_some()))
}
