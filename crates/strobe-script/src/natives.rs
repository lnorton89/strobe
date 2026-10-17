// ============================================================================
// Module:       strobe_script::natives
// Description:  The functions the Synesthesia API gives a script: setUniform,
//               setControl, the randomize/default family, events, printing.
//
// Dependencies: rquickjs, strobe_scene::names, crate::{shared, events, output}
// ============================================================================

//! Native functions.
//!
//! Each function here is installed as a JavaScript global by [`install`]
//! and talks to the host only through [`Shared`]. They never touch the
//! control bank directly (see the `shared` module for why); a control write
//! is a queued [`Command`], a uniform write is staged in the
//! [`crate::uniforms::UniformStore`]. A meta-control write (`meta/...` or
//! `bank: "meta"`) is the same queued [`Command`] aimed at the meta bank.
//!
//! ## Errors
//!
//! Two kinds, deliberately different. A *malformed call* (a non-number
//! where a number is required, five components, a uniform named `TIME`)
//! throws a JavaScript `TypeError`, so the script's author gets a line
//! number and a script that expects it can catch it. A *name that matches
//! nothing* (a control that was renamed, a callback that was never
//! defined) logs one console error and carries on, because scripts are
//! shared between scene variants and a missing control should not take the
//! whole script down; "one" matters because the call may sit in `update`
//! and run sixty times a second.

use crate::events::{EventKind, Handler};
use crate::inputs::{FINAL_PASS_BYTES, FINAL_PASS_HEIGHT, FINAL_PASS_WIDTH};
use crate::output::ConsoleLevel;
use crate::shared::{Command, Shared, Target};
use crate::uniforms::SetOutcome;
use rquickjs::function::{Opt, Rest};
use rquickjs::object::Property;
use rquickjs::{Coerced, Ctx, Exception, FromJs, Function, Object, Result, Value};
use std::collections::HashMap;
use std::rc::Rc;
use strobe_scene::names::{is_glsl_identifier, is_reserved, normalize};

/// The hidden global object holding the event callbacks the global object
/// does not carry: a function declared with a top-level `let` or `const`,
/// under its name, and a function passed by value, under `#<n>` (D414). It
/// is not enumerable, so `printGlobalKeys` and `for (k in globalThis)` do
/// not list it, and not writable, so a script cannot replace it.
pub(crate) const CALLBACKS: &str = "__strobeCallbacks";

/// Most distinct functions a script may pass by value as callbacks. One
/// that registers a fresh closure every frame would otherwise keep every
/// one of them alive; past this it is told to pass the same function, or a
/// name.
const MAX_FUNCTION_CALLBACKS: usize = 64;

/// Most components any script value is read as: a `vec4`.
const MAX_UNIFORM_COMPONENTS: usize = 4;

/// Most components a control takes: a colour's three.
const MAX_CONTROL_COMPONENTS: usize = 3;

/// Component names an object may use, tried in order: position first,
/// then colour.
const COMPONENT_KEYS: [[&str; 4]; 2] = [["x", "y", "z", "w"], ["r", "g", "b", "a"]];

/// Up to four numbers read from a script value.
#[derive(Debug, Clone, Copy, Default)]
struct Components {
    values: [f32; MAX_UNIFORM_COMPONENTS],
    len: usize,
}

impl Components {
    /// The numbers read, in order.
    fn slice(&self) -> &[f32] {
        self.values.get(..self.len).unwrap_or(&[])
    }
}

/// A script number as a finite `f32`. Booleans count as 0 and 1, since
/// toggles are numbers in SSF and `setControl("t", true)` is a natural
/// thing to write. NaN and infinities become zero: a uniform full of NaN
/// is a black screen with no explanation. So does a double past
/// `f32::MAX`, which the cast would turn into an infinity.
fn to_f32<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<f32> {
    if let Some(number) = value.as_number() {
        let single = number as f32;
        return Ok(if single.is_finite() { single } else { 0.0 });
    }
    if let Some(flag) = value.as_bool() {
        return Ok(if flag { 1.0 } else { 0.0 });
    }
    Err(Exception::throw_type(ctx, "expected a number"))
}

/// Appends one component, enforcing the limit.
fn push<'js>(
    ctx: &Ctx<'js>,
    out: &mut Components,
    value: &Value<'js>,
    max: usize,
    truncate: bool,
) -> Result<()> {
    if out.len >= max {
        return if truncate {
            Ok(())
        } else {
            Err(Exception::throw_type(
                ctx,
                &format!("expected at most {max} components"),
            ))
        };
    }
    let number = to_f32(ctx, value)?;
    if let Some(slot) = out.values.get_mut(out.len) {
        *slot = number;
    }
    out.len += 1;
    Ok(())
}

