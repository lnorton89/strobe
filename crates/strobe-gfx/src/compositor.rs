// ============================================================================
// Module:       strobe_gfx::compositor
// Description:  The transition compositor: mixes the outgoing and incoming
//               scenes' frames into one, for crossfade, fade-through-black
//               and custom GLSL transitions.
//
// Dependencies: crate::gl, crate::caps; strobe-scene (assemble: vertex shader)
// ============================================================================

//! The transition compositor.
//!
//! While a playlist moves from one cue to the next, both scenes render and
//! this mixes their two frames into the picture the output and the preview
//! show. It is a small program of its own, **compiled once when the
//! compositor is made** (at start-up, on the context the show runs on) and
//! never per transition, so the first crossfade of the night costs the same
//! as the hundredth and no shader is compiled while a show plays (D152).
//!
//! # The built-in blends
//!
//! [`Blend::Crossfade`] and [`Blend::FadeThroughBlack`] share one program.
//! The CPU turns the transition's `progress` (0 at the start, 1 at the end)
//! into two weights ([`weights`]) and the shader computes
//! `from * wa + to * wb`:
//!
//! | Blend | `wa` (outgoing) | `wb` (incoming) |
//! |---|---|---|
//! | crossfade | `1 - p` | `p` |
//! | fade through black | `1 - 2p` for the first half, then 0 | 0 for the first half, then `2p - 1` |
//!
//! So progress 0 is exactly the outgoing frame and progress 1 exactly the
//! incoming one, whatever the blend; a transition never ends on a frame that
//! is not the next scene's. A *cut* has no compositor step at all: the clock
//! reports no transition for it.
//!
//! # Custom transitions
//!
//! A custom transition is a GLSL file that defines one function,
//! `vec4 transition(vec2 uv)`, the shape the open "GL Transitions"
//! collection uses. The compositor wraps it with `getFromColor(uv)`,
//! `getToColor(uv)`, `progress` (0 to 1) and `ratio` (width over height) and
//! compiles it with [`Compositor::add_custom`], which the app calls for each
//! distinct file while a playlist loads, never during playback. Parameters
//! with defaults declared in the collection's own comment syntax are not
//! read: a file must not need any uniform beyond the ones above.
//!
//! # What it renders into
//!
//! The mixed frame goes to the compositor's own RGBA8 target, the render
//! size of the scenes, with the same orientation as a player's output, so
//! the control preview, the letterboxed present and the screenshot read it
//! exactly as they would a scene's.

use crate::caps::Caps;
use crate::gl::{Format, Framebuffer, Gl, Program, Sampling, Stage, Texture, VertexArray};
use strobe_scene::assemble::{self, Dialect};

/// How the two frames are mixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    /// Dissolve from the outgoing frame to the incoming one.
    Crossfade,
    /// Fade the outgoing frame to black, then fade the incoming one in.
    FadeThroughBlack,
    /// A custom transition added with [`Compositor::add_custom`]; the number
    /// is the handle it returned.
    Custom(usize),
}

/// The weights `(outgoing, incoming)` a built-in blend gives the two frames
/// at `progress`. `progress` outside `0..1` (or not a number) is clamped, so
/// a late or early frame cannot overshoot either scene.
///
/// A custom blend is computed by its own shader; the answer here for it is
/// the crossfade's, which is what a caller that only wants a rough "how far
/// along" reads.
pub fn weights(blend: Blend, progress: f32) -> [f32; 2] {
    let p = if progress.is_nan() {
        0.0
    } else {
        progress.clamp(0.0, 1.0)
    };
    match blend {
        Blend::Crossfade | Blend::Custom(_) => [1.0 - p, p],
        Blend::FadeThroughBlack => [(1.0 - 2.0 * p).max(0.0), (2.0 * p - 1.0).max(0.0)],
    }
}

/// Why a compositor or a custom transition could not be built.
#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    /// The shader would not compile or link; the driver's log follows.
    #[error("transition shader: {0}")]
    Shader(String),
    /// GL refused to allocate a resource.
    #[error("GL resource: {0}")]
    Resource(String),
}

/// A program and the locations a draw sets.
#[derive(Debug)]
struct Mixer {
    program: Program,
    size: Option<glow::UniformLocation>,
    weights: Option<glow::UniformLocation>,
    progress: Option<glow::UniformLocation>,
    ratio: Option<glow::UniformLocation>,
}

/// The frame-mixing program set and the target it renders into.
#[derive(Debug)]
pub struct Compositor {
    gl: Gl,
    dialect: Dialect,
    vao: VertexArray,
    builtin: Mixer,
    custom: Vec<Mixer>,
    target: Texture,
    fbo: Framebuffer,
}

