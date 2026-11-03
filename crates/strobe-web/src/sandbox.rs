// ============================================================================
// Module:       strobe_web::sandbox
// Description:  The sandbox engine over any current context: loads a scene
//               from the editor's files, keeps its controls and audio, and
//               draws a frame.
//
// Dependencies: strobe-gfx (player), strobe-scene (scene, controls,
//               synthetic audio), strobe-audio (analyser), serde_json
// ============================================================================

//! The sandbox engine.
//!
//! Everything the page does to a scene goes through [`Sandbox`], which is
//! plain Rust over a [`Gl`]: the wasm glue in `web` hands it the canvas's
//! WebGL2 context, and the native tests hand it the headless one, so the
//! behaviour the page depends on is tested without a browser.
//!
//! - **Editing keeps the picture.** A load that fails (bad JSON, a shader
//!   error) returns the error and leaves the last good scene playing, as
//!   Shadertoy does while you type. Controls whose name and type survive an
//!   edit keep their values ([`ControlBank::carry_unchanged_from`]).
//! - **Audio has three sources.** The synthetic 120 BPM track the bench
//!   uses, live samples the page pushes from the microphone or a file
//!   through the same [`Analyzer`] the app runs, or silence.
//! - **Scripts run in the page.** QuickJS is C and does not build for
//!   `wasm32-unknown-unknown`, so `script.js` runs in a Web Worker behind a
//!   shim of the script API (D524). The engine's part is the uniforms the
//!   script creates: they are declared at load like the app's
//!   ([`ScenePlayer::with_extras`]) and set each frame from the worker's
//!   latest values.

use serde_json::{Value, json};
use strobe_audio::analyzer::Analyzer;
use strobe_gfx::gl::Gl;
use strobe_gfx::{AudioTextures, Caps, FrameInputs, Mouse, Options, ScenePlayer};
use strobe_scene::assemble::ExtraType;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::ControlBank;
use strobe_scene::load::SceneFolder;

/// Where the audio uniforms come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSource {
    /// The synthetic 120 BPM track (`strobe_scene::synthetic`).
    Synthetic,
    /// Samples pushed with [`Sandbox::push_samples`], through the analyser.
    Live,
    /// Every uniform at its silent value.
    Silent,
}

impl AudioSource {
    /// Parses the page's name for a source.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "synthetic" => Some(Self::Synthetic),
            "live" => Some(Self::Live),
            "silent" => Some(Self::Silent),
            _ => None,
        }
    }
}

/// A loaded scene and what it keeps between frames.
struct Loaded {
    player: ScenePlayer,
    controls: ControlBank,
    transitions: Vec<f32>,
    extras: Vec<(String, ExtraType)>,
    extra_values: Vec<[f32; 4]>,
}

/// The audio state, whichever source is chosen.
struct Audio {
    source: AudioSource,
    analyzer: Analyzer,
    scalars: Vec<f32>,
    spectrum: Vec<[f32; 4]>,
    level_trail: Vec<[f32; 4]>,
}

/// The texture widths and trail length the bench uses for the synthetic
/// track; the analyser's default layout matches them (D269).
const SPECTRUM_TEXELS: usize = 1024;
const TRAIL_TEXELS: usize = 256;
const TRAIL_SECONDS: f32 = 4.0;

impl Audio {
    fn new(sample_rate: u32) -> Self {
        Self {
            source: AudioSource::Synthetic,
            analyzer: Analyzer::new(sample_rate),
            scalars: silent_scalars(),
            spectrum: vec![[0.0; 4]; SPECTRUM_TEXELS],
            level_trail: vec![[0.0; 4]; TRAIL_TEXELS],
        }
    }