/// Reads the numbers a call carries, in any of the forms the API allows:
/// loose numbers, one array, or one `{x,y,z,w}` / `{r,g,b,a}` object.
///
/// With `truncate`, components past `max` are ignored instead of an error,
/// which is what `setControl` wants when handed a `vec4` colour.
fn read_components<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    max: usize,
    truncate: bool,
) -> Result<Components> {
    let mut out = Components::default();
    let single = match args {
        [only] if !only.is_number() && !only.is_bool() => Some(only),
        _ => None,
    };
    match single {
        Some(value) => {
            if let Some(array) = value.as_array() {
                // One past `max` is enough to know there are too many; the
                // watchdog cannot stop a native loop over a huge array.
                for index in 0..array.len().min(max + 1) {
                    push(
                        ctx,
                        &mut out,
                        &array.get::<Value<'js>>(index)?,
                        max,
                        truncate,
                    )?;
                }
            } else if let Some(object) = value.as_object() {
                read_object(ctx, object, &mut out, max, truncate)?;
            } else {
                return Err(Exception::throw_type(
                    ctx,
                    "expected numbers, an array, or an {x,y,z,w} / {r,g,b,a} object",
                ));
            }
        }
        None => {
            for value in args {
                push(ctx, &mut out, value, max, truncate)?;
            }
        }
    }
    if out.len == 0 {
        return Err(Exception::throw_type(ctx, "expected at least one number"));
    }
    Ok(out)
}

/// Reads an `{x,y,z,w}` or `{r,g,b,a}` object, or any array-like (a typed
/// array, say), as components.
fn read_object<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    out: &mut Components,
    max: usize,
    truncate: bool,
) -> Result<()> {
    for keys in COMPONENT_KEYS {
        let mut found = 0;
        for key in keys {
            if !object.contains_key(key)? {
                break;
            }
            push(ctx, out, &object.get::<_, Value<'js>>(key)?, max, truncate)?;
            found += 1;
        }
        if found > 0 {
            return Ok(());
        }
    }
    let length: Option<u32> = object.get("length")?;
    for index in 0..length.unwrap_or(0).min(MAX_UNIFORM_COMPONENTS as u32 + 1) {
        push(
            ctx,
            out,
            &object.get::<_, Value<'js>>(index)?,
            max,
            truncate,
        )?;
    }
    Ok(())
}

/// Which bank of controls a call targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bank {
    Scene,
    Meta,
}

/// The options object of `setControl`.
#[derive(Debug, Clone, Copy, Default)]
struct Options {
    dimension: Option<usize>,
    bank: Option<Bank>,
}

/// Reads `{ dimension, bank }`; anything that is not an object is no
/// options.
fn read_options<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>) -> Result<Options> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Ok(Options::default());
    };
    let dimension = match object.get::<_, Option<f64>>("dimension")? {
        Some(d) if d.is_finite() && d >= 0.0 => Some(d as usize),
        Some(_) => {
            return Err(Exception::throw_type(
                ctx,
                "`dimension` must be a non-negative number",
            ));
        }
        None => None,
    };
    let bank = match object.get::<_, Option<String>>("bank")? {
        None => None,
        Some(text) => match text.trim().to_ascii_lowercase().as_str() {
            "meta" => Some(Bank::Meta),
            "scene" => Some(Bank::Scene),
            other => {
                return Err(Exception::throw_type(
                    ctx,
                    &format!("`bank` must be \"scene\" or \"meta\", not \"{other}\""),
                ));
            }
        },
    };
    Ok(Options { dimension, bank })
}

/// Splits a leading `meta/` off a name. Returns whether it was there and
/// the rest.
fn split_meta(name: &str) -> (bool, &str) {
    let name = name.trim();
    match name.get(..5) {
        Some(prefix) if prefix.eq_ignore_ascii_case("meta/") => {
            (true, name.get(5..).unwrap_or("").trim())
        }
        Some(_) | None => (false, name),
    }
}

/// The name tables and queue target for `bank`.
fn tables(
    shared: &Shared,
    bank: Bank,
) -> (
    &HashMap<String, usize>,
    &HashMap<String, Vec<usize>>,
    Target,
) {
    match bank {
        Bank::Scene => (&shared.names.controls, &shared.names.groups, Target::Scene),
        Bank::Meta => (
            &shared.names.meta_controls,
            &shared.names.meta_groups,
            Target::Meta,
        ),
    }
}

