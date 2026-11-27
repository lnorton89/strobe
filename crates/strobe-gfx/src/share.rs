// ============================================================================
// Module:       strobe_gfx::share
// Description:  The picture prepared for another program: scaled to the size
//               a sender asked for, blacked out with the show, and packed to
//               UYVY 4:2:2 on the GPU and read back through a fenced ring.
//
// Dependencies: crate::gl (Gl, Program, Texture, Framebuffer, Buffer, Fence),
//               crate::caps; strobe-scene (the vertex shader, the dialect)
// ============================================================================

//! Preparing the finished picture for Spout and NDI (D570, D573).
//!
//! The app hands the post stage's output (or the compositor's mix, or the
//! scene's) to a [`ShareStage`] once a frame, and asks for one or both of:
//!
//! - **A texture** for Spout, which shares GPU memory. When the sender's size
//!   is the picture's and the show is not blacked out, that is the picture's
//!   own texture and nothing is drawn. Otherwise the picture is drawn once,
//!   scaled, into the stage's own target, or that target is cleared to
//!   black, and the target is what is shared.
//! - **Pixels** for NDI. The NDI SDK's own advice is to send UYVY, convert
//!   on the GPU and send asynchronously, so the stage draws the picture into
//!   a half-width RGBA8 target whose every texel is one UYVY pair (U, Y0, V,
//!   Y1: BT.709, limited range, the equations `strobe_output::pixel` tests),
//!   rows top first, and reads it back into a ring of pixel-pack buffers
//!   behind fences. It collects the oldest read only once its fence has
//!   signalled, so the frame never waits on the GPU; the picture an NDI
//!   receiver gets is [`RING`] - 1 frames old. Half the bytes of RGBA cross
//!   the bus, and the CPU converts nothing.
//!
//! A picture whose width is odd is drawn to the even width below it for
//! NDI, since a UYVY pair needs two pixels.
//!
//! Everything is allocated when a size first appears or changes, never in
//! steady state, and nothing here queries the driver.

use crate::caps::Caps;
use crate::gl::{
    Buffer, Fence, FenceStatus, Format, Framebuffer, Gl, Program, Sampling, Stage, Texture,
    TextureShare, VertexArray,
};
use strobe_scene::assemble::{self, Dialect};

/// Pixel-pack buffers in the ring: the oldest is two frames old, which a
/// pipeline that queues up to three frames has usually finished, as for
/// `textureFinalPass` (`readback::RING`).
pub const RING: usize = 3;

/// What a frame asks of the stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Want {
    /// The size the senders want, or `None` for the picture's own.
    pub size: Option<[i32; 2]>,
    /// The show is blacked out: share black, not the picture.
    pub black: bool,
    /// A texture for a sender that shares GPU memory (Spout).
    pub texture: bool,
    /// UYVY pixels for a sender that sends frames (NDI).
    pub pixels: bool,
}

/// A program and the one location its draw sets.
#[derive(Debug)]
struct Pass {
    program: Program,
    size: Option<glow::UniformLocation>,
}

/// An RGBA8 target the stage draws into.
#[derive(Debug)]
struct Target {
    texture: Texture,
    fbo: Framebuffer,
    size: [i32; 2],
}

/// One pixel-pack buffer and the fence behind its read, if one is pending.
#[derive(Debug)]
struct Slot {
    buffer: Buffer,
    pending: Option<Fence>,
}

/// The packed target, its ring, and the newest frame that arrived.
#[derive(Debug)]
struct Packed {
    target: Target,
    /// The picture's size the packing is for (twice the target's width).
    picture: [i32; 2],
    slots: Vec<Slot>,
    next: usize,
}

/// The stage: two small programs, the targets they draw into and the ring.
#[derive(Debug)]
pub struct ShareStage {
    gl: Gl,
    vao: VertexArray,
    copy: Pass,
    pack: Pass,
    staged: Option<Target>,
    packed: Option<Packed>,
    /// The newest UYVY frame, rows top first: `picture[0] * 2` bytes a row.
    frame: Vec<u8>,
    /// The size `frame` holds, once a read has arrived.
    frame_size: Option<[u32; 2]>,
    /// Whether `frame` is new since the last [`Self::take_pixels`].
    fresh: bool,
    /// Reads abandoned because the GPU had not finished them in time.
    dropped: u64,
}

impl ShareStage {
    /// Compiles the two programs. Once, when an output is first turned on.
    ///
    /// # Errors
    ///
    /// A shader the driver refused, or a vertex array it would not make.
    pub fn new(gl: &Gl, caps: &Caps) -> Result<Self, String> {
        let dialect = caps.dialect;
        let copy = build(gl, dialect, &copy_fragment(dialect))?;
        let pack = build(gl, dialect, &pack_fragment(dialect))?;
        Ok(Self {
            gl: gl.clone(),
            vao: gl.vertex_array()?,
            copy,
            pack,
            staged: None,
            packed: None,
            frame: Vec::new(),
            frame_size: None,
            fresh: false,
            dropped: 0,
        })
    }

