// ============================================================================
// Module:       strobe_gfx::gl
// Description:  The only unsafe code in the workspace: safe one-call wrappers
//               over glow, and owning handles that free GL objects on drop.
//
// Dependencies: glow; submodules context, window (glutin context creation)
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
//! - **A lost context is never touched again.** [`Gl::mark_lost`] sets a flag
//!   shared by every clone of the `Gl`, and every owning type's `Drop` then
//!   skips its delete call. Once a context has been reset or destroyed, a
//!   GL call can fault inside the driver (on Windows the ICD may be gone
//!   entirely), and the driver frees a destroyed context's objects anyway.
//!   See [`Gl::reset_status`] for how a loss is detected.
//!
//! Each wrapper holds exactly one unsafe call and nothing else; argument
//! checking and conversions happen outside the block. GL itself validates
//! enums and handles and reports misuse through `glGetError` rather than
//! undefined behaviour, so the remaining obligation for each call is the
//! current-context invariant plus, for the few calls that take a pointer,
//! the length of the slice behind it — stated where it applies.

pub mod context;
pub mod debug;
pub mod window;

use glow::HasContext;
use std::cell::Cell;
use std::ffi::{CStr, CString, c_void};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

/// A current GL context, usable from the thread it is current on.
#[derive(Clone)]
pub struct Gl {
    raw: Arc<glow::Context>,
    /// `glGetGraphicsResetStatus` (or its `ARB`/`KHR`/`EXT` form), when the
    /// driver has one. glow 0.17 does not wrap it.
    reset: Option<ResetAddress>,
    /// Set once the context is known to be lost; shared by every clone, so
    /// every owning type sees it. `Rc` because `Gl` is `!Send` anyway.
    lost: Rc<Cell<bool>>,
    /// `!Send` and `!Sync`: the context is current on one thread only.
    _thread: PhantomData<*const ()>,
}

/// The address of the driver's reset-status function.
#[derive(Debug, Clone, Copy)]
struct ResetAddress(*const c_void);

/// What `glGetGraphicsResetStatus` said about the context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetStatus {
    /// `GL_NO_ERROR`: no reset has happened since the last query.
    NoError,
    /// `GL_GUILTY_CONTEXT_RESET`: this context's own work caused a reset
    /// (a shader that ran too long, a bad access).
    Guilty,
    /// `GL_INNOCENT_CONTEXT_RESET`: the GPU was reset by something else
    /// (another process, a power event) and this context lost its state.
    Innocent,
    /// `GL_UNKNOWN_CONTEXT_RESET`: reset, with no blame assigned. Also what
    /// [`Gl::reset_status`] returns without asking the driver once the
    /// context has been marked lost.
    Unknown,
    /// The driver offers no way to ask (no robustness extension), or
    /// answered with a value the specification does not define.
    Unsupported,
}

impl ResetStatus {
    const GUILTY: u32 = 0x8253;
    const INNOCENT: u32 = 0x8254;
    const UNKNOWN: u32 = 0x8255;

    /// Maps the `GLenum` a driver returned.
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            glow::NO_ERROR => Self::NoError,
            Self::GUILTY => Self::Guilty,
            Self::INNOCENT => Self::Innocent,
            Self::UNKNOWN => Self::Unknown,
            _ => Self::Unsupported,
        }
    }

    /// Whether the driver reported that the context was reset: the three
    /// of [`Self::Guilty`], [`Self::Innocent`] and [`Self::Unknown`].
    pub fn is_reset(self) -> bool {
        matches!(self, Self::Guilty | Self::Innocent | Self::Unknown)
    }
}

impl std::fmt::Display for ResetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoError => "no error",
            Self::Guilty => "guilty context reset",
            Self::Innocent => "innocent context reset",
            Self::Unknown => "unknown context reset",
            Self::Unsupported => "reset status unsupported",
        })
    }
}