/// What a name that matched nothing is called in a console message.
fn noun(bank: Bank) -> &'static str {
    match bank {
        Bank::Scene => "control",
        Bank::Meta => "meta control",
    }
}

/// `setControl` and `setControlNormalized`.
fn control_write<'js>(
    ctx: &Ctx<'js>,
    shared: &Shared,
    function: &str,
    name: &str,
    value: &Value<'js>,
    options: Option<&Value<'js>>,
    normalized: bool,
) -> Result<()> {
    let options = read_options(ctx, options)?;
    let (prefixed, bare) = split_meta(name);
    let bank = options
        .bank
        .unwrap_or(if prefixed { Bank::Meta } else { Bank::Scene });
    let components = read_components(
        ctx,
        std::slice::from_ref(value),
        MAX_CONTROL_COMPONENTS,
        true,
    )?;
    let mut state = shared.state.borrow_mut();
    let (controls, _, target) = tables(shared, bank);
    match controls.get(&normalize(bare)) {
        Some(&index) => {
            let mut values = [0.0_f32; MAX_CONTROL_COMPONENTS];
            for (slot, source) in values.iter_mut().zip(components.slice()) {
                *slot = *source;
            }
            state.commands.push(Command::Set {
                target,
                index,
                dimension: options.dimension,
                values,
                len: components.len.min(MAX_CONTROL_COMPONENTS),
                normalized,
            });
        }
        None => state.log_once(
            ConsoleLevel::Warn,
            &format!("{function}: no {} named `{bare}`", noun(bank)),
        ),
    }
    Ok(())
}

/// What `randomizeControl` / `defaultControl` (and their group forms) do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Randomize,
    Default,
}

/// `randomizeControl` and `defaultControl`.
fn control_operation(shared: &Shared, function: &str, name: &str, operation: Operation) {
    let (prefixed, bare) = split_meta(name);
    let bank = if prefixed { Bank::Meta } else { Bank::Scene };
    let (controls, _, target) = tables(shared, bank);
    let mut state = shared.state.borrow_mut();
    match controls.get(&normalize(bare)) {
        Some(&index) => state.commands.push(match operation {
            Operation::Randomize => Command::Randomize(target, index),
            Operation::Default => Command::Default(target, index),
        }),
        None => state.log_once(
            ConsoleLevel::Warn,
            &format!("{function}: no {} named `{bare}`", noun(bank)),
        ),
    }
}

/// `randomizeGroup` and `defaultGroup`.
fn group_operation(shared: &Shared, function: &str, name: &str, operation: Operation) {
    let (prefixed, bare) = split_meta(name);
    let bank = if prefixed { Bank::Meta } else { Bank::Scene };
    let (_, groups, target) = tables(shared, bank);
    let mut state = shared.state.borrow_mut();
    match groups.get(&normalize(bare)) {
        Some(members) => {
            for &index in members {
                state.commands.push(match operation {
                    Operation::Randomize => Command::Randomize(target, index),
                    Operation::Default => Command::Default(target, index),
                });
            }
        }
        None => state.log_once(
            ConsoleLevel::Warn,
            &format!("{function}: no {} group named `{bare}`", noun(bank)),
        ),
    }
}

/// `onChange` and friends: validate once, register once.
///
/// `callback` is a function's name or the function itself. A name is
/// looked up on the global object when the handler fires, as before; one
/// that is not a property there (a top-level `let` or `const`, D326) is
/// resolved once, here, and kept in [`CALLBACKS`] under its name. A
/// function value is kept there too, once per distinct function, so a
/// script that registers the same one every frame still has one handler
/// (D414). Anything else is a malformed call and throws.
fn register_handler<'js>(
    ctx: &Ctx<'js>,
    shared: &Shared,
    function: &str,
    kind: EventKind,
    target: &str,
    callback: &Value<'js>,
) -> Result<()> {
    let Some(&source) = shared.names.sources.get(&normalize(target)) else {
        shared.state.borrow_mut().log_once(
            ConsoleLevel::Warn,
            &format!("{function}: no input named `{target}`"),
        );
        return Ok(());
    };
    let Some(key) = callback_key(ctx, shared, function, target, callback)? else {
        return Ok(());
    };
    let handler = Handler::new(kind, source, Rc::from(key));
    let mut state = shared.state.borrow_mut();
    if !state
        .handlers
        .iter()
        .any(|existing| existing.same_as(&handler))
    {
        state.handlers.push(handler);
    }
    Ok(())
}