    /// Prepares this frame's picture from `source` as `want` asks, and
    /// returns the texture to share when one was asked for.
    ///
    /// # Errors
    ///
    /// A target or a buffer GL would not allocate, which happens only when a
    /// size changes. The caller says it once and carries on.
    pub fn prepare(
        &mut self,
        source: &Texture,
        want: Want,
    ) -> Result<Option<TextureShare>, String> {
        if self.gl.is_lost() || !(want.texture || want.pixels) {
            return Ok(None);
        }
        let own = [source.width().max(1), source.height().max(1)];
        let mut size = want.size.map_or(own, |s| [s[0].max(1), s[1].max(1)]);
        if want.pixels && size[0] % 2 != 0 {
            size[0] = (size[0] - 1).max(2);
        }
        let direct = size == own && !want.black;
        if !direct {
            self.draw_staged(source, size, want.black)?;
        }
        if want.pixels {
            self.capture(source, direct, size)?;
        } else {
            self.fresh = false;
        }
        if !want.texture {
            return Ok(None);
        }
        let shared = if direct {
            source.share()
        } else {
            match self.staged.as_ref() {
                Some(target) => target.texture.share(),
                None => return Ok(None),
            }
        };
        Ok(Some(shared))
    }

    /// The newest UYVY frame not yet taken, rows top first, with its
    /// picture size (`width` even, `width * 2` bytes a row). `None` when no
    /// new read has arrived since the last call.
    pub fn take_pixels(&mut self) -> Option<(&[u8], [u32; 2])> {
        if !self.fresh {
            return None;
        }
        self.fresh = false;
        let size = self.frame_size?;
        Some((self.frame.as_slice(), size))
    }

    /// Reads that were dropped because the GPU was late.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Draws `source` scaled into the staged target, or clears it to black.
    fn draw_staged(&mut self, source: &Texture, size: [i32; 2], black: bool) -> Result<(), String> {
        if self.staged.as_ref().is_none_or(|t| t.size != size) {
            self.staged = Some(target(&self.gl, size)?);
        }
        let Some(staged) = self.staged.as_ref() else {
            return Ok(());
        };
        let gl = &self.gl;
        gl.plain_state();
        gl.bind_framebuffer(Some(&staged.fbo));
        gl.viewport(size[0], size[1]);
        if black {
            gl.clear([0.0, 0.0, 0.0, 1.0]);
        } else {
            gl.bind_vertex_array(&self.vao);
            gl.use_program(&self.copy.program);
            gl.bind_unit(0, source);
            if let Some(loc) = &self.copy.size {
                gl.uniform_vec2(loc, [size[0] as f32, size[1] as f32]);
            }
            gl.draw_fullscreen_triangle();
        }
        gl.bind_framebuffer(None);
        Ok(())
    }

    /// Packs the picture to UYVY, starts its read, and collects the oldest.
    fn capture(&mut self, source: &Texture, direct: bool, size: [i32; 2]) -> Result<(), String> {
        if self.packed.as_ref().is_none_or(|p| p.picture != size) {
            self.packed = Some(packed(&self.gl, size)?);
            self.fresh = false;
        }
        let picture = if direct {
            source
        } else {
            match self.staged.as_ref() {
                Some(staged) => &staged.texture,
                None => return Ok(()),
            }
        };
        let Some(packed) = self.packed.as_mut() else {
            return Ok(());
        };
        let gl = &self.gl;
        let [width, height] = packed.target.size;
        gl.plain_state();
        gl.bind_vertex_array(&self.vao);
        gl.bind_framebuffer(Some(&packed.target.fbo));
        gl.viewport(width, height);
        gl.use_program(&self.pack.program);
        gl.bind_unit(0, picture);
        gl.draw_fullscreen_triangle();
        gl.bind_framebuffer(None);

        let index = packed.next;
        packed.next = (packed.next + 1) % RING;
        if let Some(slot) = packed.slots.get_mut(index) {
            if slot.pending.take().is_some() {
                self.dropped += 1;
            }
            gl.read_pixels_into(&packed.target.fbo, &slot.buffer, width, height)?;
            slot.pending = Some(gl.fence()?);
        }

        let oldest = packed.next;
        let Some(slot) = packed.slots.get_mut(oldest) else {
            return Ok(());
        };
        let Some(fence) = slot.pending.as_ref() else {
            return Ok(());
        };
        match gl.fence_status(fence) {
            FenceStatus::Pending => {}
            FenceStatus::Failed => {
                slot.pending = None;
                self.dropped += 1;
            }
            FenceStatus::Signaled => {
                slot.pending = None;
                let bytes = (width.max(0) as usize) * (height.max(0) as usize) * 4;
                self.frame.resize(bytes, 0);
                if gl.read_buffer(&slot.buffer, &mut self.frame) {
                    self.frame_size = Some([
                        packed.picture[0].max(0) as u32,
                        packed.picture[1].max(0) as u32,
                    ]);
                    self.fresh = true;
                } else {
                    self.dropped += 1;
                }
            }
        }
        Ok(())
    }
}

