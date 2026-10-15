// ============================================================================
// Module:       strobe_bench::probe
// Description:  `--probe`: for a ladder of sizes, whether the context's
//               driver clears, draws into and reads back an offscreen
//               target, and what viewport it holds afterwards. Finds the
//               size at which a driver starts dropping work.
//
// Dependencies: anyhow; strobe-gfx (Gl, formats, programs)
// ============================================================================

//! A driver probe for contexts that render black above some size (D296).
//!
//! On the JaJa mini-PC's Vega 3 (AMD, native WGL) the bench renders
//! correctly at 160x90 and all black at 200x90, in about a fifth of the time
//! the work should take. The renderer draws only into its own framebuffer
//! objects and sets the viewport for each (see the regression test in
//! `strobe-gfx/tests/render.rs`), so the cause is in what the driver does
//! with a target, not in a binding the code gets wrong. This probe takes the
//! scene out of the question: for each size and storage format it makes one
//! texture and framebuffer, then
//!
//! 1. sets the viewport and **reads it back** (`GL_VIEWPORT`), which shows a
//!    driver that clamps it to the window;
//! 2. clears to red and counts red pixels read back;
//! 3. draws a full-screen triangle that writes green, and counts green.
//!
//! A row with `clear` full and `draw` empty means the driver drops draws but
//! not clears. A viewport that comes back smaller than asked means it clamps
//! to the drawable. Both full means this size is fine. Rows are in
//! ascending size, so the first bad one is the limit. Run it with different
//! `--window-size` values to see whether the limit moves with the window.
//!
//! This is a diagnostic: it queries the driver freely, which the frame loop
//! never does.

use anyhow::Result;
use std::fmt::Write as _;
use strobe_gfx::Caps;
use strobe_gfx::gl::{Format, Gl, Sampling, Stage};
use strobe_scene::assemble::Dialect;

/// The sizes probed when the command line gives none: the sizes seen to
/// work and to fail on the mini-PC, the ones either side of 16384 pixels
/// (128x128), and a few that are long on one axis, to tell a per-axis limit
/// from a pixel count.
const LADDER: [[i32; 2]; 14] = [
    [16, 16],
    [100, 60],
    [160, 90],
    [161, 90],
    [160, 91],
    [128, 128],
    [129, 128],
    [200, 90],
    [160, 120],
    [1000, 16],
    [16, 1000],
    [320, 180],
    [640, 360],
    [1920, 1080],
];

/// The formats probed: the final pass's, and the one FLOAT passes use.
const FORMATS: [Format; 2] = [Format::Rgba8, Format::Rgba16f];

/// What one size and format did.
struct Row {
    size: [i32; 2],
    format: Format,
    /// The viewport the driver held after `glViewport(0, 0, w, h)`.
    viewport: [i32; 4],
    /// Share of pixels that read back red after a clear, 0 to 100.
    clear_red: f64,
    /// Share that read back green after a draw.
    draw_green: f64,
    /// The first GL error seen, if any.
    error: Option<u32>,
}

