// ============================================================================
// Module:       strobe_gfx::gl::prime
// Description:  The once-per-context prime (D311): one tiny draw into a 16x16
//               target before anything else draws, for AMD's WGL driver,
//               which drops draws into large targets until it has drawn into
//               a small one.
//
// Dependencies: crate::gl (Gl and its owning types); log
// ============================================================================

//! The once-per-context prime (D311).
//!
//! On the venue mini PC (Radeon Vega 3, native WGL, driver
//! 31.0.21925.1001), every draw into a colour target larger than 16384
//! pixels (64 KiB of RGBA8) was silently dropped, with no GL error and no
//! debug message, until the context had made one draw into a target at or
//! below that size. After that one small draw, everything landed at every
//! size, in every target, for the life of the context (hardware tasks
//! M12-M14 in `docs/hardware/MINIPC.md`). Clears, reads and `glFinish` did
//! not unwedge it (M11); only a draw did.
//!
//! So every context does that small draw first. [`on_new_context`] runs from
//! [`Gl::from_loader`](super::Gl), the one place a [`Gl`] is built, which
//! makes it automatic for the app's contexts, the bench, the corpus, the
//! autocontrol tool, the tests and anything written later.
//!
//! The recipe: a 16x16 RGBA8 texture and framebuffer, a two-stage program
//! in the context's own dialect drawing one full-screen triangle in a
//! constant colour, clear, one draw, then a read-back of the 16x16 pixels.
//! The read-back is not what primes (M11 showed a read alone does not), but
//! every recipe that unwedged the driver on hardware read its pixels back
//! after the small draw, so this one does too; it also says whether the
//! prime's own draw landed, which is logged. Afterwards nothing is bound
//! (framebuffer 0, program 0, vertex array 0, no 2D texture on the active
//! unit), the viewport and unpack alignment are what they were, and the
//! objects are freed by their owning types.
//!
//! It runs once per context, never in a frame. A failure (a compile error
//! on an odd driver) is a warning and the context is used as normal:
//! priming is a workaround, not a requirement. `STROBE_GL_NO_PRIME=1` skips
//! it, to check on hardware whether a driver still needs it.

use super::{Format, Gl, Sampling, Stage};
use std::time::{Duration, Instant};

/// The prime target's side, in pixels: 256 pixels, far under the 16384
/// (64 KiB of RGBA8) above which the AMD driver dropped first draws.
const SIDE: i32 = 16;

/// The colour the prime draws, as the fragment shader writes it and as it
/// reads back.
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// The variable that turns the prime off.
const SKIP_ENV: &str = "STROBE_GL_NO_PRIME";

/// A full-screen triangle from `gl_VertexID` (0, 1, 2 give (-1,-1), (3,-1),
/// (-1,3)), without a vertex buffer. The `#version` line is prepended.
const VERTEX_BODY: &str = "void main() {\n\
     vec2 p = vec2(float((gl_VertexID & 1) << 2) - 1.0, float((gl_VertexID & 2) << 1) - 1.0);\n\
     gl_Position = vec4(p, 0.0, 1.0);\n\
     }\n";

/// A constant green. The `#version` line (and ES precision) is prepended.
const FRAGMENT_BODY: &str = "out vec4 strobe_prime_color;\n\
     void main() { strobe_prime_color = vec4(0.0, 1.0, 0.0, 1.0); }\n";

/// What one prime did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Primed {
    /// Whether every pixel of the prime target read back as the colour the
    /// draw wrote. On the AMD driver this is the small draw that lands.
    pub landed: bool,
    /// Wall-clock time for the whole prime, compile and read-back included.
    pub elapsed: Duration,
    /// GL errors pending afterwards (cleared, so they are not blamed on the
    /// caller's first call).
    pub errors: usize,
}

/// Primes a context just built by [`Gl::from_loader`](super::Gl), unless
/// `STROBE_GL_NO_PRIME` says not to, and logs the outcome. Never fails.
pub(super) fn on_new_context(gl: &Gl) {
    if skip_requested(std::env::var(SKIP_ENV).ok().as_deref()) {
        log::info!("GL prime skipped ({SKIP_ENV} is set; D311)");
        return;
    }
    match run(gl) {
        Ok(primed) if primed.landed && primed.errors == 0 => log::debug!(
            "GL prime: one {SIDE}x{SIDE} draw landed in {} us (D311)",
            primed.elapsed.as_micros()
        ),
        Ok(primed) => log::warn!(
            "GL prime: the {SIDE}x{SIDE} draw {} with {} GL error(s) in {} us; \
             large offscreen targets may render black on this driver (D311)",
            if primed.landed {
                "landed"
            } else {
                "did not land"
            },
            primed.errors,
            primed.elapsed.as_micros()
        ),
        Err(error) => log::warn!(
            "GL prime could not run ({error}); continuing without it, so large \
             offscreen targets may render black on an AMD WGL driver (D311)"
        ),
    }
}