/// The entry-point names to try for `glGetGraphicsResetStatus`, in order,
/// for a context of this `version` (major, minor, is ES) offering these
/// extensions. A name is tried only when the context should have it,
/// because some loaders (Mesa's `glXGetProcAddress`) hand back a stub for
/// any name they know.
fn reset_status_names(
    (major, minor, embedded): (u32, u32, bool),
    has_extension: impl Fn(&str) -> bool,
) -> Vec<&'static str> {
    let mut names = Vec::new();
    let core = if embedded {
        (major, minor) >= (3, 2)
    } else {
        (major, minor) >= (4, 5)
    };
    // KHR_robustness on desktop GL uses the unsuffixed name.
    if core || (!embedded && has_extension("GL_KHR_robustness")) {
        names.push("glGetGraphicsResetStatus");
    }
    if has_extension("GL_ARB_robustness") {
        names.push("glGetGraphicsResetStatusARB");
    }
    if embedded && has_extension("GL_KHR_robustness") {
        names.push("glGetGraphicsResetStatusKHR");
    }
    if has_extension("GL_EXT_robustness") {
        names.push("glGetGraphicsResetStatusEXT");
    }
    names
}

/// Calls the driver's reset-status function.
fn call_reset_status(address: ResetAddress) -> u32 {
    // SAFETY: `address` was returned by the display's proc loader for an
    // entry point named `glGetGraphicsResetStatus*`, which is the C function
    // `GLenum f(void)`; the transmute only gives that address its type, and
    // function and data pointers are the same size on every platform this
    // builds for. The fn-pointer type is written without the unsafe
    // qualifier so this block holds the one operation; the call itself
    // relies on the current-context invariant (callers document that the
    // context must be current).
    let function =
        unsafe { std::mem::transmute::<*const c_void, extern "system" fn() -> u32>(address.0) };
    function()
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
    /// Whether `GL_TIME_ELAPSED` queries work: core in desktop GL 3.3 (or
    /// `ARB_timer_query`), and `EXT_disjoint_timer_query` on ES.
    pub gpu_timer: bool,
}

/// `GL_GPU_DISJOINT_EXT`: set when a timer result may be garbage because
/// the GPU was reset or throttled (the ES extension's one extra query).
const GPU_DISJOINT_EXT: u32 = 0x8FBB;

impl Gl {
    /// Builds a `Gl` from a proc-address loader. Only [`context`] and
    /// [`window`] call this, immediately after making the context current on
    /// this thread. The same loader that builds the glow context also looks
    /// up the reset-status function glow does not wrap.
    fn from_loader(mut loader: impl FnMut(&CStr) -> *const c_void) -> Self {
        let raw = context::glow_from_loader(&mut loader);
        let version = raw.version();
        let extensions = raw.supported_extensions();
        let names = reset_status_names(
            (version.major, version.minor, version.is_embedded),
            |name| extensions.contains(name),
        );
        let reset = names.iter().find_map(|name| {
            let name = CString::new(*name).ok()?;
            let address = loader(&name);
            (!address.is_null()).then_some(ResetAddress(address))
        });
        Self {
            raw: Arc::new(raw),
            reset,
            lost: Rc::new(Cell::new(false)),
            _thread: PhantomData,
        }
    }

    /// Records that this context is lost (reset, destroyed, or no longer
    /// current anywhere). From now on no owning type calls into GL when it
    /// is dropped: the driver releases a destroyed context's objects, and
    /// a delete against a dead context can fault in the ICD. Shared by
    /// every clone of this `Gl`.
    pub fn mark_lost(&self) {
        self.lost.set(true);
    }

    /// Whether [`Self::mark_lost`] was called.
    pub fn is_lost(&self) -> bool {
        self.lost.get()
    }

    /// Whether the driver exposes a reset-status query at all.
    pub fn has_reset_status(&self) -> bool {
        self.reset.is_some()
    }

