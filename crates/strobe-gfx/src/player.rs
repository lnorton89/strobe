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

use crate::assets::{self, AssetProblem, MediaRequest};
use crate::caps::{Caps, FloatPrecision};
use crate::gl::{Format, Framebuffer, Gl, Program, Sampling, Stage, Texture, VertexArray};
use crate::media::Media;
use crate::readback::FinalPassReader;
use std::path::Path;
use std::rc::Rc;
use strobe_scene::SceneFolder;
use strobe_scene::assemble::{self, ErrorSite, ExtraType, PassIndex, Request};
use strobe_scene::audio::SCALARS;
use strobe_scene::control::{ControlBank, UniformValue};
use strobe_scene::scene::{Filter, ImageDef, PassDef, SceneDef, Wrap};

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
    /// branching on a `PASSINDEX` uniform (`false`). A scene with
    /// `PASS_SOURCES` has a different source per pass, so it is always
    /// built per pass whatever this says (a warning is logged).
    pub specialise_passes: bool,
    /// Storage for `FLOAT` passes.
    pub float_precision: FloatPrecision,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            specialise_passes: true,
            float_precision: FloatPrecision::default(),
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

/// The audio data behind `syn_Spectrum` and `syn_LevelTrail`, as plain
/// slices so this crate needs nothing from the audio crate.
///
/// One `[r, g, b, a]` per texel, every channel `0..1` (clamped on upload).
/// What each channel means is the audio crate's contract; the renderer only
/// moves the texels. A slice whose length differs from the texture's
/// reallocates that texture once, so layouts other than 256 work.
#[derive(Debug, Clone, Copy)]
pub struct AudioTextures<'a> {
    /// `syn_Spectrum`.
    pub spectrum: &'a [[f32; 4]],
    /// `syn_LevelTrail`.
    pub level_trail: &'a [[f32; 4]],
    /// The browser-matched Shadertoy audio texture (`strobe_ShadertoyAudio`):
    /// 512x2 bytes of R8, the spectrum row then the waveform row, exactly
    /// what the audio crate's `AudioFrame::shadertoy` holds. When it is
    /// `Some` with exactly that length it is uploaded as given; otherwise
    /// (`None`, or any other length) the texture is resampled from
    /// `spectrum` as before.
    pub shadertoy: Option<&'a [u8]>,
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
    /// The Shadertoy audio texture, `strobe_ShadertoyAudio`.
    ShadertoyAudio,
    Pass(usize),
    Image(usize),
    /// A declared cube map, by its position in `SceneDef::cubemaps`. Its
    /// unit is reserved like every other, so a samplerCube never shares a
    /// unit with a sampler2D, which GL rejects at draw time.
    Cubemap(usize),
}

type Loc = Option<glow::UniformLocation>;

/// A linked program with every location it needs resolved.
#[derive(Debug)]
struct Compiled {
    program: Program,
    rendersize: Loc,
    time: Loc,
    framecount: Loc,
    date: Loc,
    passindex: Loc,
    mouse: [Loc; 4],
    media_type: Loc,
    audio: Vec<Loc>,
    controls: Vec<Loc>,
    transitions: Vec<Loc>,
    /// Script-created uniforms, in the order the player was built with.
    extras: Vec<(Loc, ExtraType)>,
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
    /// Set by [`ScenePlayer::allocate_passes`] on a pass whose size a resize
    /// left unchanged: it has no targets of its own yet and takes the old
    /// pass's, contents and current buffer included, in
    /// [`adopt_unchanged`] (D287).
    awaiting_adoption: bool,
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
    /// The size the final pass renders at: `requested`, unless the scene
    /// pins its final pass (D280).
    render_size: [i32; 2],
    /// The size the host last asked for.
    requested: [i32; 2],
    /// The size the final pass is pinned to, decided once from the scene's
    /// passes and shader ([`strobe_scene::pin`], D280).
    pinned: Option<[i32; 2]>,
    vao: VertexArray,
    black: Texture,
    spectrum: Texture,
    level_trail: Texture,
    shadertoy_audio: Texture,
    images: Vec<Texture>,
    /// One slot per declared cube map; `None` where it failed to load, and
    /// the sampler then reads `black_cube`.
    cubemaps: Vec<Option<Texture>>,
    black_cube: Texture,
    /// The picture behind `syn_Media` (D186).
    media: Media,
    /// A replacement still decoding; it takes over when its first picture
    /// lands, so changing media never flashes black.
    incoming: Option<Media>,
    /// The scene's own `MEDIA`, resolved, for [`Self::choose_media`] to go
    /// back to.
    scene_media: Option<MediaRequest>,
    /// Whether any pass reads `syn_Media` or `syn_MediaType`.
    reads_media: bool,
    /// Assets that did not load, waiting to be reported ([`Self::take_asset_problems`]).
    problems: Vec<AssetProblem>,
    audio: AudioUpload,
    /// What `strobe_Date` holds, when the host has set it; otherwise the
    /// player reads the wall clock (UTC) each frame.
    date: Option<[f32; 4]>,
    /// Uniforms a script creates, declared in every pass program.
    extras: Vec<(String, ExtraType)>,
    /// This frame's values for `extras`, in the same order.
    extra_values: Vec<[f32; 4]>,
    /// The asynchronous 64x36 readback behind `textureFinalPass`, when a
    /// host asked for it ([`ScenePlayer::enable_final_pass_readback`]).
    readback: Option<FinalPassReader>,
}

