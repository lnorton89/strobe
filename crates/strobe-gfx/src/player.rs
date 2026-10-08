// ============================================================================
// Module:       strobe_gfx::player
// Description:  Runs one scene: compiles its pass programs, owns its render
//               targets, and renders every pass each frame.
//
// Dependencies: strobe-scene (definition, assembly, controls); crate::gl,
//               crate::caps; image (scene image files)
// ============================================================================

//! The scene player.
//!
//! ## What a frame does
//!
//! For each pass in order: bind its target, set its uniforms, bind its
//! samplers, draw one triangle. The last pass writes the frame that
//! reaches the screen, and becomes `syn_FinalPass` for the next frame.
//!
//! ## Which frame a pass target reads
//!
//! Pass `i` sampling target `j`:
//!
//! - `j < i` reads what `j` rendered *this* frame;
//! - `j >= i` reads what `j` rendered *last* frame — including `j == i`,
//!   which is only artefact-free when that pass is `DOUBLE_BUFFER`, as the
//!   SSF documentation says.
//!
//! A double-buffered pass keeps two textures and alternates which one it
//! writes; everything else keeps one. The final pass is always double
//! buffered, because every scene can read `syn_FinalPass` while drawing it.
//!
//! ## Per-frame cost
//!
//! Every uniform location and sampler unit is resolved when the program is
//! linked; a frame sets values through those cached locations and binds
//! only samplers the program actually uses. Nothing allocates per frame.

use crate::caps::{Caps, FloatPrecision};
use crate::gl::{Format, Framebuffer, Gl, Program, Sampling, Stage, Texture, VertexArray};
use std::path::Path;
use std::rc::Rc;
use strobe_scene::SceneFolder;
use strobe_scene::assemble::{self, ErrorSite, PassIndex, Request};
use strobe_scene::audio::SCALARS;
use strobe_scene::control::{ControlBank, UniformValue};
use strobe_scene::scene::{Filter, SceneDef, Wrap};

/// Why a scene could not be prepared.
#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    /// A pass program failed to compile or link.
    #[error("pass {pass}: {message}")]
    Program {
        /// The pass index.
        pass: usize,
        /// The driver's log, with user-code lines mapped to `main.glsl`.
        message: String,
    },
    /// GL refused to allocate a resource.
    #[error("GL resource: {0}")]
    Resource(String),
}

/// How programs are built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// One specialised program per pass (`true`), or one shared program
    /// branching on a `PASSINDEX` uniform (`false`).
    pub specialise_passes: bool,
    /// Storage for `FLOAT` passes.
    pub float_precision: FloatPrecision,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            specialise_passes: true,
            float_precision: FloatPrecision::Full,
        }
    }
}

/// Mouse state, in SSF's uniform shapes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mouse {
    /// `_mouse`: Shadertoy `iMouse` (x, y, click x, click y) in pixels.
    pub mouse: [f32; 4],
    /// `_click`: left, right, middle held.
    pub click: [f32; 3],
    /// `_muv`: position, 0..1.
    pub muv: [f32; 2],
    /// `_muvc`: position, aspect-corrected and centred.
    pub muvc: [f32; 2],
}

/// Everything that varies per frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameInputs<'a> {
    /// `TIME`, seconds.
    pub time: f32,
    /// `FRAMECOUNT`.
    pub frame: u64,
    /// Mouse uniforms.
    pub mouse: Mouse,
    /// Audio scalars, in [`SCALARS`] order.
    pub audio: &'a [f32],
    /// The scene's controls.
    pub controls: &'a ControlBank,
    /// Transition uniform values, in the scene's declaration order.
    pub transitions: &'a [f32],
}

/// A texture and the framebuffer that renders into it.
#[derive(Debug)]
struct Target {
    texture: Texture,
    fbo: Framebuffer,
}

/// Where a sampler's texture comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    FinalPass,
    Media,
    Spectrum,
    LevelTrail,
    Pass(usize),
    Image(usize),
    /// Cube maps are not loaded yet; the unit is reserved so a samplerCube
    /// never shares a unit with a sampler2D, which GL rejects at draw time.
    Cubemap,
}

type Loc = Option<glow::UniformLocation>;

/// A linked program with every location it needs resolved.
#[derive(Debug)]
struct Compiled {
    program: Program,
    rendersize: Loc,
    time: Loc,
    framecount: Loc,
    passindex: Loc,
    mouse: [Loc; 4],
    media_type: Loc,
    audio: Vec<Loc>,
    controls: Vec<Loc>,
    transitions: Vec<Loc>,
    /// Active samplers and their units.
    samplers: Vec<(Source, u32)>,
}