    /// Advances the chosen source by `dt` to `time`.
    fn advance(&mut self, time: f32, dt: f32) {
        match self.source {
            AudioSource::Synthetic => {
                strobe_scene::synthetic::fill(time, &mut self.scalars);
                strobe_scene::synthetic::fill_textures(
                    time,
                    &mut self.spectrum,
                    &mut self.level_trail,
                    TRAIL_SECONDS,
                );
            }
            AudioSource::Live => {
                let frame = self.analyzer.analyze(dt);
                self.scalars.clone_from(&frame.scalars);
                self.spectrum.clone_from(&frame.spectrum);
                self.level_trail.clone_from(&frame.level_trail);
            }
            AudioSource::Silent => {
                self.scalars = silent_scalars();
                self.spectrum.fill([0.0; 4]);
                self.level_trail.fill([0.0; 4]);
            }
        }
    }
}

/// Every audio scalar at its silent value, in [`SCALARS`] order.
fn silent_scalars() -> Vec<f32> {
    SCALARS
        .iter()
        .map(|name| strobe_scene::audio::silent_value(name))
        .collect()
}

/// What the page passes for one frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameRequest {
    /// Seconds since the scene started.
    pub time: f32,
    /// Seconds since the last frame.
    pub dt: f32,
    /// Frames since the scene started.
    pub frame: u64,
    /// The mouse, already in render pixels with the origin bottom-left.
    pub mouse: Mouse,
    /// The wall clock, for `strobe_Date`.
    pub unix_seconds: f64,
}

/// The sandbox: one canvas, one scene at a time.
pub struct Sandbox {
    gl: Gl,
    caps: Caps,
    size: [i32; 2],
    scene: Option<Loaded>,
    audio: Audio,
}

impl Sandbox {
    /// A sandbox drawing into `gl`'s default framebuffer at `size` pixels.
    pub fn new(gl: Gl, size: [i32; 2], sample_rate: u32) -> Self {
        let caps = Caps::from_info(gl.info());
        Self {
            gl,
            caps,
            size: clamp_size(size),
            scene: None,
            audio: Audio::new(sample_rate),
        }
    }

    /// What the context is, for the page's status line.
    pub fn renderer(&self) -> String {
        format!("{} ({})", self.caps.info.renderer, self.caps.info.version)
    }

