// ============================================================================
// Module:       strobe_web::web
// Description:  The wasm-bindgen surface the /play page calls: a canvas's
//               WebGL2 context wrapped as a Sandbox.
//
// Dependencies: wasm-bindgen, js-sys, web-sys; crate::sandbox
// ============================================================================

//! The JavaScript API.
//!
//! One [`Engine`] per canvas. Values cross as numbers, typed arrays and JSON
//! strings, so the page needs no generated types beyond the class. Errors
//! come back as `Err(string)`, which wasm-bindgen throws as a JavaScript
//! exception the page catches and shows in the editor.

use crate::sandbox::{AudioSource, FrameRequest, Sandbox, extra_type};
use strobe_gfx::Mouse;
use strobe_gfx::gl::Gl;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

/// Routes `log` to the browser console, once per page.
fn init_logging() {
    struct Console;
    impl log::Log for Console {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Info
        }
        fn log(&self, record: &log::Record<'_>) {
            let text = JsValue::from_str(&format!("strobe: {}", record.args()));
            match record.level() {
                log::Level::Error => web_sys::console::error_1(&text),
                log::Level::Warn => web_sys::console::warn_1(&text),
                log::Level::Info | log::Level::Debug | log::Level::Trace => {
                    web_sys::console::log_1(&text);
                }
            }
        }
        fn flush(&self) {}
    }
    static CONSOLE: Console = Console;
    if log::set_logger(&CONSOLE).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
        // A panic aborts the module (`unreachable` in the page's error);
        // the hook runs first and says where, as a crash report would.
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&JsValue::from_str(&format!("strobe panicked: {info}")));
        }));
    }
}

/// The sandbox on one canvas.
#[wasm_bindgen]
pub struct Engine {
    sandbox: Sandbox,
}