/// One pass's runtime state.
#[derive(Debug)]
struct PassState {
    program: Rc<Compiled>,
    /// One target, or two when double-buffered.
    targets: Vec<Target>,
    /// Index into `targets` holding the most recent output.
    current: usize,
    size: [i32; 2],
    mipmap: bool,
}

/// A scene, compiled and ready to render.
#[derive(Debug)]
pub struct ScenePlayer {
    gl: Gl,
    caps: Caps,
    options: Options,
    def: SceneDef,
    passes: Vec<PassState>,
    finals: [Target; 2],
    final_current: usize,
    render_size: [i32; 2],
    vao: VertexArray,
    black: Texture,
    spectrum: Texture,
    level_trail: Texture,
    images: Vec<Texture>,
}

impl ScenePlayer {
    /// Compiles `scene` and allocates its targets at `render_size`.
    pub fn new(
        gl: &Gl,
        caps: &Caps,
        scene: &SceneFolder,
        render_size: [i32; 2],
        options: Options,
    ) -> Result<Self, PlayerError> {
        let res = |e: String| PlayerError::Resource(e);
        let def = scene.def.clone();
        let render_size = [render_size[0].max(1), render_size[1].max(1)];

        let vertex_src = assemble::vertex(caps.dialect);
        let vertex =
            gl.compile(Stage::Vertex, &vertex_src)
                .map_err(|log| PlayerError::Program {
                    pass: 0,
                    message: format!("vertex: {log}"),
                })?;

        let mut shared: Option<Rc<Compiled>> = None;
        let mut programs = Vec::with_capacity(def.passes.len());
        for index in 0..def.passes.len() {
            let program = match (&shared, options.specialise_passes) {
                (Some(program), false) => Rc::clone(program),
                (None, false) | (_, true) => {
                    let pass_index = if options.specialise_passes {
                        PassIndex::Constant(index)
                    } else {
                        PassIndex::Uniform
                    };
                    let source = assemble::fragment(&Request {
                        scene: &def,
                        user_code: &scene.shader,
                        dialect: caps.dialect,
                        pass_index,
                        extra: &[],
                    });
                    let compiled = Rc::new(build_program(gl, &def, &vertex, &source, index)?);
                    if !options.specialise_passes {
                        shared = Some(Rc::clone(&compiled));
                    }
                    compiled
                }
            };
            programs.push(program);
        }

        let black = gl
            .texture_2d(
                1,
                1,
                Format::Rgba8,
                Sampling::LINEAR_CLAMP,
                Some(&[0, 0, 0, 255]),
            )
            .map_err(res)?;
        let (spectrum, level_trail) = audio_textures(gl, caps).map_err(res)?;
        let images = def
            .images
            .iter()
            .map(|image| load_image(gl, &scene.dir.join(&image.path)))
            .collect::<Result<Vec<_>, _>>()?;

        let finals = [
            new_target(gl, render_size, Format::Rgba8, Sampling::LINEAR_CLAMP).map_err(res)?,
            new_target(gl, render_size, Format::Rgba8, Sampling::LINEAR_CLAMP).map_err(res)?,
        ];
        let vao = gl.vertex_array().map_err(res)?;

        let mut player = Self {
            gl: gl.clone(),
            caps: caps.clone(),
            options,
            def,
            passes: Vec::new(),
            finals,
            final_current: 0,
            render_size,
            vao,
            black,
            spectrum,
            level_trail,
            images,
        };
        player.passes = player.allocate_passes(programs)?;
        Ok(player)
    }

    /// The scene being played.
    pub fn def(&self) -> &SceneDef {
        &self.def
    }

    /// The current render resolution.
    pub fn render_size(&self) -> [i32; 2] {
        self.render_size
    }

    /// Changes the render resolution. Pass contents restart from black.
    pub fn resize(&mut self, render_size: [i32; 2]) -> Result<(), PlayerError> {
        let render_size = [render_size[0].max(1), render_size[1].max(1)];
        if render_size == self.render_size {
            return Ok(());
        }
        let res = |e: String| PlayerError::Resource(e);
        self.render_size = render_size;
        self.finals = [
            new_target(&self.gl, render_size, Format::Rgba8, Sampling::LINEAR_CLAMP)
                .map_err(res)?,
            new_target(&self.gl, render_size, Format::Rgba8, Sampling::LINEAR_CLAMP)
                .map_err(res)?,
        ];
        let programs = self.passes.iter().map(|p| Rc::clone(&p.program)).collect();
        self.passes = self.allocate_passes(programs)?;
        Ok(())
    }

