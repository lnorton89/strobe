// ============================================================================
// Module:       strobe_script::conformance
// Description:  One test per Synesthesia API surface, run headless against
//               a real QuickJS runtime and a real control bank.
//
// Dependencies: anyhow, serde_json; strobe_scene (control bank fixtures)
// ============================================================================

//! Conformance tests.
//!
//! Each test writes a small `script.js`, builds a real
//! [`strobe_scene::control::ControlBank`] from `scene.json`-style control
//! objects, and drives [`ScriptHost`] frame by frame the way the app will:
//! `frame`, then `tick`. Assertions are on the harvested output (the
//! uniform list, console, meta writes, status) and on the bank, never on
//! pixels, so nothing here needs a GPU.
//!
//! Most tests run under a generous time budget so that a loaded CI machine
//! cannot turn a slow first frame into a spurious fault; the watchdog tests
//! use the 4 ms default on purpose.

use crate::{
    Budget, ConsoleLevel, FINAL_PASS_BYTES, FINAL_PASS_HEIGHT, FINAL_PASS_WIDTH, FrameOutput,
    ScriptError, ScriptHost, ScriptInputs, ScriptStatus, ScriptUniform,
};
use anyhow::{Result, anyhow};
use std::time::{Duration, Instant};
use strobe_scene::assemble::ExtraType;
use strobe_scene::audio::SCALARS;
use strobe_scene::control::{ControlBank, ControlDef};
use strobe_scene::json::RawControl;

/// One frame at 60 fps.
const DT: f32 = 1.0 / 60.0;

/// Builds a bank from `scene.json` control objects.
fn bank(controls: &[&str]) -> Result<ControlBank> {
    let mut defs = Vec::new();
    for json in controls {
        let raw: RawControl = serde_json::from_str(json)?;
        defs.push(ControlDef::from_raw(&raw)?);
    }
    Ok(ControlBank::new(defs)?)
}

/// A budget that a loaded machine will not trip by accident.
fn relaxed() -> Budget {
    Budget {
        frame: Duration::from_secs(2),
        load: Duration::from_secs(5),
        ..Budget::default()
    }
}

/// A host, its bank, and a frame counter.
struct Rig {
    host: ScriptHost,
    bank: ControlBank,
    meta: ControlBank,
    frame: u64,
}

impl Rig {
    fn new(script: &str, controls: &[&str]) -> Result<Self> {
        Self::with_budget(script, controls, relaxed())
    }

    fn with_budget(script: &str, controls: &[&str], budget: Budget) -> Result<Self> {
        let bank = bank(controls)?;
        let meta = strobe_scene::meta::bank()?;
        let host = ScriptHost::new(script, &bank, &meta, budget)?;
        Ok(Self {
            host,
            bank,
            meta,
            frame: 0,
        })
    }

    /// One frame with the given inputs, then the bank tick, as the app does.
    fn step_with(&mut self, inputs: &ScriptInputs<'_>) -> FrameOutput {
        let inputs = ScriptInputs {
            frame_count: self.frame,
            ..*inputs
        };
        let output = self.host.frame(DT, &inputs, &mut self.bank, &mut self.meta);
        self.bank.tick(DT);
        self.meta.tick(DT);
        self.frame += 1;
        output
    }

    fn step(&mut self) -> FrameOutput {
        self.step_with(&ScriptInputs::default())
    }

    /// One frame at `TIME == time`.
    fn at_time(&mut self, time: f32) -> FrameOutput {
        self.step_with(&ScriptInputs {
            time,
            ..ScriptInputs::default()
        })
    }

    fn index(&self, name: &str) -> Result<usize> {
        self.bank
            .find(name)
            .ok_or_else(|| anyhow!("no control `{name}`"))
    }

    /// A control's target, first `dims` components.
    fn target(&self, name: &str, dims: usize) -> Result<Vec<f32>> {
        let values = self
            .bank
            .target(self.index(name)?)
            .ok_or_else(|| anyhow!("no target for `{name}`"))?;
        Ok(values.iter().take(dims).copied().collect())
    }

    /// A control's output, first `dims` components.
    fn output(&self, name: &str, dims: usize) -> Result<Vec<f32>> {
        let values = self
            .bank
            .output(self.index(name)?)
            .ok_or_else(|| anyhow!("no output for `{name}`"))?;
        Ok(values.iter().take(dims).copied().collect())
    }

    fn set_target(&mut self, name: &str, values: &[f32]) -> Result<()> {
        let index = self.index(name)?;
        self.bank.set(index, None, values);
        Ok(())
    }
}

fn uniform<'a>(output: &'a FrameOutput, name: &str) -> Result<&'a ScriptUniform> {
    output
        .uniforms
        .iter()
        .find(|uniform| &*uniform.name == name)
        .ok_or_else(|| {
            let known: Vec<&str> = output.uniforms.iter().map(|u| &*u.name).collect();
            anyhow!("no uniform `{name}`; have {known:?}")
        })
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

/// Asserts a uniform's type and its first `expected.len()` components.
fn expect_uniform(output: &FrameOutput, name: &str, ty: ExtraType, expected: &[f32]) -> Result<()> {
    let found = uniform(output, name)?;
    assert_eq!(found.ty, ty, "type of `{name}`");
    for (index, want) in expected.iter().enumerate() {
        let got = found.value.get(index).copied().unwrap_or(f32::NAN);
        assert!(
            near(got, *want),
            "`{name}` component {index}: got {got}, wanted {want}"
        );
    }
    Ok(())
}

fn errors(output: &FrameOutput) -> Vec<&str> {
    at_level(output, ConsoleLevel::Error)
}

/// The console lines at `level`.
fn at_level(output: &FrameOutput, level: ConsoleLevel) -> Vec<&str> {
    output
        .console
        .iter()
        .filter(|line| line.level == level)
        .map(|line| line.text.as_str())
        .collect()
}

/// Audio scalars all silent except `name`.
fn audio_with(name: &str, value: f32) -> Result<Vec<f32>> {
    let mut audio: Vec<f32> = SCALARS
        .iter()
        .map(|n| strobe_scene::audio::silent_value(n))
        .collect();
    let index = SCALARS
        .iter()
        .position(|n| *n == name)
        .ok_or_else(|| anyhow!("`{name}` is not an audio scalar"))?;
    if let Some(slot) = audio.get_mut(index) {
        *slot = value;
    }
    Ok(audio)
}

const SLIDER: &str = r#"{ "NAME": "mySlider", "TYPE": "slider", "MIN": 0, "MAX": 10 }"#;
const PAD: &str =
    r#"{ "NAME": "myXyPad", "TYPE": "xy", "MIN": -1, "MAX": 1, "DEFAULT": [0.25, 0.75] }"#;
const COLOR: &str = r#"{ "NAME": "myColor", "TYPE": "color", "DEFAULT": [0.1, 0.5, 0.0] }"#;

// ---- setUniform -----------------------------------------------------------

#[test]
fn set_uniform_infers_its_type_from_the_arity_in_every_form() -> Result<()> {
    let script = r#"
        function update(dt) {
            setUniform("float1", 0.5);
            setUniform("vec2_args", 0.5, 0.3);
            setUniform("vec2_array", [0.5, 0.3]);
            setUniform("vec2_xy", { x: 0.5, y: 0.3 });
            setUniform("vec3_args", 0.1, 0.0, 1.0);
            setUniform("vec3_array", [0.1, 0.0, 1.0]);
            setUniform("vec3_rgb", { r: 0.1, g: 0.0, b: 1.0 });
            setUniform("vec3_xyz", { x: 0.1, y: 0.0, z: 1.0 });
            setUniform("vec4_args", 0.1, 0.0, 1.0, 0.25);
            setUniform("vec4_array", [0.1, 0.0, 1.0, 0.25]);
            setUniform("vec4_rgba", { r: 0.1, g: 0.0, b: 1.0, a: 0.25 });
            setUniform("vec4_xyzw", { x: 0.1, y: 0.0, z: 1.0, w: 0.25 });
            setUniform("from_bool", true);
            setUniform("typed", new Float32Array([0.5, 0.25]));
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let out = rig.step();
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);

    expect_uniform(&out, "float1", ExtraType::Float, &[0.5])?;
    for name in ["vec2_args", "vec2_array", "vec2_xy"] {
        expect_uniform(&out, name, ExtraType::Vec2, &[0.5, 0.3])?;
    }
    for name in ["vec3_args", "vec3_array", "vec3_rgb", "vec3_xyz"] {
        expect_uniform(&out, name, ExtraType::Vec3, &[0.1, 0.0, 1.0])?;
    }
    for name in ["vec4_args", "vec4_array", "vec4_rgba", "vec4_xyzw"] {
        expect_uniform(&out, name, ExtraType::Vec4, &[0.1, 0.0, 1.0, 0.25])?;
    }
    expect_uniform(&out, "from_bool", ExtraType::Float, &[1.0])?;
    expect_uniform(&out, "typed", ExtraType::Vec2, &[0.5, 0.25])?;
    Ok(())
}