#[wasm_bindgen]
impl Engine {
    /// Opens a WebGL2 context on `canvas` (no antialiasing, since the
    /// player blits its output to the default framebuffer, which WebGL
    /// refuses into a multisampled one) and sizes the picture to the
    /// canvas's pixels.
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement, sample_rate: u32) -> Result<Engine, String> {
        init_logging();
        let options = js_sys::Object::new();
        for (key, value) in [
            ("antialias", false),
            ("alpha", false),
            ("depth", false),
            ("stencil", false),
            ("preserveDrawingBuffer", true),
        ] {
            js_sys::Reflect::set(&options, &key.into(), &value.into())
                .map_err(|_| "could not build the context options".to_string())?;
        }
        let context = canvas
            .get_context_with_context_options("webgl2", &options)
            .map_err(|_| "this browser refused a WebGL2 context".to_string())?
            .ok_or_else(|| "this browser has no WebGL2".to_string())?
            .dyn_into::<WebGl2RenderingContext>()
            .map_err(|_| "the canvas's context is not WebGL2".to_string())?;
        let size = [canvas.width() as i32, canvas.height() as i32];
        Ok(Self {
            sandbox: Sandbox::new(Gl::from_webgl2(context), size, sample_rate),
        })
    }

    /// The renderer and GL version, for the status line.
    pub fn renderer(&self) -> String {
        self.sandbox.renderer()
    }

    /// Loads a scene from `files` (an object of file name to text) with the
    /// script uniforms `extras` (`[[name, dims], ...]` as JSON). Returns the
    /// controls as JSON; throws the error text and keeps the old scene.
    pub fn load(&mut self, title: &str, files: JsValue, extras: &str) -> Result<String, String> {
        let extras: Vec<(String, strobe_scene::assemble::ExtraType)> =
            serde_json::from_str::<Vec<(String, usize)>>(extras)
                .map_err(|e| format!("script uniforms: {e}"))?
                .into_iter()
                .filter_map(|(name, dims)| extra_type(dims).map(|ty| (name, ty)))
                .collect();
        let file = |name: &str| {
            js_sys::Reflect::get(&files, &JsValue::from_str(name))
                .ok()
                .and_then(|value| value.as_string())
        };
        self.sandbox.load(title, file, extras)
    }

    /// The controls with their current values, as JSON.
    #[wasm_bindgen(js_name = controlsJson)]
    pub fn controls_json(&self) -> String {
        self.sandbox.controls_json()
    }

    /// Every control's output by name, as JSON, for the script worker.
    #[wasm_bindgen(js_name = controlOutputsJson)]
    pub fn control_outputs_json(&self) -> String {
        self.sandbox.control_outputs_json()
    }

    /// The script uniforms the scene was built with, as JSON.
    #[wasm_bindgen(js_name = extrasJson)]
    pub fn extras_json(&self) -> String {
        self.sandbox.extras_json()
    }

    /// Sets a control in its own units (a bang is pressed).
    #[wasm_bindgen(js_name = setControl)]
    pub fn set_control(&mut self, name: &str, values: &[f32]) -> bool {
        self.sandbox.set_control(name, values)
    }

    /// Sets a control from `0..1` per component.
    #[wasm_bindgen(js_name = setControlNormalized)]
    pub fn set_control_normalized(&mut self, name: &str, values: &[f32]) -> bool {
        self.sandbox.set_control_normalized(name, values)
    }

    /// Puts a control back to its default.
    #[wasm_bindgen(js_name = defaultControl)]
    pub fn default_control(&mut self, name: &str) -> bool {
        self.sandbox.default_control(name)
    }

    /// The script's uniform values, four floats per uniform.
    #[wasm_bindgen(js_name = setScriptUniforms)]
    pub fn set_script_uniforms(&mut self, flat: &[f32]) {
        self.sandbox.set_script_uniforms(flat);
    }

    /// `"synthetic"`, `"live"` or `"silent"`; `false` for anything else.
    #[wasm_bindgen(js_name = setAudioSource)]
    pub fn set_audio_source(&mut self, name: &str) -> bool {
        match AudioSource::parse(name) {
            Some(source) => {
                self.sandbox.set_audio_source(source);
                true
            }
            None => false,
        }
    }

    /// Live mono samples.
    #[wasm_bindgen(js_name = pushSamples)]
    pub fn push_samples(&mut self, samples: &[f32]) {
        self.sandbox.push_samples(samples);
    }

    /// The live input's sample rate.
    #[wasm_bindgen(js_name = setSampleRate)]
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.sandbox.set_sample_rate(sample_rate);
    }

    /// This frame's audio scalars, in `SCALARS` order.
    #[wasm_bindgen(js_name = audioScalars)]
    pub fn audio_scalars(&self) -> Vec<f32> {
        self.sandbox.audio_scalars().to_vec()
    }

    /// The audio scalar names, in order, as JSON.
    #[wasm_bindgen(js_name = scalarNames)]
    pub fn scalar_names() -> String {
        serde_json::json!(strobe_scene::audio::SCALARS).to_string()
    }

    /// Resizes the picture to `width` by `height` pixels.
    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), String> {
        self.sandbox.resize([width, height])
    }

    /// Clears the scene's feedback buffers.
    pub fn restart(&mut self) -> Result<(), String> {
        self.sandbox.restart()
    }

    /// Draws one frame. `mouse` is `[x, y, click x, click y, left, right,
    /// middle]` in render pixels from the bottom left, the click position
    /// negated while no button is held, as `_mouse` wants it.
    #[expect(
        clippy::too_many_arguments,
        reason = "a flat call keeps the per-frame crossing allocation-free"
    )]
    pub fn frame(
        &mut self,
        time: f32,
        dt: f32,
        frame: f64,
        mouse: &[f32],
        unix_seconds: f64,
        width: i32,
        height: i32,
    ) {
        let get = |i: usize| mouse.get(i).copied().unwrap_or(0.0);
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let (x, y) = (get(0), get(1));
        let request = FrameRequest {
            time,
            dt,
            frame: frame.max(0.0) as u64,
            mouse: Mouse {
                mouse: [x, y, get(2), get(3)],
                click: [get(4), get(5), get(6)],
                muv: [x / w, y / h],
                muvc: [(x - 0.5 * w) / h, (y - 0.5 * h) / h],
            },
            unix_seconds,
        };
        self.sandbox.frame(&request);
    }
}