    fn allocate_passes(&self, programs: Vec<Rc<Compiled>>) -> Result<Vec<PassState>, PlayerError> {
        let last = self.def.final_pass();
        let mut passes = Vec::with_capacity(programs.len());
        for (index, (pass, program)) in self.def.passes.iter().zip(programs).enumerate() {
            if index == last {
                passes.push(PassState {
                    program,
                    targets: Vec::new(),
                    current: 0,
                    size: self.render_size,
                    mipmap: false,
                });
                continue;
            }
            let size = [
                ((self.render_size[0] as f32 * pass.scale[0]).round() as i32).max(1),
                ((self.render_size[1] as f32 * pass.scale[1]).round() as i32).max(1),
            ];
            let format = self
                .caps
                .pass_format(pass.float, self.options.float_precision);
            let sampling = sampling_for(pass.wrap, pass.filter);
            let count = if pass.double_buffer { 2 } else { 1 };
            let targets = (0..count)
                .map(|_| new_target(&self.gl, size, format, sampling))
                .collect::<Result<Vec<_>, _>>()
                .map_err(PlayerError::Resource)?;
            passes.push(PassState {
                program,
                targets,
                current: 0,
                size,
                mipmap: pass.filter == Filter::Mipmap,
            });
        }
        Ok(passes)
    }

    /// Renders every pass for one frame.
    pub fn render(&mut self, inputs: &FrameInputs<'_>) {
        let gl = self.gl.clone();
        gl.plain_state();
        gl.bind_vertex_array(&self.vao);
        let last = self.passes.len().saturating_sub(1);

        for index in 0..self.passes.len() {
            let Some(pass) = self.passes.get(index) else {
                continue;
            };
            let (write_fbo, size) = if index == last {
                let write = 1 - self.final_current;
                (&self.finals[write].fbo, self.render_size)
            } else {
                let write = if pass.targets.len() == 2 {
                    1 - pass.current
                } else {
                    0
                };
                let Some(target) = pass.targets.get(write) else {
                    continue;
                };
                (&target.fbo, pass.size)
            };
            gl.bind_framebuffer(Some(write_fbo));
            gl.viewport(size[0], size[1]);

            let program = &pass.program;
            gl.use_program(&program.program);
            self.set_uniforms(&gl, program, index, size, inputs);
            for (source, unit) in &program.samplers {
                let texture = self.sampler_texture(*source);
                gl.bind_unit(*unit, texture);
            }
            gl.draw_fullscreen_triangle();

            if index == last {
                self.final_current = 1 - self.final_current;
            } else if let Some(pass) = self.passes.get_mut(index) {
                if pass.targets.len() == 2 {
                    pass.current = 1 - pass.current;
                }
                if pass.mipmap
                    && let Some(target) = pass.targets.get(pass.current)
                {
                    gl.generate_mipmaps(&target.texture);
                }
            }
        }
        gl.bind_framebuffer(None);
    }

    fn set_uniforms(
        &self,
        gl: &Gl,
        program: &Compiled,
        index: usize,
        size: [i32; 2],
        inputs: &FrameInputs<'_>,
    ) {
        if let Some(loc) = &program.rendersize {
            gl.uniform_vec2(loc, [size[0] as f32, size[1] as f32]);
        }
        if let Some(loc) = &program.time {
            gl.uniform_f32(loc, inputs.time);
        }
        if let Some(loc) = &program.framecount {
            gl.uniform_f32(loc, inputs.frame as f32);
        }
        if let Some(loc) = &program.passindex {
            gl.uniform_i32(loc, index as i32);
        }
        let [mouse, click, muv, muvc] = &program.mouse;
        if let Some(loc) = mouse {
            gl.uniform_vec4(loc, inputs.mouse.mouse);
        }
        if let Some(loc) = click {
            gl.uniform_vec3(loc, inputs.mouse.click);
        }
        if let Some(loc) = muv {
            gl.uniform_vec2(loc, inputs.mouse.muv);
        }
        if let Some(loc) = muvc {
            gl.uniform_vec2(loc, inputs.mouse.muvc);
        }
        if let Some(loc) = &program.media_type {
            gl.uniform_f32(loc, 0.0);
        }
        for (loc, value) in program.audio.iter().zip(inputs.audio) {
            if let Some(loc) = loc {
                gl.uniform_f32(loc, *value);
            }
        }
        for (loc, (_, value)) in program.controls.iter().zip(inputs.controls.uniforms()) {
            let Some(loc) = loc else {
                continue;
            };
            match value {
                UniformValue::Float(x) => gl.uniform_f32(loc, x),
                UniformValue::Vec2(v) => gl.uniform_vec2(loc, v),
                UniformValue::Vec3(v) => gl.uniform_vec3(loc, v),
            }
        }
        for (loc, value) in program.transitions.iter().zip(inputs.transitions) {
            if let Some(loc) = loc {
                gl.uniform_f32(loc, *value);
            }
        }
    }