/// What the audio texture upload path keeps between frames.
#[derive(Debug, Default)]
struct AudioUpload {
    /// Whether any pass samples `syn_Spectrum`, `syn_LevelTrail` or
    /// `strobe_ShadertoyAudio`; a texture nothing reads is never uploaded.
    wanted: [bool; 3],
    /// 8-bit staging for each texture, sized once.
    staging: [Vec<u8>; 3],
    /// Set after a failed upload so the log gets one line, not one a frame.
    failed: bool,
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
        Self::with_extras(gl, caps, scene, render_size, options, &[])
    }

    /// [`Self::new`] for a scene whose script creates uniforms.
    ///
    /// `extras` are the script's `(name, type)` pairs. They are declared in
    /// every pass program and their locations resolved at link time, like
    /// every other uniform, so setting them each frame
    /// ([`Self::set_script_uniforms`]) costs no lookups. All of this happens
    /// here, at load: a show never compiles a shader while it plays. The
    /// host seals the script's uniforms right after, so a uniform created
    /// later is refused rather than answered with a rebuild (D161).
    pub fn with_extras(
        gl: &Gl,
        caps: &Caps,
        scene: &SceneFolder,
        render_size: [i32; 2],
        options: Options,
        extras: &[(String, ExtraType)],
    ) -> Result<Self, PlayerError> {
        let res = |e: String| PlayerError::Resource(e);
        let def = scene.def.clone();
        let requested = [render_size[0].max(1), render_size[1].max(1)];
        let pinned = strobe_scene::pin::pinned_size(scene);
        let render_size = final_size(pinned, requested);

        let vertex_src = assemble::vertex(caps.dialect);
        let vertex =
            gl.compile(Stage::Vertex, &vertex_src)
                .map_err(|log| PlayerError::Program {
                    pass: 0,
                    message: format!("vertex: {log}"),
                })?;

        // Different source per pass means different programs: sharing one
        // is not an option, so `PASS_SOURCES` overrides the benchmark's
        // shared mode rather than failing a whole library run.
        let per_pass_sources = def.pass_sources.is_some();
        if per_pass_sources && !options.specialise_passes {
            log::warn!(
                "{}: PASS_SOURCES gives each pass its own source, so its programs are built \
                 per pass even though a shared program was asked for",
                def.title
            );
        }
        let specialise = options.specialise_passes || per_pass_sources;
        let options = Options {
            specialise_passes: specialise,
            ..options
        };
        let mut shared: Option<Rc<Compiled>> = None;
        let mut programs = Vec::with_capacity(def.passes.len());
        for index in 0..def.passes.len() {
            let program = match (&shared, specialise) {
                (Some(program), false) => Rc::clone(program),
                (None, false) | (_, true) => {
                    let pass_index = if specialise {
                        PassIndex::Constant(index)
                    } else {
                        PassIndex::Uniform
                    };
                    let source = assemble::fragment(&Request {
                        scene: &def,
                        user_code: scene.shader_for(index),
                        dialect: caps.dialect,
                        pass_index,
                        extra: extras,
                    });
                    dump_fragment(&def.title, caps.dialect, index, &source);
                    let compiled = Rc::new(build_program(
                        gl,
                        &def,
                        extras,
                        &vertex,
                        Fragment {
                            source: &source,
                            user_code: scene.shader_for(index),
                            file: scene.file_for(index),
                        },
                        index,
                    )?);
                    if !specialise {
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
        let spectrum = audio_texture(gl, caps, SILENT_SPECTRUM_WIDTH).map_err(res)?;
        let level_trail = audio_texture(gl, caps, SILENT_TRAIL_WIDTH).map_err(res)?;
        let samples = |wanted: Source| {
            programs
                .iter()
                .any(|program| program.samplers.iter().any(|(source, _)| *source == wanted))
        };
        let reads_media =
            samples(Source::Media) || programs.iter().any(|program| program.media_type.is_some());
        let audio = AudioUpload {
            wanted: [
                samples(Source::Spectrum),
                samples(Source::LevelTrail),
                samples(Source::ShadertoyAudio),
            ],
            ..AudioUpload::default()
        };
        // The Shadertoy audio texture is only worth its 1 KB when a shader
        // reads it. Until the first upload it holds silence: a dark
        // spectrum row and a waveform row at its centre.
        let shadertoy_audio = shadertoy_audio_texture(gl, audio.wanted[2]).map_err(res)?;
        // A picture or cube map that will not load leaves its sampler black
        // and the scene running; the problem is handed to the host to
        // report. Real scenes name files that are not there (TORNADO T6: a
        // renamed PNG), and Synesthesia plays them.
        let mut problems = Vec::new();
        let images = def
            .images
            .iter()
            .map(
                |image| match load_image(gl, &scene.dir.join(&image.path), image) {
                    Ok(texture) => Ok(texture),
                    Err(error) => {
                        problems.push(AssetProblem {
                            asset: image.name.clone(),
                            path: image.path.clone(),
                            message: error.to_string(),
                        });
                        gl.texture_2d(
                            1,
                            1,
                            Format::Rgba8,
                            Sampling::LINEAR_CLAMP,
                            Some(&[0, 0, 0, 255]),
                        )
                        .map_err(res)
                    }
                },
            )
            .collect::<Result<Vec<_>, _>>()?;

        let black_face = [0_u8, 0, 0, 255];
        let black_cube = gl.texture_cube(1, [&black_face; 6], false).map_err(res)?;
        if !def.cubemaps.is_empty() && !caps.dialect.is_es() {
            gl.enable_seamless_cube_maps();
        }
        let cubemaps = def
            .cubemaps
            .iter()
            .map(|cube| {
                let loaded = assets::load_cubemap(&scene.dir, cube).and_then(|cut| {
                    let [a, b, c, d, e, f] = &cut.faces;
                    gl.texture_cube(cut.size as i32, [a, b, c, d, e, f].map(Vec::as_slice), true)
                });
                match loaded {
                    Ok(texture) => Some(texture),
                    Err(message) => {
                        problems.push(AssetProblem {
                            asset: cube.name.clone(),
                            path: cube.paths.join(", "),
                            message,
                        });
                        None
                    }
                }
            })
            .collect::<Vec<_>>();
        let scene_media = def
            .media
            .as_ref()
            .map(|media_def| MediaRequest::from_scene(&scene.dir, media_def));
        let media = match (&def.media, &scene_media) {
            (Some(media_def), Some(request)) => {
                Media::start(request.clone()).unwrap_or_else(|message| {
                    problems.push(AssetProblem {
                        asset: "syn_Media".to_string(),
                        path: media_def.path.clone(),
                        message,
                    });
                    Media::none()
                })
            }
            (None, _) | (_, None) => Media::none(),
        };

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
            requested,
            pinned,
            vao,
            black,
            spectrum,
            level_trail,
            shadertoy_audio,
            images,
            cubemaps,
            black_cube,
            media,
            incoming: None,
            scene_media,
            reads_media,
            problems,
            audio,
            date: None,
            extras: extras.to_vec(),
            extra_values: Vec::with_capacity(extras.len()),
            readback: None,
        };
        player.passes = player.allocate_passes(programs, render_size, &[], &[])?;
        Ok(player)
    }

    /// The scene being played.
    pub fn def(&self) -> &SceneDef {
        &self.def
    }

    /// Whether buffer pass `pass` has two targets: because the scene asked
    /// for `DOUBLE_BUFFER`, or because its program samples its own output
    /// (D203). `false` for the final pass and an index out of range.
    pub fn is_double_buffered(&self, pass: usize) -> bool {
        self.passes
            .get(pass)
            .is_some_and(|state| state.targets.len() == 2)
    }

    /// Hands over the assets that did not load since this was last called
    /// (a cube map, or the `syn_Media` picture), so the host can report
    /// them through its failure path. The scene keeps running without them:
    /// a failed cube map reads black. Allocates nothing when there are none.
    pub fn take_asset_problems(&mut self) -> Vec<AssetProblem> {
        std::mem::take(&mut self.problems)
    }

    /// Shows `request` (a picture, or a folder of pictures) behind
    /// `syn_Media` in place of the scene's own. It decodes on a thread of
    /// its own; the old picture stays up until the first new one has landed,
    /// unless there is none yet (a scene just loaded), when the old one is
    /// dropped at once rather than flashed for a frame or two (T480 Task
    /// 26). A request that cannot start is recorded as an asset problem.
    pub fn set_media(&mut self, request: MediaRequest) {
        let path = request.path.display().to_string();
        log::info!("syn_Media: {path}");
        match Media::start(request) {
            Ok(media) if !self.media.has_picture() => {
                self.media = media;
                self.incoming = None;
            }
            Ok(media) => self.incoming = Some(media),
            Err(message) => self.problems.push(AssetProblem {
                asset: "syn_Media".to_string(),
                path,
                message,
            }),
        }
    }

    /// Shows `choice` behind `syn_Media`, or the scene's own media when it
    /// is `None` (no media at all if the scene has none). Asking for what is
    /// already showing or on its way does nothing, so a host can call this
    /// at every cue without restarting a decode (D217).
    pub fn choose_media(&mut self, choice: Option<MediaRequest>) {
        // The operator's media is sampled as the scene's `MEDIA` slot says.
        let (wrap, filter) = self.def.media_sampling;
        let wanted = match choice {
            Some(request) => Some(MediaRequest {
                wrap,
                filter,
                ..request
            }),
            None => self.scene_media.clone(),
        };
        if wanted.as_ref() == self.media_request() {
            return;
        }
        match wanted {
            Some(request) => self.set_media(request),
            None => {
                log::info!("syn_Media: none");
                self.incoming = None;
                self.media = Media::none();
            }
        }
    }

    /// What `syn_Media` shows, or is about to: the replacement still
    /// decoding if there is one, else the current picture's source. `None`
    /// when the scene shows no media.
    pub fn media_request(&self) -> Option<&MediaRequest> {
        match &self.incoming {
            Some(incoming) => incoming.request(),
            None => self.media.request(),
        }
    }

    /// Whether the scene's shaders read `syn_Media` (or `syn_MediaType`) at
    /// all, so a host knows whether choosing media would show anywhere.
    pub fn reads_media(&self) -> bool {
        self.reads_media
    }

    /// Whether `syn_Media` shows the scene's own `MEDIA` (or nothing, for a
    /// scene without one), as opposed to a source chosen in its place.
    pub fn media_is_scenes_own(&self) -> bool {
        self.media_request() == self.scene_media.as_ref()
    }

    /// Waits for every picture still decoding to arrive and uploads it. For
    /// tests and the benchmark, whose frames must not depend on disk speed;
    /// a live host never calls it.
    pub fn finish_media_loading(&mut self) {
        let gl = self.gl.clone();
        if let Some(incoming) = self.incoming.as_mut() {
            incoming.finish(&gl);
        }
        self.promote_incoming();
        self.media.finish(&gl);
        self.collect_media_failure();
    }

    /// Whether `syn_Media` has a picture to show.
    pub fn media_ready(&self) -> bool {
        self.media.has_picture()
    }

    /// Whether any media is still decoding.
    pub fn media_loading(&self) -> bool {
        self.media.loading() || self.incoming.is_some()
    }

    /// Takes at most one arrived picture for each media being loaded.
    fn poll_media(&mut self) {
        let gl = self.gl.clone();
        if self.incoming.is_some() {
            if let Some(incoming) = self.incoming.as_mut() {
                incoming.poll(&gl);
            }
            self.promote_incoming();
        }
        self.media.poll(&gl);
        self.collect_media_failure();
    }

    /// Swaps in the replacement media once it has something to show, or
    /// drops it (with its failure) if it ended with nothing.
    fn promote_incoming(&mut self) {
        let ready = self
            .incoming
            .as_ref()
            .is_some_and(|incoming| incoming.has_picture() || !incoming.loading());
        if ready && let Some(mut incoming) = self.incoming.take() {
            if let Some(message) = incoming.take_failure() {
                self.problems.push(AssetProblem {
                    asset: "syn_Media".to_string(),
                    path: incoming.label(),
                    message,
                });
            }
            if incoming.has_picture() {
                self.media = incoming;
            }
        }
    }

    fn collect_media_failure(&mut self) {
        if let Some(message) = self.media.take_failure() {
            self.problems.push(AssetProblem {
                asset: "syn_Media".to_string(),
                path: self.media.label(),
                message,
            });
        }
    }

    /// The script-created uniforms this player was built with.
    pub fn extras(&self) -> &[(String, ExtraType)] {
        &self.extras
    }

    /// Sets this frame's values for the script-created uniforms, in the
    /// order of [`Self::extras`]; a value past the list is ignored and a
    /// missing one stays at zero. Copies into a buffer reused each frame.
    pub fn set_script_uniforms(&mut self, values: &[[f32; 4]]) {
        self.extra_values.clear();
        self.extra_values.extend_from_slice(values);
    }

    /// Sets `strobe_Date` (Shadertoy's `iDate`): year, month 0 to 11, day
    /// 1 to 31, seconds since midnight. Until this is called the player
    /// reads the wall clock in UTC each frame, which is right for a shader
    /// that only needs *a* date and wrong by the time-zone offset for one
    /// that shows the time. A host that knows the local offset should call
    /// this every frame; tests call it to render deterministic frames.
    pub fn set_date(&mut self, date: [f32; 4]) {
        self.date = Some(date);
    }

    /// The resolution the final pass renders at, which is what
    /// [`Self::read_output`] returns and [`Self::present`] scales. It is the
    /// size last asked for, unless the scene pins its final pass (D280).
    pub fn render_size(&self) -> [i32; 2] {
        self.render_size
    }

    /// The render size the host last asked for ([`Self::new`],
    /// [`Self::resize`] or [`Self::resize_keeping`]). A host deciding
    /// whether to resize compares against this, not [`Self::render_size`],
    /// which a pinned scene keeps whatever it is asked.
    pub fn requested_size(&self) -> [i32; 2] {
        self.requested
    }

    /// The size the scene pins its final pass to, if it does (D280). The
    /// render scale then changes nothing this scene draws.
    pub fn pinned_size(&self) -> Option<[i32; 2]> {
        self.pinned
    }

    /// Changes the render resolution. Pass contents restart from black,
    /// except for passes whose own size the resize does not change (a
    /// `FIXED_SIZE` or literal-size pass, D194), which keep their targets and
    /// what is in them (D287) unless a pass that does restart reads them: a
    /// Shadertoy import's buffers read the `st_clock` pass for `iFrame`, and
    /// buffers wiped to black need `iFrame` back at 0 to initialise again
    /// (D91). A scene whose final pass is pinned (D280) draws at the same
    /// size whatever it is asked, so for it this only records the request.
    /// On failure the player is unchanged.
    pub fn resize(&mut self, render_size: [i32; 2]) -> Result<(), PlayerError> {
        let requested = [render_size[0].max(1), render_size[1].max(1)];
        if requested == self.requested {
            return Ok(());
        }
        let render_size = final_size(self.pinned, requested);
        if render_size == self.render_size {
            self.requested = requested;
            return Ok(());
        }
        let finals = self.new_finals(render_size)?;
        let programs = self.passes.iter().map(|p| Rc::clone(&p.program)).collect();
        let restart = self.passes_to_restart(render_size);
        let mut passes = self.allocate_passes(programs, render_size, &self.passes, &restart)?;
        let readback = self.new_readback(render_size)?;
        // Nothing below can fail, so a failed allocation left the player as
        // it was. A pass whose size did not change keeps its targets (D287).
        adopt_unchanged(&mut self.passes, &mut passes);
        self.requested = requested;
        self.render_size = render_size;
        self.finals = finals;
        self.passes = passes;
        self.readback = readback;
        Ok(())
    }

    /// Restarts every pass from black at the current size, `FIXED_SIZE`
    /// and literal-size passes included, as a freshly built player would
    /// start. For a caller that renders the same scene several times from
    /// scratch (`strobe-autocontrol`'s probes), since a resize no longer
    /// clears a pass whose size it does not change (D287). On failure the
    /// player is unchanged.
    pub fn restart(&mut self) -> Result<(), PlayerError> {
        let size = self.render_size;
        let finals = self.new_finals(size)?;
        let programs = self.passes.iter().map(|p| Rc::clone(&p.program)).collect();
        let passes = self.allocate_passes(programs, size, &[], &[])?;
        let readback = self.new_readback(size)?;
        self.finals = finals;
        self.passes = passes;
        self.readback = readback;
        Ok(())
    }

    /// Which passes a [`Self::resize`] to `render_size` restarts from black,
    /// by index: every pass whose size changes, the final pass, and every
    /// pass one of those (the final pass aside) reads, transitively. A pass
    /// whose state feeds a restarted pass restarts with it, so the two stay
    /// consistent (D287): the `st_clock` behind a wiped Shadertoy buffer's
    /// `iFrame` is the case that matters (D91). Runs once per resize.
    fn passes_to_restart(&self, render_size: [i32; 2]) -> Vec<bool> {
        let last = self.def.final_pass();
        let mut restart: Vec<bool> = self
            .def
            .passes
            .iter()
            .zip(&self.passes)
            .enumerate()
            .map(|(index, (pass, state))| {
                index == last || pass_size(pass, render_size) != state.size
            })
            .collect();
        loop {
            let mut changed = false;
            for (index, state) in self.passes.iter().enumerate() {
                if index == last || !restart.get(index).copied().unwrap_or(false) {
                    continue;
                }
                for (source, _) in &state.program.samplers {
                    if let Source::Pass(read) = *source
                        && let Some(flag) = restart.get_mut(read)
                        && !*flag
                    {
                        *flag = true;
                        changed = true;
                    }
                }
            }
            if !changed {
                return restart;
            }
        }
    }

    /// The two finished-frame targets at `size`.
    fn new_finals(&self, size: [i32; 2]) -> Result<[Target; 2], PlayerError> {
        let res = |e: String| PlayerError::Resource(e);
        Ok([
            new_target(&self.gl, size, Format::Rgba8, Sampling::LINEAR_CLAMP).map_err(res)?,
            new_target(&self.gl, size, Format::Rgba8, Sampling::LINEAR_CLAMP).map_err(res)?,
        ])
    }

    /// A fresh final-pass readback at `size` when one is running, so a
    /// resize can build it before it changes anything.
    fn new_readback(&self, size: [i32; 2]) -> Result<Option<FinalPassReader>, PlayerError> {
        match self.readback {
            Some(_) => FinalPassReader::new(&self.gl, size)
                .map(Some)
                .map_err(PlayerError::Resource),
            None => Ok(None),
        }
    }

    /// Changes the render resolution **keeping what the scene has drawn**:
    /// every pass target and the finished frame are scaled into the new
    /// targets (a GPU blit, filtered, except for 32-bit float passes, which
    /// GL only copies nearest) instead of restarting from black.
    ///
    /// This is what lets a feedback scene warmed up at a small size arrive
    /// at full size with its trails in place (D153). A pass whose own size
    /// does not change (`FIXED_SIZE`, a literal size) is not touched at all:
    /// same textures, same contents, same current buffer (D287). Nothing is
    /// compiled; only textures are allocated. On failure the player is
    /// unchanged.
    pub fn resize_keeping(&mut self, render_size: [i32; 2]) -> Result<(), PlayerError> {
        let requested = [render_size[0].max(1), render_size[1].max(1)];
        let render_size = final_size(self.pinned, requested);
        if render_size == self.render_size {
            // A pinned scene (D280) draws the same whatever it is asked.
            self.requested = requested;
            return Ok(());
        }
        let old_size = self.render_size;
        let finals = self.new_finals(render_size)?;
        let programs = self.passes.iter().map(|p| Rc::clone(&p.program)).collect();
        // Allocated against the new size without touching `self` yet.
        // Nothing restarts here (what changes size is blitted), so every
        // pass whose size is unchanged keeps its targets (D287).
        let mut passes = self.allocate_passes(programs, render_size, &self.passes, &[])?;
        let readback = self.new_readback(render_size)?;
        for (old, new) in self.finals.iter().zip(&finals) {
            self.gl
                .blit_between_filtered(&old.fbo, old_size, &new.fbo, render_size, true);
        }
        for (old, new) in self.passes.iter_mut().zip(passes.iter_mut()) {
            if new.awaiting_adoption {
                adopt_one(old, new);
                continue;
            }
            new.current = old.current;
            for (from, to) in old.targets.iter().zip(&new.targets) {
                let linear = from.texture.format() != Format::Rgba32f;
                self.gl
                    .blit_between_filtered(&from.fbo, old.size, &to.fbo, new.size, linear);
            }
            if new.mipmap
                && let Some(target) = new.targets.get(new.current)
            {
                self.gl.generate_mipmaps(&target.texture);
            }
        }
        self.render_size = render_size;
        self.requested = requested;
        self.finals = finals;
        self.passes = passes;
        self.readback = readback;
        Ok(())
    }

    /// Starts the asynchronous readback behind `textureFinalPass`: from the
    /// next [`Self::render`] on, each frame's final pass is downsampled to
    /// 64x36 and read into a ring of pixel-pack buffers (see
    /// [`crate::readback`]). Allocates at once and costs a few small blits
    /// and one fence per frame; leave it off for scenes whose script never
    /// calls `textureFinalPass`. Safe to call twice.
    pub fn enable_final_pass_readback(&mut self) -> Result<(), PlayerError> {
        if self.readback.is_none() {
            self.readback = Some(
                FinalPassReader::new(&self.gl, self.render_size).map_err(PlayerError::Resource)?,
            );
        }
        Ok(())
    }

    /// The newest final-pass downsample that has arrived: 64x36 RGB8, rows
    /// from the bottom up, from two frames before the frame last rendered.
    /// `None` when the readback is off or its first read has not landed.
    pub fn final_pass_pixels(&self) -> Option<&[u8]> {
        self.readback.as_ref().and_then(FinalPassReader::pixels)
    }

    /// Reads abandoned because the GPU was slow, or 0 with the readback off.
    pub fn final_pass_dropped(&self) -> u64 {
        self.readback.as_ref().map_or(0, FinalPassReader::dropped)
    }

    /// Uploads this frame's audio texels to `syn_Spectrum` and
    /// `syn_LevelTrail`, for whichever of them a pass actually samples.
    ///
    /// Call once per frame before [`Self::render`]. After the first call at
    /// a given width it allocates nothing: texels are quantised to 8 bits
    /// into staging buffers kept between frames, then sent with the same
    /// upload wrapper as every other texture (a 1D texture on desktop GL, a
    /// one-texel-high 2D texture on GLES, as the shader declares them).
    pub fn upload_audio(&mut self, audio: &AudioTextures<'_>) {
        let result = self
            .upload_one(0, audio.spectrum)
            .and_then(|()| self.upload_one(1, audio.level_trail))
            .and_then(|()| self.upload_shadertoy(audio.spectrum, audio.shadertoy));
        if let Err(error) = result
            && !self.audio.failed
        {
            self.audio.failed = true;
            log::warn!("audio texture upload failed (reported once): {error}");
        }
    }

    fn upload_one(&mut self, which: usize, texels: &[[f32; 4]]) -> Result<(), String> {
        if !self.audio.wanted.get(which).copied().unwrap_or(false) || texels.is_empty() {
            return Ok(());
        }
        let width = i32::try_from(texels.len()).map_err(|e| e.to_string())?;
        let texture = if which == 0 {
            &mut self.spectrum
        } else {
            &mut self.level_trail
        };
        if texture.width() != width {
            *texture = audio_texture(&self.gl, &self.caps, width)?;
        }
        let Some(staging) = self.audio.staging.get_mut(which) else {
            return Ok(());
        };
        staging.resize(texels.len() * 4, 0);
        let (packed, _) = staging.as_chunks_mut::<4>();
        for (bytes, texel) in packed.iter_mut().zip(texels) {
            for (byte, value) in bytes.iter_mut().zip(texel) {
                *byte = quantise(*value);
            }
        }
        self.gl.upload(texture, staging)
    }

    /// Uploads the 512x2 Shadertoy audio texture, when a pass samples it.
    ///
    /// Row 0 is the spectrum and row 1 the waveform, as on Shadertoy. When
    /// the host supplies the browser-matched bytes (`exact`, from the audio
    /// crate's `AudioFrame::shadertoy`, already on the browser analyser's
    /// decibel window and temporally smoothed) they are uploaded as they
    /// are, with no per-frame work beyond the copy GL makes (D93, D112).
    ///
    /// Without them both rows are resampled (linearly) from the spectrum
    /// texture's `r` (raw FFT) and `a` (waveform) channels, which the audio
    /// crate documents in `AudioFrame::spectrum`. The raw channel spans
    /// -100 dB to -20 dB where the browser's analyser, and so Shadertoy's
    /// texture, spans -100 dB to -30 dB with a 6 dB lower scale;
    /// [`shadertoy_level`] converts. That path has none of the analyser's
    /// smoothing, which is why a host with the exact bytes should pass them.
    fn upload_shadertoy(
        &mut self,
        spectrum: &[[f32; 4]],
        exact: Option<&[u8]>,
    ) -> Result<(), String> {
        if !self.audio.wanted.get(2).copied().unwrap_or(false) {
            return Ok(());
        }
        if let Some(bytes) = exact.filter(|bytes| bytes.len() == SHADERTOY_AUDIO_WIDTH * 2) {
            return self.gl.upload(&self.shadertoy_audio, bytes);
        }
        if spectrum.is_empty() {
            return Ok(());
        }
        let Some(staging) = self.audio.staging.get_mut(2) else {
            return Ok(());
        };
        staging.resize(SHADERTOY_AUDIO_WIDTH * 2, 0);
        let (spectrum_row, wave_row) = staging.split_at_mut(SHADERTOY_AUDIO_WIDTH);
        for (i, byte) in spectrum_row.iter_mut().enumerate() {
            *byte = quantise(shadertoy_level(resample(spectrum, i, 0)));
        }
        for (i, byte) in wave_row.iter_mut().enumerate() {
            *byte = quantise(resample(spectrum, i, 3));
        }
        self.gl.upload(&self.shadertoy_audio, staging)
    }

    fn allocate_passes(
        &self,
        programs: Vec<Rc<Compiled>>,
        render_size: [i32; 2],
        previous: &[PassState],
        restart: &[bool],
    ) -> Result<Vec<PassState>, PlayerError> {
        let last = self.def.final_pass();
        let mut passes = Vec::with_capacity(programs.len());
        for (index, (pass, program)) in self.def.passes.iter().zip(programs).enumerate() {
            if index == last {
                passes.push(PassState {
                    program,
                    targets: Vec::new(),
                    current: 0,
                    size: render_size,
                    mipmap: false,
                    awaiting_adoption: false,
                });
                continue;
            }
            let size = pass_size(pass, render_size);
            // A pass the resize leaves at the same size keeps its texture,
            // framebuffer and contents (D287): a one-texel clock pass or a
            // literal-size data pass must not restart on every probe. Unless
            // `restart` says it must, because a restarted pass reads it.
            if !restart.get(index).copied().unwrap_or(false)
                && previous
                    .get(index)
                    .is_some_and(|old| old.size == size && !old.targets.is_empty())
            {
                passes.push(PassState {
                    program,
                    targets: Vec::new(),
                    current: 0,
                    size,
                    mipmap: pass.filter == Filter::Mipmap,
                    awaiting_adoption: true,
                });
                continue;
            }
            let precision = if pass.full_precision {
                FloatPrecision::Full
            } else {
                self.options.float_precision
            };
            let format = self.caps.pass_format(pass.float, precision);
            let sampling = sampling_for(pass.wrap, pass.filter);
            // A pass whose program samples its own target reads what it is
            // writing, which GL leaves undefined: NVIDIA returns the previous
            // frame, D3D11 under ANGLE returns zeros (TORNADO T7). Double
            // buffering gives every driver NVIDIA's answer, which is also
            // what DOUBLE_BUFFER means (D203). The sampler list is the
            // linked program's active samplers, so a pass that only names
            // its target on a branch the driver removed is not affected.
            let reads_itself = program
                .samplers
                .iter()
                .any(|(source, _)| *source == Source::Pass(index));
            if reads_itself && !pass.double_buffer {
                log::info!(
                    "pass {} samples its own output; double-buffered so it reads the previous \
                     frame on every driver (D203)",
                    pass.target.as_deref().unwrap_or("?")
                );
            }
            let count = if pass.double_buffer || reads_itself {
                2
            } else {
                1
            };
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
                awaiting_adoption: false,
            });
        }
        Ok(passes)
    }

    /// Renders every pass for one frame.
    pub fn render(&mut self, inputs: &FrameInputs<'_>) {
        self.poll_media();
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
                let texture = self.sampler_texture(*source, inputs.time);
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
        if let Some(readback) = self.readback.as_mut() {
            readback.capture(&self.finals[self.final_current].fbo);
        }
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
        if let Some(loc) = &program.date {
            gl.uniform_vec4(loc, self.date.unwrap_or_else(wall_clock_date));
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
            gl.uniform_f32(loc, self.media.media_type());
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
        for ((loc, ty), value) in program.extras.iter().zip(&self.extra_values) {
            let Some(loc) = loc else {
                continue;
            };
            let [x, y, z, w] = *value;
            match ty {
                ExtraType::Float => gl.uniform_f32(loc, x),
                ExtraType::Vec2 => gl.uniform_vec2(loc, [x, y]),
                ExtraType::Vec3 => gl.uniform_vec3(loc, [x, y, z]),
                ExtraType::Vec4 => gl.uniform_vec4(loc, [x, y, z, w]),
            }
        }
    }

    fn sampler_texture(&self, source: Source, time: f32) -> &Texture {
        match source {
            Source::FinalPass => &self.finals[self.final_current].texture,
            Source::Media => self.media.current(time).unwrap_or(&self.black),
            Source::Cubemap(i) => self
                .cubemaps
                .get(i)
                .and_then(Option::as_ref)
                .unwrap_or(&self.black_cube),
            Source::Spectrum => &self.spectrum,
            Source::LevelTrail => &self.level_trail,
            Source::ShadertoyAudio => &self.shadertoy_audio,
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

/// Moves the targets of every pass a resize left at the same size from the
/// old passes into the new ones ([`PassState::awaiting_adoption`]).
fn adopt_unchanged(old: &mut [PassState], new: &mut [PassState]) {
    for (old, new) in old.iter_mut().zip(new.iter_mut()) {
        if new.awaiting_adoption {
            adopt_one(old, new);
        }
    }
}

/// Hands `old`'s targets and current buffer to `new` (D287). A move, so
/// nothing is allocated, copied or cleared.
fn adopt_one(old: &mut PassState, new: &mut PassState) {
    new.targets = std::mem::take(&mut old.targets);
    new.current = old.current;
    new.awaiting_adoption = false;
}

/// A pass's size at `render_size`. A FIXED_SIZE pass keeps its own grid
/// (D191); every other pass follows the render size, and so the render
/// scale (D11). Not for the final pass, which is the render size.
fn pass_size(pass: &PassDef, render_size: [i32; 2]) -> [i32; 2] {
    pass.fixed.unwrap_or([
        ((render_size[0] as f32 * pass.scale[0]).round() as i32).max(1),
        ((render_size[1] as f32 * pass.scale[1]).round() as i32).max(1),
    ])
}

/// The final pass's size for a `requested` render size: the scene's pinned
/// size when it has one (D280), so `ivec2(_xy)` in the final pass addresses
/// a root-sized literal pass texel for texel. Every relative pass follows
/// it, so a pinned scene renders at its own design size throughout.
fn final_size(pinned: Option<[i32; 2]>, requested: [i32; 2]) -> [i32; 2] {
    pinned.map_or(requested, |[w, h]| [w.max(1), h.max(1)])
}

/// Writes one pass's assembled fragment shader to the folder named by
/// `STROBE_DUMP_GLSL`, when it is set, as `<title>.<dialect>.pass<N>.frag`.
/// Debugging only: it runs at scene load, never per frame, and a failed
/// write is logged and otherwise ignored.
fn dump_fragment(title: &str, dialect: assemble::Dialect, pass: usize, source: &str) {
    const ENV: &str = "STROBE_DUMP_GLSL";
    let Some(dir) = std::env::var_os(ENV) else {
        return;
    };
    let label = match dialect {
        assemble::Dialect::Glsl330 => "glsl330",
        assemble::Dialect::Glsl410 => "glsl410",
        assemble::Dialect::Es300 => "es300",
    };
    let stem: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let dir = Path::new(&dir);
    let path = dir.join(format!("{stem}.{label}.pass{pass}.frag"));
    let written = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, source));
    match written {
        Ok(()) => log::info!("{ENV}: wrote {}", path.display()),
        Err(error) => log::warn!("{ENV}: could not write {}: {error}", path.display()),
    }
}

/// One pass's fragment shader: what the driver compiles, and what the user
/// wrote, for error messages.
#[derive(Clone, Copy)]
struct Fragment<'a> {
    /// The assembled shader.
    source: &'a str,
    /// The pass's code as the scene holds it (includes expanded).
    user_code: &'a str,
    /// The file it came from, relative to the scene folder.
    file: &'a str,
}

/// Compiles and links one pass program and resolves its locations.
fn build_program(
    gl: &Gl,
    def: &SceneDef,
    extras: &[(String, ExtraType)],
    vertex: &crate::gl::Shader,
    fragment: Fragment<'_>,
    pass: usize,
) -> Result<Compiled, PlayerError> {
    let fragment = gl
        .compile(Stage::Fragment, fragment.source)
        .map_err(|log| PlayerError::Program {
            pass,
            message: describe_log(&log, fragment),
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
    assign("strobe_ShadertoyAudio", Source::ShadertoyAudio);
    for (j, p) in def.passes.iter().enumerate() {
        if let Some(target) = &p.target {
            assign(target, Source::Pass(j));
        }
    }
    for (i, image) in def.images.iter().enumerate() {
        assign(&image.name, Source::Image(i));
    }
    for (i, cube) in def.cubemaps.iter().enumerate() {
        assign(&cube.name, Source::Cubemap(i));
    }

    Ok(Compiled {
        rendersize: loc("RENDERSIZE"),
        time: loc("TIME"),
        framecount: loc("FRAMECOUNT"),
        date: loc("strobe_Date"),
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
        extras: extras.iter().map(|(name, ty)| (loc(name), *ty)).collect(),
        samplers,
        program,
    })
}

/// A driver log with user-code locations rewritten as `file:N`, `file`
/// being the pass's shader (`main.glsl`, or its `PASS_SOURCES` entry), or
/// `file:N:C` when the driver gives a column (Mesa does). The column is put
/// back on the user's line, since the assembler's rewrites move columns
/// but never lines ([`assemble::original_column`]); it is left out when
/// either line cannot be found.
fn describe_log(log: &str, fragment: Fragment<'_>) -> String {
    let file = fragment.file;
    log.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match assemble::map_error_line(line) {
            Some(ErrorSite::User(n)) => match user_column(line, n, fragment) {
                Some(column) => format!("{file}:{n}:{column}: {}", line.trim()),
                None => format!("{file}:{n}: {}", line.trim()),
            },
            Some(ErrorSite::Generated) => format!("(generated): {}", line.trim()),
            None => line.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The column on the user's line `line` that the driver's log line points
/// at, when the driver names a column.
fn user_column(log_line: &str, line: u32, fragment: Fragment<'_>) -> Option<u32> {
    use assemble::source;
    let column = assemble::driver_column(log_line)?;
    let compiled = assemble::source_line(fragment.source, source::PREAMBLE, source::USER, line)?;
    let original = assemble::source_line(fragment.user_code, source::USER, source::USER, line)?;
    let column = assemble::compiled_column(compiled, column);
    Some(assemble::original_column(original, compiled, column))
}

/// Sampling state for a pass's wrap and filter settings.
pub(crate) fn sampling_for(wrap: Wrap, filter: Filter) -> Sampling {
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

/// Width of the silent `syn_Spectrum` a scene starts with: Synesthesia's
/// 1024 (D269), so a scene that `texelFetch`es by that count stays in range
/// before audio arrives, and the analyser's first upload reallocates nothing.
const SILENT_SPECTRUM_WIDTH: i32 = 1024;

/// Width of the silent `syn_LevelTrail` a scene starts with, the analyser's
/// default trail length.
const SILENT_TRAIL_WIDTH: i32 = 256;

/// A silent audio data texture `width` texels wide, in the shape the
/// context's dialect declared it: 1D on desktop GL, 2D one texel tall on
/// GLES, which has no `sampler1D`.
fn audio_texture(gl: &Gl, caps: &Caps, width: i32) -> Result<Texture, String> {
    let zeros = vec![0_u8; width.max(1) as usize * 4];
    if caps.has_1d_textures {
        gl.texture_1d(width, Format::Rgba8, Some(&zeros))
    } else {
        gl.texture_2d(
            width,
            1,
            Format::Rgba8,
            Sampling::LINEAR_CLAMP,
            Some(&zeros),
        )
    }
}

/// A `0..1` channel as a byte; NaN reads as zero.
fn quantise(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Decodes an image file into a texture. By default the bottom row comes
/// first so `_uv` reads it upright; the scene can turn that off.
fn load_image(gl: &Gl, path: &Path, def: &ImageDef) -> Result<Texture, PlayerError> {
    let opened = crate::assets::decode_file(path)
        .map_err(|e| PlayerError::Resource(format!("{}: {e}", path.display())))?;
    let decoded = if def.vflip { opened.flipv() } else { opened }.into_rgba8();
    let (w, h) = decoded.dimensions();
    let sampling = sampling_for(def.wrap, def.filter);
    let texture = gl
        .texture_2d(
            w as i32,
            h as i32,
            Format::Rgba8,
            sampling,
            Some(decoded.as_raw()),
        )
        .map_err(PlayerError::Resource)?;
    if def.filter == Filter::Mipmap {
        gl.generate_mipmaps(&texture);
    }
    Ok(texture)
}

/// Width of Shadertoy's audio texture (rows: spectrum, waveform).
const SHADERTOY_AUDIO_WIDTH: usize = 512;

/// The Shadertoy audio texture: 512x2 when a shader reads it, 1x2
/// otherwise, holding silence (zero spectrum, waveform at 128).
fn shadertoy_audio_texture(gl: &Gl, wanted: bool) -> Result<Texture, String> {
    let width = if wanted { SHADERTOY_AUDIO_WIDTH } else { 1 };
    let mut silence = vec![0_u8; width * 2];
    if let Some(wave_row) = silence.get_mut(width..) {
        wave_row.fill(128);
    }
    gl.texture_2d(
        width as i32,
        2,
        Format::R8,
        Sampling::LINEAR_CLAMP,
        Some(&silence),
    )
}

/// Channel `channel` of `texels` at column `i` of a 512-column row,
/// interpolating linearly between neighbouring texels.
fn resample(texels: &[[f32; 4]], i: usize, channel: usize) -> f32 {
    let last = texels.len().saturating_sub(1);
    let position = (i as f32 + 0.5) / SHADERTOY_AUDIO_WIDTH as f32 * texels.len() as f32 - 0.5;
    let position = position.clamp(0.0, last as f32);
    let low = position.floor() as usize;
    let high = (low + 1).min(last);
    let at = |index: usize| texels.get(index).and_then(|t| t.get(channel)).copied();
    let t = position - low as f32;
    at(low).unwrap_or(0.0) * (1.0 - t) + at(high).unwrap_or(0.0) * t
}

/// Converts the raw-FFT channel (-100 to -20 dB) to Shadertoy's texture
/// scale (-100 to -30 dB, with the browser analyser's 6 dB lower scaling).
fn shadertoy_level(raw: f32) -> f32 {
    ((raw * 80.0 - 6.0206) / 70.0).clamp(0.0, 1.0)
}

/// `strobe_Date` from the wall clock, in UTC.
fn wall_clock_date() -> [f32; 4] {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    strobe_scene::date::from_unix_seconds(seconds)
}