impl Compositor {
    /// Compiles the built-in program and allocates a `size` target.
    ///
    /// Call once, right after the context is made current (the app does it
    /// when it opens the context); this is the only place the built-in
    /// program is compiled.
    pub fn new(gl: &Gl, caps: &Caps, size: [i32; 2]) -> Result<Self, CompositorError> {
        let dialect = caps.dialect;
        let builtin = build(gl, dialect, &builtin_fragment(dialect))?;
        let vao = gl.vertex_array().map_err(CompositorError::Resource)?;
        let (target, fbo) = new_target(gl, size)?;
        Ok(Self {
            gl: gl.clone(),
            dialect,
            vao,
            builtin,
            custom: Vec::new(),
            target,
            fbo,
        })
    }

    /// Compiles a custom transition (`vec4 transition(vec2 uv)`, see the
    /// module docs) and returns the handle for [`Blend::Custom`]. This
    /// compiles a shader, so it belongs in a playlist's load, not its
    /// playback.
    pub fn add_custom(&mut self, source: &str) -> Result<usize, CompositorError> {
        let mixer = build(
            &self.gl,
            self.dialect,
            &custom_fragment(self.dialect, source),
        )?;
        self.custom.push(mixer);
        Ok(self.custom.len() - 1)
    }

    /// The size of the mixed frame.
    pub fn size(&self) -> [i32; 2] {
        [self.target.width(), self.target.height()]
    }

    /// Reallocates the target at `size` (its contents are not kept: the next
    /// [`Self::render`] fills it). The same size is a no-op.
    pub fn resize(&mut self, size: [i32; 2]) -> Result<(), CompositorError> {
        let size = [size[0].max(1), size[1].max(1)];
        if size == self.size() {
            return Ok(());
        }
        let (target, fbo) = new_target(&self.gl, size)?;
        self.target = target;
        self.fbo = fbo;
        Ok(())
    }

    /// Mixes `from` and `to` at `progress` into the target. A `Custom`
    /// handle that does not exist falls back to a crossfade.
    ///
    /// Allocates nothing and queries nothing: locations were resolved when
    /// the program was linked.
    pub fn render(&self, from: &Texture, to: &Texture, blend: Blend, progress: f32) {
        let gl = &self.gl;
        let [width, height] = self.size();
        gl.plain_state();
        gl.bind_vertex_array(&self.vao);
        gl.bind_framebuffer(Some(&self.fbo));
        gl.viewport(width, height);
        let custom = match blend {
            Blend::Custom(handle) => self.custom.get(handle),
            Blend::Crossfade | Blend::FadeThroughBlack => None,
        };
        let mixer = custom.unwrap_or(&self.builtin);
        gl.use_program(&mixer.program);
        gl.bind_unit(0, from);
        gl.bind_unit(1, to);
        if let Some(loc) = &mixer.size {
            gl.uniform_vec2(loc, [width as f32, height as f32]);
        }
        if let Some(loc) = &mixer.weights {
            let [wa, wb] = weights(blend, progress);
            gl.uniform_vec2(loc, [wa, wb]);
        }
        if let Some(loc) = &mixer.progress {
            gl.uniform_f32(loc, progress.clamp(0.0, 1.0));
        }
        if let Some(loc) = &mixer.ratio {
            gl.uniform_f32(loc, width as f32 / height.max(1) as f32);
        }
        gl.draw_fullscreen_triangle();
        gl.bind_framebuffer(None);
    }

    /// The mixed frame, e.g. for a UI preview.
    pub fn output_texture(&self) -> &Texture {
        &self.target
    }

    /// Draws the mixed frame into the default framebuffer, letterboxed to
    /// `window` pixels (as `ScenePlayer::present` does for a scene).
    pub fn present(&self, window: [i32; 2]) {
        let [w, h] = self.size();
        let scale = (window[0] as f32 / w as f32).min(window[1] as f32 / h as f32);
        let (dw, dh) = ((w as f32 * scale) as i32, (h as f32 * scale) as i32);
        let (x, y) = ((window[0] - dw) / 2, (window[1] - dh) / 2);
        self.gl.bind_framebuffer(None);
        self.gl.viewport(window[0], window[1]);
        self.gl.clear([0.0, 0.0, 0.0, 1.0]);
        self.gl
            .blit_to_default(&self.fbo, [w, h], [x, y, x + dw, y + dh]);
    }

    /// Reads the mixed frame back as RGBA8, top row first.
    pub fn read_output(&self) -> Vec<u8> {
        let [w, h] = self.size();
        let bottom_up = self.gl.read_rgba8(&self.fbo, w, h);
        let row = (w.max(0) as usize) * 4;
        let mut top_down = Vec::with_capacity(bottom_up.len());
        for chunk in bottom_up.chunks_exact(row.max(1)).rev() {
            top_down.extend_from_slice(chunk);
        }
        top_down
    }
}

/// An RGBA8 target of `size`, cleared to black.
fn new_target(gl: &Gl, size: [i32; 2]) -> Result<(Texture, Framebuffer), CompositorError> {
    let size = [size[0].max(1), size[1].max(1)];
    let res = CompositorError::Resource;
    let texture = gl
        .texture_2d(
            size[0],
            size[1],
            Format::Rgba8,
            Sampling::LINEAR_CLAMP,
            None,
        )
        .map_err(res)?;
    let fbo = gl
        .framebuffer(&texture)
        .map_err(CompositorError::Resource)?;
    gl.bind_framebuffer(Some(&fbo));
    gl.viewport(size[0], size[1]);
    gl.clear([0.0, 0.0, 0.0, 1.0]);
    gl.bind_framebuffer(None);
    Ok((texture, fbo))
}

