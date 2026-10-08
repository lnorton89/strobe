// ============================================================================
// Module:       strobe_gfx::gl
// Description:  The only unsafe code in the workspace: safe one-call wrappers
//               over glow, and owning handles that free GL objects on drop.
//
// Dependencies: glow; submodule context (glutin context creation)
// ============================================================================

//! The GL boundary.
//!
//! Every `glow` method is `unsafe`, because a GL call is only defined when
//! a context is current on the calling thread and every handle passed in
//! came from that context. This module is where those two facts are
//! established once, so nothing above it needs the keyword:
//!
//! - A [`Gl`] is only ever built by [`context`], from a context it has
//!   just made current on this thread, and `Gl` is `!Send` — it cannot
//!   reach a thread where the context is not current. That is the
//!   invariant every `// SAFETY:` comment below leans on.
//! - Every object GL allocates (program, texture, framebuffer, vertex
//!   array) is held by an owning type here whose `Drop` releases it
//!   through the same `Gl`, so an early return cannot leak GPU memory.
//!
//! Each wrapper holds exactly one unsafe call and nothing else; argument
//! checking and conversions happen outside the block. GL itself validates
//! enums and handles and reports misuse through `glGetError` rather than
//! undefined behaviour, so the remaining obligation for each call is the
//! current-context invariant plus, for the few calls that take a pointer,
//! the length of the slice behind it — stated where it applies.

pub mod context;

use glow::HasContext;
use std::marker::PhantomData;
use std::sync::Arc;

/// A current GL context, usable from the thread it is current on.
#[derive(Clone)]
pub struct Gl {
    raw: Arc<glow::Context>,
    /// `!Send` and `!Sync`: the context is current on one thread only.
    _thread: PhantomData<*const ()>,
}

impl std::fmt::Debug for Gl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gl").finish_non_exhaustive()
    }
}

/// A shader stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Vertex shader.
    Vertex,
    /// Fragment shader.
    Fragment,
}

impl Stage {
    fn raw(self) -> u32 {
        match self {
            Self::Vertex => glow::VERTEX_SHADER,
            Self::Fragment => glow::FRAGMENT_SHADER,
        }
    }
}

/// A texture's storage format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 8 bits per channel.
    Rgba8,
    /// 16-bit float per channel.
    Rgba16f,
    /// 32-bit float per channel.
    Rgba32f,
    /// One 8-bit channel.
    R8,
}

impl Format {
    /// `(internal format, format, type)`.
    fn raw(self) -> (i32, u32, u32) {
        match self {
            Self::Rgba8 => (glow::RGBA8 as i32, glow::RGBA, glow::UNSIGNED_BYTE),
            Self::Rgba16f => (glow::RGBA16F as i32, glow::RGBA, glow::HALF_FLOAT),
            Self::Rgba32f => (glow::RGBA32F as i32, glow::RGBA, glow::FLOAT),
            Self::R8 => (glow::R8 as i32, glow::RED, glow::UNSIGNED_BYTE),
        }
    }

    /// Bytes per texel of the upload/readback type.
    pub fn bytes_per_texel(self) -> usize {
        match self {
            Self::Rgba8 => 4,
            Self::Rgba16f => 8,
            Self::Rgba32f => 16,
            Self::R8 => 1,
        }
    }
}

/// Texture sampling state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sampling {
    /// Wrap mode: `glow::REPEAT`, `CLAMP_TO_EDGE` or `MIRRORED_REPEAT`.
    pub wrap: u32,
    /// Minification filter.
    pub min: u32,
    /// Magnification filter.
    pub mag: u32,
}

impl Sampling {
    /// Linear filtering, edge clamped.
    pub const LINEAR_CLAMP: Self = Self {
        wrap: glow::CLAMP_TO_EDGE,
        min: glow::LINEAR,
        mag: glow::LINEAR,
    };
}

/// What GL reports about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// `GL_VERSION`.
    pub version: String,
    /// `GL_RENDERER`.
    pub renderer: String,
    /// `GL_VENDOR`.
    pub vendor: String,
    /// Major, minor, and whether this is OpenGL ES.
    pub parsed: (u32, u32, bool),
    /// Whether float color attachments are renderable on an ES context.
    pub es_float_render: bool,
    /// `GL_MAX_TEXTURE_SIZE`.
    pub max_texture_size: i32,
}

impl Gl {
    /// Wraps a glow context. Only [`context`] calls this, immediately after
    /// making the context current on this thread.
    fn from_current(raw: glow::Context) -> Self {
        Self {
            raw: Arc::new(raw),
            _thread: PhantomData,
        }
    }

