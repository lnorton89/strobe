// ============================================================================
// Module:       strobe_gfx::post
// Description:  The output post stage: master, invert, brightness, gamma,
//               contrast, hue, saturation and mirrors, applied once to the
//               finished frame.
//
// Dependencies: crate::gl, crate::caps; strobe-scene (meta values, assemble)
// ============================================================================

//! The output post stage (D180).
//!
//! Synesthesia's meta controls apply to the whole output rather than to one
//! scene. Strobe applies them here, to the frame that is about to be shown:
//! the final pass of the live scene, or the compositor's mix while a
//! playlist transition runs. See `docs/ARCHITECTURE.md` for where it sits.
//!
//! # One program, compiled once
//!
//! [`Post::new`] compiles the stage's single program when the context
//! opens, on the same terms as the compositor (D154, D152): a show never
//! compiles it again, and a change of values is only a handful of uniforms.
//!
//! # Zero cost at rest
//!
//! The stage costs one full-screen pass, and an old laptop notices that.
//! [`Post::apply`] therefore does nothing at all when every value is
//! neutral ([`MetaValues::post_is_neutral`]): no draw, no target touched,
//! and the caller keeps presenting the frame it already had. A pass is spent
//! only while the operator or a script is actually changing the picture.
//! [`Post::render`] always runs the program and exists so tests can prove
//! that neutral values through the shader are bit-for-bit the input.
//!
//! # What each control does
//!
//! Per pixel, in this order, on the 8-bit frame as drawn:
//!
//! 1. **Mirrors.** *Horizontal* reflects the left half onto the right;
//!    *vertical* reflects the bottom half onto the top. Texels are fetched
//!    exactly, so a mirror is a pixel-perfect flip, not a resample.
//! 2. **Brightness**: `c + b`.
//! 3. **Contrast**: `(c - 0.5) * k + 0.5`.
//! 4. **Gamma**: `c ^ (1 / g)`, so `g > 1` lifts the mid-tones.
//! 5. **Hue and saturation**: one 3x3 matrix ([`color_matrix`]) built on the
//!    CPU. Hue is a rotation about the grey axis `(1, 1, 1)`; saturation
//!    mixes towards Rec. 709 luma. The result is clamped to `0..1`.
//! 6. **Limit colors**, when on: the luma (Rec. 709) of the result picks a
//!    colour between *low* and *high*, `mix(low, high, luma)`, so the
//!    picture becomes a two-colour gradient map.
//! 7. **Invert**: `1 - c`.
//! 8. **Master**: `c * m`, last, so `0` is black whatever else is set. Alpha
//!    passes through untouched.
//!
//! The order, the hue method and which half a mirror keeps are Strobe's
//! choices. Synesthesia's differ in order and method (R16, read from its
//! engine); C-01 is to follow them.
//!
//! # Blackout and master
//!
//! Master is part of the picture: the preview, a screenshot and any future
//! capture output see it. The blackout key (D135) still only clears what
//! the shown output window presents. They compose; see D181.

use crate::caps::Caps;
use crate::gl::{Format, Framebuffer, Gl, Program, Sampling, Stage, Texture, VertexArray};
use strobe_scene::assemble::{self, Dialect};
use strobe_scene::meta::MetaValues;

/// Why the post stage could not be built or resized.
#[derive(Debug, thiserror::Error)]
pub enum PostError {
    /// The shader would not compile or link; the driver's log follows.
    #[error("post shader: {0}")]
    Shader(String),
    /// GL refused to allocate a resource.
    #[error("GL resource: {0}")]
    Resource(String),
}

/// The luma weights saturation mixes towards (Rec. 709).
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Smallest gamma the stage accepts, so `1 / gamma` stays finite.
const MIN_GAMMA: f32 = 0.05;

/// The hue-and-saturation matrix for `hue` turns and `saturation`, as nine
/// numbers in column-major order (what `uniform mat3` reads).
///
/// Hue is a rotation about the grey axis `(1, 1, 1)`; saturation is
/// `s * I + (1 - s) * L`, where every row of `L` is the luma weights. They
/// are multiplied as saturation after hue. At `hue = 0` and
/// `saturation = 1` the result is the identity **exactly** (no rounding
/// noise), which is what lets a neutral pass equal a bypass.
pub fn color_matrix(hue: f32, saturation: f32) -> [f32; 9] {
    let angle = hue * std::f32::consts::TAU;
    let (sin, cos) = angle.sin_cos();
    let third = (1.0 - cos) / 3.0;
    let side = sin / 3.0_f32.sqrt();
    // Row-major rotation about (1,1,1)/sqrt(3).
    let rotation = [
        [cos + third, third - side, third + side],
        [third + side, cos + third, third - side],
        [third - side, third + side, cos + third],
    ];
    let mut saturate = [[0.0_f32; 3]; 3];
    for (row, entries) in saturate.iter_mut().enumerate() {
        for (column, entry) in entries.iter_mut().enumerate() {
            let identity = if row == column { 1.0 } else { 0.0 };
            *entry = saturation * identity + (1.0 - saturation) * LUMA[column];
        }
    }
    let mut product = [[0.0_f32; 3]; 3];
    for (row, entries) in product.iter_mut().enumerate() {
        for (column, entry) in entries.iter_mut().enumerate() {
            *entry = (0..3)
                .map(|k| saturate[row][k] * rotation[k][column])
                .sum::<f32>();
        }
    }
    let mut columns = [0.0_f32; 9];
    for column in 0..3 {
        for row in 0..3 {
            columns[column * 3 + row] = product[row][column];
        }
    }
    columns
}