/// Whether `STROBE_GL_NO_PRIME`'s value asks to skip the prime. Unset,
/// empty, `0`, `off` and `false` keep it.
fn skip_requested(value: Option<&str>) -> bool {
    match value.map(str::trim) {
        None | Some("" | "0" | "off" | "false") => false,
        Some(_) => true,
    }
}

/// Runs the prime on `gl`'s context: one draw into a 16x16 target, a
/// read-back, then the context put back with nothing bound and its viewport
/// and unpack alignment as they were.
///
/// It is meant for a fresh context, where nothing is bound to begin with;
/// called later, it still unbinds what it bound, so it leaves framebuffer 0,
/// program 0 and vertex array 0 current rather than whatever the caller had.
///
/// # Errors
///
/// The driver's message when the program does not compile or link, or a
/// texture or framebuffer cannot be made. The state is restored either way.
pub fn run(gl: &Gl) -> Result<Primed, String> {
    let started = now();
    let viewport = gl.viewport_box();
    let unpack = gl.get_i32(glow::UNPACK_ALIGNMENT);
    gl.clear_errors();
    let drawn = draw_and_read(gl);
    gl.bind_framebuffer(None);
    gl.use_raw_program(None);
    gl.bind_raw_vertex_array(None);
    gl.bind_texture(glow::TEXTURE_2D, None);
    gl.unpack_alignment(unpack);
    gl.restore_viewport(viewport);
    let errors = gl.clear_errors();
    let landed = drawn?;
    Ok(Primed {
        landed,
        elapsed: started.map_or(Duration::ZERO, |started| started.elapsed()),
        errors,
    })
}

/// The clock the prime is timed by. `wasm32-unknown-unknown` has none
/// (`Instant::now` panics there), so in a browser the prime reports zero.
fn now() -> Option<Instant> {
    if cfg!(target_arch = "wasm32") {
        None
    } else {
        Some(Instant::now())
    }
}

/// The draw itself; true when every pixel read back is [`GREEN`]. Every
/// object is dropped (and so deleted) on return, after the program and
/// vertex array are unbound.
fn draw_and_read(gl: &Gl) -> Result<bool, String> {
    let (vertex_source, fragment_source) = sources(gl.is_embedded());
    let vertex = gl.compile(Stage::Vertex, &vertex_source)?;
    let fragment = gl.compile(Stage::Fragment, &fragment_source)?;
    let program = gl.link(&vertex, &fragment)?;
    let texture = gl.texture_2d(SIDE, SIDE, Format::Rgba8, Sampling::LINEAR_CLAMP, None)?;
    let framebuffer = gl.framebuffer(&texture)?;
    let vertex_array = gl.vertex_array()?;

    gl.bind_framebuffer(Some(&framebuffer));
    gl.viewport(SIDE, SIDE);
    gl.clear([0.0; 4]);
    gl.use_program(&program);
    gl.bind_vertex_array(&vertex_array);
    gl.draw_fullscreen_triangle();
    gl.use_raw_program(None);
    gl.bind_raw_vertex_array(None);
    let pixels = gl.read_rgba8(&framebuffer, SIDE, SIDE);
    let (texels, _) = pixels.as_chunks::<4>();
    Ok(!texels.is_empty() && texels.iter().all(|texel| *texel == GREEN))
}

/// The vertex and fragment sources in the context's dialect: desktop GLSL
/// 3.30 core, or GLSL ES 3.00 for ANGLE and the headless GLES context.
fn sources(embedded: bool) -> (String, String) {
    if embedded {
        (
            format!("#version 300 es\n{VERTEX_BODY}"),
            format!("#version 300 es\nprecision mediump float;\n{FRAGMENT_BODY}"),
        )
    } else {
        (
            format!("#version 330 core\n{VERTEX_BODY}"),
            format!("#version 330 core\n{FRAGMENT_BODY}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prime_runs_unless_switched_off() {
        for value in [None, Some(""), Some("0"), Some(" off "), Some("false")] {
            assert!(!skip_requested(value), "{value:?} keeps the prime");
        }
        for value in [Some("1"), Some("yes"), Some("true")] {
            assert!(skip_requested(value), "{value:?} skips the prime");
        }
    }

    #[test]
    fn each_dialect_gets_its_own_version_line() {
        let (vertex, fragment) = sources(true);
        assert!(
            vertex.starts_with("#version 300 es\n") && fragment.contains("precision mediump"),
            "ES sources: {vertex} / {fragment}"
        );
        let (vertex, fragment) = sources(false);
        assert!(
            vertex.starts_with("#version 330 core\n")
                && fragment.starts_with("#version 330 core\n"),
            "desktop sources: {vertex} / {fragment}"
        );
    }
}