    /// The glow context, for libraries (egui's painter) that drive GL
    /// themselves.
    pub fn glow(&self) -> Arc<glow::Context> {
        Arc::clone(&self.raw)
    }

    /// Version, renderer and capability facts.
    pub fn info(&self) -> Info {
        let version = self.raw.version();
        Info {
            version: self.get_string(glow::VERSION),
            renderer: self.get_string(glow::RENDERER),
            vendor: self.get_string(glow::VENDOR),
            parsed: (version.major, version.minor, version.is_embedded),
            es_float_render: self
                .raw
                .supported_extensions()
                .contains("GL_EXT_color_buffer_float"),
            max_texture_size: self.get_i32(glow::MAX_TEXTURE_SIZE),
        }
    }

    fn get_string(&self, name: u32) -> String {
        // SAFETY: current-context invariant (module docs); `name` is a GL
        // string enum, and glow copies the returned C string.
        unsafe { self.raw.get_parameter_string(name) }
    }

    fn get_i32(&self, name: u32) -> i32 {
        // SAFETY: current-context invariant; glow passes a stack slot of the
        // right size for a single-valued integer query.
        unsafe { self.raw.get_parameter_i32(name) }
    }

    /// The oldest pending GL error, or `None`.
    pub fn error(&self) -> Option<u32> {
        // SAFETY: current-context invariant.
        let code = unsafe { self.raw.get_error() };
        (code != glow::NO_ERROR).then_some(code)
    }

    /// Blocks until all submitted GL work has completed. Benchmarks only.
    pub fn finish(&self) {
        // SAFETY: current-context invariant.
        unsafe { self.raw.finish() }
    }

    /// Sets the viewport.
    pub fn viewport(&self, width: i32, height: i32) {
        // SAFETY: current-context invariant; negative sizes are a GL error,
        // not undefined behaviour.
        unsafe { self.raw.viewport(0, 0, width, height) }
    }

    /// Disables blending, depth, scissor and culling: every pass writes
    /// every pixel it covers exactly once.
    pub fn plain_state(&self) {
        for cap in [
            glow::BLEND,
            glow::DEPTH_TEST,
            glow::SCISSOR_TEST,
            glow::CULL_FACE,
        ] {
            self.disable(cap);
        }
    }

    fn disable(&self, cap: u32) {
        // SAFETY: current-context invariant.
        unsafe { self.raw.disable(cap) }
    }

    /// Clears the bound framebuffer's color to `rgba`.
    pub fn clear(&self, rgba: [f32; 4]) {
        self.clear_color(rgba);
        self.clear_color_bit();
    }

    fn clear_color(&self, [r, g, b, a]: [f32; 4]) {
        // SAFETY: current-context invariant.
        unsafe { self.raw.clear_color(r, g, b, a) }
    }

    fn clear_color_bit(&self) {
        // SAFETY: current-context invariant.
        unsafe { self.raw.clear(glow::COLOR_BUFFER_BIT) }
    }

    /// Draws one full-screen triangle with the bound program and VAO.
    pub fn draw_fullscreen_triangle(&self) {
        // SAFETY: current-context invariant; no vertex buffer is read, the
        // vertex shader derives positions from gl_VertexID.
        unsafe { self.raw.draw_arrays(glow::TRIANGLES, 0, 3) }
    }

    // ------------------------------------------------------------ shaders

    /// Compiles one shader stage, returning the driver's log on failure.
    pub fn compile(&self, stage: Stage, source: &str) -> Result<Shader, String> {
        let raw = self.create_shader(stage)?;
        let shader = Shader {
            gl: self.clone(),
            raw,
        };
        self.shader_source(raw, source);
        self.compile_shader(raw);
        if self.shader_compiled(raw) {
            Ok(shader)
        } else {
            Err(self.shader_log(raw))
        }
    }

    fn create_shader(&self, stage: Stage) -> Result<glow::Shader, String> {
        // SAFETY: current-context invariant.
        unsafe { self.raw.create_shader(stage.raw()) }
    }

    fn shader_source(&self, shader: glow::Shader, source: &str) {
        // SAFETY: current-context invariant; glow passes the &str's pointer
        // and exact length, so no terminator is read past its end.
        unsafe { self.raw.shader_source(shader, source) }
    }

    fn compile_shader(&self, shader: glow::Shader) {
        // SAFETY: current-context invariant; `shader` was created above.
        unsafe { self.raw.compile_shader(shader) }
    }