/// The locations a draw sets.
#[derive(Debug)]
struct Locations {
    size: Option<glow::UniformLocation>,
    mirror: Option<glow::UniformLocation>,
    color: Option<glow::UniformLocation>,
    tone: Option<glow::UniformLocation>,
    invert: Option<glow::UniformLocation>,
    limit_low: Option<glow::UniformLocation>,
    limit_high: Option<glow::UniformLocation>,
}

/// The post program and the target it renders into.
#[derive(Debug)]
pub struct Post {
    gl: Gl,
    vao: VertexArray,
    program: Program,
    at: Locations,
    target: Texture,
    fbo: Framebuffer,
}

impl Post {
    /// Compiles the program and allocates a `size` target. Call once, right
    /// after the context is made current; this is the only place the
    /// program is compiled.
    pub fn new(gl: &Gl, caps: &Caps, size: [i32; 2]) -> Result<Self, PostError> {
        let shader = PostError::Shader;
        let vertex = gl
            .compile(Stage::Vertex, &assemble::vertex(caps.dialect))
            .map_err(|log| shader(format!("vertex: {log}")))?;
        let fragment = gl
            .compile(Stage::Fragment, &fragment_source(caps.dialect))
            .map_err(|log| shader(log.trim().to_string()))?;
        let program = gl
            .link(&vertex, &fragment)
            .map_err(|log| shader(format!("link: {log}")))?;
        gl.use_program(&program);
        if let Some(loc) = gl.uniform_location(&program, "u_src") {
            gl.uniform_i32(&loc, 0);
        }
        let at = Locations {
            size: gl.uniform_location(&program, "u_size"),
            mirror: gl.uniform_location(&program, "u_mirror"),
            color: gl.uniform_location(&program, "u_color"),
            tone: gl.uniform_location(&program, "u_tone"),
            invert: gl.uniform_location(&program, "u_invert"),
            limit_low: gl.uniform_location(&program, "u_limit_low"),
            limit_high: gl.uniform_location(&program, "u_limit_high"),
        };
        let vao = gl.vertex_array().map_err(PostError::Resource)?;
        let (target, fbo) = new_target(gl, size)?;
        Ok(Self {
            gl: gl.clone(),
            vao,
            program,
            at,
            target,
            fbo,
        })
    }

    /// The size of the processed frame.
    pub fn size(&self) -> [i32; 2] {
        [self.target.width(), self.target.height()]
    }

    /// Applies `values` to `src` when they change the picture.
    ///
    /// Returns `Ok(false)` having done **nothing** when every value is
    /// neutral (the caller keeps using `src`), and `Ok(true)` after
    /// rendering into [`Self::output_texture`]. If `src` is a different
    /// size from the target the target is reallocated first (a resize; it
    /// does not happen in steady state).
    pub fn apply(&mut self, src: &Texture, values: &MetaValues) -> Result<bool, PostError> {
        if values.post_is_neutral() {
            return Ok(false);
        }
        self.render(src, values)?;
        Ok(true)
    }

    /// Runs the program whatever the values are. [`Self::apply`] is what the
    /// app calls; this is for tests that compare a neutral pass to a bypass.
    pub fn render(&mut self, src: &Texture, values: &MetaValues) -> Result<(), PostError> {
        let size = [src.width(), src.height()];
        if size != self.size() {
            let (target, fbo) = new_target(&self.gl, size)?;
            self.target = target;
            self.fbo = fbo;
        }
        let gl = &self.gl;
        let [width, height] = size;
        gl.plain_state();
        gl.bind_vertex_array(&self.vao);
        gl.bind_framebuffer(Some(&self.fbo));
        gl.viewport(width, height);
        gl.use_program(&self.program);
        gl.bind_unit(0, src);
        if let Some(loc) = &self.at.size {
            gl.uniform_vec2(loc, [width as f32, height as f32]);
        }
        if let Some(loc) = &self.at.mirror {
            gl.uniform_vec2(
                loc,
                [
                    f32::from(u8::from(values.horizontal_mirror)),
                    f32::from(u8::from(values.vertical_mirror)),
                ],
            );
        }
        if let Some(loc) = &self.at.color {
            gl.uniform_mat3(loc, &color_matrix(values.hue, values.saturation));
        }
        if let Some(loc) = &self.at.tone {
            gl.uniform_vec4(
                loc,
                [
                    values.brightness,
                    values.contrast,
                    1.0 / values.gamma.max(MIN_GAMMA),
                    values.master.clamp(0.0, 1.0),
                ],
            );
        }
        if let Some(loc) = &self.at.invert {
            gl.uniform_f32(loc, f32::from(u8::from(values.invert)));
        }
        if let Some(loc) = &self.at.limit_low {
            let [r, g, b] = values.limit_low;
            gl.uniform_vec4(loc, [r, g, b, f32::from(u8::from(values.limit_colors))]);
        }
        if let Some(loc) = &self.at.limit_high {
            let [r, g, b] = values.limit_high;
            gl.uniform_vec4(loc, [r, g, b, 0.0]);
        }
        gl.draw_fullscreen_triangle();
        gl.bind_framebuffer(None);
        Ok(())
    }