    fn sampler_texture(&self, source: Source) -> &Texture {
        match source {
            Source::FinalPass => &self.finals[self.final_current].texture,
            Source::Media | Source::Cubemap => &self.black,
            Source::Spectrum => &self.spectrum,
            Source::LevelTrail => &self.level_trail,
            Source::Image(i) => self.images.get(i).unwrap_or(&self.black),
            Source::Pass(j) => self
                .passes
                .get(j)
                .and_then(|pass| pass.targets.get(pass.current))
                .map_or(&self.black, |target| &target.texture),
        }
    }

    /// The finished frame's framebuffer and size.
    fn output(&self) -> &Target {
        &self.finals[self.final_current]
    }

    /// The finished frame's texture, e.g. for a UI preview.
    pub fn output_texture(&self) -> &Texture {
        &self.output().texture
    }

    /// Draws the finished frame into the default framebuffer, letterboxed
    /// to `window` pixels.
    pub fn present(&self, window: [i32; 2]) {
        let [w, h] = self.render_size;
        let scale = (window[0] as f32 / w as f32).min(window[1] as f32 / h as f32);
        let (dw, dh) = ((w as f32 * scale) as i32, (h as f32 * scale) as i32);
        let (x, y) = ((window[0] - dw) / 2, (window[1] - dh) / 2);
        self.gl.bind_framebuffer(None);
        self.gl.viewport(window[0], window[1]);
        self.gl.clear([0.0, 0.0, 0.0, 1.0]);
        self.gl
            .blit_to_default(&self.output().fbo, self.render_size, [x, y, x + dw, y + dh]);
    }

    /// Reads the finished frame back as RGBA8, top row first.
    pub fn read_output(&self) -> Vec<u8> {
        let [w, h] = self.render_size;
        let bottom_up = self.gl.read_rgba8(&self.output().fbo, w, h);
        let row = (w.max(0) as usize) * 4;
        let mut top_down = Vec::with_capacity(bottom_up.len());
        for chunk in bottom_up.chunks_exact(row.max(1)).rev() {
            top_down.extend_from_slice(chunk);
        }
        top_down
    }
}

/// Compiles and links one pass program and resolves its locations.
fn build_program(
    gl: &Gl,
    def: &SceneDef,
    vertex: &crate::gl::Shader,
    source: &str,
    pass: usize,
) -> Result<Compiled, PlayerError> {
    let fragment = gl
        .compile(Stage::Fragment, source)
        .map_err(|log| PlayerError::Program {
            pass,
            message: describe_log(&log),
        })?;
    let program = gl
        .link(vertex, &fragment)
        .map_err(|log| PlayerError::Program {
            pass,
            message: format!("link: {log}"),
        })?;
    let loc = |name: &str| gl.uniform_location(&program, name);

    gl.use_program(&program);
    let mut samplers = Vec::new();
    let mut unit = 0_u32;
    let mut assign = |name: &str, source: Source| {
        if let Some(location) = gl.uniform_location(&program, name) {
            gl.uniform_i32(&location, unit as i32);
            samplers.push((source, unit));
        }
        // Units are reserved whether or not the sampler is active, so a
        // sampler's unit never depends on what the driver optimised out.
        unit += 1;
    };
    assign("syn_FinalPass", Source::FinalPass);
    assign("syn_Media", Source::Media);
    assign("syn_Spectrum", Source::Spectrum);
    assign("syn_LevelTrail", Source::LevelTrail);
    for (j, p) in def.passes.iter().enumerate() {
        if let Some(target) = &p.target {
            assign(target, Source::Pass(j));
        }
    }
    for (i, image) in def.images.iter().enumerate() {
        assign(&image.name, Source::Image(i));
    }
    for cube in &def.cubemaps {
        assign(&cube.name, Source::Cubemap);
    }

    Ok(Compiled {
        rendersize: loc("RENDERSIZE"),
        time: loc("TIME"),
        framecount: loc("FRAMECOUNT"),
        passindex: loc("PASSINDEX"),
        mouse: [loc("_mouse"), loc("_click"), loc("_muv"), loc("_muvc")],
        media_type: loc("syn_MediaType"),
        audio: SCALARS.iter().map(|name| loc(name)).collect(),
        controls: def.controls.iter().map(|c| loc(&c.name)).collect(),
        transitions: def
            .transition_uniforms
            .iter()
            .map(|t| loc(&t.name))
            .collect(),
        samplers,
        program,
    })
}