    fn shader_compiled(&self, shader: glow::Shader) -> bool {
        // SAFETY: current-context invariant.
        unsafe { self.raw.get_shader_compile_status(shader) }
    }

    fn shader_log(&self, shader: glow::Shader) -> String {
        // SAFETY: current-context invariant; glow sizes the buffer from
        // GL_INFO_LOG_LENGTH before reading.
        unsafe { self.raw.get_shader_info_log(shader) }
    }

    fn delete_shader(&self, shader: glow::Shader) {
        // SAFETY: current-context invariant; called once, from Shader::drop.
        unsafe { self.raw.delete_shader(shader) }
    }

    /// Links a vertex and fragment shader into a program.
    pub fn link(&self, vertex: &Shader, fragment: &Shader) -> Result<Program, String> {
        let raw = self.create_program()?;
        let program = Program {
            gl: self.clone(),
            raw,
        };
        self.attach(raw, vertex.raw);
        self.attach(raw, fragment.raw);
        self.link_program(raw);
        let linked = self.program_linked(raw);
        self.detach(raw, vertex.raw);
        self.detach(raw, fragment.raw);
        if linked {
            Ok(program)
        } else {
            Err(self.program_log(raw))
        }
    }

    fn create_program(&self) -> Result<glow::Program, String> {
        // SAFETY: current-context invariant.
        unsafe { self.raw.create_program() }
    }

    fn attach(&self, program: glow::Program, shader: glow::Shader) {
        // SAFETY: current-context invariant; both objects are live.
        unsafe { self.raw.attach_shader(program, shader) }
    }

    fn detach(&self, program: glow::Program, shader: glow::Shader) {
        // SAFETY: current-context invariant; both objects are live.
        unsafe { self.raw.detach_shader(program, shader) }
    }

    fn link_program(&self, program: glow::Program) {
        // SAFETY: current-context invariant.
        unsafe { self.raw.link_program(program) }
    }

    fn program_linked(&self, program: glow::Program) -> bool {
        // SAFETY: current-context invariant.
        unsafe { self.raw.get_program_link_status(program) }
    }

    fn program_log(&self, program: glow::Program) -> String {
        // SAFETY: current-context invariant; glow sizes the buffer first.
        unsafe { self.raw.get_program_info_log(program) }
    }

    fn delete_program(&self, program: glow::Program) {
        // SAFETY: current-context invariant; called once, from Program::drop.
        unsafe { self.raw.delete_program(program) }
    }

    /// Makes `program` current.
    pub fn use_program(&self, program: &Program) {
        self.use_raw_program(Some(program.raw));
    }

    fn use_raw_program(&self, program: Option<glow::Program>) {
        // SAFETY: current-context invariant; the handle is owned by a live
        // Program.
        unsafe { self.raw.use_program(program) }
    }

    /// The location of uniform `name` in `program`, if it is active.
    pub fn uniform_location(&self, program: &Program, name: &str) -> Option<glow::UniformLocation> {
        // SAFETY: current-context invariant; glow passes a NUL-terminated
        // copy of `name`.
        unsafe { self.raw.get_uniform_location(program.raw, name) }
    }

    /// Sets a float uniform on the current program.
    pub fn uniform_f32(&self, location: &glow::UniformLocation, value: f32) {
        // SAFETY: current-context invariant; the location came from the
        // program currently in use.
        unsafe { self.raw.uniform_1_f32(Some(location), value) }
    }

    /// Sets a vec2 uniform on the current program.
    pub fn uniform_vec2(&self, location: &glow::UniformLocation, [x, y]: [f32; 2]) {
        // SAFETY: as uniform_f32.
        unsafe { self.raw.uniform_2_f32(Some(location), x, y) }
    }

    /// Sets a vec3 uniform on the current program.
    pub fn uniform_vec3(&self, location: &glow::UniformLocation, [x, y, z]: [f32; 3]) {
        // SAFETY: as uniform_f32.
        unsafe { self.raw.uniform_3_f32(Some(location), x, y, z) }
    }

    /// Sets a vec4 uniform on the current program.
    pub fn uniform_vec4(&self, location: &glow::UniformLocation, [x, y, z, w]: [f32; 4]) {
        // SAFETY: as uniform_f32.
        unsafe { self.raw.uniform_4_f32(Some(location), x, y, z, w) }
    }