    /// The processed frame, e.g. for a UI preview. Only meaningful after
    /// [`Self::apply`] returned `true` this frame.
    pub fn output_texture(&self) -> &Texture {
        &self.target
    }

    /// Draws the processed frame into the default framebuffer, letterboxed
    /// to `window` pixels (as `ScenePlayer::present` does for a scene).
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

    /// Reads the processed frame back as RGBA8, top row first.
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
fn new_target(gl: &Gl, size: [i32; 2]) -> Result<(Texture, Framebuffer), PostError> {
    let size = [size[0].max(1), size[1].max(1)];
    let texture = gl
        .texture_2d(
            size[0],
            size[1],
            Format::Rgba8,
            Sampling::LINEAR_CLAMP,
            None,
        )
        .map_err(PostError::Resource)?;
    let fbo = gl.framebuffer(&texture).map_err(PostError::Resource)?;
    gl.bind_framebuffer(Some(&fbo));
    gl.viewport(size[0], size[1]);
    gl.clear([0.0, 0.0, 0.0, 1.0]);
    gl.bind_framebuffer(None);
    Ok((texture, fbo))
}

/// The fragment shader. `texelFetch` reads the source exactly, so a stage
/// whose values are neutral reproduces it bit for bit.
fn fragment_source(dialect: Dialect) -> String {
    format!(
        "{}\n{}\
         uniform sampler2D u_src;\n\
         uniform vec2 u_size;\n\
         uniform vec2 u_mirror;\n\
         uniform mat3 u_color;\n\
         uniform vec4 u_tone;\n\
         uniform float u_invert;\n\
         uniform vec4 u_limit_low;\n\
         uniform vec4 u_limit_high;\n\
         out vec4 frag;\n\
         void main() {{\n\
         \x20   ivec2 size = ivec2(u_size);\n\
         \x20   ivec2 p = ivec2(gl_FragCoord.xy);\n\
         \x20   if (u_mirror.x > 0.5 && p.x * 2 > size.x - 1) p.x = size.x - 1 - p.x;\n\
         \x20   if (u_mirror.y > 0.5 && p.y * 2 > size.y - 1) p.y = size.y - 1 - p.y;\n\
         \x20   vec4 texel = texelFetch(u_src, p, 0);\n\
         \x20   vec3 c = texel.rgb + u_tone.x;\n\
         \x20   c = (c - 0.5) * u_tone.y + 0.5;\n\
         \x20   c = pow(max(c, vec3(0.0)), vec3(u_tone.z));\n\
         \x20   c = clamp(u_color * c, 0.0, 1.0);\n\
         \x20   if (u_limit_low.w > 0.5) c = mix(u_limit_low.rgb, u_limit_high.rgb, dot(c, vec3(0.2126, 0.7152, 0.0722)));\n\
         \x20   if (u_invert > 0.5) c = 1.0 - c;\n\
         \x20   frag = vec4(c * u_tone.w, texel.a);\n\
         }}\n",
        dialect.version_line(),
        dialect.precision()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

    /// Applies a column-major matrix to a colour.
    fn apply(m: &[f32; 9], c: [f32; 3]) -> [f32; 3] {
        let at = |row: usize| m[row] * c[0] + m[3 + row] * c[1] + m[6 + row] * c[2];
        [at(0), at(1), at(2)]
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1.0e-5)
    }

    #[test]
    fn neutral_hue_and_saturation_are_exactly_the_identity() {
        assert_eq!(color_matrix(0.0, 1.0), IDENTITY, "no rounding noise");
    }

    #[test]
    fn a_third_of_a_turn_sends_red_to_green_and_green_to_blue() {
        let m = color_matrix(1.0 / 3.0, 1.0);
        assert!(close(apply(&m, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]), "red");
        assert!(close(apply(&m, [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]), "green");
        assert!(close(apply(&m, [0.5; 3]), [0.5; 3]), "grey stays grey");
    }

    #[test]
    fn zero_saturation_is_the_luma_grey_and_two_exaggerates() {
        let grey = apply(&color_matrix(0.0, 0.0), [1.0, 0.0, 0.0]);
        assert!(close(grey, [LUMA[0]; 3]), "red becomes its luma: {grey:?}");
        let more = apply(&color_matrix(0.0, 2.0), [0.6, 0.4, 0.4]);
        assert!(more[0] > 0.6 && more[1] < 0.4, "pushed apart: {more:?}");
    }
}
