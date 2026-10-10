// ============================================================================
// Module:       strobe_script::host
// Description:  ScriptHost: one QuickJS runtime per scene instance, its
//               per-frame sequence, and the fault and watchdog policy.
//
// Dependencies: rquickjs, strobe_scene, crate::{natives, shared, events,
//               output, inputs, budget, error}
// ============================================================================

//! The script host.
//!
//! A frame runs in a fixed order, and the order is part of the contract:
//!
//! 1. **Write globals.** Time, audio, every control's output and the mouse
//!    become JavaScript globals. Vector values are objects built once at
//!    load; a frame only overwrites their numeric fields, and a scalar
//!    global is a single property write, so steady state creates no
//!    garbage on the JavaScript side.
//! 2. **Dispatch events.** Each registered handler compares this frame's
//!    value with last frame's and calls its callback if the condition
//!    holds, in registration order.
//! 3. **`update(dt)`.**
//! 4. **Harvest.** The deprecated `uniforms` object is scanned, then the
//!    frame's uniforms are committed (or, after a fault, discarded) and the
//!    queued control writes are applied to the bank.
//!
//! The caller then ticks the control bank, so a script's `setControl` is
//! visible to the shader in the same frame it was made.
//!
//! ## The watchdog
//!
//! The interpreter calls the interrupt handler every few thousand
//! instructions; the handler reads one `Cell` and the clock. That makes
//! the budget a *watchdog*, not a hard real-time guarantee: a single
//! native call that takes long (a huge `Array.prototype.sort`) runs to
//! completion, and the overrun shows up in the next check. For the loops a
//! script can write by accident it is tight.

use crate::budget::Budget;
use crate::error::ScriptError;
use crate::events::Source;
use crate::inputs::{FINAL_PASS_BYTES, ScriptInputs};
use crate::natives::{display, install, stage_uniform};
use crate::output::{ConsoleLevel, FrameOutput, ScriptStatus};
use crate::shared::{Command, Shared, Target};
use rquickjs::context::EvalOptions;
use rquickjs::{Context, Ctx, Exception, Object, Persistent, Runtime, Value};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;
use strobe_scene::audio::{SCALARS, silent_value};
use strobe_scene::control::{ControlBank, ControlKind};

/// Consecutive faulted frames after which the script is switched off.
const MAX_CONSECUTIVE_FAULTS: u32 = 3;

/// File name shown in stack traces.
const SCRIPT_NAME: &str = "script.js";

/// Installs `inputs`, the deprecated read-only view of the globals: every
/// read goes to the global of the same name, so there is no second copy to
/// keep in step. A write is refused (silently in sloppy mode, with a
/// `TypeError` in strict mode, as for any read-only property).
const INPUTS_PRELUDE: &str = "new Proxy({}, {\
    get: function (_target, key) { return globalThis[key]; },\
    has: function (_target, key) { return key in globalThis; },\
    set: function () { return false; }\
})";

/// How `script.js` is evaluated: as a global script in sloppy mode, since
/// scenes in the wild are sloppy-mode scripts that assign to undeclared
/// variables, named so stack traces point at `script.js`.
fn script_options() -> EvalOptions {
    let mut options = EvalOptions::default();
    options.strict = false;
    options.filename = Some(SCRIPT_NAME.to_string());
    options
}

/// A vector-valued global: its object is created once and rewritten each
/// frame.
struct VectorGlobal {
    /// Index of the control in the bank.
    control: usize,
    /// Components to write (2 or 3).
    dims: usize,
    /// Whether `r`, `g`, `b` aliases are written too.
    color: bool,
    /// The JavaScript object, kept alive across `with` calls.
    object: Persistent<Object<'static>>,
}

/// Every global the host rewrites each frame.
///
/// Declared before the runtime in [`ScriptHost`] so it is dropped first:
/// a [`Persistent`] that outlives its runtime makes the runtime abort.
struct Globals {
    /// Float-valued controls by global name and bank index.
    scalars: Vec<(String, usize)>,
    /// Vector-valued controls.
    vectors: Vec<VectorGlobal>,
    /// `_mouse`, `_click`, `_muv`, `_muvc`, in that order.
    mouse: Vec<Persistent<Object<'static>>>,
}

