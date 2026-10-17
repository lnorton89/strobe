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
//!   the session to declare in the shaders and upload each frame.
//! - **Meta controls.** A script may aim writes at `meta/...` controls
//!   (brightness, hue, mirror). They land in the session's meta bank
//!   ([`strobe_scene::meta`]) through the same queued path as every other
//!   control write; see D160 for what the bank does and does not drive.
//!
//! The host runs on the render thread. It allocates a little each frame
//! (its output is built as owned vectors); a script that does nothing costs
//! far less than its 4 ms budget, and a script that is slow is interrupted
//! at the budget, not waited for.

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

/// What a frame of script leaves for the session.
#[derive(Debug, Default)]
pub struct Harvest {
    /// Every uniform the script has published, last good values. After
    /// [`ScriptRun::seal_uniforms`] this list cannot grow.
    pub uniforms: Vec<ScriptUniform>,
}

/// One scene's running script.
pub struct ScriptRun {
    host: ScriptHost,
    scene: String,
    state: ScriptState,
    /// Fault messages already reported as failures, so a fault that comes
    /// back streak after streak is reported once.
    reported: HashSet<String>,
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
        })
    }

    /// How the script is doing.
    pub fn state(&self) -> &ScriptState {
        &self.state
    }

    /// Runs one frame: events, then `update(dt)`, applying control writes
    /// to `controls` (and `meta/` writes to `meta`) before returning.
    pub fn frame(
        &mut self,
        dt: f32,
        inputs: &ScriptInputs<'_>,
        controls: &mut ControlBank,
        meta: &mut ControlBank,
    ) -> Harvest {
        let output = self.host.frame(dt, inputs, controls, meta);
        self.harvest(output)
    }

    /// Logs a frame's console, tracks its status and keeps its uniforms.
    fn harvest(&mut self, output: FrameOutput) -> Harvest {
        for line in &output.console {
            let level = match line.level {
                ConsoleLevel::Log => log::Level::Info,
                ConsoleLevel::Warn => log::Level::Warn,
                ConsoleLevel::Error => log::Level::Error,
            };
            log::log!(target: LOG_TARGET, level, "{}", line.text);
        }
        self.track(output.status);
        Harvest {
            uniforms: output.uniforms,
        }
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
    ) -> Harvest {
        let output = self.host.load_frame(dt, inputs, controls, meta);
        self.harvest(output)
    }

    /// Seals the script's uniforms: the shaders have been compiled with
    /// what it published so far, so a later new name is refused with a
    /// console error instead of ever causing a recompile (D161).
    pub fn seal_uniforms(&mut self) {
        self.host.seal_uniforms();
    }

    /// Seals the script's uniforms to `names`, the ones the shaders were
    /// compiled with, for a script started again under them (D323).
    pub fn seal_uniforms_to<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        self.host.seal_uniforms_to(names);
    }

    /// Updates [`Self::state`] and reports the transitions that matter.
    fn track(&mut self, status: ScriptStatus) {
        let message = self
            .host
            .last_fault()
            .unwrap_or("unknown fault")
            .to_string();
        match status {
            ScriptStatus::Running => {
                if matches!(self.state, ScriptState::Faulted { .. }) {
                    log::info!(target: LOG_TARGET, "script recovered");
                }
                self.state = ScriptState::Running;
            }
            ScriptStatus::Faulted { consecutive } => {
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
                if !matches!(self.state, ScriptState::Disabled { .. }) {
                    failure::report(
                        Failure::new(Subsystem::Script, "update")
                            .scene(&self.scene)
                            .asset("script.js")
                            .message(&message)
                            .action("script disabled; reload the scene to run it again"),
                    );
                }
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
        let harvest = run.frame(1.0 / 60.0, &ScriptInputs::default(), &mut bank, &mut meta);
        let first = harvest
            .uniforms
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
