// ============================================================================
// Module:       strobe_app::script
// Description:  A scene's script.js at run time: one ScriptHost per scene,
//               its console routed to the log, its faults to the UI and to
//               structured failures, and the uniforms it creates.
//
// Dependencies: strobe-script (ScriptHost); strobe-scene (SceneDef,
//               ControlBank, ExtraType); strobe-diag (failures)
// ============================================================================

//! Running a scene's script.
//!
//! `strobe-script` produces values and leaves applying them to the app;
//! this module is the app's half of that contract.
//!
//! - **Console.** `print` and `console.*` become log records under the
//!   target `script` (so `STROBE_LOG=script=debug` works and the UI console
//!   shows them in colour by level).
//! - **Faults.** A thrown exception or a blown budget discards the frame's
//!   uniforms and keeps the last good ones. The *first* fault of a streak
//!   (once per distinct message, D325) and the disabling are reported as
//!   structured `SCRIPT` failures; the
//!   UI shows the state ([`ScriptState`]) beside the scene's controls so a
//!   dead script is visible without reading the log.
//! - **Uniforms.** What the script creates with `setUniform` is returned for
//!   the session to declare in the shaders ([`ScriptRun::uniforms`]), and once
//!   the shaders are compiled the session binds their names
//!   ([`ScriptRun::seal_uniforms`], [`ScriptRun::seal_uniforms_to`]) so each
//!   frame hands back the values in the order the player uploads them
//!   ([`ScriptRun::values`]), with no name lookups.
//! - **Meta controls.** A script may aim writes at `meta/...` controls
//!   (brightness, hue, mirror). They land in the session's meta bank
//!   ([`strobe_scene::meta`]) through the same queued path as every other
//!   control write; see D160 for what the bank does and does not drive.
//!
//! The host runs on the render thread. [`ScriptRun`] owns the frame output
//! the host fills in place, so a frame allocates nothing of its own once
//! those buffers have grown to size (the console, when a script logs, and
//! the script's own work in the engine, still do). A script that does
//! nothing costs far less than its 4 ms budget, and a script that is slow is
//! interrupted at the budget, not waited for.

use std::collections::HashSet;
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_scene::assemble::ExtraType;
use strobe_scene::control::ControlBank;
use strobe_scene::scene::SceneDef;
use strobe_script::{
    Budget, ConsoleLevel, FrameOutput, ScriptError, ScriptHost, ScriptInputs, ScriptStatus,
    ScriptUniform,
};

/// The log target script output is written under.
pub const LOG_TARGET: &str = "script";

/// How the script is doing, for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptState {
    /// Running normally.
    Running,
    /// The last frame faulted; the last good uniforms are in force.
    Faulted {
        /// Faulted frames in a row.
        consecutive: u32,
        /// The engine's message, with its line.
        message: String,
    },
    /// Switched off after repeated faults; reloading the scene restarts it.
    Disabled {
        /// The last fault.
        message: String,
    },
}

/// One scene's running script.
pub struct ScriptRun {
    host: ScriptHost,
    scene: String,
    state: ScriptState,
    /// Fault messages already reported as failures, so a fault that comes
    /// back streak after streak is reported once.
    reported: HashSet<String>,
    /// The last frame's output, refilled in place each frame.
    output: FrameOutput,
}