/// The name a handler finds `callback` by when it fires: on the global
/// object first, then in [`CALLBACKS`]. `None`, after a console error,
/// when there is nothing to call.
fn callback_key<'js>(
    ctx: &Ctx<'js>,
    shared: &Shared,
    function: &str,
    target: &str,
    callback: &Value<'js>,
) -> Result<Option<String>> {
    if let Some(name) = callback.as_string() {
        let name = name.to_string()?;
        if ctx
            .globals()
            .get::<_, Value<'js>>(name.as_str())?
            .is_function()
        {
            return Ok(Some(name));
        }
        let kept = callbacks(ctx)?;
        if kept.get::<_, Value<'js>>(name.as_str())?.is_function() {
            return Ok(Some(name));
        }
        if let Some(lexical) = lexical_function(ctx, &name) {
            kept.set(name.as_str(), lexical)?;
            return Ok(Some(name));
        }
        shared.state.borrow_mut().log_once(
            ConsoleLevel::Error,
            &format!("{function}(`{target}`): there is no function named `{name}`"),
        );
        return Ok(None);
    }
    if !callback.is_function() {
        return Err(Exception::throw_type(
            ctx,
            &format!("{function}(`{target}`): the callback must be a function or the name of one"),
        ));
    }
    let kept = callbacks(ctx)?;
    // The count is read and written in separate borrows: reading the
    // registry runs no script code, but nothing here holds the state
    // across a call into the engine either way.
    let count = shared.state.borrow().function_callbacks;
    for slot in 0..count {
        let key = format!("#{slot}");
        if kept.get::<_, Value<'js>>(key.as_str())? == *callback {
            return Ok(Some(key));
        }
    }
    if count >= MAX_FUNCTION_CALLBACKS {
        shared.state.borrow_mut().log_once(
            ConsoleLevel::Error,
            &format!(
                "{function}(`{target}`): more than {MAX_FUNCTION_CALLBACKS} different functions \
                 passed as callbacks; pass the same function each time, or its name"
            ),
        );
        return Ok(None);
    }
    let key = format!("#{count}");
    kept.set(key.as_str(), callback.clone())?;
    shared.state.borrow_mut().function_callbacks = count + 1;
    Ok(Some(key))
}

/// The [`CALLBACKS`] object.
pub(crate) fn callbacks<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    ctx.globals().get(CALLBACKS)
}

/// The function a top-level `let` or `const` named `name` holds, if any.
/// Only a plain identifier is evaluated, so a callback "name" can never be
/// code; an evaluation that throws (a reserved word) finds nothing.
fn lexical_function<'js>(ctx: &Ctx<'js>, name: &str) -> Option<Function<'js>> {
    let mut chars = name.chars();
    let starts = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '_' | '$'));
    if !starts || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$')) {
        return None;
    }
    match ctx.eval::<Value<'js>, _>(format!("typeof {name} === 'function' ? {name} : undefined")) {
        Ok(value) => value.as_function().cloned(),
        Err(_) => {
            let _ = ctx.catch();
            None
        }
    }
}

/// `setUniform(name, ...)`.
fn set_uniform<'js>(ctx: &Ctx<'js>, shared: &Shared, args: &[Value<'js>]) -> Result<()> {
    let Some((name, rest)) = args.split_first() else {
        return Err(Exception::throw_type(ctx, "setUniform: expected a name"));
    };
    let Some(name) = name.as_string() else {
        return Err(Exception::throw_type(
            ctx,
            "setUniform: the name must be a string",
        ));
    };
    stage_uniform(ctx, shared, &name.to_string()?, rest)
}