/// A driver log with user-code locations rewritten as `main.glsl:N`.
fn describe_log(log: &str) -> String {
    log.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match assemble::map_error_line(line) {
            Some(ErrorSite::User(n)) => format!("main.glsl:{n}: {}", line.trim()),
            Some(ErrorSite::Generated) => format!("(generated): {}", line.trim()),
            None => line.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sampling state for a pass's wrap and filter settings.
fn sampling_for(wrap: Wrap, filter: Filter) -> Sampling {
    let wrap = match wrap {
        Wrap::Repeat => glow::REPEAT,
        Wrap::Clamp => glow::CLAMP_TO_EDGE,
        Wrap::Mirror => glow::MIRRORED_REPEAT,
    };
    let (min, mag) = match filter {
        Filter::Linear => (glow::LINEAR, glow::LINEAR),
        Filter::Nearest => (glow::NEAREST, glow::NEAREST),
        Filter::Mipmap => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR),
    };
    Sampling { wrap, min, mag }
}

/// A cleared render target.
fn new_target(
    gl: &Gl,
    size: [i32; 2],
    format: Format,
    sampling: Sampling,
) -> Result<Target, String> {
    let texture = gl.texture_2d(size[0], size[1], format, sampling, None)?;
    let fbo = gl.framebuffer(&texture)?;
    // Passes start at vec4(0): GL leaves new storage undefined.
    gl.bind_framebuffer(Some(&fbo));
    gl.viewport(size[0], size[1]);
    gl.clear([0.0; 4]);
    gl.bind_framebuffer(None);
    if sampling.min == glow::LINEAR_MIPMAP_LINEAR {
        gl.generate_mipmaps(&texture);
    }
    Ok(Target { texture, fbo })
}

/// Silent placeholders for `syn_Spectrum` and `syn_LevelTrail`, in the
/// shape the context's dialect declared them.
fn audio_textures(gl: &Gl, caps: &Caps) -> Result<(Texture, Texture), String> {
    const WIDTH: i32 = 256;
    let zeros = vec![0_u8; WIDTH as usize * 4];
    if caps.has_1d_textures {
        Ok((
            gl.texture_1d(WIDTH, Format::Rgba8, Some(&zeros))?,
            gl.texture_1d(WIDTH, Format::Rgba8, Some(&zeros))?,
        ))
    } else {
        Ok((
            gl.texture_2d(
                WIDTH,
                1,
                Format::Rgba8,
                Sampling::LINEAR_CLAMP,
                Some(&zeros),
            )?,
            gl.texture_2d(
                WIDTH,
                1,
                Format::Rgba8,
                Sampling::LINEAR_CLAMP,
                Some(&zeros),
            )?,
        ))
    }
}

/// Decodes an image file into a texture, bottom row first so `_uv` reads
/// it upright.
fn load_image(gl: &Gl, path: &Path) -> Result<Texture, PlayerError> {
    let decoded = image::open(path)
        .map_err(|e| PlayerError::Resource(format!("{}: {e}", path.display())))?
        .flipv()
        .into_rgba8();
    let (w, h) = decoded.dimensions();
    let sampling = Sampling {
        wrap: glow::REPEAT,
        min: glow::LINEAR_MIPMAP_LINEAR,
        mag: glow::LINEAR,
    };
    let texture = gl
        .texture_2d(
            w as i32,
            h as i32,
            Format::Rgba8,
            sampling,
            Some(decoded.as_raw()),
        )
        .map_err(PlayerError::Resource)?;
    gl.generate_mipmaps(&texture);
    Ok(texture)
}