/// Runs the probe, returning the report and whether every row was good.
pub(crate) fn run(gl: &Gl, caps: &Caps, context: &str, extra: [i32; 2]) -> Result<(String, bool)> {
    let program = Painter::new(gl, caps.dialect)?;
    let mut sizes = LADDER.to_vec();
    if !sizes.contains(&extra) {
        sizes.push(extra);
    }
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for size in sizes {
        for format in FORMATS {
            if matches!(format, Format::Rgba16f) && !caps.float_render {
                continue;
            }
            match probe_one(gl, &program, size, format, caps.dialect) {
                Ok(row) => rows.push(row),
                Err(why) => {
                    failures.push(format!("{}x{} {}: {why}", size[0], size[1], format.name()))
                }
            }
        }
    }
    let good = |row: &Row| {
        row.clear_red > 99.0
            && row.draw_green > 99.0
            && row.viewport == [0, 0, row.size[0], row.size[1]]
            && row.error.is_none()
    };
    let passed = failures.is_empty() && rows.iter().all(good);

    let mut out = String::new();
    let _ = writeln!(out, "# Strobe driver probe\n");
    let _ = writeln!(out, "- Renderer: {}", caps.info.renderer);
    let _ = writeln!(out, "- Context: {context}");
    let _ = writeln!(
        out,
        "- Start state: viewport {:?}, scissor box {:?}, scissor test {}, max viewport {:?}, max texture {}\n",
        gl.viewport_box(),
        gl.scissor_box(),
        if gl.scissor_enabled() { "ON" } else { "off" },
        gl.max_viewport_dims(),
        caps.info.max_texture_size,
    );
    let _ = writeln!(
        out,
        "| Size | Pixels | Format | Viewport read back | Clear red % | Draw green % | GL error | |"
    );
    let _ = writeln!(out, "|---|---:|---|---|---:|---:|---|---|");
    for row in &rows {
        let _ = writeln!(
            out,
            "| {}x{} | {} | {} | {:?} | {:.1} | {:.1} | {} | {} |",
            row.size[0],
            row.size[1],
            i64::from(row.size[0]) * i64::from(row.size[1]),
            row.format.name(),
            row.viewport,
            row.clear_red,
            row.draw_green,
            row.error
                .map_or_else(|| "none".to_owned(), |code| format!("0x{code:x}")),
            if good(row) { "ok" } else { "BAD" },
        );
    }
    for failure in &failures {
        let _ = writeln!(out, "\nCould not probe {failure}");
    }
    let _ = writeln!(out, "\n{}", reading(&rows, passed));
    Ok((out, passed))
}

/// What the table says, in words, for the person reading it.
fn reading(rows: &[Row], passed: bool) -> String {
    if passed {
        return "Every size cleared, drew and kept its viewport: the driver is not the \
                reason a scene renders black at these sizes."
            .to_owned();
    }
    let bad = |row: &&Row| {
        row.clear_red <= 99.0
            || row.draw_green <= 99.0
            || row.viewport != [0, 0, row.size[0], row.size[1]]
    };
    let clamped = rows
        .iter()
        .any(|row| row.viewport != [0, 0, row.size[0], row.size[1]]);
    let dropped_draws = rows
        .iter()
        .filter(bad)
        .any(|row| row.clear_red > 99.0 && row.draw_green <= 99.0);
    let dropped_clears = rows.iter().filter(bad).any(|row| row.clear_red <= 99.0);
    let mut out = String::from("Reading:");
    if clamped {
        out.push_str(
            "\n- The driver changed the viewport: it clamps `glViewport` to something \
             (compare the read-back size with the window's, `--window-size`).",
        );
    }
    if dropped_draws {
        out.push_str(
            "\n- Some sizes cleared but did not draw: the driver drops draw calls into \
             targets of that size while still clearing them.",
        );
    }
    if dropped_clears {
        out.push_str(
            "\n- Some sizes did not even clear: the target itself is not being written \
             (or not read back) at that size.",
        );
    }
    out.push_str(
        "\n- Run again with `--window-size` larger than the failing size. If the failures \
         go away, the driver limits work to the window's drawable.",
    );
    out
}