    /// Loads a scene from its files' text. On success the new scene
    /// replaces the old one and the controls are returned as JSON (see
    /// [`Self::controls_json`]). On failure the old scene keeps playing and
    /// the error is returned as text, with shader errors located as
    /// `main.glsl:N:` (the player's own error format).
    ///
    /// `extras` are the uniforms the scene's script creates, as the worker
    /// reported them after `setup()`.
    pub fn load(
        &mut self,
        title: &str,
        file: impl Fn(&str) -> Option<String>,
        extras: Vec<(String, ExtraType)>,
    ) -> Result<String, String> {
        let texts: Vec<(String, String)> = scene_files(&file);
        let lookup = |name: &str| {
            texts
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, text)| text.as_str())
        };
        let scene = SceneFolder::from_files(title, lookup).map_err(|e| e.to_string())?;
        let mut controls =
            ControlBank::new(scene.def.controls.clone()).map_err(|e| e.to_string())?;
        if let Some(old) = &self.scene {
            controls.carry_unchanged_from(&old.controls);
        }
        let transitions = scene
            .def
            .transition_uniforms
            .iter()
            .map(|t| t.initial)
            .collect();
        let player = ScenePlayer::with_extras(
            &self.gl,
            &self.caps,
            &scene,
            self.size,
            Options::default(),
            &extras,
        )
        .map_err(|e| e.to_string())?;
        let extra_values = vec![[0.0; 4]; extras.len()];
        self.scene = Some(Loaded {
            player,
            controls,
            transitions,
            extras,
            extra_values,
        });
        Ok(self.controls_json())
    }

    /// The scene's controls as a JSON array: each control's `scene.json`
    /// form (`ControlDef::to_json`) with its current target as `VALUE`.
    /// `[]` with no scene.
    pub fn controls_json(&self) -> String {
        let Some(scene) = &self.scene else {
            return "[]".to_string();
        };
        let controls: Vec<Value> = scene
            .controls
            .defs()
            .iter()
            .enumerate()
            .map(|(index, def)| {
                let mut out = def.to_json();
                let values = scene.controls.target(index).copied().unwrap_or_default();
                if let Value::Object(map) = &mut out {
                    map.insert("VALUE".into(), json!(values.get(..def.dims())));
                }
                out
            })
            .collect();
        Value::Array(controls).to_string()
    }

    /// Every control's current output by name, for the script worker's
    /// globals: `{"name": [v, ...]}`.
    pub fn control_outputs_json(&self) -> String {
        let Some(scene) = &self.scene else {
            return "{}".to_string();
        };
        let map: serde_json::Map<String, Value> = scene
            .controls
            .defs()
            .iter()
            .enumerate()
            .map(|(index, def)| {
                let values = scene.controls.output(index).copied().unwrap_or_default();
                (def.name.clone(), json!(values.get(..def.dims())))
            })
            .collect();
        Value::Object(map).to_string()
    }

    /// Sets a control's target, in its own units. A bang is pressed
    /// instead. `false` when no control has that name.
    pub fn set_control(&mut self, name: &str, values: &[f32]) -> bool {
        let Some(scene) = &mut self.scene else {
            return false;
        };
        let Some(index) = scene.controls.find(name) else {
            return false;
        };
        let bang = scene
            .controls
            .defs()
            .get(index)
            .is_some_and(|def| matches!(def.kind, strobe_scene::control::ControlKind::Bang(_)));
        if bang {
            scene.controls.press(index);
        } else {
            scene.controls.set(index, None, values);
        }
        true
    }

    /// Sets a control's target from `0..1` per component, as a script's
    /// `setControlNormalized` does.
    pub fn set_control_normalized(&mut self, name: &str, values: &[f32]) -> bool {
        let Some(scene) = &mut self.scene else {
            return false;
        };
        let Some(index) = scene.controls.find(name) else {
            return false;
        };
        scene.controls.set_normalized(index, None, values);
        true
    }

    /// Puts a control back to its default.
    pub fn default_control(&mut self, name: &str) -> bool {
        let Some(scene) = &mut self.scene else {
            return false;
        };
        let Some(index) = scene.controls.find(name) else {
            return false;
        };
        scene.controls.set_default(index);
        true
    }

    /// The script's uniforms for the next frames: four floats per extra, in
    /// the order the scene was loaded with. A short slice leaves the rest.
    pub fn set_script_uniforms(&mut self, flat: &[f32]) {
        let Some(scene) = &mut self.scene else {
            return;
        };
        for (slot, chunk) in scene.extra_values.iter_mut().zip(flat.chunks(4)) {
            for (value, new) in slot.iter_mut().zip(chunk) {
                *value = *new;
            }
        }
        scene.player.set_script_uniforms(&scene.extra_values);
    }

    /// The script uniforms the scene was built with, as `[[name, dims]]`.
    pub fn extras_json(&self) -> String {
        let extras = self.scene.as_ref().map_or(&[][..], |scene| &scene.extras);
        let list: Vec<Value> = extras
            .iter()
            .map(|(name, ty)| json!([name, extra_dims(*ty)]))
            .collect();
        Value::Array(list).to_string()
    }

    /// Chooses the audio source.
    pub fn set_audio_source(&mut self, source: AudioSource) {
        self.audio.source = source;
    }

    /// Live samples, mono, at the analyser's rate.
    pub fn push_samples(&mut self, samples: &[f32]) {
        self.audio.analyzer.push(samples);
    }

    /// The live input's sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.audio.analyzer.set_sample_rate(sample_rate);
    }

    /// This frame's audio scalars, in [`SCALARS`] order.
    pub fn audio_scalars(&self) -> &[f32] {
        &self.audio.scalars
    }

    /// Resizes the canvas's picture. A failure keeps the old size.
    pub fn resize(&mut self, size: [i32; 2]) -> Result<(), String> {
        let size = clamp_size(size);
        if size == self.size {
            return Ok(());
        }
        if let Some(scene) = &mut self.scene {
            scene
                .player
                .resize_keeping(size)
                .map_err(|e| e.to_string())?;
        }
        self.size = size;
        Ok(())
    }

    /// Restarts the scene's feedback buffers, as the page's rewind does.
    pub fn restart(&mut self) -> Result<(), String> {
        match &mut self.scene {
            Some(scene) => scene.player.restart().map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    /// Advances audio and controls and draws one frame to the default
    /// framebuffer. Nothing is drawn but black without a scene.
    pub fn frame(&mut self, request: &FrameRequest) {
        self.audio.advance(request.time, request.dt);
        let Some(scene) = &mut self.scene else {
            self.gl.bind_framebuffer(None);
            self.gl.viewport(self.size[0], self.size[1]);
            self.gl.clear([0.0, 0.0, 0.0, 1.0]);
            return;
        };
        scene.controls.tick(request.dt);
        scene
            .player
            .set_date(strobe_scene::date::from_unix_seconds(request.unix_seconds));
        scene.player.upload_audio(&AudioTextures {
            spectrum: &self.audio.spectrum,
            level_trail: &self.audio.level_trail,
            shadertoy: None,
        });
        scene.player.render(&FrameInputs {
            time: request.time,
            frame: request.frame,
            mouse: request.mouse,
            audio: &self.audio.scalars,
            controls: &scene.controls,
            transitions: &scene.transitions,
        });
        scene.player.present(self.size);
    }

    /// The finished frame as RGBA8, top row first: the page's thumbnail
    /// and the tests' look at the picture. Empty with no scene.
    pub fn read_output(&self) -> Vec<u8> {
        self.scene
            .as_ref()
            .map_or_else(Vec::new, |scene| scene.player.read_output())
    }

    /// The picture's size in pixels.
    pub fn render_size(&self) -> [i32; 2] {
        self.scene
            .as_ref()
            .map_or(self.size, |scene| scene.player.render_size())
    }
}

/// Every file a scene can name, read once: `main.glsl`, `scene.json`,
/// `script.js`, and the `PASS_SOURCES` files `scene.json` lists (looked up
/// lazily by name, so any `.glsl` the page holds is offered).
fn scene_files(file: &impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let mut names: Vec<String> = ["main.glsl", "scene.json", "script.js"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let listed = file("scene.json")
        .and_then(|json| serde_json::from_str::<Value>(&json).ok())
        .and_then(|json| json.get("PASS_SOURCES").cloned());
    if let Some(Value::Array(sources)) = listed {
        names.extend(sources.iter().filter_map(Value::as_str).map(str::to_string));
    }
    names
        .into_iter()
        .filter_map(|name| file(&name).map(|text| (name, text)))
        .collect()
}

/// Components in a script uniform of type `ty`.
fn extra_dims(ty: ExtraType) -> usize {
    match ty {
        ExtraType::Float => 1,
        ExtraType::Vec2 => 2,
        ExtraType::Vec3 => 3,
        ExtraType::Vec4 => 4,
    }
}

/// The script uniform type with `dims` components, if there is one.
pub fn extra_type(dims: usize) -> Option<ExtraType> {
    match dims {
        1 => Some(ExtraType::Float),
        2 => Some(ExtraType::Vec2),
        3 => Some(ExtraType::Vec3),
        4 => Some(ExtraType::Vec4),
        _ => None,
    }
}

/// A canvas size the player accepts: at least one pixel, at most 8192.
fn clamp_size(size: [i32; 2]) -> [i32; 2] {
    const MAX: i32 = 8192;
    size.map(|n| n.clamp(1, MAX))
}