    /// Asks the driver whether the context has been reset
    /// (`glGetGraphicsResetStatus`).
    ///
    /// This is a driver round trip, so it is **not** a per-frame call: make
    /// it after a failed swap, or at most once a second as a health check.
    /// The context must be current. A reset is reported once, then the query
    /// reads [`ResetStatus::NoError`] again, and a context that was not
    /// created robust may always answer `NoError`. Returns
    /// [`ResetStatus::Unknown`] without asking if the context was already
    /// marked lost, and [`ResetStatus::Unsupported`] if the driver has no
    /// such function.
    pub fn reset_status(&self) -> ResetStatus {
        if self.is_lost() {
            return ResetStatus::Unknown;
        }
        match self.reset {
            Some(address) => ResetStatus::from_raw(call_reset_status(address)),
            None => ResetStatus::Unsupported,
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
        let extensions = self.raw.supported_extensions();
        let gpu_timer = if version.is_embedded {
            extensions.contains("GL_EXT_disjoint_timer_query")
        } else {
            (version.major, version.minor) >= (3, 3) || extensions.contains("GL_ARB_timer_query")
        };
        Info {
            version: self.get_string(glow::VERSION),
            renderer: self.get_string(glow::RENDERER),
            vendor: self.get_string(glow::VENDOR),
            parsed: (version.major, version.minor, version.is_embedded),
            es_float_render: extensions.contains("GL_EXT_color_buffer_float"),
            max_texture_size: self.get_i32(glow::MAX_TEXTURE_SIZE),
            gpu_timer,
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

    /// Reads back `width * height` RGBA8 pixels from the default
    /// framebuffer's back buffer, bottom row first. For screenshots; call
    /// it after drawing and before the swap.
    pub fn read_default_rgba8(&self, width: i32, height: i32) -> Vec<u8> {
        let mut pixels = vec![0_u8; (width.max(0) as usize) * (height.max(0) as usize) * 4];
        self.bind_raw_framebuffer(glow::FRAMEBUFFER, None);
        self.read_pixels(width, height, &mut pixels);
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

    // ------------------------------------------------------------ timer queries

    /// A timer query object. Check [`Info::gpu_timer`] first: without it,
    /// beginning one is a GL error.
    pub fn timer_query(&self) -> Result<TimerQuery, String> {
        let raw = self.create_query()?;
        Ok(TimerQuery {
            gl: self.clone(),
            raw,
        })
    }

    fn create_query(&self) -> Result<glow::Query, String> {
        // SAFETY: current-context invariant.
        unsafe { self.raw.create_query() }
    }

    /// Starts timing GPU work with `query`; only one may be active.
    pub fn begin_timer(&self, query: &TimerQuery) {
        self.begin_query(glow::TIME_ELAPSED, query.raw);
    }

    fn begin_query(&self, target: u32, query: glow::Query) {
        // SAFETY: current-context invariant; the handle is owned by a live
        // TimerQuery and no other query of this target is active (the
        // caller, `GpuTimer`, begins at most one at a time).
        unsafe { self.raw.begin_query(target, query) }
    }

    /// Stops the active timer query.
    pub fn end_timer(&self) {
        self.end_query(glow::TIME_ELAPSED);
    }

    fn end_query(&self, target: u32) {
        // SAFETY: current-context invariant; ending with none active is a
        // GL error, not undefined behaviour.
        unsafe { self.raw.end_query(target) }
    }

    /// Whether `query`'s result is ready. Never waits.
    pub fn timer_ready(&self, query: &TimerQuery) -> bool {
        self.query_u32(query.raw, glow::QUERY_RESULT_AVAILABLE) != 0
    }

    /// The elapsed GPU time `query` measured, in nanoseconds, saturating at
    /// about 4.29 s. Only call after [`Self::timer_ready`] says so: this
    /// otherwise waits for the GPU.
    pub fn timer_nanoseconds(&self, query: &TimerQuery) -> u32 {
        self.query_u32(query.raw, glow::QUERY_RESULT)
    }

    fn query_u32(&self, query: glow::Query, parameter: u32) -> u32 {
        // SAFETY: current-context invariant; the handle is owned by a live
        // TimerQuery and the parameter is a query-object enum.
        unsafe { self.raw.get_query_parameter_u32(query, parameter) }
    }

    /// Whether the GPU was disturbed since the last call, so recent timer
    /// results may be garbage. Meaningful on ES (`EXT_disjoint_timer_query`);
    /// the one `glGet` a timer needs, made only when reading results.
    pub fn gpu_disjoint(&self) -> bool {
        self.get_i32(GPU_DISJOINT_EXT) != 0
    }

    fn delete_query(&self, query: glow::Query) {
        // SAFETY: current-context invariant; called once, from
        // TimerQuery::drop.
        unsafe { self.raw.delete_query(query) }
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
        if !self.gl.is_lost() {
            self.gl.delete_shader(self.raw);
        }
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
        if !self.gl.is_lost() {
            self.gl.delete_program(self.raw);
        }
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
        if !self.gl.is_lost() {
            self.gl.delete_texture(self.raw);
        }
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
        if !self.gl.is_lost() {
            self.gl.delete_framebuffer(self.raw);
        }
    }
}

/// A GPU timer query object.
#[derive(Debug)]
pub struct TimerQuery {
    gl: Gl,
    raw: glow::Query,
}

impl Drop for TimerQuery {
    fn drop(&mut self) {
        if !self.gl.is_lost() {
            self.gl.delete_query(self.raw);
        }
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
        if !self.gl.is_lost() {
            self.gl.delete_vertex_array(self.raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(version: (u32, u32, bool), extensions: &[&str]) -> Vec<&'static str> {
        reset_status_names(version, |name| extensions.contains(&name))
    }

    #[test]
    fn the_enum_values_map_to_the_four_defined_statuses() {
        assert_eq!(ResetStatus::from_raw(0), ResetStatus::NoError, "no error");
        assert_eq!(ResetStatus::from_raw(0x8253), ResetStatus::Guilty, "guilty");
        assert_eq!(
            ResetStatus::from_raw(0x8254),
            ResetStatus::Innocent,
            "innocent"
        );
        assert_eq!(
            ResetStatus::from_raw(0x8255),
            ResetStatus::Unknown,
            "unknown"
        );
        assert_eq!(
            ResetStatus::from_raw(0x0500),
            ResetStatus::Unsupported,
            "an undefined value says nothing"
        );
    }

    #[test]
    fn only_the_three_reset_answers_count_as_a_reset() {
        for (status, reset) in [
            (ResetStatus::NoError, false),
            (ResetStatus::Guilty, true),
            (ResetStatus::Innocent, true),
            (ResetStatus::Unknown, true),
            (ResetStatus::Unsupported, false),
        ] {
            assert_eq!(status.is_reset(), reset, "{status}");
        }
    }

    #[test]
    fn desktop_gl_45_has_the_plain_name_and_an_old_driver_has_the_arb_one() {
        assert_eq!(
            names((4, 6, false), &["GL_ARB_robustness"]),
            ["glGetGraphicsResetStatus", "glGetGraphicsResetStatusARB"],
            "plain first, then ARB"
        );
        assert_eq!(
            names((3, 3, false), &["GL_ARB_robustness"]),
            ["glGetGraphicsResetStatusARB"],
            "a 3.3 core context relies on the extension"
        );
        assert!(
            names((3, 3, false), &[]).is_empty(),
            "no extension, no function"
        );
    }

    #[test]
    fn desktop_khr_robustness_uses_the_plain_name() {
        assert_eq!(
            names((3, 3, false), &["GL_KHR_robustness"]),
            ["glGetGraphicsResetStatus"],
            "KHR on desktop is unsuffixed"
        );
    }

    #[test]
    fn es_contexts_use_the_khr_or_ext_names_until_es_32() {
        assert_eq!(
            names((3, 0, true), &["GL_EXT_robustness"]),
            ["glGetGraphicsResetStatusEXT"],
            "ANGLE exposes the EXT form"
        );
        assert_eq!(
            names((3, 0, true), &["GL_KHR_robustness", "GL_EXT_robustness"]),
            ["glGetGraphicsResetStatusKHR", "glGetGraphicsResetStatusEXT"],
            "KHR before EXT"
        );
        assert_eq!(
            names((3, 2, true), &[]),
            ["glGetGraphicsResetStatus"],
            "core in ES 3.2"
        );
    }
}