    /// Sets an int (or sampler unit) uniform on the current program.
    pub fn uniform_i32(&self, location: &glow::UniformLocation, value: i32) {
        // SAFETY: as uniform_f32.
        unsafe { self.raw.uniform_1_i32(Some(location), value) }
    }

    // ------------------------------------------------------------ textures

    /// Allocates a 2D texture, optionally uploading `data` (tightly packed
    /// rows of `format`'s upload type).
    pub fn texture_2d(
        &self,
        width: i32,
        height: i32,
        format: Format,
        sampling: Sampling,
        data: Option<&[u8]>,
    ) -> Result<Texture, String> {
        let expected =
            (width.max(0) as usize) * (height.max(0) as usize) * format.bytes_per_texel();
        let data = match data {
            Some(bytes) if bytes.len() != expected => {
                return Err(format!(
                    "texture upload of {} bytes for a {width}x{height} texture needing {expected}",
                    bytes.len()
                ));
            }
            other => other,
        };
        let raw = self.create_texture()?;
        let texture = Texture {
            gl: self.clone(),
            raw,
            target: glow::TEXTURE_2D,
            width,
            height,
            format,
        };
        self.bind_texture(glow::TEXTURE_2D, Some(raw));
        let (internal, fmt, ty) = format.raw();
        self.tex_image_2d(width, height, internal, fmt, ty, data);
        self.apply_sampling(glow::TEXTURE_2D, sampling);
        Ok(texture)
    }

    /// Allocates a 1D texture (desktop GL only).
    pub fn texture_1d(
        &self,
        width: i32,
        format: Format,
        data: Option<&[u8]>,
    ) -> Result<Texture, String> {
        let expected = width.max(0) as usize * format.bytes_per_texel();
        if data.is_some_and(|bytes| bytes.len() != expected) {
            return Err(format!("1D texture upload is not {expected} bytes"));
        }
        let raw = self.create_texture()?;
        let texture = Texture {
            gl: self.clone(),
            raw,
            target: glow::TEXTURE_1D,
            width,
            height: 1,
            format,
        };
        self.bind_texture(glow::TEXTURE_1D, Some(raw));
        let (internal, fmt, ty) = format.raw();
        self.tex_image_1d(width, internal, fmt, ty, data);
        self.apply_sampling(glow::TEXTURE_1D, Sampling::LINEAR_CLAMP);
        Ok(texture)
    }

    /// Replaces the whole contents of a texture.
    pub fn upload(&self, texture: &Texture, data: &[u8]) -> Result<(), String> {
        let expected = (texture.width.max(0) as usize)
            * (texture.height.max(0) as usize)
            * texture.format.bytes_per_texel();
        if data.len() != expected {
            return Err(format!(
                "upload of {} bytes, texture holds {expected}",
                data.len()
            ));
        }
        self.bind_texture(texture.target, Some(texture.raw));
        let (_, fmt, ty) = texture.format.raw();
        if texture.target == glow::TEXTURE_1D {
            self.tex_image_1d(texture.width, texture.format.raw().0, fmt, ty, Some(data));
        } else {
            self.tex_sub_image_2d(texture.width, texture.height, fmt, ty, data);
        }
        Ok(())
    }

    fn create_texture(&self) -> Result<glow::Texture, String> {
        // SAFETY: current-context invariant.
        unsafe { self.raw.create_texture() }
    }

    fn bind_texture(&self, target: u32, texture: Option<glow::Texture>) {
        // SAFETY: current-context invariant; the handle is owned by a live
        // Texture.
        unsafe { self.raw.bind_texture(target, texture) }
    }