/// One QuickJS runtime driving one scene's `script.js`.
///
/// Not `Send`: build it and call [`Self::frame`] on the same thread.
pub struct ScriptHost {
    // Field order is drop order: the persistent handles go before the
    // context and runtime that own what they point at.
    globals: Globals,
    shared: Rc<Shared>,
    context: Context,
    // Held so the runtime lives as long as the host; not otherwise read.
    _runtime: Runtime,
    budget: Budget,
    consecutive_faults: u32,
    disabled: bool,
    last_fault: Option<String>,
}

impl std::fmt::Debug for ScriptHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptHost")
            .field("consecutive_faults", &self.consecutive_faults)
            .field("disabled", &self.disabled)
            .field("last_fault", &self.last_fault)
            .finish_non_exhaustive()
    }
}

/// Turns an engine error into one line of text, with the line number when
/// the engine gave a stack.
fn describe(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if !matches!(error, rquickjs::Error::Exception) {
        return error.to_string();
    }
    let thrown = ctx.catch();
    let exception = thrown
        .as_object()
        .and_then(|object| Exception::from_object(object.clone()));
    let Some(exception) = exception else {
        // QuickJS cannot build an Error object once the heap is full, so a
        // memory-limit breach surfaces as an exception with no value.
        if thrown.is_null() || thrown.is_undefined() {
            return "out of memory (the engine raised an exception with no value)".to_string();
        }
        return display(ctx, &thrown);
    };
    let message = exception.message().unwrap_or_default();
    let location = exception.stack().and_then(|stack| {
        stack
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string)
    });
    match location {
        Some(location) => format!("{message} ({location})"),
        None => message,
    }
}

/// Overwrites `keys` on `object` with `values`.
fn write_fields(object: &Object<'_>, keys: &[&str], values: &[f32]) -> rquickjs::Result<()> {
    for (key, value) in keys.iter().zip(values) {
        object.set(*key, f64::from(*value))?;
    }
    Ok(())
}

/// Creates the object for a vector global with every field zero.
fn new_vector<'js>(
    ctx: &Ctx<'js>,
    keys: &[&str],
) -> rquickjs::Result<(Object<'js>, Persistent<Object<'static>>)> {
    let object = Object::new(ctx.clone())?;
    for key in keys {
        object.set(*key, 0.0_f64)?;
    }
    let handle = Persistent::save(ctx, object.clone());
    Ok((object, handle))
}