/// Stages a uniform write from `components` (loose numbers, one array, or
/// one object). Shared by `setUniform` and the deprecated `uniforms`
/// object, so both accept the same names and values.
pub(crate) fn stage_uniform<'js>(
    ctx: &Ctx<'js>,
    shared: &Shared,
    name: &str,
    components: &[Value<'js>],
) -> Result<()> {
    if !is_glsl_identifier(name) || is_reserved(name) || shared.names.control_exact.contains(name) {
        return Err(Exception::throw_type(
            ctx,
            &format!(
                "setUniform: `{name}` cannot be a script uniform (it is not a valid GLSL name, \
                 or the scene already declares it)"
            ),
        ));
    }
    let components = read_components(ctx, components, MAX_UNIFORM_COMPONENTS, false)?;
    let mut state = shared.state.borrow_mut();
    match state.uniforms.set(name, components.slice()) {
        SetOutcome::Stored => {}
        SetOutcome::Coerced(ty) => state.log_once(
            ConsoleLevel::Warn,
            &format!(
                "setUniform: `{name}` was first set as a {} and keeps that type; a different \
                 number of components was padded or truncated",
                ty.glsl()
            ),
        ),
        SetOutcome::Refused => state.log_once(
            ConsoleLevel::Error,
            &format!(
                "setUniform: `{name}` was created after the scene loaded, so the shaders were \
                 compiled without it and it is ignored; create every uniform in setup() or in \
                 the first update() (a show never compiles shaders while it plays)"
            ),
        ),
    }
    Ok(())
}

/// One console argument as text.
pub(crate) fn display<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> String {
    if let Some(text) = value.as_string() {
        return text.to_string().unwrap_or_default();
    }
    if value.is_object()
        && !value.is_function()
        && !value.is_error()
        && let Ok(Some(json)) = ctx.json_stringify(value.clone())
    {
        return json.to_string().unwrap_or_default();
    }
    Coerced::<String>::from_js(ctx, value.clone())
        .map(|text| text.0)
        .unwrap_or_else(|_| "<unprintable>".to_string())
}

/// The console arguments joined with spaces.
fn display_all<'js>(ctx: &Ctx<'js>, args: &[Value<'js>]) -> String {
    args.iter()
        .map(|value| display(ctx, value))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A printing function logging at `level`.
fn printer<'js>(ctx: &Ctx<'js>, shared: &Rc<Shared>, level: ConsoleLevel) -> Result<Function<'js>> {
    let shared = Rc::clone(shared);
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> Result<()> {
            let text = display_all(&ctx, &args.0);
            shared.state.borrow_mut().log(level, text);
            Ok(())
        },
    )
}

/// `setControl` or `setControlNormalized`.
fn control_setter<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<Shared>,
    function: &'static str,
    normalized: bool,
) -> Result<Function<'js>> {
    let shared = Rc::clone(shared);
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              name: String,
              value: Value<'js>,
              options: Opt<Value<'js>>|
              -> Result<()> {
            control_write(
                &ctx,
                &shared,
                function,
                &name,
                &value,
                options.0.as_ref(),
                normalized,
            )
        },
    )
}

/// `randomizeControl` or `defaultControl`.
fn control_operator<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<Shared>,
    function: &'static str,
    operation: Operation,
) -> Result<Function<'js>> {
    let shared = Rc::clone(shared);
    Function::new(ctx.clone(), move |name: String| {
        control_operation(&shared, function, &name, operation);
    })
}

/// `randomizeGroup` or `defaultGroup`.
fn group_operator<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<Shared>,
    function: &'static str,
    operation: Operation,
) -> Result<Function<'js>> {
    let shared = Rc::clone(shared);
    Function::new(ctx.clone(), move |name: String| {
        group_operation(&shared, function, &name, operation);
    })
}

/// One of the five event registration functions.
fn event_registrar<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<Shared>,
    function: &'static str,
    kind: EventKind,
) -> Result<Function<'js>> {
    let shared = Rc::clone(shared);
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, target: String, callback: Value<'js>| -> Result<()> {
            register_handler(&ctx, &shared, function, kind, &target, &callback)
        },
    )
}

/// The index of the cell `t` (a coordinate in `0..1`) falls in, out of `n`.
fn cell(t: f64, n: usize) -> usize {
    if !t.is_finite() {
        return 0;
    }
    ((t.clamp(0.0, 1.0) * n as f64) as usize).min(n.saturating_sub(1))
}

/// `textureFinalPass(x, y)`: nearest-texel colour of a recent frame, as
/// `[r, g, b]` in `0..1`. `x` and `y` are `0..1` across the picture, `y`
/// up, and clamp to the edge. The colour comes from the host's 64x36
/// asynchronous readback, so it lags the picture by a few frames and is
/// black until the first one lands (D162).
fn texture_final_pass(shared: &Shared, x: f64, y: f64) -> Vec<f64> {
    let state = shared.state.borrow();
    if state.final_pass.len() != FINAL_PASS_BYTES {
        return vec![0.0; 3];
    }
    let offset = (cell(y, FINAL_PASS_HEIGHT) * FINAL_PASS_WIDTH + cell(x, FINAL_PASS_WIDTH)) * 3;
    match state.final_pass.get(offset..offset + 3) {
        Some(rgb) => rgb.iter().map(|&byte| f64::from(byte) / 255.0).collect(),
        None => vec![0.0; 3],
    }
}