    fn tex_image_2d(
        &self,
        width: i32,
        height: i32,
        internal: i32,
        fmt: u32,
        ty: u32,
        data: Option<&[u8]>,
    ) {
        // SAFETY: current-context invariant; when `data` is Some its length
        // was checked by the caller against width * height * texel size for
        // exactly this format/type pair, so GL reads only inside the slice.
        unsafe {
            self.raw.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal,
                width,
                height,
                0,
                fmt,
                ty,
                glow::PixelUnpackData::Slice(data),
            )
        }
    }

    fn tex_image_1d(&self, width: i32, internal: i32, fmt: u32, ty: u32, data: Option<&[u8]>) {
        // SAFETY: as tex_image_2d, with height 1.
        unsafe {
            self.raw.tex_image_1d(
                glow::TEXTURE_1D,
                0,
                internal,
                width,
                0,
                fmt,
                ty,
                glow::PixelUnpackData::Slice(data),
            )
        }
    }

    fn tex_sub_image_2d(&self, width: i32, height: i32, fmt: u32, ty: u32, data: &[u8]) {
        // SAFETY: as tex_image_2d; the caller checked the slice covers the
        // whole width * height region of this format.
        unsafe {
            self.raw.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                width,
                height,
                fmt,
                ty,
                glow::PixelUnpackData::Slice(Some(data)),
            )
        }
    }

    fn apply_sampling(&self, target: u32, sampling: Sampling) {
        self.tex_parameter(target, glow::TEXTURE_WRAP_S, sampling.wrap as i32);
        self.tex_parameter(target, glow::TEXTURE_WRAP_T, sampling.wrap as i32);
        self.tex_parameter(target, glow::TEXTURE_MIN_FILTER, sampling.min as i32);
        self.tex_parameter(target, glow::TEXTURE_MAG_FILTER, sampling.mag as i32);
    }

    fn tex_parameter(&self, target: u32, name: u32, value: i32) {
        // SAFETY: current-context invariant; a texture is bound to `target`.
        unsafe { self.raw.tex_parameter_i32(target, name, value) }
    }

    /// Regenerates a texture's mip chain from level 0.
    pub fn generate_mipmaps(&self, texture: &Texture) {
        self.bind_texture(texture.target, Some(texture.raw));
        self.generate_mipmap(texture.target);
    }

    fn generate_mipmap(&self, target: u32) {
        // SAFETY: current-context invariant; a texture is bound to `target`.
        unsafe { self.raw.generate_mipmap(target) }
    }

    /// Binds `texture` to texture unit `unit`.
    pub fn bind_unit(&self, unit: u32, texture: &Texture) {
        self.active_texture(unit);
        self.bind_texture(texture.target, Some(texture.raw));
    }

    fn active_texture(&self, unit: u32) {
        // SAFETY: current-context invariant; an out-of-range unit is a GL
        // error, not undefined behaviour.
        unsafe { self.raw.active_texture(glow::TEXTURE0 + unit) }
    }

    fn delete_texture(&self, texture: glow::Texture) {
        // SAFETY: current-context invariant; called once, from Texture::drop.
        unsafe { self.raw.delete_texture(texture) }
    }

    // ------------------------------------------------------------ framebuffers

    /// A framebuffer rendering into `texture`'s level 0.
    pub fn framebuffer(&self, texture: &Texture) -> Result<Framebuffer, String> {
        let raw = self.create_framebuffer()?;
        let fbo = Framebuffer {
            gl: self.clone(),
            raw,
        };
        self.bind_raw_framebuffer(glow::FRAMEBUFFER, Some(raw));
        self.attach_color(texture.raw);
        let status = self.framebuffer_status();
        self.bind_raw_framebuffer(glow::FRAMEBUFFER, None);
        if status == glow::FRAMEBUFFER_COMPLETE {
            Ok(fbo)
        } else {
            Err(format!("framebuffer incomplete (status 0x{status:x})"))
        }
    }

    fn create_framebuffer(&self) -> Result<glow::Framebuffer, String> {
        // SAFETY: current-context invariant.
        unsafe { self.raw.create_framebuffer() }
    }

    fn bind_raw_framebuffer(&self, target: u32, fbo: Option<glow::Framebuffer>) {
        // SAFETY: current-context invariant; the handle is owned by a live
        // Framebuffer, or None for the default framebuffer.
        unsafe { self.raw.bind_framebuffer(target, fbo) }
    }

    fn attach_color(&self, texture: glow::Texture) {
        // SAFETY: current-context invariant; a framebuffer is bound and the
        // texture is live.
        unsafe {
            self.raw.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            )
        }
    }

    fn framebuffer_status(&self) -> u32 {
        // SAFETY: current-context invariant.
        unsafe { self.raw.check_framebuffer_status(glow::FRAMEBUFFER) }
    }

    /// Binds `fbo` for drawing, or the default framebuffer for `None`.
    pub fn bind_framebuffer(&self, fbo: Option<&Framebuffer>) {
        self.bind_raw_framebuffer(glow::FRAMEBUFFER, fbo.map(|f| f.raw));
    }

    /// Copies `src` (a `src_size` image) into the default framebuffer at
    /// `dst` = `[x0, y0, x1, y1]`, filtered.
    pub fn blit_to_default(&self, src: &Framebuffer, src_size: [i32; 2], dst: [i32; 4]) {
        self.bind_raw_framebuffer(glow::READ_FRAMEBUFFER, Some(src.raw));
        self.bind_raw_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        self.blit(src_size, dst);
        self.bind_raw_framebuffer(glow::FRAMEBUFFER, None);
    }

    fn blit(&self, [w, h]: [i32; 2], [x0, y0, x1, y1]: [i32; 4]) {
        // SAFETY: current-context invariant; read and draw framebuffers are
        // bound by the caller.
        unsafe {
            self.raw.blit_framebuffer(
                0,
                0,
                w,
                h,
                x0,
                y0,
                x1,
                y1,
                glow::COLOR_BUFFER_BIT,
                glow::LINEAR,
            )
        }
    }

    /// Reads back `width * height` RGBA8 pixels from `fbo`, bottom row first.
    pub fn read_rgba8(&self, fbo: &Framebuffer, width: i32, height: i32) -> Vec<u8> {
        let mut pixels = vec![0_u8; (width.max(0) as usize) * (height.max(0) as usize) * 4];
        self.bind_raw_framebuffer(glow::FRAMEBUFFER, Some(fbo.raw));
        self.read_pixels(width, height, &mut pixels);
        self.bind_raw_framebuffer(glow::FRAMEBUFFER, None);
        pixels
    }

    fn read_pixels(&self, width: i32, height: i32, out: &mut [u8]) {
        // SAFETY: current-context invariant; `out` holds exactly
        // width * height * 4 bytes, the size GL writes for RGBA /
        // UNSIGNED_BYTE with the default pack alignment of 4.
        unsafe {
            self.raw.read_pixels(
                0,
                0,
                width,
                height,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(out)),
            )
        }
    }

    fn delete_framebuffer(&self, fbo: glow::Framebuffer) {
        // SAFETY: current-context invariant; called once, from
        // Framebuffer::drop.
        unsafe { self.raw.delete_framebuffer(fbo) }
    }

    // ------------------------------------------------------------ vertex arrays

    /// An empty vertex array, which core profiles require to draw at all.
    pub fn vertex_array(&self) -> Result<VertexArray, String> {
        let raw = self.create_vertex_array()?;
        Ok(VertexArray {
            gl: self.clone(),
            raw,
        })
    }

    fn create_vertex_array(&self) -> Result<glow::VertexArray, String> {
        // SAFETY: current-context invariant.
        unsafe { self.raw.create_vertex_array() }
    }

    /// Binds `vao`.
    pub fn bind_vertex_array(&self, vao: &VertexArray) {
        self.bind_raw_vertex_array(Some(vao.raw));
    }

    fn bind_raw_vertex_array(&self, vao: Option<glow::VertexArray>) {
        // SAFETY: current-context invariant; the handle is owned by a live
        // VertexArray.
        unsafe { self.raw.bind_vertex_array(vao) }
    }

    fn delete_vertex_array(&self, vao: glow::VertexArray) {
        // SAFETY: current-context invariant; called once, from
        // VertexArray::drop.
        unsafe { self.raw.delete_vertex_array(vao) }
    }
}