impl Globals {
    /// Creates every global object and publishes it.
    fn build<'js>(ctx: &Ctx<'js>, controls: &ControlBank) -> rquickjs::Result<Self> {
        let globals = ctx.globals();
        let mut scalars = Vec::new();
        let mut vectors = Vec::new();
        for (index, def) in controls.defs().iter().enumerate() {
            let dims = def.dims();
            if dims < 2 {
                scalars.push((def.name.clone(), index));
                continue;
            }
            let color = matches!(def.kind, ControlKind::Color { .. });
            let keys: &[&str] = if color {
                &["x", "y", "z", "r", "g", "b"]
            } else {
                &["x", "y", "z"]
            };
            let (object, handle) = new_vector(ctx, keys.get(..dims).unwrap_or(keys))?;
            if color {
                for key in ["r", "g", "b"] {
                    object.set(key, 0.0_f64)?;
                }
            }
            globals.set(def.name.as_str(), object)?;
            vectors.push(VectorGlobal {
                control: index,
                dims,
                color,
                object: handle,
            });
        }
        let mut mouse = Vec::new();
        for (name, keys) in [
            ("_mouse", &["x", "y", "z", "w"][..]),
            ("_click", &["x", "y", "z"][..]),
            ("_muv", &["x", "y"][..]),
            ("_muvc", &["x", "y"][..]),
        ] {
            let (object, handle) = new_vector(ctx, keys)?;
            globals.set(name, object)?;
            mouse.push(handle);
        }
        Ok(Self {
            scalars,
            vectors,
            mouse,
        })
    }

    /// Writes this frame's values into the JavaScript globals.
    fn write<'js>(
        &self,
        ctx: &Ctx<'js>,
        inputs: &ScriptInputs<'_>,
        controls: &ControlBank,
    ) -> rquickjs::Result<()> {
        let globals = ctx.globals();
        globals.set("TIME", f64::from(inputs.time))?;
        globals.set("FRAMECOUNT", inputs.frame_count as f64)?;
        for (index, name) in SCALARS.iter().enumerate() {
            let value = inputs
                .audio
                .get(index)
                .copied()
                .unwrap_or_else(|| silent_value(name));
            globals.set(*name, f64::from(value))?;
        }
        for (name, index) in &self.scalars {
            let value = controls.output(*index).map_or(0.0, |output| output[0]);
            globals.set(name.as_str(), f64::from(value))?;
        }
        for vector in &self.vectors {
            let object = vector.object.clone().restore(ctx)?;
            let Some(output) = controls.output(vector.control) else {
                continue;
            };
            let values = output.get(..vector.dims).unwrap_or(&output[..]);
            write_fields(&object, &["x", "y", "z"], values)?;
            if vector.color {
                write_fields(&object, &["r", "g", "b"], values)?;
            }
        }
        let mouse: [(&[&str], &[f32]); 4] = [
            (&["x", "y", "z", "w"], &inputs.mouse),
            (&["x", "y", "z"], &inputs.click),
            (&["x", "y"], &inputs.muv),
            (&["x", "y"], &inputs.muvc),
        ];
        for (handle, (keys, values)) in self.mouse.iter().zip(mouse) {
            write_fields(&handle.clone().restore(ctx)?, keys, values)?;
        }
        Ok(())
    }
}

/// The bank a queued write is aimed at.
fn pick<'a>(
    target: Target,
    controls: &'a mut ControlBank,
    meta: &'a mut ControlBank,
) -> &'a mut ControlBank {
    match target {
        Target::Scene => controls,
        Target::Meta => meta,
    }
}

/// What a handler of `source` sees this frame.
fn source_value(source: Source, inputs: &ScriptInputs<'_>, controls: &ControlBank) -> f32 {
    match source {
        Source::Audio(index) => inputs
            .audio
            .get(index)
            .copied()
            .unwrap_or_else(|| SCALARS.get(index).map_or(0.0, |name| silent_value(name))),
        Source::Time => inputs.time,
        Source::FrameCount => inputs.frame_count as f32,
        Source::Control(index) => controls.output(index).map_or(0.0, |output| output[0]),
    }
}