impl std::fmt::Debug for ScriptRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptRun")
            .field("scene", &self.scene)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl ScriptRun {
    /// Evaluates `source` (the scene's `script.js`) and runs its `setup()`.
    pub fn start(
        source: &str,
        scene: &str,
        controls: &ControlBank,
        meta: &ControlBank,
    ) -> Result<Self, ScriptError> {
        let host = ScriptHost::new(source, controls, meta, Budget::default())?;
        Ok(Self {
            host,
            scene: scene.to_string(),
            state: ScriptState::Running,
            reported: HashSet::new(),
            output: FrameOutput::default(),
        })
    }

    /// How the script is doing.
    pub fn state(&self) -> &ScriptState {
        &self.state
    }

    /// Runs one frame: events, then `update(dt)`, applying control writes
    /// to `controls` (and `meta/` writes to `meta`) before returning. The
    /// uniforms it left are at [`Self::uniforms`] and [`Self::values`].
    pub fn frame(
        &mut self,
        dt: f32,
        inputs: &ScriptInputs<'_>,
        controls: &mut ControlBank,
        meta: &mut ControlBank,
    ) {
        self.host
            .frame_into(dt, inputs, controls, meta, &mut self.output);
        self.harvest();
    }

    /// Every uniform the script has published as of the last frame, last
    /// good values. After [`Self::seal_uniforms`] this list cannot grow.
    pub fn uniforms(&self) -> &[ScriptUniform] {
        &self.output.uniforms
    }

    /// The values of the uniforms bound by [`Self::seal_uniforms`] or
    /// [`Self::seal_uniforms_to`], in the order given there, as of the last
    /// frame. Empty until one of them has been called.
    pub fn values(&self) -> &[[f32; 4]] {
        &self.output.bound
    }

    /// Logs the last frame's console and tracks its status.
    fn harvest(&mut self) {
        for line in &self.output.console {
            let level = match line.level {
                ConsoleLevel::Log => log::Level::Info,
                ConsoleLevel::Warn => log::Level::Warn,
                ConsoleLevel::Error => log::Level::Error,
            };
            log::log!(target: LOG_TARGET, level, "{}", line.text);
        }
        self.track(self.output.status);
    }

    /// Runs the frame a load runs before the shaders are compiled: as
    /// [`Self::frame`], but under the script's load budget rather than a
    /// frame's (D324).
    pub fn load_frame(
        &mut self,
        dt: f32,
        inputs: &ScriptInputs<'_>,
        controls: &mut ControlBank,
        meta: &mut ControlBank,
    ) {
        self.host
            .load_frame_into(dt, inputs, controls, meta, &mut self.output);
        self.harvest();
    }

    /// Seals the script's uniforms: the shaders have been compiled with
    /// what it published so far, so a later new name is refused with a
    /// console error instead of ever causing a recompile (D161).
    ///
    /// `shader_order` names the uniforms the shaders declare, in the order
    /// the player uploads them. They are resolved to the script's uniforms
    /// here, once, and [`Self::values`] carries them in that order from now
    /// on (D340).
    pub fn seal_uniforms<'a>(&mut self, shader_order: impl IntoIterator<Item = &'a str>) {
        self.host.seal_uniforms();
        self.host.bind_uniforms(shader_order);
        self.host.bound_uniforms_into(&mut self.output.bound);
    }

    /// Seals the script's uniforms to `names`, the ones the shaders were
    /// compiled with and in the order they declare them, for a script
    /// started again under them (D323). Bound like [`Self::seal_uniforms`].
    pub fn seal_uniforms_to<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        self.host.seal_uniforms_to(names);
        self.host.bound_uniforms_into(&mut self.output.bound);
    }

    /// Updates [`Self::state`] and reports the transitions that matter.
    fn track(&mut self, status: ScriptStatus) {
        // Only a fault needs its text, and a healthy frame is every frame.
        let fault_text = || {
            self.host
                .last_fault()
                .unwrap_or("unknown fault")
                .to_string()
        };
        match status {
            ScriptStatus::Running => {
                if matches!(self.state, ScriptState::Faulted { .. }) {
                    log::info!(target: LOG_TARGET, "script recovered");
                }
                self.state = ScriptState::Running;
            }
            ScriptStatus::Faulted { consecutive } => {
                let message = fault_text();
                // Distinct faults past this many are left to the UI state
                // and the script's console.
                const MAX_REPORTED: usize = 32;
                if consecutive == 1
                    && self.reported.len() < MAX_REPORTED
                    && self.reported.insert(message.clone())
                {
                    failure::report(
                        Failure::new(Subsystem::Script, "update")
                            .scene(&self.scene)
                            .asset("script.js")
                            .message(&message)
                            .action("this frame's uniforms discarded; the last good ones stay"),
                    );
                }
                self.state = ScriptState::Faulted {
                    consecutive,
                    message,
                };
            }
            ScriptStatus::Disabled => {
                // Disabled for good: the message cannot change, so a dead
                // script costs nothing each frame.
                if matches!(self.state, ScriptState::Disabled { .. }) {
                    return;
                }
                let message = fault_text();
                failure::report(
                    Failure::new(Subsystem::Script, "update")
                        .scene(&self.scene)
                        .asset("script.js")
                        .message(&message)
                        .action("script disabled; reload the scene to run it again"),
                );
                self.state = ScriptState::Disabled { message };
            }
        }
    }
}

/// The names a scene already declares as uniforms or samplers.
fn declared_names(def: &SceneDef) -> HashSet<&str> {
    let mut names: HashSet<&str> = HashSet::new();
    names.extend(def.controls.iter().map(|control| control.name.as_str()));
    names.extend(def.passes.iter().filter_map(|pass| pass.target.as_deref()));
    names.extend(def.images.iter().map(|image| image.name.as_str()));
    names.extend(def.cubemaps.iter().map(|cube| cube.name.as_str()));
    names.extend(
        def.transition_uniforms
            .iter()
            .map(|transition| transition.name.as_str()),
    );
    names
}