/// `printGlobalKeys()`: the names a script can use, split into values and
/// functions.
fn print_global_keys(ctx: &Ctx<'_>, shared: &Shared) {
    let mut values = Vec::new();
    let mut functions = Vec::new();
    for entry in ctx.globals().props::<String, Value<'_>>() {
        let Ok((key, value)) = entry else {
            continue;
        };
        if value.is_function() {
            functions.push(key);
        } else {
            values.push(key);
        }
    }
    values.sort();
    functions.sort();
    let mut state = shared.state.borrow_mut();
    state.log(ConsoleLevel::Log, format!("globals: {}", values.join(", ")));
    state.log(
        ConsoleLevel::Log,
        format!("functions: {}", functions.join(", ")),
    );
}

/// Installs every native function as a global of `ctx`.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, shared: &Rc<Shared>) -> Result<()> {
    let globals = ctx.globals();

    let set_uniform_shared = Rc::clone(shared);
    globals.set(
        "setUniform",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> Result<()> {
                set_uniform(&ctx, &set_uniform_shared, &args.0)
            },
        )?,
    )?;

    globals.set(
        "setControl",
        control_setter(ctx, shared, "setControl", false)?,
    )?;
    globals.set(
        "setControlNormalized",
        control_setter(ctx, shared, "setControlNormalized", true)?,
    )?;
    globals.set(
        "randomizeControl",
        control_operator(ctx, shared, "randomizeControl", Operation::Randomize)?,
    )?;
    globals.set(
        "defaultControl",
        control_operator(ctx, shared, "defaultControl", Operation::Default)?,
    )?;
    globals.set(
        "randomizeGroup",
        group_operator(ctx, shared, "randomizeGroup", Operation::Randomize)?,
    )?;
    globals.set(
        "defaultGroup",
        group_operator(ctx, shared, "defaultGroup", Operation::Default)?,
    )?;

    // No prototype, so a name such as `toString` finds nothing inherited.
    let kept = Object::new(ctx.clone())?;
    kept.set_prototype(None)?;
    globals.prop(CALLBACKS, Property::from(kept))?;
    for (function, kind) in [
        ("onChange", EventKind::Change),
        ("onOffToOn", EventKind::OffToOn),
        ("onOnToOff", EventKind::OnToOff),
        ("whileOn", EventKind::WhileOn),
        ("whileOff", EventKind::WhileOff),
    ] {
        globals.set(function, event_registrar(ctx, shared, function, kind)?)?;
    }

    let pixel_shared = Rc::clone(shared);
    globals.set(
        "textureFinalPass",
        Function::new(ctx.clone(), move |x: f64, y: f64| {
            texture_final_pass(&pixel_shared, x, y)
        })?,
    )?;

    globals.set("print", printer(ctx, shared, ConsoleLevel::Log)?)?;
    let console = Object::new(ctx.clone())?;
    console.set("log", printer(ctx, shared, ConsoleLevel::Log)?)?;
    console.set("info", printer(ctx, shared, ConsoleLevel::Log)?)?;
    console.set("debug", printer(ctx, shared, ConsoleLevel::Log)?)?;
    console.set("warn", printer(ctx, shared, ConsoleLevel::Warn)?)?;
    console.set("error", printer(ctx, shared, ConsoleLevel::Error)?)?;
    globals.set("console", console)?;

    let keys_shared = Rc::clone(shared);
    globals.set(
        "printGlobalKeys",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            print_global_keys(&ctx, &keys_shared);
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_is_a_case_insensitive_prefix() {
        assert_eq!(split_meta("meta/brightness"), (true, "brightness"));
        assert_eq!(split_meta("Meta/Hue"), (true, "Hue"));
        assert_eq!(split_meta("brightness"), (false, "brightness"));
        assert_eq!(split_meta("meta"), (false, "meta"), "no slash, no prefix");
    }

    #[test]
    fn texture_coordinates_clamp_to_the_grid() {
        assert_eq!(cell(0.0, 64), 0, "left edge");
        assert_eq!(cell(1.0, 64), 63, "right edge stays inside");
        assert_eq!(cell(-3.0, 64), 0, "below clamps");
        assert_eq!(cell(f64::NAN, 64), 0, "NaN is the origin");
    }
}