#[test]
fn a_malformed_set_uniform_is_a_script_fault_with_a_line() -> Result<()> {
    for (call, why) in [
        (r#"setUniform("x", 1, 2, 3, 4, 5)"#, "five components"),
        (r#"setUniform("x")"#, "no components"),
        (r#"setUniform("x", "red")"#, "a string value"),
        (r#"setUniform("TIME", 1)"#, "a reserved name"),
        (r#"setUniform("syn_Level", 1)"#, "an audio uniform's name"),
        (r#"setUniform("not valid", 1)"#, "a non-identifier"),
        (r#"setUniform("mySlider", 1)"#, "a control's name"),
        (r#"setUniform(5, 1)"#, "a non-string name"),
    ] {
        let script = format!("function update(dt) {{\n  {call};\n}}");
        let mut rig = Rig::new(&script, &[SLIDER])?;
        let out = rig.step();
        assert_eq!(
            out.status,
            ScriptStatus::Faulted { consecutive: 1 },
            "{why} should fault"
        );
        assert!(
            out.uniforms.is_empty(),
            "{why} should publish nothing, got {:?}",
            out.uniforms
        );
        let message = errors(&out).join("\n");
        assert!(
            message.contains("script.js:2"),
            "{why}: the fault should name line 2, got `{message}`"
        );
    }
    Ok(())
}

#[test]
fn a_value_no_f32_can_hold_reaches_the_shader_as_zero() -> Result<()> {
    // 1e39 is a finite double but past f32::MAX, so a plain cast makes it
    // infinity, which a shader turns into NaN on the first multiply by 0.
    let script = r#"
        function update(dt) {
            setUniform("odd", NaN, Infinity, 1e39, -1e39);
            setUniform("fine", 3.0e38);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let out = rig.step();
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    let odd = uniform(&out, "odd")?;
    assert_eq!(
        odd.value, [0.0; 4],
        "NaN, infinities and out-of-range doubles all become zero"
    );
    expect_uniform(&out, "fine", ExtraType::Float, &[])?;
    assert!(
        uniform(&out, "fine")?.value[0] == 3.0e38,
        "a large value f32 can hold is kept"
    );
    Ok(())
}

#[test]
fn a_uniform_keeps_its_first_type_and_warns_once() -> Result<()> {
    let script = r#"
        function update(dt) {
            if (FRAMECOUNT === 0) setUniform("p", 1, 2);
            else setUniform("p", 7);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    rig.step();
    let second = rig.step();
    let third = rig.step();
    expect_uniform(&second, "p", ExtraType::Vec2, &[7.0, 0.0])?;
    let warnings = second
        .console
        .iter()
        .filter(|line| line.level == ConsoleLevel::Warn)
        .count();
    assert_eq!(warnings, 1, "the type clash is reported once");
    assert!(
        third.console.is_empty(),
        "and not again: {:?}",
        third.console
    );
    Ok(())
}

// ---- globals ----------------------------------------------------------------

#[test]
fn vector_and_color_controls_are_objects_and_scalars_are_numbers() -> Result<()> {
    let script = r#"
        function update(dt) {
            setUniform("pad_out", myXyPad.x, myXyPad.y);
            setUniform("rgb_out", myColor.r, myColor.g, myColor.b);
            setUniform("xyz_out", myColor.x, myColor.y, myColor.z);
            setUniform("slider_out", mySlider);
            setUniform("pad_is_plain", (typeof myXyPad.r) === "undefined" ? 1 : 0);
        }
    "#;
    let mut rig = Rig::new(script, &[SLIDER, PAD, COLOR])?;
    rig.set_target("mySlider", &[4.0])?;
    rig.bank.tick(DT);
    let out = rig.step();
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    expect_uniform(&out, "pad_out", ExtraType::Vec2, &[0.25, 0.75])?;
    expect_uniform(&out, "rgb_out", ExtraType::Vec3, &[0.1, 0.5, 0.0])?;
    expect_uniform(&out, "xyz_out", ExtraType::Vec3, &[0.1, 0.5, 0.0])?;
    expect_uniform(&out, "slider_out", ExtraType::Float, &[4.0])?;
    expect_uniform(&out, "pad_is_plain", ExtraType::Float, &[1.0])?;
    Ok(())
}

#[test]
fn clock_audio_and_mouse_are_globals_and_shader_internals_are_not() -> Result<()> {
    let script = r#"
        function update(dt) {
            setUniform("clock", TIME, FRAMECOUNT, dt);
            setUniform("bass", syn_BassLevel, syn_BPM);
            setUniform("mouse", _muv.x, _muv.y, _click.z, _mouse.w);
            var hidden = 0;
            if (typeof _uvc !== "undefined") hidden += 1;
            if (typeof _uv !== "undefined") hidden += 1;
            if (typeof _xy !== "undefined") hidden += 1;
            if (typeof PASSINDEX !== "undefined") hidden += 1;
            if (typeof RENDERSIZE !== "undefined") hidden += 1;
            setUniform("hidden", hidden);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let audio = audio_with("syn_BassLevel", 0.75)?;
    // Skip to frame 5 so FRAMECOUNT is not trivially zero.
    for _ in 0..5 {
        rig.step();
    }
    let out = rig.step_with(&ScriptInputs {
        time: 2.5,
        audio: &audio,
        mouse: [10.0, 20.0, 30.0, 40.0],
        click: [1.0, 2.0, 1.0],
        muv: [0.25, 0.5],
        ..ScriptInputs::default()
    });
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    expect_uniform(&out, "clock", ExtraType::Vec3, &[2.5, 5.0, DT])?;
    expect_uniform(&out, "bass", ExtraType::Vec2, &[0.75, 120.0])?;
    expect_uniform(&out, "mouse", ExtraType::Vec4, &[0.25, 0.5, 1.0, 40.0])?;
    expect_uniform(&out, "hidden", ExtraType::Float, &[0.0])?;
    Ok(())
}

// ---- setControl -------------------------------------------------------------

#[test]
fn syn_prime_bang_is_defined_and_never_fires() -> Result<()> {
    // Two real scenes test it inside `update()` (Q25).
    let script = r#"
        function update(dt) {
            setUniform("primed", syn_PrimeBang > 0.5 ? 1 : 0);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let out = rig.step();
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    expect_uniform(&out, "primed", ExtraType::Float, &[0.0])?;
    Ok(())
}

#[test]
fn set_control_writes_raw_values_and_lands_in_the_same_frame() -> Result<()> {
    let script = r#"
        function update(dt) {
            setControl("mySlider", 5.0);
            setControl("myXyPad", [-1.0, 0.5]);
            setControl("myColor", [0.4, 0.5, 0.6]);
        }
    "#;
    let mut rig = Rig::new(script, &[SLIDER, PAD, COLOR])?;
    let out = rig.step();
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    assert_eq!(
        rig.output("mySlider", 1)?,
        vec![5.0],
        "slider, after one tick"
    );
    assert_eq!(rig.output("myXyPad", 2)?, vec![-1.0, 0.5], "xy pad");
    assert_eq!(rig.output("myColor", 3)?, vec![0.4, 0.5, 0.6], "colour");
    Ok(())
}

#[test]
fn a_huge_array_handed_to_set_control_is_read_only_as_far_as_it_needs() -> Result<()> {
    // The watchdog cannot stop a native function, so one that walked a
    // hundred million holes would hold the frame for seconds.
    let script = r#"
        var big = [0.25, 0.5, 0.75];
        big.length = 100000000;
        function update() { setControl("myColor", big); }
    "#;
    let mut rig = Rig::with_budget(script, &[COLOR], Budget::default())?;
    let started = Instant::now();
    let out = rig.step();
    let elapsed = started.elapsed();
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    assert!(
        elapsed < Duration::from_millis(200),
        "the call read three components, not the whole array: took {elapsed:?}"
    );
    assert_eq!(
        rig.target("myColor", 3)?,
        vec![0.25, 0.5, 0.75],
        "the components it needs"
    );
    Ok(())
}

#[test]
fn set_control_clamps_to_the_control_range() -> Result<()> {
    let mut rig = Rig::new(
        r#"function update() { setControl("mySlider", 99); }"#,
        &[SLIDER],
    )?;
    rig.step();
    assert_eq!(rig.target("mySlider", 1)?, vec![10.0], "clamped to MAX");
    Ok(())
}

#[test]
fn set_control_normalized_scales_into_the_range() -> Result<()> {
    let script = r#"
        function update(dt) {
            setControlNormalized("mySlider", 0.25);
            setControlNormalized("myXyPad", [0.0, 1.0]);
        }
    "#;
    let mut rig = Rig::new(script, &[SLIDER, PAD])?;
    rig.step();
    assert_eq!(rig.output("mySlider", 1)?, vec![2.5], "0.25 of 0..10");
    assert_eq!(
        rig.output("myXyPad", 2)?,
        vec![-1.0, 1.0],
        "0..1 across -1..1"
    );
    Ok(())
}

#[test]
fn set_control_dimension_touches_one_component() -> Result<()> {
    let script = r#"
        function update(dt) {
            setControl("myXyPad", -1.0, { dimension: 0 });
            setControlNormalized("myColor", 1.0, { dimension: 2 });
        }
    "#;
    let mut rig = Rig::new(script, &[PAD, COLOR])?;
    rig.step();
    assert_eq!(
        rig.output("myXyPad", 2)?,
        vec![-1.0, 0.75],
        "x written, y left at its default"
    );
    assert_eq!(
        rig.output("myColor", 3)?,
        vec![0.1, 0.5, 1.0],
        "only blue written"
    );
    Ok(())
}

#[test]
fn control_names_ignore_case_spaces_underscores_and_dashes() -> Result<()> {
    let script = r#"
        function update(dt) {
            setControl("MYSLIDER", 1);
            setControl("my_xy_pad", [0.5, 0.5]);
            setControl("m-Y-c-O-l-O-r", [0.9, 0.9, 0.9]);
        }
    "#;
    let mut rig = Rig::new(script, &[SLIDER, PAD, COLOR])?;
    let out = rig.step();
    assert!(
        errors(&out).is_empty(),
        "every spelling resolves: {:?}",
        out.console
    );
    assert_eq!(rig.target("mySlider", 1)?, vec![1.0], "upper case");
    assert_eq!(rig.target("myXyPad", 2)?, vec![0.5, 0.5], "snake case");
    assert_eq!(rig.target("myColor", 3)?, vec![0.9, 0.9, 0.9], "dashes");
    Ok(())
}

#[test]
fn an_unknown_control_is_one_console_warning_not_one_per_frame() -> Result<()> {
    // A warning, not an error (D193): real template scripts name controls
    // their scene lacks.
    let script = r#"function update() { setControl("nope", 1); randomizeControl("nada"); }"#;
    let mut rig = Rig::new(script, &[SLIDER])?;
    let first = rig.step();
    let second = rig.step();
    assert_eq!(first.status, ScriptStatus::Running, "not a fault");
    assert_eq!(
        at_level(&first, ConsoleLevel::Warn).len(),
        2,
        "one per distinct problem: {:?}",
        first.console
    );
    assert!(errors(&first).is_empty(), "no errors: {:?}", first.console);
    assert!(
        second.console.is_empty(),
        "silent after: {:?}",
        second.console
    );
    Ok(())
}

#[test]
fn a_control_write_survives_a_fault_in_the_same_frame() -> Result<()> {
    let script = r#"function update() { setControl("mySlider", 3); throw new Error("late"); }"#;
    let mut rig = Rig::new(script, &[SLIDER])?;
    let out = rig.step();
    assert_eq!(
        out.status,
        ScriptStatus::Faulted { consecutive: 1 },
        "faulted"
    );
    assert_eq!(
        rig.target("mySlider", 1)?,
        vec![3.0],
        "the write already happened"
    );
    Ok(())
}

// ---- randomize / default ------------------------------------------------------

#[test]
fn randomize_and_default_control_and_group() -> Result<()> {
    let controls = [
        r#"{ "NAME": "a", "TYPE": "slider", "MIN": 0, "MAX": 10, "DEFAULT": 3, "UI_GROUP": "Colors" }"#,
        r#"{ "NAME": "b", "TYPE": "slider", "MIN": 0, "MAX": 10, "DEFAULT": 4, "UI_GROUP": "colors" }"#,
        r#"{ "NAME": "c", "TYPE": "slider", "MIN": 0, "MAX": 10, "DEFAULT": 5 }"#,
    ];
    let script = r#"
        function update(dt) {
            if (TIME === 0) { setControl("a", 9); setControl("b", 9); setControl("c", 9); }
            if (TIME === 1) randomizeControl("A");
            if (TIME === 2) defaultControl("a");
            if (TIME === 3) randomizeGroup("COLORS");
            if (TIME === 4) defaultGroup("c o l o r s");
        }
    "#;
    let mut rig = Rig::new(script, &controls)?;
    let value = |rig: &Rig, name: &str| -> Result<f32> { Ok(rig.target(name, 1)?[0]) };

    rig.at_time(0.0);
    assert_eq!(
        (value(&rig, "a")?, value(&rig, "b")?, value(&rig, "c")?),
        (9.0, 9.0, 9.0),
        "set up"
    );

    rig.at_time(1.0);
    let randomized = value(&rig, "a")?;
    assert!(
        (0.0..=10.0).contains(&randomized) && !near(randomized, 9.0),
        "randomizeControl moved `a` within range, got {randomized}"
    );
    assert_eq!(value(&rig, "b")?, 9.0, "`b` was not touched");

    rig.at_time(2.0);
    assert_eq!(
        value(&rig, "a")?,
        3.0,
        "defaultControl returns `a` to DEFAULT"
    );

    rig.at_time(3.0);
    assert!(
        !near(value(&rig, "a")?, 3.0) && !near(value(&rig, "b")?, 9.0),
        "randomizeGroup moved both members (a={}, b={})",
        value(&rig, "a")?,
        value(&rig, "b")?
    );
    assert_eq!(
        value(&rig, "c")?,
        9.0,
        "`c` is in no group and was left alone"
    );

    rig.at_time(4.0);
    assert_eq!(
        (value(&rig, "a")?, value(&rig, "b")?, value(&rig, "c")?),
        (3.0, 4.0, 9.0),
        "defaultGroup resets the group's members only"
    );
    Ok(())
}

#[test]
fn randomize_is_reproducible_from_the_seed() -> Result<()> {
    let controls = [r#"{ "NAME": "a", "TYPE": "slider" }"#];
    let script = "function update() { randomizeControl('a'); }";
    let mut first = Rig::new(script, &controls)?;
    let mut second = Rig::new(script, &controls)?;
    first.step();
    second.step();
    assert_eq!(
        first.target("a", 1)?,
        second.target("a", 1)?,
        "same seed, same calls, same value"
    );
    Ok(())
}

// ---- meta controls ------------------------------------------------------------

impl Rig {
    /// A meta control's target, first component.
    fn meta_target(&self, name: &str) -> Result<f32> {
        let index = self
            .meta
            .find(name)
            .ok_or_else(|| anyhow!("no meta control `{name}`"))?;
        self.meta
            .target(index)
            .map(|values| values[0])
            .ok_or_else(|| anyhow!("no target for meta `{name}`"))
    }

    /// A meta control's output, first component.
    fn meta_output(&self, name: &str) -> Result<f32> {
        let index = self
            .meta
            .find(name)
            .ok_or_else(|| anyhow!("no meta control `{name}`"))?;
        self.meta
            .output(index)
            .map(|values| values[0])
            .ok_or_else(|| anyhow!("no output for meta `{name}`"))
    }
}

#[test]
fn meta_writes_land_in_the_meta_bank_and_not_the_scene_bank() -> Result<()> {
    // The scene deliberately has its own `brightness`, so a misrouted
    // write would show up on it.
    let script = r#"
        function update(dt) {
            setControl("meta/brightness", -0.5);
            setControl("Meta/Hue", 0.75);
            setControl("gamma", 2.0, { bank: "meta" });
            setControlNormalized("meta/Vertical Mirror", 1);
        }
    "#;
    let brightness = r#"{ "NAME": "brightness", "TYPE": "slider" }"#;
    let mut rig = Rig::new(script, &[brightness])?;
    let out = rig.step();
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    assert!(out.console.is_empty(), "no complaints: {:?}", out.console);
    assert_eq!(rig.meta_target("brightness")?, -0.5, "meta/ prefix");
    assert_eq!(rig.meta_target("hue")?, 0.75, "prefix is case-insensitive");
    assert_eq!(rig.meta_target("gamma")?, 2.0, "bank: \"meta\" option");
    assert_eq!(
        rig.meta_target("vertical_mirror")?,
        1.0,
        "spacing-insensitive"
    );
    assert_eq!(
        rig.target("brightness", 1)?,
        vec![0.0],
        "the scene's own `brightness` control was not touched"
    );
    Ok(())
}

#[test]
fn meta_writes_are_clamped_and_snapped_like_any_control() -> Result<()> {
    let script = r#"
        function update(dt) {
            setControl("meta/brightness", 9);
            setControl("meta/invert", 0.9);
            setControlNormalized("meta/gamma", 0);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    rig.step();
    assert_eq!(rig.meta_target("brightness")?, 1.0, "clamped to the range");
    assert_eq!(rig.meta_target("invert")?, 1.0, "a toggle snaps to MAX");
    assert_eq!(rig.meta_target("gamma")?, 0.2, "normalised 0 is MIN");
    Ok(())
}

#[test]
fn meta_values_ease_and_persist_across_frames() -> Result<()> {
    let script =
        "function update(dt) { if (FRAMECOUNT === 0) { setControl('meta/master', 0.25); } }";
    let mut rig = Rig::new(script, &[])?;
    for _ in 0..4 {
        rig.step();
    }
    assert_eq!(rig.meta_target("master")?, 0.25, "the write is sticky");
    assert_eq!(
        rig.meta_output("master")?,
        0.25,
        "a plain slider follows at once"
    );
    Ok(())
}

#[test]
fn meta_default_randomize_and_groups_use_the_same_operations() -> Result<()> {
    let script = r#"
        function setup() {
            setControl("meta/brightness", 0.5);
            setControl("meta/gamma", 2.5);
            setControl("meta/contrast", 1.9);
            setControl("meta/reactivity", 0.1);
        }
        function update(dt) {
            if (FRAMECOUNT === 1) { defaultControl("meta/brightness"); }
            if (FRAMECOUNT === 2) { defaultGroup("meta/color"); }
            if (FRAMECOUNT === 3) { randomizeControl("meta/reactivity"); }
            if (FRAMECOUNT === 4) { randomizeGroup("meta/mirror"); }
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let frame = |rig: &mut Rig| rig.step();
    frame(&mut rig); // FRAMECOUNT 0: setup's writes land
    assert_eq!(rig.meta_target("brightness")?, 0.5, "setup's write landed");
    frame(&mut rig);
    assert_eq!(rig.meta_target("brightness")?, 0.0, "defaultControl");
    assert_eq!(
        rig.meta_target("gamma")?,
        2.5,
        "only the named control moved"
    );
    frame(&mut rig);
    assert_eq!(
        rig.meta_target("gamma")?,
        1.0,
        "defaultGroup reaches the group"
    );
    assert_eq!(rig.meta_target("contrast")?, 1.0, "...every member of it");
    assert_eq!(
        rig.meta_target("reactivity")?,
        0.1,
        "...and nothing outside"
    );
    frame(&mut rig);
    let random = rig.meta_target("reactivity")?;
    assert!(
        (0.0..=2.0).contains(&random) && (random - 0.1).abs() > 1e-6,
        "randomizeControl picked a new value in range: {random}"
    );
    let last = frame(&mut rig);
    assert!(
        last.console.is_empty() && last.status == ScriptStatus::Running,
        "randomizeGroup(\"meta/mirror\") resolved: {:?}",
        last.console
    );
    let mirror = rig.meta_target("vertical_mirror")?;
    assert!(
        mirror == 0.0 || mirror == 1.0,
        "a toggle stays at an end: {mirror}"
    );
    Ok(())
}

#[test]
fn an_unknown_meta_name_is_one_console_error_not_a_fault() -> Result<()> {
    let script = "function update() { setControl('meta/sparkle', 1); defaultGroup('meta/nope'); }";
    let mut rig = Rig::new(script, &[])?;
    let first = rig.step();
    assert_eq!(first.status, ScriptStatus::Running, "not a fault");
    let texts: Vec<&str> = first.console.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "setControl: no meta control named `sparkle`",
            "defaultGroup: no meta control group named `nope`",
        ],
        "named after the meta bank"
    );
    assert!(
        rig.step().console.is_empty(),
        "repeats are swallowed, as for scene controls"
    );
    Ok(())
}

// ---- events -------------------------------------------------------------------

const EVENT_SCRIPT: &str = r#"
    var counts = [0, 0, 0, 0];
    var whileOffCount = 0;
    var lastArgs = [-1, -1];
    function changed(v, p) { counts[0]++; lastArgs = [v, p]; }
    function risen(v, p) { counts[1]++; }
    function fell(v, p) { counts[2]++; }
    function high(v, p) { counts[3]++; }
    function low(v, p) { whileOffCount++; }
    function setup() {
        onChange("my_toggle", "changed");
        onOffToOn("MyToggle", "risen");
        onOnToOff("my toggle", "fell");
        whileOn("my-toggle", "high");
        whileOff("mytoggle", "low");
    }
    function update(dt) {
        setUniform("counts", counts);
        setUniform("low", whileOffCount);
        setUniform("args", lastArgs);
    }
"#;

#[test]
fn every_event_type_fires_on_its_condition_across_frames() -> Result<()> {
    let toggle = r#"{ "NAME": "myToggle", "TYPE": "toggle" }"#;
    let mut rig = Rig::new(EVENT_SCRIPT, &[toggle])?;
    // Each step: (toggle value, then expected counts and whileOff and args).
    let expected: [(f32, [f32; 4], f32, [f32; 2]); 6] = [
        (0.0, [0.0, 0.0, 0.0, 0.0], 1.0, [-1.0, -1.0]),
        (0.0, [0.0, 0.0, 0.0, 0.0], 2.0, [-1.0, -1.0]),
        (1.0, [1.0, 1.0, 0.0, 1.0], 2.0, [1.0, 0.0]),
        (1.0, [1.0, 1.0, 0.0, 2.0], 2.0, [1.0, 0.0]),
        (0.0, [2.0, 1.0, 1.0, 2.0], 3.0, [0.0, 1.0]),
        (1.0, [3.0, 2.0, 1.0, 3.0], 3.0, [1.0, 0.0]),
    ];
    for (frame, (value, counts, low, args)) in expected.into_iter().enumerate() {
        rig.set_target("myToggle", &[value])?;
        rig.bank.tick(DT);
        let out = rig.step();
        assert_eq!(
            out.status,
            ScriptStatus::Running,
            "frame {frame}: {:?}",
            out.console
        );
        expect_uniform(&out, "counts", ExtraType::Vec4, &counts)?;
        expect_uniform(&out, "low", ExtraType::Float, &[low])?;
        expect_uniform(&out, "args", ExtraType::Vec2, &args)?;
    }
    Ok(())
}

#[test]
fn events_watch_audio_and_the_clock_too() -> Result<()> {
    let script = r#"
        var loud = 0; var ticks = 0;
        function onLoud(v, p) { loud++; }
        function onTick(v, p) { ticks++; }
        function setup() {
            onOffToOn("SYN_bass_level", "onLoud");
            onChange("time", "onTick");
        }
        function update() { setUniform("seen", loud, ticks); }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let quiet = audio_with("syn_BassLevel", 0.1)?;
    let loud = audio_with("syn_BassLevel", 0.9)?;
    let mut run = |audio: &[f32], time: f32| {
        rig.step_with(&ScriptInputs {
            time,
            audio,
            ..ScriptInputs::default()
        })
    };
    run(&quiet, 0.0);
    run(&quiet, 0.1);
    let out = run(&loud, 0.2);
    expect_uniform(&out, "seen", ExtraType::Vec2, &[1.0, 2.0])?;
    let out = run(&loud, 0.3);
    expect_uniform(&out, "seen", ExtraType::Vec2, &[1.0, 3.0])?;
    Ok(())
}

#[test]
fn handlers_run_before_update_and_may_write_controls() -> Result<()> {
    let script = r#"
        var seq = 0;
        function early(v, p) { seq = seq * 10 + 1; setControl("mySlider", 7); }
        function setup() { whileOff("myToggle", "early"); }
        function update(dt) { seq = seq * 10 + 2; setUniform("seq", seq); }
    "#;
    let toggle = r#"{ "NAME": "myToggle", "TYPE": "toggle" }"#;
    let mut rig = Rig::new(script, &[toggle, SLIDER])?;
    let out = rig.step();
    expect_uniform(&out, "seq", ExtraType::Float, &[12.0])?;
    assert_eq!(
        rig.output("mySlider", 1)?,
        vec![7.0],
        "a handler's write lands too"
    );
    Ok(())
}

#[test]
fn a_missing_callback_is_an_error_and_an_unknown_input_a_warning() -> Result<()> {
    let script = r#"
        function setup() {
            onChange("mySlider", "doesNotExist");
            onChange("noSuchInput", "update");
        }
        function update() {}
    "#;
    let mut rig = Rig::new(script, &[SLIDER])?;
    let first = rig.step();
    let second = rig.step();
    let messages = errors(&first);
    let warnings = at_level(&first, ConsoleLevel::Warn);
    assert_eq!(messages.len(), 1, "the missing callback: {messages:?}");
    assert!(
        messages.iter().any(|m| m.contains("doesNotExist")),
        "names the missing callback: {messages:?}"
    );
    assert!(
        warnings.iter().any(|m| m.contains("noSuchInput")),
        "names the unknown input, as a warning (D193): {warnings:?}"
    );
    assert!(
        second.console.is_empty(),
        "nothing per frame: {:?}",
        second.console
    );
    Ok(())
}

#[test]
fn re_registering_every_frame_does_not_stack_handlers() -> Result<()> {
    let script = r#"
        var n = 0;
        function bump() { n++; }
        function update() { whileOff("mySlider", "bump"); setUniform("n", n); }
    "#;
    let mut rig = Rig::new(script, &[SLIDER])?;
    rig.step();
    rig.step();
    let out = rig.step();
    // Registered in frame 0's update, so it first fires in frame 1.
    expect_uniform(&out, "n", ExtraType::Float, &[2.0])?;
    Ok(())
}

#[test]
fn callbacks_declared_with_const_or_let_or_passed_as_functions_fire() -> Result<()> {
    // A top-level `const` or `let` is not a property of the global object
    // (D326), so a callback declared that way was "no function named".
    // Passing the function itself is accepted too (D414).
    let script = r#"
        let named = 0, arrow = 0, passed = 0, again = 0;
        const onNamed = (v, p) => { named++; };
        let onArrow = function (v, p) { arrow++; };
        const shared = function (v, p) { again++; };
        function setup() {
            onChange("myToggle", "onNamed");
            onOffToOn("myToggle", "onArrow");
            onChange("myToggle", function (v, p) { passed++; });
        }
        function update() {
            onChange("myToggle", shared);
            setUniform("seen", named, arrow, passed, again);
        }
    "#;
    let toggle = r#"{ "NAME": "myToggle", "TYPE": "toggle" }"#;
    let mut rig = Rig::new(script, &[toggle])?;
    let mut console = Vec::new();
    for value in [0.0, 0.0, 1.0, 1.0] {
        rig.set_target("myToggle", &[value])?;
        rig.bank.tick(DT);
        let out = rig.step();
        assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
        console.extend(out.console.iter().map(|line| line.text.clone()));
        if value > 0.5 {
            // Each fired once on the rise; `shared`, registered every
            // frame, is one handler, not one a frame.
            expect_uniform(&out, "seen", ExtraType::Vec4, &[1.0, 1.0, 1.0, 1.0])?;
        }
    }
    assert!(console.is_empty(), "no complaint: {console:?}");
    Ok(())
}

#[test]
fn a_callback_that_is_neither_a_name_nor_a_function_is_a_type_error() -> Result<()> {
    // A malformed call throws, as for every native (see `natives`).
    let toggle = r#"{ "NAME": "myToggle", "TYPE": "toggle" }"#;
    let bank = bank(&[toggle])?;
    let meta = strobe_scene::meta::bank()?;
    let script = r#"function setup() { onChange("myToggle", 42); }"#;
    match ScriptHost::new(script, &bank, &meta, relaxed()) {
        Err(ScriptError::Setup { message }) => assert!(
            message.contains("onChange") && message.contains("a function or the name of one"),
            "says what it wants: `{message}`"
        ),
        other => return Err(anyhow!("expected a setup error, got {other:?}")),
    }
    Ok(())
}

// ---- textureFinalPass ---------------------------------------------------------

#[test]
fn texture_final_pass_samples_the_previous_frame() -> Result<()> {
    let script = r#"
        function update(dt) {
            var c = textureFinalPass(10.5 / 64, 20.5 / 36);
            setUniform("px", c);
            setControl("myColor", c);
        }
    "#;
    let mut pixels = vec![0_u8; FINAL_PASS_BYTES];
    let offset = (20 * FINAL_PASS_WIDTH + 10) * 3;
    for (slot, value) in pixels[offset..offset + 3].iter_mut().zip([255, 51, 0]) {
        *slot = value;
    }
    let mut rig = Rig::new(script, &[COLOR])?;
    let out = rig.step_with(&ScriptInputs {
        final_pass: Some(&pixels),
        ..ScriptInputs::default()
    });
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    expect_uniform(&out, "px", ExtraType::Vec3, &[1.0, 0.2, 0.0])?;
    assert_eq!(
        rig.output("myColor", 3)?,
        vec![1.0, 0.2, 0.0],
        "the sampled colour can feed setControl directly"
    );
    Ok(())
}

#[test]
fn texture_final_pass_reads_corners_and_clamps() -> Result<()> {
    let script = r#"
        function update(dt) {
            setUniform("origin", textureFinalPass(0, 0));
            setUniform("far", textureFinalPass(5, 5));
        }
    "#;
    let mut pixels = vec![0_u8; FINAL_PASS_BYTES];
    pixels[0] = 255;
    let last = (FINAL_PASS_HEIGHT * FINAL_PASS_WIDTH - 1) * 3;
    pixels[last + 2] = 255;
    let mut rig = Rig::new(script, &[])?;
    let out = rig.step_with(&ScriptInputs {
        final_pass: Some(&pixels),
        ..ScriptInputs::default()
    });
    expect_uniform(&out, "origin", ExtraType::Vec3, &[1.0, 0.0, 0.0])?;
    expect_uniform(&out, "far", ExtraType::Vec3, &[0.0, 0.0, 1.0])?;
    Ok(())
}

#[test]
fn texture_final_pass_is_black_without_usable_data() -> Result<()> {
    let script = "function update() { setUniform('px', textureFinalPass(0.5, 0.5)); }";
    let mut rig = Rig::new(script, &[])?;
    let none = rig.step();
    expect_uniform(&none, "px", ExtraType::Vec3, &[0.0, 0.0, 0.0])?;

    let short = vec![255_u8; 10];
    let wrong = rig.step_with(&ScriptInputs {
        final_pass: Some(&short),
        ..ScriptInputs::default()
    });
    expect_uniform(&wrong, "px", ExtraType::Vec3, &[0.0, 0.0, 0.0])?;

    let full = vec![255_u8; FINAL_PASS_BYTES];
    let white = rig.step_with(&ScriptInputs {
        final_pass: Some(&full),
        ..ScriptInputs::default()
    });
    expect_uniform(&white, "px", ExtraType::Vec3, &[1.0, 1.0, 1.0])?;

    let gone = rig.step();
    expect_uniform(&gone, "px", ExtraType::Vec3, &[0.0, 0.0, 0.0])?;
    Ok(())
}

// ---- console ------------------------------------------------------------------

#[test]
fn print_and_console_log_reach_the_console() -> Result<()> {
    let script = r#"
        function update() {
            print("bass", 0.5, { a: 1 }, [1, 2]);
            console.log("log");
            console.warn("warn");
            console.error("error");
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let out = rig.step();
    let lines: Vec<(ConsoleLevel, &str)> = out
        .console
        .iter()
        .map(|line| (line.level, line.text.as_str()))
        .collect();
    assert_eq!(
        lines,
        vec![
            (ConsoleLevel::Log, r#"bass 0.5 {"a":1} [1,2]"#),
            (ConsoleLevel::Log, "log"),
            (ConsoleLevel::Warn, "warn"),
            (ConsoleLevel::Error, "error"),
        ],
        "print joins arguments with spaces and stringifies objects"
    );
    Ok(())
}

#[test]
fn print_global_keys_lists_values_and_functions() -> Result<()> {
    let mut rig = Rig::new("function update() { printGlobalKeys(); }", &[SLIDER])?;
    let out = rig.step();
    let text = out
        .console
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        "mySlider",
        "syn_BassLevel",
        "TIME",
        "setUniform",
        "setControl",
        "update",
    ] {
        assert!(
            text.contains(needle),
            "`{needle}` should be listed:\n{text}"
        );
    }
    Ok(())
}

#[test]
fn a_console_flood_is_capped_per_frame() -> Result<()> {
    let mut rig = Rig::new(
        "function update() { for (var i = 0; i < 5000; i++) print(i); }",
        &[],
    )?;
    let out = rig.step();
    assert!(
        out.console.len() < 300,
        "the host caps console lines per frame, got {}",
        out.console.len()
    );
    Ok(())
}

// ---- deprecated inputs / uniforms ---------------------------------------------

#[test]
fn deprecated_inputs_reads_the_same_values_and_is_read_only() -> Result<()> {
    let script = r#"
        function update(dt) {
            inputs.syn_BassLevel = 99;
            setUniform("legacy", inputs.syn_BassLevel, inputs.TIME, inputs.mySlider);
            setUniform("has", ("syn_Level" in inputs) ? 1 : 0, ("nothing_here" in inputs) ? 1 : 0);
        }
    "#;
    let mut rig = Rig::new(script, &[SLIDER])?;
    rig.set_target("mySlider", &[6.0])?;
    rig.bank.tick(DT);
    let audio = audio_with("syn_BassLevel", 0.5)?;
    let out = rig.step_with(&ScriptInputs {
        time: 1.5,
        audio: &audio,
        ..ScriptInputs::default()
    });
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    expect_uniform(&out, "legacy", ExtraType::Vec3, &[0.5, 1.5, 6.0])?;
    expect_uniform(&out, "has", ExtraType::Vec2, &[1.0, 0.0])?;
    Ok(())
}

#[test]
fn deprecated_uniforms_object_is_scanned_after_update() -> Result<()> {
    let script = r#"
        function update(dt) {
            uniforms.scalar = 0.5;
            uniforms.pair = [1, 2];
            uniforms.colour = { r: 0.1, g: 0.2, b: 0.3 };
            uniforms.TIME = 1;
            uniforms.bad = "text";
            // A script's own state kept in the table (Synesthesia's
            // thresholder does this): not a uniform, and not a mistake.
            uniforms.state = { count: 3, tick: function () {} };
            uniforms.helper = function () { return 1; };
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let out = rig.step();
    assert_eq!(
        out.status,
        ScriptStatus::Running,
        "bad entries do not fault the frame"
    );
    expect_uniform(&out, "scalar", ExtraType::Float, &[0.5])?;
    expect_uniform(&out, "pair", ExtraType::Vec2, &[1.0, 2.0])?;
    expect_uniform(&out, "colour", ExtraType::Vec3, &[0.1, 0.2, 0.3])?;
    assert_eq!(
        out.uniforms.len(),
        3,
        "the reserved name, the string and the script's state were skipped"
    );
    assert_eq!(
        errors(&out).len(),
        2,
        "the reserved name and the string are reported, the state is not: {:?}",
        out.console
    );
    let next = rig.step();
    assert!(
        next.console.is_empty(),
        "and reported once: {:?}",
        next.console
    );
    Ok(())
}

// ---- sealing ----------------------------------------------------------------------

#[test]
fn uniforms_made_in_setup_are_listed_when_sealed() -> Result<()> {
    let script = "function setup() { setUniform('seed', 1, 2, 3); } function update() {}";
    let mut rig = Rig::new(script, &[])?;
    let extras = rig.host.seal_uniforms();
    assert_eq!(
        extras,
        vec![("seed".to_string(), ExtraType::Vec3)],
        "setup's uniforms are known as soon as the host is built"
    );
    let out = rig.step();
    expect_uniform(&out, "seed", ExtraType::Vec3, &[1.0, 2.0, 3.0])?;
    Ok(())
}

#[test]
fn a_uniform_first_set_after_the_seal_is_refused_once_and_never_published() -> Result<()> {
    let script = r#"
        function update() {
            setUniform("a", FRAMECOUNT);
            if (FRAMECOUNT >= 2) setUniform("late", 1, 2);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    rig.step(); // frame 0 declares `a`
    let extras = rig.host.seal_uniforms();
    assert_eq!(
        extras,
        vec![("a".to_string(), ExtraType::Float)],
        "only what the load-time frame published is declared"
    );
    rig.step();
    let third = rig.step();
    assert_eq!(
        third.status,
        ScriptStatus::Running,
        "refusal is not a fault"
    );
    assert_eq!(
        third.extras(),
        extras,
        "the uniform list did not grow: {:?}",
        third.uniforms
    );
    expect_uniform(&third, "a", ExtraType::Float, &[2.0])?;
    let lines = errors(&third);
    assert_eq!(lines.len(), 1, "one explanation: {lines:?}");
    assert!(
        lines[0].contains("`late`") && lines[0].contains("after the scene loaded"),
        "it names the uniform and the rule: {lines:?}"
    );
    assert!(
        errors(&rig.step()).is_empty(),
        "and says it once, not every frame"
    );
    Ok(())
}

#[test]
fn the_legacy_uniforms_object_is_sealed_too() -> Result<()> {
    let script = "function update() { uniforms.x = 1; if (FRAMECOUNT > 0) { uniforms.y = 2; } }";
    let mut rig = Rig::new(script, &[])?;
    rig.step();
    rig.host.seal_uniforms();
    let out = rig.step();
    assert_eq!(
        out.extras(),
        vec![("x".to_string(), ExtraType::Float)],
        "`y` is refused on the legacy path as well: {:?}",
        out.console
    );
    Ok(())
}

// ---- the caller's output, reused ------------------------------------------------

impl Rig {
    /// One frame into `output`, then the bank tick, as the app does.
    fn step_into(&mut self, output: &mut FrameOutput) {
        let inputs = ScriptInputs {
            frame_count: self.frame,
            ..ScriptInputs::default()
        };
        self.host
            .frame_into(DT, &inputs, &mut self.bank, &mut self.meta, output);
        self.bank.tick(DT);
        self.meta.tick(DT);
        self.frame += 1;
    }
}

#[test]
fn a_frame_into_the_callers_output_matches_a_frame_that_returns_one() -> Result<()> {
    let script = r#"
        function update() {
            setUniform("a", FRAMECOUNT);
            setUniform("b", 1, 2);
            if (FRAMECOUNT === 1) console.log("hello");
        }
    "#;
    let mut kept = Rig::new(script, &[])?;
    let mut fresh = Rig::new(script, &[])?;
    let mut output = FrameOutput::default();
    for frame in 0..4 {
        kept.step_into(&mut output);
        let expected = fresh.step();
        assert_eq!(
            output.uniforms, expected.uniforms,
            "frame {frame}: uniforms"
        );
        assert_eq!(output.console, expected.console, "frame {frame}: console");
        assert_eq!(output.status, expected.status, "frame {frame}: status");
    }
    assert!(
        output.console.is_empty(),
        "the console of an earlier frame was cleared, not kept: {:?}",
        output.console
    );
    Ok(())
}

#[test]
fn bound_uniforms_follow_the_order_they_were_bound_in() -> Result<()> {
    let script = r#"
        function update() {
            setUniform("first", 1);
            setUniform("clash", 9);
            setUniform("last", FRAMECOUNT, 4);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let mut output = FrameOutput::default();
    rig.step_into(&mut output);
    assert!(output.bound.is_empty(), "nothing is bound yet");
    rig.host.seal_uniforms();
    rig.host.bind_uniforms(["last", "first"]);
    let mut now = Vec::new();
    rig.host.bound_uniforms_into(&mut now);
    assert_eq!(
        now,
        [[0.0, 4.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
        "readable at once, before another frame"
    );
    rig.step_into(&mut output);
    rig.step_into(&mut output);
    assert_eq!(
        output.bound,
        [[2.0, 4.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
        "each frame carries them in bound order, without `clash`"
    );
    Ok(())
}

#[test]
fn a_restarted_script_binds_a_declared_uniform_it_sets_late() -> Result<()> {
    // The shaders were compiled with `late` and `early`, in that order. This
    // run of the script publishes `early` from its first frame and `late`
    // only once a beat is heard.
    let script = r#"
        function update() {
            setUniform("early", 1);
            if (FRAMECOUNT >= 2) setUniform("late", 5);
            setUniform("unknown", 3);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let mut output = FrameOutput::default();
    rig.step_into(&mut output);
    rig.host.seal_uniforms_to(["late", "early"]);
    rig.step_into(&mut output);
    assert_eq!(
        output.bound,
        [[0.0; 4], [1.0, 0.0, 0.0, 0.0]],
        "`late` reads as zeros until it is set"
    );
    assert_eq!(
        errors(&output).len(),
        1,
        "`unknown` is refused, and said so once: {:?}",
        output.console
    );
    rig.step_into(&mut output);
    assert_eq!(
        output.bound,
        [[5.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
        "and then it is found in its place"
    );
    assert!(
        errors(&output).is_empty(),
        "and it is not said again: {:?}",
        output.console
    );
    Ok(())
}

#[test]
fn steady_state_frames_refill_the_same_buffers() -> Result<()> {
    // The host cannot count its own allocations (no `unsafe`, so no counting
    // allocator), so this pins what makes a frame allocation-free: after the
    // first frames the output's vectors keep their storage.
    let script = r#"
        function update() {
            setUniform("a", FRAMECOUNT);
            setUniform("b", 1, 2, 3);
            setUniform("c", 0.5);
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let mut output = FrameOutput::default();
    rig.step_into(&mut output);
    rig.host.seal_uniforms();
    rig.host.bind_uniforms(["c", "a", "b"]);
    for _ in 0..3 {
        rig.step_into(&mut output);
    }
    let uniforms = (output.uniforms.as_ptr(), output.uniforms.capacity());
    let bound = (output.bound.as_ptr(), output.bound.capacity());
    let console = output.console.capacity();
    for frame in 0..200 {
        rig.step_into(&mut output);
        assert_eq!(output.uniforms.len(), 3, "frame {frame}: uniforms");
        assert_eq!(output.bound.len(), 3, "frame {frame}: bound");
        assert_eq!(output.status, ScriptStatus::Running, "frame {frame}");
    }
    assert_eq!(
        (output.uniforms.as_ptr(), output.uniforms.capacity()),
        uniforms,
        "the uniform list stayed where it was"
    );
    assert_eq!(
        (output.bound.as_ptr(), output.bound.capacity()),
        bound,
        "the bound values stayed where they were"
    );
    assert_eq!(output.console.capacity(), console, "and the console");
    Ok(())
}

// ---- lifecycle ------------------------------------------------------------------

#[test]
fn setup_runs_once_and_its_control_writes_apply_on_the_first_frame() -> Result<()> {
    let script = r#"
        var calls = 0;
        function setup() { calls++; setControl("mySlider", 8); }
        function update() { setUniform("calls", calls); }
    "#;
    let mut rig = Rig::new(script, &[SLIDER])?;
    assert_eq!(
        rig.target("mySlider", 1)?,
        vec![0.0],
        "nothing applied at construction"
    );
    rig.step();
    let out = rig.step();
    expect_uniform(&out, "calls", ExtraType::Float, &[1.0])?;
    assert_eq!(
        rig.target("mySlider", 1)?,
        vec![8.0],
        "applied by the first frame"
    );
    Ok(())
}

#[test]
fn state_persists_across_frames() -> Result<()> {
    let script = r#"
        var total = 0;
        function update(dt) { total += dt; setUniform("total", total); }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let mut out = rig.step();
    for _ in 0..59 {
        out = rig.step();
    }
    expect_uniform(&out, "total", ExtraType::Float, &[1.0])?;
    Ok(())
}

#[test]
fn promise_callbacks_run_by_the_end_of_the_frame_that_queued_them() -> Result<()> {
    // Every JavaScript engine runs queued promise callbacks once the
    // script returns. Never running them loses the callback and keeps
    // every queued job alive, so a script that awaits once a frame grows
    // the heap until it hits the memory cap.
    let script = r#"
        var resolved = 0;
        var loaded = 0;
        Promise.resolve().then(function () { loaded = 1; });
        async function tick() { await null; resolved++; }
        function update() {
            setUniform("seen", resolved, loaded);
            tick();
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let mut out = rig.step();
    for _ in 0..2 {
        out = rig.step();
    }
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    expect_uniform(&out, "seen", ExtraType::Vec2, &[2.0, 1.0])?;
    Ok(())
}

#[test]
fn an_endless_promise_chain_is_interrupted() -> Result<()> {
    let script = r#"
        function again() { Promise.resolve().then(again); }
        function update() { again(); }
    "#;
    let mut rig = Rig::with_budget(script, &[], Budget::default())?;
    let started = Instant::now();
    let out = rig.step();
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "the chain was cut at the budget, took {:?}",
        started.elapsed()
    );
    assert_eq!(
        out.status,
        ScriptStatus::Faulted { consecutive: 1 },
        "an endless chain is a fault"
    );
    Ok(())
}

#[test]
fn setup_and_update_declared_with_const_or_let_run_too() -> Result<()> {
    // A top-level `const` or `let` is a global binding but not a property
    // of the global object, so a host that only reads `globalThis.update`
    // never calls it, and the script silently does nothing.
    let script = r#"
        let calls = 0;
        const setup = () => { calls++; };
        let update = function (dt) { setUniform("seen", calls, dt > 0 ? 1 : 0); };
    "#;
    let mut rig = Rig::new(script, &[])?;
    let out = rig.step();
    assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
    expect_uniform(&out, "seen", ExtraType::Vec2, &[1.0, 1.0])?;
    Ok(())
}

#[test]
fn a_script_with_neither_setup_nor_update_is_fine() -> Result<()> {
    let mut rig = Rig::new("var x = 1;", &[])?;
    let out = rig.step();
    assert_eq!(
        out.status,
        ScriptStatus::Running,
        "nothing to run is not a fault"
    );
    assert!(out.uniforms.is_empty(), "and publishes nothing");
    Ok(())
}

#[test]
fn load_errors_name_what_failed() -> Result<()> {
    let bank = bank(&[])?;
    let meta = strobe_scene::meta::bank()?;
    let syntax = ScriptHost::new("function (", &bank, &meta, relaxed());
    assert!(
        matches!(syntax, Err(ScriptError::Load { .. })),
        "a syntax error is a load error, got {syntax:?}"
    );
    let throws = ScriptHost::new("\n\nthrow new Error('at load');", &bank, &meta, relaxed());
    match throws {
        Err(ScriptError::Load { message }) => assert!(
            message.contains("at load") && message.contains("script.js:3"),
            "message and line, got `{message}`"
        ),
        other => return Err(anyhow!("expected a load error, got {other:?}")),
    }
    let setup = ScriptHost::new("function setup() { nope(); }", &bank, &meta, relaxed());
    assert!(
        matches!(setup, Err(ScriptError::Setup { .. })),
        "a throwing setup() is a setup error, got {setup:?}"
    );
    Ok(())
}

#[test]
fn a_runaway_load_is_interrupted() -> Result<()> {
    let bank = bank(&[])?;
    let meta = strobe_scene::meta::bank()?;
    let budget = Budget {
        load: Duration::from_millis(30),
        ..Budget::default()
    };
    let started = Instant::now();
    let result = ScriptHost::new("while (true) {}", &bank, &meta, budget);
    assert!(
        matches!(result, Err(ScriptError::Load { .. })),
        "an endless top level is a load error, got {result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "and it returned promptly, took {:?}",
        started.elapsed()
    );
    Ok(())
}

// ---- watchdog and faults ---------------------------------------------------------

#[test]
fn an_endless_loop_in_update_is_interrupted_and_the_frame_returns() -> Result<()> {
    let script = r#"
        var n = 0;
        function update() {
            n++;
            if (n === 2) { while (true) {} }
            setUniform("n", n);
        }
    "#;
    // The default 4 ms budget, on purpose.
    let mut rig = Rig::with_budget(script, &[], Budget::default())?;
    let good = rig.step();
    assert_eq!(good.status, ScriptStatus::Running, "{:?}", good.console);

    let started = Instant::now();
    let hung = rig.step();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(200),
        "the loop was interrupted within budget, took {elapsed:?}"
    );
    assert_eq!(
        hung.status,
        ScriptStatus::Faulted { consecutive: 1 },
        "an interrupt is a fault"
    );
    assert!(
        errors(&hung).iter().any(|m| m.contains("interrupted")),
        "the console says why: {:?}",
        hung.console
    );
    expect_uniform(&hung, "n", ExtraType::Float, &[1.0])?;

    let recovered = rig.step();
    assert_eq!(
        recovered.status,
        ScriptStatus::Running,
        "the next frame runs normally"
    );
    expect_uniform(&recovered, "n", ExtraType::Float, &[3.0])?;
    Ok(())
}

#[test]
fn the_load_frame_runs_under_the_load_budget_and_later_frames_do_not() -> Result<()> {
    // 150 ms of work: far past a 4 ms frame, well inside a 2 s load. The
    // margin matters on a loaded machine: the loop ends on wall time, so a
    // thread descheduled past the end between two interrupt checks would
    // otherwise finish without ever being interrupted.
    let script = r#"
        function update() {
            var until = Date.now() + 150;
            while (Date.now() < until) {}
            setUniform("ready", 1);
        }
    "#;
    let budget = Budget {
        frame: Duration::from_millis(4),
        load: Duration::from_secs(2),
        ..Budget::default()
    };
    let mut rig = Rig::with_budget(script, &[], budget)?;
    let loading = rig
        .host
        .load_frame(DT, &ScriptInputs::default(), &mut rig.bank, &mut rig.meta);
    assert_eq!(
        loading.status,
        ScriptStatus::Running,
        "150 ms is inside a 2 s load budget: {:?}",
        loading.console
    );
    expect_uniform(&loading, "ready", ExtraType::Float, &[1.0])?;
    assert_eq!(
        rig.step().status,
        ScriptStatus::Faulted { consecutive: 1 },
        "but not inside a frame's"
    );
    Ok(())
}

#[test]
fn an_endless_event_handler_is_interrupted_too() -> Result<()> {
    let script = r#"
        function spin() { while (true) {} }
        function setup() { whileOff("mySlider", "spin"); }
    "#;
    let mut rig = Rig::with_budget(script, &[SLIDER], Budget::default())?;
    let started = Instant::now();
    let out = rig.step();
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(
        out.status,
        ScriptStatus::Faulted { consecutive: 1 },
        "faulted"
    );
    Ok(())
}

#[test]
fn a_faulted_frame_keeps_the_last_good_uniforms() -> Result<()> {
    let script = r#"
        function update() {
            setUniform("v", FRAMECOUNT);
            if (FRAMECOUNT === 1) { setUniform("half", 1); throw new Error("mid-frame"); }
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    rig.step();
    let out = rig.step();
    assert_eq!(
        out.status,
        ScriptStatus::Faulted { consecutive: 1 },
        "faulted"
    );
    expect_uniform(&out, "v", ExtraType::Float, &[0.0])?;
    assert!(
        out.uniforms.iter().all(|u| &*u.name != "half"),
        "writes from the faulted frame are discarded: {:?}",
        out.uniforms
    );
    let message = errors(&out).join("\n");
    assert!(
        message.contains("mid-frame") && message.contains("script.js:4"),
        "message and line, got `{message}`"
    );
    assert!(
        rig.host.last_fault().is_some(),
        "the host remembers the fault"
    );
    Ok(())
}

#[test]
fn three_faulted_frames_in_a_row_disable_the_script() -> Result<()> {
    let script = r#"
        function update() {
            if (FRAMECOUNT === 0) setUniform("v", 5);
            else throw new Error("boom");
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    assert_eq!(rig.step().status, ScriptStatus::Running, "frame 0 is good");
    assert_eq!(
        rig.step().status,
        ScriptStatus::Faulted { consecutive: 1 },
        "first strike"
    );
    assert_eq!(
        rig.step().status,
        ScriptStatus::Faulted { consecutive: 2 },
        "second strike"
    );
    let third = rig.step();
    assert_eq!(
        third.status,
        ScriptStatus::Disabled,
        "third strike disables"
    );
    assert!(
        errors(&third).iter().any(|m| m.contains("disabled")),
        "and says so: {:?}",
        third.console
    );
    assert!(rig.host.is_disabled(), "the host reports it");

    let after = rig.step();
    assert_eq!(after.status, ScriptStatus::Disabled, "it stays disabled");
    assert!(after.console.is_empty(), "silently: {:?}", after.console);
    expect_uniform(&after, "v", ExtraType::Float, &[5.0])?;
    Ok(())
}

#[test]
fn a_fault_that_keeps_coming_back_is_one_console_error() -> Result<()> {
    // Never three in a row, so never disabled: without the once rule this
    // is thirty error lines a second for as long as the scene plays.
    let script = r#"
        function update() { if (FRAMECOUNT % 2 === 1) throw new Error("odd frame"); }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let mut lines = 0;
    let mut faults = 0;
    for _ in 0..60 {
        let out = rig.step();
        if matches!(out.status, ScriptStatus::Faulted { .. }) {
            faults += 1;
        }
        lines += errors(&out)
            .iter()
            .filter(|line| line.contains("odd frame"))
            .count();
    }
    assert_eq!(faults, 30, "every odd frame still faults");
    assert_eq!(lines, 1, "but the console says so once");
    Ok(())
}

#[test]
fn an_unhandled_rejection_is_one_console_error_and_a_handled_one_none() -> Result<()> {
    // A rejected promise nobody handles is lost without a word: the
    // `async` function's error never reaches the person (D413).
    let script = r#"
        Promise.reject(new Error("rejected at load"));
        function update() {
            Promise.reject(new Error("nobody listens"));
            (async function () { throw new Error("thrown in async"); })();
            var later = Promise.reject(new Error("handled at once"));
            later.catch(function () {});
            (async function () {
                try { await Promise.reject("awaited"); } catch (e) {}
            })();
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    let mut lines: Vec<String> = Vec::new();
    for _ in 0..10 {
        let out = rig.step();
        assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
        lines.extend(errors(&out).iter().map(|line| (*line).to_string()));
    }
    for needle in ["rejected at load", "nobody listens", "thrown in async"] {
        let count = lines.iter().filter(|line| line.contains(needle)).count();
        assert_eq!(count, 1, "`{needle}` reported once: {lines:?}");
    }
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("handled at once") && !line.contains("awaited")),
        "a handled rejection is not reported: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| line.contains("unhandled promise rejection")),
        "each says what it is: {lines:?}"
    );
    // A load that fails with a rejection still held lets go of it before
    // the runtime goes; a handle that outlives the runtime aborts the
    // process.
    let bank = bank(&[])?;
    let meta = strobe_scene::meta::bank()?;
    let failed = ScriptHost::new(
        "Promise.reject(1); throw new Error('no');",
        &bank,
        &meta,
        relaxed(),
    );
    assert!(
        matches!(failed, Err(ScriptError::Load { .. })),
        "{failed:?}"
    );
    Ok(())
}

#[test]
fn a_good_frame_resets_the_strike_count() -> Result<()> {
    let script = r#"
        function update() {
            if (FRAMECOUNT % 3 !== 2) throw new Error("two of every three");
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    for round in 0..4 {
        assert_eq!(
            rig.step().status,
            ScriptStatus::Faulted { consecutive: 1 },
            "round {round}: first strike"
        );
        assert_eq!(
            rig.step().status,
            ScriptStatus::Faulted { consecutive: 2 },
            "round {round}: second strike"
        );
        assert_eq!(
            rig.step().status,
            ScriptStatus::Running,
            "round {round}: reset"
        );
    }
    assert!(!rig.host.is_disabled(), "never three in a row");
    Ok(())
}

#[test]
fn a_memory_limit_breach_faults_cleanly_and_recovers() -> Result<()> {
    let script = r#"
        function update() {
            if (TIME > 0.5) {
                var hoard = [];
                for (var i = 0; i < 100000000; i++) hoard.push({ i: i, s: "x" + i });
            }
            setUniform("alive", 1);
        }
    "#;
    let budget = Budget {
        memory_limit: 8 * 1024 * 1024,
        ..relaxed()
    };
    let mut rig = Rig::with_budget(script, &[], budget)?;
    let before = rig.at_time(0.0);
    assert_eq!(before.status, ScriptStatus::Running, "{:?}", before.console);

    let started = Instant::now();
    let breach = rig.at_time(1.0);
    assert_eq!(
        breach.status,
        ScriptStatus::Faulted { consecutive: 1 },
        "an allocation past the limit is a fault, not a crash"
    );
    assert!(
        errors(&breach)
            .iter()
            .any(|m| m.to_lowercase().contains("memory")),
        "the console names memory: {:?}",
        breach.console
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "and it failed fast, took {:?}",
        started.elapsed()
    );

    let after = rig.at_time(0.0);
    assert_eq!(
        after.status,
        ScriptStatus::Running,
        "the heap was reclaimed: {:?}",
        after.console
    );
    expect_uniform(&after, "alive", ExtraType::Float, &[1.0])?;
    Ok(())
}

#[test]
fn throwing_undefined_is_not_reported_as_out_of_memory() -> Result<()> {
    let mut rig = Rig::new("function update() { throw undefined; }", &[])?;
    let out = rig.step();
    assert_eq!(
        out.status,
        ScriptStatus::Faulted { consecutive: 1 },
        "throw undefined faults"
    );
    let fault = rig.host.last_fault().unwrap_or_default().to_string();
    assert!(
        !fault.contains("memory") && fault.contains("undefined"),
        "it is reported as what it threw, got `{fault}`"
    );
    // A full heap throws null, so `throw null` cannot be told apart from
    // one; the message names both.
    let mut rig = Rig::new("function update() { throw null; }", &[])?;
    rig.step();
    let fault = rig.host.last_fault().unwrap_or_default().to_string();
    assert!(
        fault.contains("memory") && fault.contains("threw null"),
        "throw null names both readings, got `{fault}`"
    );
    Ok(())
}

#[test]
fn runaway_recursion_is_a_fault_not_a_crash() -> Result<()> {
    let script = "function r(n) { return r(n + 1) + 1; } function update() { r(0); }";
    let mut rig = Rig::new(script, &[])?;
    let out = rig.step();
    assert_eq!(
        out.status,
        ScriptStatus::Faulted { consecutive: 1 },
        "stack overflow throws inside the script"
    );
    Ok(())
}

#[test]
fn runaway_recursion_on_a_one_mib_thread_is_a_fault_not_a_stack_overflow() -> Result<()> {
    // A Windows executable's main thread, where the frame loop and so the
    // script run, has a 1 MiB stack. The script's own stack limit must fit
    // inside it with room for the app's frames, or deep recursion runs off
    // the end of the real stack and the process dies.
    const WINDOWS_MAIN_THREAD_STACK: usize = 1024 * 1024;
    let worker = std::thread::Builder::new()
        .stack_size(WINDOWS_MAIN_THREAD_STACK)
        .spawn(|| -> Result<ScriptStatus> {
            let script = "function r(n) { return r(n + 1) + 1; } function update() { r(0); }";
            let mut rig = Rig::with_budget(
                script,
                &[],
                Budget {
                    max_stack: Budget::default().max_stack,
                    ..relaxed()
                },
            )?;
            Ok(rig.step().status)
        })?;
    let status = worker
        .join()
        .map_err(|_| anyhow!("the script thread died"))??;
    assert_eq!(
        status,
        ScriptStatus::Faulted { consecutive: 1 },
        "the recursion is stopped inside the script"
    );
    Ok(())
}

// ---- performance ------------------------------------------------------------------

#[test]
fn ten_thousand_set_uniform_calls_in_one_update_are_cheap() -> Result<()> {
    let script = r#"
        function update(dt) {
            for (var i = 0; i < 10000; i++) {
                setUniform("u" + (i % 200), i, i * 0.5);
            }
        }
    "#;
    let mut rig = Rig::new(script, &[])?;
    // The first frame creates the 200 uniforms; time steady-state frames.
    // The best of five is the cost: a wall-clock time on a shared machine
    // (CI, or several builds at once) only ever adds to it.
    rig.step();
    let mut elapsed = Duration::MAX;
    for _ in 0..5 {
        let started = Instant::now();
        let out = rig.step();
        elapsed = elapsed.min(started.elapsed());
        assert_eq!(out.status, ScriptStatus::Running, "{:?}", out.console);
        assert_eq!(out.uniforms.len(), 200, "200 distinct names");
    }
    println!("10,000 setUniform calls in one update (best of 5): {elapsed:?}");
    assert!(
        elapsed < Duration::from_millis(50),
        "10,000 setUniform calls took {elapsed:?}, budget 50 ms"
    );
    Ok(())
}