/// A compiled shader stage. Deleted on drop (GL keeps it alive while a
/// program it is attached to exists).
#[derive(Debug)]
pub struct Shader {
    gl: Gl,
    raw: glow::Shader,
}

impl Drop for Shader {
    fn drop(&mut self) {
        self.gl.delete_shader(self.raw);
    }
}

/// A linked program.
#[derive(Debug)]
pub struct Program {
    gl: Gl,
    raw: glow::Program,
}

impl Drop for Program {
    fn drop(&mut self) {
        self.gl.delete_program(self.raw);
    }
}

/// A texture.
#[derive(Debug)]
pub struct Texture {
    gl: Gl,
    raw: glow::Texture,
    target: u32,
    width: i32,
    height: i32,
    format: Format,
}

impl Texture {
    /// Width in texels.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// Height in texels.
    pub fn height(&self) -> i32 {
        self.height
    }

    /// Storage format.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The raw handle, for egui's painter to draw as a user texture.
    pub fn raw(&self) -> glow::Texture {
        self.raw
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        self.gl.delete_texture(self.raw);
    }
}

/// A framebuffer object.
#[derive(Debug)]
pub struct Framebuffer {
    gl: Gl,
    raw: glow::Framebuffer,
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        self.gl.delete_framebuffer(self.raw);
    }
}

/// A vertex array object.
#[derive(Debug)]
pub struct VertexArray {
    gl: Gl,
    raw: glow::VertexArray,
}

impl Drop for VertexArray {
    fn drop(&mut self) {
        self.gl.delete_vertex_array(self.raw);
    }
}