/// An RGBA8 target of `size`, cleared to black.
fn target(gl: &Gl, size: [i32; 2]) -> Result<Target, String> {
    let texture = gl.texture_2d(
        size[0],
        size[1],
        Format::Rgba8,
        Sampling::LINEAR_CLAMP,
        None,
    )?;
    let fbo = gl.framebuffer(&texture)?;
    gl.bind_framebuffer(Some(&fbo));
    gl.viewport(size[0], size[1]);
    gl.clear([0.0, 0.0, 0.0, 1.0]);
    gl.bind_framebuffer(None);
    Ok(Target { texture, fbo, size })
}

/// The half-width packed target for a `picture` and its ring of buffers.
fn packed(gl: &Gl, picture: [i32; 2]) -> Result<Packed, String> {
    let size = [packed_width(picture[0]), picture[1].max(1)];
    let bytes = (size[0] as usize) * (size[1] as usize) * 4;
    let mut slots = Vec::with_capacity(RING);
    for _ in 0..RING {
        slots.push(Slot {
            buffer: gl.pack_buffer(bytes)?,
            pending: None,
        });
    }
    Ok(Packed {
        target: target(gl, size)?,
        picture,
        slots,
        next: 0,
    })
}

/// Texels a row of the packed target holds: one per pixel pair.
pub fn packed_width(picture_width: i32) -> i32 {
    (picture_width.max(2) + 1) / 2
}

/// Compiles and links one of the stage's programs, its sampler on unit 0.
fn build(gl: &Gl, dialect: Dialect, fragment: &str) -> Result<Pass, String> {
    let vertex = gl
        .compile(Stage::Vertex, &assemble::vertex(dialect))
        .map_err(|log| format!("share vertex shader: {log}"))?;
    let fragment = gl
        .compile(Stage::Fragment, fragment)
        .map_err(|log| format!("share fragment shader: {}", log.trim()))?;
    let program = gl
        .link(&vertex, &fragment)
        .map_err(|log| format!("share program: {log}"))?;
    gl.use_program(&program);
    if let Some(loc) = gl.uniform_location(&program, "u_src") {
        gl.uniform_i32(&loc, 0);
    }
    Ok(Pass {
        size: gl.uniform_location(&program, "u_size"),
        program,
    })
}

/// Draws the picture scaled to the target, filtered.
fn copy_fragment(dialect: Dialect) -> String {
    format!(
        "{}\n{}\
         uniform sampler2D u_src;\n\
         uniform vec2 u_size;\n\
         out vec4 frag;\n\
         void main() {{\n\
         \x20   frag = vec4(texture(u_src, gl_FragCoord.xy / u_size).rgb, 1.0);\n\
         }}\n",
        dialect.version_line(),
        dialect.precision()
    )
}

/// Packs one UYVY pair per texel, rows top first, BT.709 limited range.
fn pack_fragment(dialect: Dialect) -> String {
    format!(
        "{}\n{}\
         uniform sampler2D u_src;\n\
         out vec4 frag;\n\
         const vec3 LUMA = vec3(0.2126, 0.7152, 0.0722);\n\
         void main() {{\n\
         \x20   ivec2 size = textureSize(u_src, 0);\n\
         \x20   ivec2 at = ivec2(gl_FragCoord.xy);\n\
         \x20   int row = size.y - 1 - at.y;\n\
         \x20   int left = min(at.x * 2, size.x - 1);\n\
         \x20   int right = min(left + 1, size.x - 1);\n\
         \x20   vec3 a = clamp(texelFetch(u_src, ivec2(left, row), 0).rgb, 0.0, 1.0);\n\
         \x20   vec3 b = clamp(texelFetch(u_src, ivec2(right, row), 0).rgb, 0.0, 1.0);\n\
         \x20   vec3 m = (a + b) * 0.5;\n\
         \x20   float ym = dot(m, LUMA);\n\
         \x20   float y0 = (16.0 + dot(a, LUMA) * 219.0) / 255.0;\n\
         \x20   float y1 = (16.0 + dot(b, LUMA) * 219.0) / 255.0;\n\
         \x20   float cb = (128.0 + (m.b - ym) * 224.0 / 1.8556) / 255.0;\n\
         \x20   float cr = (128.0 + (m.r - ym) * 224.0 / 1.5748) / 255.0;\n\
         \x20   frag = vec4(cb, y0, cr, y1);\n\
         }}\n",
        dialect.version_line(),
        dialect.precision()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_row_holds_one_texel_per_pixel_pair() {
        assert_eq!(packed_width(1920), 960, "an even width halves");
        assert_eq!(packed_width(1921), 961, "an odd width rounds up");
        assert_eq!(packed_width(0), 1, "never empty");
    }
}