impl ScriptHost {
    /// Starts a script for a scene whose controls are `controls`.
    ///
    /// Evaluates `source` (the text of `script.js`), then calls `setup()`
    /// if it defined one, all under [`Budget::load`]. Control writes and
    /// event registrations `setup()` makes take effect from the first
    /// [`Self::frame`]. An empty `source` is a valid script that does
    /// nothing.
    ///
    /// `controls` and `meta` must be the banks later passed to `frame`: the
    /// names the script can use are fixed here. `meta` is normally
    /// [`strobe_scene::meta::bank`].
    pub fn new(
        source: &str,
        controls: &ControlBank,
        meta: &ControlBank,
        budget: Budget,
    ) -> Result<Self, ScriptError> {
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(budget.memory_limit);
        runtime.set_max_stack_size(budget.max_stack);
        let deadline = Rc::new(Cell::new(None::<Instant>));
        let watch = Rc::clone(&deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            watch.get().is_some_and(|limit| Instant::now() >= limit)
        })));
        let context = Context::full(&runtime)?;
        let shared = Rc::new(Shared::new(controls, meta, budget.seed, deadline));

        shared.deadline.set(Some(Instant::now() + budget.load));
        let built = context.with(|ctx| -> Result<Globals, ScriptError> {
            install(&ctx, &shared)?;
            let inputs: Value<'_> = ctx.eval(INPUTS_PRELUDE)?;
            ctx.globals().set("inputs", inputs)?;
            ctx.globals().set("uniforms", Object::new(ctx.clone())?)?;
            let globals = Globals::build(&ctx, controls)?;
            globals.write(&ctx, &ScriptInputs::default(), controls)?;

            if let Err(error) = ctx.eval_with_options::<Value<'_>, _>(source, script_options()) {
                return Err(ScriptError::Load {
                    message: describe(&ctx, error),
                });
            }
            let setup: Value<'_> = ctx.globals().get("setup")?;
            if let Some(function) = setup.as_function()
                && let Err(error) = function.call::<_, ()>(())
            {
                return Err(ScriptError::Setup {
                    message: describe(&ctx, error),
                });
            }
            Ok(globals)
        });
        shared.deadline.set(None);
        let globals = built?;
        shared.state.borrow_mut().uniforms.commit();

        Ok(Self {
            globals,
            shared,
            context,
            _runtime: runtime,
            budget,
            consecutive_faults: 0,
            disabled: false,
            last_fault: None,
        })
    }

    /// Runs one frame: write globals, dispatch events, `update(dt)`,
    /// harvest. See the module docs for the exact order.
    ///
    /// Control writes the script made are applied to `controls` (and, for
    /// `meta/` writes, to `meta`) before this returns, so call `tick(dt)` on
    /// both banks afterwards. A fault does not stop those writes: they
    /// already happened, and unlike uniforms they are not replaced by the
    /// next good frame.
    pub fn frame(
        &mut self,
        dt: f32,
        inputs: &ScriptInputs<'_>,
        controls: &mut ControlBank,
        meta: &mut ControlBank,
    ) -> FrameOutput {
        if self.disabled {
            return self.finish(ScriptStatus::Disabled);
        }
        self.stage_final_pass(inputs);

        self.shared
            .deadline
            .set(Some(Instant::now() + self.budget.frame));
        let bank: &ControlBank = controls;
        let outcome = self.context.with(|ctx| self.run(&ctx, dt, inputs, bank));
        self.shared.deadline.set(None);

        let status = match outcome {
            Ok(()) => {
                self.shared.state.borrow_mut().uniforms.commit();
                self.consecutive_faults = 0;
                ScriptStatus::Running
            }
            Err(message) => self.fault(&message),
        };
        self.apply_commands(controls, meta);
        self.finish(status)
    }

    /// The text of the most recent fault, if the script has faulted.
    pub fn last_fault(&self) -> Option<&str> {
        self.last_fault.as_deref()
    }

    /// Whether three faults in a row have switched the script off.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Lists every uniform the script has published so far as
    /// `(name, type)`, for compiling the shader before the first frame, and
    /// marks them known: the next [`FrameOutput::new_uniforms`] reports only
    /// uniforms that appear after this call.
    pub fn announce_extras(&mut self) -> Vec<(String, strobe_scene::assemble::ExtraType)> {
        self.shared.state.borrow_mut().uniforms.announce()
    }

    /// Copies the final-pass downsample where `textureFinalPass` reads it.
    fn stage_final_pass(&self, inputs: &ScriptInputs<'_>) {
        let mut state = self.shared.state.borrow_mut();
        state.final_pass.clear();
        if let Some(pixels) = inputs.final_pass
            && pixels.len() == FINAL_PASS_BYTES
        {
            state.final_pass.extend_from_slice(pixels);
        }
    }

    /// The scripted part of a frame. Returns the fault text on failure.
    fn run<'js>(
        &self,
        ctx: &Ctx<'js>,
        dt: f32,
        inputs: &ScriptInputs<'_>,
        controls: &ControlBank,
    ) -> Result<(), String> {
        self.globals
            .write(ctx, inputs, controls)
            .map_err(|error| describe(ctx, error))?;
        self.dispatch(ctx, inputs, controls)?;

        let globals = ctx.globals();
        let update: Value<'js> = globals
            .get("update")
            .map_err(|error| describe(ctx, error))?;
        if let Some(function) = update.as_function() {
            function
                .call::<_, ()>((f64::from(dt),))
                .map_err(|error| describe(ctx, error))?;
        }
        self.harvest_legacy_uniforms(ctx);
        Ok(())
    }

    /// Calls the callback of every handler whose condition holds.
    fn dispatch<'js>(
        &self,
        ctx: &Ctx<'js>,
        inputs: &ScriptInputs<'_>,
        controls: &ControlBank,
    ) -> Result<(), String> {
        let globals = ctx.globals();
        let mut position = 0;
        loop {
            // The borrow ends before the callback runs: the callback may
            // register handlers or write controls, which borrow again.
            let due = {
                let mut state = self.shared.state.borrow_mut();
                let Some(handler) = state.handlers.get_mut(position) else {
                    break;
                };
                let value = source_value(handler.source, inputs, controls);
                handler
                    .step(value)
                    .map(|previous| (Rc::clone(&handler.callback), value, previous))
            };
            position += 1;
            let Some((callback, value, previous)) = due else {
                continue;
            };
            let function: Value<'js> = globals
                .get(&*callback)
                .map_err(|error| describe(ctx, error))?;
            if let Some(function) = function.as_function() {
                function
                    .call::<_, ()>((f64::from(value), f64::from(previous)))
                    .map_err(|error| describe(ctx, error))?;
            }
        }
        Ok(())
    }

    /// Scans the deprecated `uniforms` object for values to publish.
    ///
    /// A bad entry is reported once and skipped; it does not fault the
    /// frame, because the rest of the object is still good.
    fn harvest_legacy_uniforms<'js>(&self, ctx: &Ctx<'js>) {
        let Ok(object) = ctx.globals().get::<_, Object<'js>>("uniforms") else {
            // Replaced by something that is not an object; nothing to scan.
            let _ = ctx.catch();
            return;
        };
        for entry in object.props::<String, Value<'js>>() {
            let Ok((name, value)) = entry else {
                let _ = ctx.catch();
                continue;
            };
            if let Err(error) = stage_uniform(ctx, &self.shared, &name, &[value]) {
                let message = describe(ctx, error);
                self.shared
                    .state
                    .borrow_mut()
                    .log_once(ConsoleLevel::Error, &format!("uniforms.{name}: {message}"));
            }
        }
    }

    /// Records a fault and decides whether it is the last.
    fn fault(&mut self, message: &str) -> ScriptStatus {
        let mut state = self.shared.state.borrow_mut();
        state.uniforms.discard();
        self.consecutive_faults += 1;
        state.log(ConsoleLevel::Error, format!("{SCRIPT_NAME}: {message}"));
        self.last_fault = Some(message.to_string());
        if self.consecutive_faults >= MAX_CONSECUTIVE_FAULTS {
            self.disabled = true;
            state.log(
                ConsoleLevel::Error,
                format!(
                    "{SCRIPT_NAME} disabled after {MAX_CONSECUTIVE_FAULTS} faulted frames in a \
                     row; reload the scene to run it again"
                ),
            );
            ScriptStatus::Disabled
        } else {
            ScriptStatus::Faulted {
                consecutive: self.consecutive_faults,
            }
        }
    }

    /// Applies the control writes queued during the frame, in order, each
    /// to the bank it was aimed at.
    fn apply_commands(&self, controls: &mut ControlBank, meta: &mut ControlBank) {
        let mut state = self.shared.state.borrow_mut();
        let mut commands = std::mem::take(&mut state.commands);
        for command in commands.drain(..) {
            match command {
                Command::Set {
                    target,
                    index,
                    dimension,
                    values,
                    len,
                    normalized,
                } => {
                    let values = values.get(..len).unwrap_or(&values);
                    if normalized {
                        pick(target, controls, meta).set_normalized(index, dimension, values);
                    } else {
                        pick(target, controls, meta).set(index, dimension, values);
                    }
                }
                Command::Randomize(target, index) => {
                    pick(target, controls, meta).randomize(index, || state.next_unit());
                }
                Command::Default(target, index) => pick(target, controls, meta).set_default(index),
            }
        }
        state.commands = commands;
    }

    /// Collects the frame's output.
    fn finish(&self, status: ScriptStatus) -> FrameOutput {
        let mut state = self.shared.state.borrow_mut();
        FrameOutput {
            uniforms: state.uniforms.snapshot(),
            new_uniforms: state.uniforms.take_fresh(),
            console: std::mem::take(&mut state.console),
            status,
        }
    }
}