/// Clears, draws and reads one target.
fn probe_one(
    gl: &Gl,
    painter: &Painter,
    size: [i32; 2],
    format: Format,
    dialect: Dialect,
) -> Result<Row, String> {
    let [w, h] = size;
    let texture = gl.texture_2d(w, h, format, Sampling::LINEAR_CLAMP, None)?;
    let fbo = gl.framebuffer(&texture)?;
    // GLES reads a float target back only as FLOAT, never as
    // RGBA/UNSIGNED_BYTE, so on an ES context (ANGLE) a float target is
    // copied into an RGBA8 one to be read. Desktop GL converts in the
    // read itself, so the native path the probe exists for adds no step.
    let copy = match (format, dialect) {
        (Format::Rgba16f | Format::Rgba32f, Dialect::Es300) => {
            let texture = gl.texture_2d(w, h, Format::Rgba8, Sampling::LINEAR_CLAMP, None)?;
            let fbo = gl.framebuffer(&texture)?;
            Some((texture, fbo))
        }
        (Format::Rgba8 | Format::R8, _)
        | (Format::Rgba16f | Format::Rgba32f, Dialect::Glsl330 | Dialect::Glsl410) => None,
    };
    let read = || match &copy {
        Some((_, rgba8)) => {
            gl.blit_between(&fbo, size, rgba8, size);
            gl.read_rgba8(rgba8, w, h)
        }
        None => gl.read_rgba8(&fbo, w, h),
    };
    gl.clear_errors();
    gl.plain_state();
    gl.bind_framebuffer(Some(&fbo));
    gl.viewport(w, h);
    let viewport = gl.viewport_box();

    gl.clear([1.0, 0.0, 0.0, 1.0]);
    gl.finish();
    let clear_red = share(&read(), |px| px[0] > 200 && px[1] < 50);

    gl.bind_framebuffer(Some(&fbo));
    gl.use_program(&painter.program);
    gl.bind_vertex_array(&painter.vao);
    gl.draw_fullscreen_triangle();
    gl.finish();
    let draw_green = share(&read(), |px| px[1] > 200 && px[0] < 50);
    gl.bind_framebuffer(None);
    Ok(Row {
        size,
        format,
        viewport,
        clear_red,
        draw_green,
        error: gl.error(),
    })
}

/// Percentage of RGBA pixels in `bytes` that `is` accepts.
fn share(bytes: &[u8], is: impl Fn(&[u8]) -> bool) -> f64 {
    let total = bytes.len() / 4;
    if total == 0 {
        return 0.0;
    }
    let (pixels, _) = bytes.as_chunks::<4>();
    let hits = pixels.iter().filter(|px| is(px.as_slice())).count();
    hits as f64 * 100.0 / total as f64
}

/// A program that paints everything green, and the vertex array to draw it.
struct Painter {
    program: strobe_gfx::gl::Program,
    vao: strobe_gfx::gl::VertexArray,
}

impl Painter {
    fn new(gl: &Gl, dialect: Dialect) -> Result<Self> {
        let header = match dialect {
            Dialect::Es300 => "#version 300 es\nprecision highp float;\n",
            Dialect::Glsl330 => "#version 330 core\n",
            Dialect::Glsl410 => "#version 410 core\n",
        };
        let vertex = format!(
            "{header}void main() {{ vec2 p = vec2(float((gl_VertexID << 1) & 2), \
             float(gl_VertexID & 2)); gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0); }}"
        );
        let fragment =
            format!("{header}out vec4 color; void main() {{ color = vec4(0.0, 1.0, 0.0, 1.0); }}");
        let vertex = gl
            .compile(Stage::Vertex, &vertex)
            .map_err(|log| anyhow::anyhow!("probe vertex shader: {log}"))?;
        let fragment = gl
            .compile(Stage::Fragment, &fragment)
            .map_err(|log| anyhow::anyhow!("probe fragment shader: {log}"))?;
        let program = gl
            .link(&vertex, &fragment)
            .map_err(|log| anyhow::anyhow!("probe program: {log}"))?;
        let vao = gl
            .vertex_array()
            .map_err(|e| anyhow::anyhow!("probe vertex array: {e}"))?;
        Ok(Self { program, vao })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_counts_the_pixels_a_test_accepts() -> Result<()> {
        let bytes = [255, 0, 0, 255, 0, 0, 0, 255, 255, 0, 0, 255, 9, 9, 9, 9];
        let red = share(&bytes, |px| px[0] > 200);
        assert!((red - 50.0).abs() < 1e-9, "two of four are red, got {red}");
        assert!(share(&[], |_| true).abs() < 1e-9, "no pixels is zero");
        Ok(())
    }

    #[test]
    fn the_ladder_straddles_the_sizes_seen_to_fail() -> Result<()> {
        for size in [[160, 90], [200, 90], [160, 120], [1920, 1080]] {
            assert!(LADDER.contains(&size), "{size:?} is on the ladder");
        }
        Ok(())
    }
}