/// Compiles and links `fragment` with the shared vertex shader and resolves
/// its locations, binding `u_from` to unit 0 and `u_to` to unit 1.
fn build(gl: &Gl, dialect: Dialect, fragment: &str) -> Result<Mixer, CompositorError> {
    let shader = CompositorError::Shader;
    let vertex = gl
        .compile(Stage::Vertex, &assemble::vertex(dialect))
        .map_err(|log| shader(format!("vertex: {log}")))?;
    let fragment = gl
        .compile(Stage::Fragment, fragment)
        .map_err(|log| shader(log.trim().to_string()))?;
    let program = gl
        .link(&vertex, &fragment)
        .map_err(|log| shader(format!("link: {log}")))?;
    gl.use_program(&program);
    // The two frames always sit on units 0 and 1, so these are set once.
    if let Some(loc) = gl.uniform_location(&program, "u_from") {
        gl.uniform_i32(&loc, 0);
    }
    if let Some(loc) = gl.uniform_location(&program, "u_to") {
        gl.uniform_i32(&loc, 1);
    }
    Ok(Mixer {
        size: gl.uniform_location(&program, "u_size"),
        weights: gl.uniform_location(&program, "u_weights"),
        progress: gl.uniform_location(&program, "progress"),
        ratio: gl.uniform_location(&program, "ratio"),
        program,
    })
}

/// The fragment shader for the built-in blends.
fn builtin_fragment(dialect: Dialect) -> String {
    format!(
        "{}\n{}\
         uniform sampler2D u_from;\n\
         uniform sampler2D u_to;\n\
         uniform vec2 u_size;\n\
         uniform vec2 u_weights;\n\
         out vec4 frag;\n\
         void main() {{\n\
         \x20   vec2 uv = gl_FragCoord.xy / u_size;\n\
         \x20   vec3 mixed = texture(u_from, uv).rgb * u_weights.x\n\
         \x20              + texture(u_to, uv).rgb * u_weights.y;\n\
         \x20   frag = vec4(mixed, 1.0);\n\
         }}\n",
        dialect.version_line(),
        dialect.precision()
    )
}

/// The fragment shader around a custom `vec4 transition(vec2 uv)`. The
/// `#line` keeps the driver's line numbers equal to the file's.
fn custom_fragment(dialect: Dialect, user: &str) -> String {
    format!(
        "{}\n{}\
         uniform sampler2D u_from;\n\
         uniform sampler2D u_to;\n\
         uniform vec2 u_size;\n\
         uniform float progress;\n\
         uniform float ratio;\n\
         out vec4 frag;\n\
         vec4 getFromColor(vec2 uv) {{ return texture(u_from, uv); }}\n\
         vec4 getToColor(vec2 uv) {{ return texture(u_to, uv); }}\n\
         #line 1\n\
         {user}\n\
         void main() {{\n\
         \x20   frag = vec4(transition(gl_FragCoord.xy / u_size).rgb, 1.0);\n\
         }}\n",
        dialect.version_line(),
        dialect.precision()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crossfade_starts_on_the_outgoing_frame_and_ends_on_the_incoming_one() {
        assert_eq!(weights(Blend::Crossfade, 0.0), [1.0, 0.0], "start");
        assert_eq!(weights(Blend::Crossfade, 0.5), [0.5, 0.5], "middle");
        assert_eq!(weights(Blend::Crossfade, 1.0), [0.0, 1.0], "end");
    }

    #[test]
    fn a_fade_through_black_is_dark_in_the_middle() {
        assert_eq!(weights(Blend::FadeThroughBlack, 0.0), [1.0, 0.0], "start");
        assert_eq!(
            weights(Blend::FadeThroughBlack, 0.5),
            [0.0, 0.0],
            "black at the halfway point"
        );
        assert_eq!(weights(Blend::FadeThroughBlack, 1.0), [0.0, 1.0], "end");
        let quarter = weights(Blend::FadeThroughBlack, 0.25);
        assert!(
            (quarter[0] - 0.5).abs() < 1e-6 && quarter[1] == 0.0,
            "a quarter of the way: half the outgoing frame, none of the incoming: {quarter:?}"
        );
    }

    #[test]
    fn progress_outside_the_range_never_overshoots() {
        for blend in [Blend::Crossfade, Blend::FadeThroughBlack] {
            assert_eq!(weights(blend, -1.0), weights(blend, 0.0), "{blend:?} early");
            assert_eq!(weights(blend, 2.0), weights(blend, 1.0), "{blend:?} late");
            assert_eq!(
                weights(blend, f32::NAN),
                weights(blend, 0.0),
                "{blend:?} not a number"
            );
        }
    }
}