/// The script's uniforms minus any that would redeclare something the scene
/// already declares: redeclaring is a shader compile error, and one bad
/// name must not take the scene down. Each dropped name is warned about.
pub fn usable_extras(def: &SceneDef, wanted: Vec<(String, ExtraType)>) -> Vec<(String, ExtraType)> {
    let declared = declared_names(def);
    wanted
        .into_iter()
        .filter(|(name, _)| {
            let clash = declared.contains(name.as_str());
            if clash {
                log::warn!(
                    target: LOG_TARGET,
                    "setUniform(\"{name}\") collides with a name the scene already declares; \
                     the script's value is not sent to the shader (use setControl for controls)"
                );
            }
            !clash
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn controls(json: &str) -> Result<(SceneDef, ControlBank)> {
        let def = SceneDef::parse(json, "t")?;
        let bank = ControlBank::new(def.controls.clone())?;
        Ok((def, bank))
    }

    #[test]
    fn names_the_scene_declares_are_not_redeclared() -> Result<()> {
        let (def, _) = controls(
            r#"{ "CONTROLS": [ {"NAME": "amount", "TYPE": "slider"} ],
                 "PASSES": [ {"TARGET": "buf"} ] }"#,
        )?;
        let kept = usable_extras(
            &def,
            vec![
                ("amount".to_string(), ExtraType::Float),
                ("buf".to_string(), ExtraType::Vec2),
                ("orbit".to_string(), ExtraType::Vec4),
            ],
        );
        assert_eq!(
            kept,
            vec![("orbit".to_string(), ExtraType::Vec4)],
            "only the new name survives"
        );
        Ok(())
    }

    #[test]
    fn a_running_script_publishes_uniforms_and_reaches_controls() -> Result<()> {
        let (_, mut bank) = controls(r#"{ "CONTROLS": [ {"NAME": "level", "TYPE": "slider"} ] }"#)?;
        let mut meta = strobe_scene::meta::bank()?;
        let source =
            "function update(dt) { setUniform('glow', 0.25, 0.5); setControl('level', 0.75); }";
        let mut run = ScriptRun::start(source, "t", &bank, &meta)?;
        run.frame(1.0 / 60.0, &ScriptInputs::default(), &mut bank, &mut meta);
        let first = run
            .uniforms()
            .first()
            .ok_or_else(|| anyhow::anyhow!("no uniform published"))?;
        assert_eq!(&*first.name, "glow", "the uniform's name");
        assert_eq!(first.ty, ExtraType::Vec2, "typed by arity");
        assert!(
            (bank.target(0).map_or(0.0, |t| t[0]) - 0.75).abs() < 1e-6,
            "control write"
        );
        assert_eq!(run.state(), &ScriptState::Running, "healthy");
        Ok(())
    }

    #[test]
    fn values_come_back_in_the_order_the_shaders_declared_them() -> Result<()> {
        let (_, mut bank) = controls("{}")?;
        let mut meta = strobe_scene::meta::bank()?;
        // `buf` clashes with a pass target, so the shaders are built without
        // it, and `glow` is set before `tint` but declared after it.
        let source = "function update(dt) { setUniform('buf', 9); setUniform('glow', 1, 2); \
                      setUniform('tint', FRAMECOUNT); }";
        let mut run = ScriptRun::start(source, "t", &bank, &meta)?;
        run.load_frame(1.0 / 60.0, &ScriptInputs::default(), &mut bank, &mut meta);
        assert_eq!(run.uniforms().len(), 3, "all three were published");
        assert!(run.values().is_empty(), "nothing is bound before the seal");
        run.seal_uniforms(["tint", "glow"]);
        assert_eq!(
            run.values(),
            [[0.0; 4], [1.0, 2.0, 0.0, 0.0]],
            "the load frame's values, in shader order, at once"
        );
        let inputs = ScriptInputs {
            frame_count: 7,
            ..ScriptInputs::default()
        };
        run.frame(1.0 / 60.0, &inputs, &mut bank, &mut meta);
        assert_eq!(
            run.values(),
            [[7.0, 0.0, 0.0, 0.0], [1.0, 2.0, 0.0, 0.0]],
            "and every frame after"
        );
        Ok(())
    }

    #[test]
    fn a_restart_binds_to_the_shaders_names_and_a_late_one_is_found() -> Result<()> {
        let (_, mut bank) = controls("{}")?;
        let mut meta = strobe_scene::meta::bank()?;
        let source = "function update(dt) { setUniform('now', 1); \
                      if (FRAMECOUNT >= 3) setUniform('beat', 5); }";
        let mut run = ScriptRun::start(source, "t", &bank, &meta)?;
        run.load_frame(1.0 / 60.0, &ScriptInputs::default(), &mut bank, &mut meta);
        run.seal_uniforms_to(["beat", "now"]);
        assert_eq!(
            run.values(),
            [[0.0; 4], [1.0, 0.0, 0.0, 0.0]],
            "`beat` is declared and not set yet"
        );
        for frame_count in 1..=3 {
            let inputs = ScriptInputs {
                frame_count,
                ..ScriptInputs::default()
            };
            run.frame(1.0 / 60.0, &inputs, &mut bank, &mut meta);
        }
        assert_eq!(
            run.values(),
            [[5.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
            "found once the script sets it"
        );
        Ok(())
    }

    #[test]
    fn a_healthy_frame_refills_the_same_buffers() -> Result<()> {
        // The crate has no counting allocator (`unsafe` is banned outside
        // the GL layer), so this pins what keeps a steady-state frame from
        // allocating: the run's buffers keep their storage, and a frame with
        // nothing to report builds no strings.
        let (_, mut bank) = controls("{}")?;
        let mut meta = strobe_scene::meta::bank()?;
        let source = "function update(dt) { setUniform('a', FRAMECOUNT); \
                      setUniform('b', 1, 2, 3); }";
        let mut run = ScriptRun::start(source, "t", &bank, &meta)?;
        run.load_frame(1.0 / 60.0, &ScriptInputs::default(), &mut bank, &mut meta);
        run.seal_uniforms(["b", "a"]);
        let frame = |run: &mut ScriptRun, bank: &mut ControlBank, meta: &mut ControlBank, n| {
            let inputs = ScriptInputs {
                frame_count: n,
                ..ScriptInputs::default()
            };
            run.frame(1.0 / 60.0, &inputs, bank, meta);
        };
        frame(&mut run, &mut bank, &mut meta, 1);
        let values = (run.values().as_ptr(), run.output.bound.capacity());
        let uniforms = (run.uniforms().as_ptr(), run.output.uniforms.capacity());
        for n in 2..200 {
            frame(&mut run, &mut bank, &mut meta, n);
            assert_eq!(run.values().len(), 2, "frame {n}");
            assert_eq!(run.state(), &ScriptState::Running, "frame {n}");
        }
        assert_eq!(
            (run.values().as_ptr(), run.output.bound.capacity()),
            values,
            "the values were refilled in place"
        );
        assert_eq!(
            (run.uniforms().as_ptr(), run.output.uniforms.capacity()),
            uniforms,
            "and so was the uniform list"
        );
        Ok(())
    }

    #[test]
    fn faults_are_visible_and_three_in_a_row_disable_the_script() -> Result<()> {
        let (_, mut bank) = controls("{}")?;
        let mut meta = strobe_scene::meta::bank()?;
        let mut run = ScriptRun::start(
            "function update() { throw new Error('boom'); }",
            "t",
            &bank,
            &meta,
        )?;
        run.frame(0.016, &ScriptInputs::default(), &mut bank, &mut meta);
        assert!(
            matches!(run.state(), ScriptState::Faulted { consecutive: 1, .. }),
            "first fault: {:?}",
            run.state()
        );
        run.frame(0.016, &ScriptInputs::default(), &mut bank, &mut meta);
        run.frame(0.016, &ScriptInputs::default(), &mut bank, &mut meta);
        assert!(
            matches!(run.state(), ScriptState::Disabled { message } if message.contains("boom")),
            "disabled with the reason: {:?}",
            run.state()
        );
        Ok(())
    }

    #[test]
    fn a_fault_that_keeps_coming_back_is_reported_once() -> Result<()> {
        // A script that throws on every other frame (or on every beat)
        // never reaches three in a row, so it is never disabled. Each
        // streak used to be a new failure report: thirty a second, pushing
        // everything else out of the failure history and the log.
        const SCENE: &str = "intermittent-fault-scene";
        let (_, mut bank) = controls("{}")?;
        let mut meta = strobe_scene::meta::bank()?;
        let mut run = ScriptRun::start(
            "function update() { if (FRAMECOUNT % 2 === 1) throw new Error('odd frame'); }",
            SCENE,
            &bank,
            &meta,
        )?;
        for frame in 0..60 {
            let inputs = ScriptInputs {
                frame_count: frame,
                ..ScriptInputs::default()
            };
            run.frame(1.0 / 60.0, &inputs, &mut bank, &mut meta);
        }
        let reports = failure::recent(failure::KEPT)
            .iter()
            .filter(|reported| reported.to_string().contains(SCENE))
            .count();
        assert_eq!(
            reports, 1,
            "the same fault is reported once, not per streak"
        );
        assert!(
            matches!(run.state(), ScriptState::Faulted { .. }),
            "the state still shows it: {:?}",
            run.state()
        );
        Ok(())
    }

    #[test]
    fn a_script_that_does_not_load_is_an_error_not_a_panic() {
        let (Ok(bank), Ok(meta)) = (ControlBank::new(Vec::new()), strobe_scene::meta::bank())
        else {
            return;
        };
        assert!(
            ScriptRun::start("this is not javascript (", "t", &bank, &meta).is_err(),
            "a syntax error must be reported"
        );
    }
}
