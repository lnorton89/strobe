// ============================================================================
// Module:       strobe_app::failures
// Description:  Turns the errors the app can hit into strobe-diag structured
//               failures: subsystem, scene, phase, line, root error, action.
//
// Dependencies: strobe-diag (failure); strobe-gfx (PlayerError);
//               strobe-scene (SceneError); crate::session (LoadError)
// ============================================================================

//! User-visible failures.
//!
//! Every failure a person can see should say what failed, where, in which
//! phase, why, and what the program did about it
//! (`docs/AGENT_RESEARCH.md` section 12). The conversions live here, away
//! from the event loop, so each shape is a unit test rather than something
//! only a broken scene on a projector would show.

use crate::recovery::{Decision, Rebuild};
use crate::session::LoadError;
use std::path::Path;
use strobe_diag::failure::{Failure, Subsystem};
use strobe_gfx::{Backend, PlayerError};
use strobe_scene::SceneError;
use strobe_script::ScriptError;

/// What the app did about a scene that would not load.
fn load_action(keeps_playing: bool) -> &'static str {
    if keeps_playing {
        "scene not loaded; the previous scene keeps playing"
    } else {
        "scene not loaded; nothing is playing"
    }
}

/// The failure for a scene that could not be loaded or compiled.
pub fn scene_load(name: &str, dir: &Path, error: &LoadError, keeps_playing: bool) -> Failure {
    let action = load_action(keeps_playing);
    match error {
        LoadError::Scene(inner) => {
            let phase = match inner {
                SceneError::Io { .. } | SceneError::MissingShader(_) => "read",
                SceneError::Json(_)
                | SceneError::UnknownControlType { .. }
                | SceneError::InvalidModifier { .. }
                | SceneError::InvalidName(_)
                | SceneError::NameCollision { .. }
                | SceneError::ReservedName(_)
                | SceneError::Arity { .. }
                | SceneError::Dropdown(_)
                | SceneError::PassSize { .. }
                | SceneError::PassOption { .. } => "validate",
            };
            Failure::new(Subsystem::Io, phase)
                .scene(name)
                .path(dir)
                .message(inner)
                .action(action)
        }
        LoadError::Player(PlayerError::Program { pass, message }) => {
            let phase = if message.starts_with("link:") {
                "link"
            } else {
                "compile"
            };
            let mut failure = Failure::new(Subsystem::Shader, phase)
                .scene(name)
                .detail("pass", pass);
            // The compiler log names the pass's own file (`main.glsl`, or
            // `passes/buffer_a.glsl` for an imported Shadertoy buffer); a
            // failure in generated code or at link time has no user site.
            if let Some((file, line)) = user_site(message) {
                failure = failure.detail("file", file).detail("line", line);
            }
            failure
                .message(without_preamble_warnings(message))
                .action(action)
        }
        LoadError::Player(PlayerError::Resource(message)) => {
            Failure::new(Subsystem::Context, "allocate")
                .scene(name)
                .path(dir)
                .message(message)
                .action(action)
        }
    }
}

/// The failure for a `script.js` that did not start. The scene runs
/// without its script, so the person is told exactly that.
pub fn script_load(name: &str, error: &ScriptError) -> Failure {
    let phase = match error {
        ScriptError::Engine(_) => "start",
        ScriptError::Load { .. } => "load",
        ScriptError::Setup { .. } => "setup",
    };
    Failure::new(Subsystem::Script, phase)
        .scene(name)
        .asset("script.js")
        .message(error)
        .action("the scene runs without its script")
}

/// The short name of a backend, as the session context and the failure
/// records spell it.
pub fn backend_label(backend: Backend) -> &'static str {
    match backend {
        Backend::Native => "native",
        Backend::Angle => "angle",
    }
}

/// What to say the app is about to do for a rebuild.
fn rebuild_action(rebuild: &Rebuild) -> String {
    if rebuild.failover {
        format!(
            "failing over to {}; the output goes dark until it is up",
            backend_label(rebuild.backend)
        )
    } else {
        format!(
            "rebuilding the context on {}; the output goes dark until it is up",
            backend_label(rebuild.backend)
        )
    }
}

/// The failure for a GL context that was lost or became unusable: which
/// window noticed, what the driver said, what the reset query answered,
/// which backend it was, and what the app will do next.
pub fn context_lost(rebuild: &Rebuild, before: Backend, scene: Option<&str>) -> Failure {
    let mut failure = Failure::new(Subsystem::Context, "lost");
    if let Some(scene) = scene {
        failure = failure.scene(scene);
    }
    if let Some(window) = rebuild.cause.window() {
        failure = failure.detail("window", window);
    }
    failure
        .detail(
            "reset_status",
            rebuild.cause.reset_status().map_or_else(
                || "not queried (the context was not current)".to_string(),
                |status| status.to_string(),
            ),
        )
        .detail("backend", backend_label(before))
        .message(&rebuild.cause)
        .action(rebuild_action(rebuild))
}

/// The failure record for a rebuild that worked. It is a record, not an
/// error: the point is that the next person to read the log sees that the
/// output went away, for how long, and on which backend it came back.
pub fn context_rebuilt(
    rebuild: &Rebuild,
    before: Backend,
    elapsed_ms: u128,
    scene: Option<&str>,
    reloaded: bool,
) -> Failure {
    let mut failure = Failure::new(Subsystem::Context, "rebuild");
    if let Some(scene) = scene {
        failure = failure.scene(scene);
    }
    failure
        .detail("backend_before", backend_label(before))
        .detail("backend_after", backend_label(rebuild.backend))
        .detail("elapsed_ms", elapsed_ms)
        .detail("outcome", "recovered")
        .message(&rebuild.cause)
        .action(match (scene, reloaded) {
            (Some(name), true) => format!("output recovered; {name} reloaded"),
            (Some(name), false) => {
                format!("output recovered; {name} could not be reloaded, showing black")
            }
            (None, _) => "output recovered; no scene was playing".to_string(),
        })
}

/// The failure for a rebuild that did not come up, with what happens next.
pub fn context_rebuild_failed(
    backend: Backend,
    error: &str,
    before: Backend,
    next: &Decision,
) -> Failure {
    Failure::new(Subsystem::Context, "rebuild")
        .detail("backend_before", backend_label(before))
        .detail("backend_attempted", backend_label(backend))
        .detail("outcome", "failed")
        .message(error)
        .action(match next {
            Decision::Rebuild(rebuild) => rebuild_action(rebuild),
            Decision::GiveUp(reason) => format!("giving up: {reason}"),
            Decision::Continue | Decision::RetryNextFrame => "continuing".to_string(),
        })
}

/// A compiler log without the warnings the generated preamble itself
/// provokes (`_uv` used uninitialized, on every scene, on Mesa). They bury
/// the one error a person can act on. Errors in generated code are kept.
fn without_preamble_warnings(log: &str) -> String {
    log.lines()
        .filter(|line| !(line.starts_with("(generated):") && line.contains("warning:")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first user-code site in a mapped compiler log: the shader file as the
/// scene names it and the line in it. `strobe-gfx` rewrites every driver
/// line that points into user code as `<file>:<line>: ...` where `<file>` is
/// a path relative to the scene folder (`main.glsl`, `passes/buffer_a.glsl`).
fn user_site(log: &str) -> Option<(&str, u32)> {
    log.lines().find_map(|line| {
        let (stem, rest) = line.split_once(".glsl:")?;
        // The file is the whole start of the line, so no spaces: a driver
        // message that merely mentions a `.glsl:` is not a site.
        if stem.is_empty() || stem.contains(char::is_whitespace) {
            return None;
        }
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let number = digits.parse().ok()?;
        Some((&line[..stem.len() + ".glsl".len()], number))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::{Cause, Surface};
    use anyhow::Result;
    use std::path::PathBuf;
    use strobe_gfx::gl::ResetStatus;

    #[test]
    fn a_compile_error_carries_the_pass_the_file_and_the_line() -> Result<()> {
        let error = LoadError::Player(PlayerError::Program {
            pass: 2,
            message: "main.glsl:142: 0:142(7): error: syntax error".to_string(),
        });
        let failure = scene_load("09-sdf-cathedral", Path::new("scenes/09"), &error, true);
        let text = failure.to_string();
        for needle in [
            "SHADER",
            "scene=09-sdf-cathedral",
            "phase=compile",
            "pass=2",
            "file=main.glsl",
            "line=142",
            "keeps playing",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        Ok(())
    }

    #[test]
    fn a_link_error_is_a_link_phase_and_a_generated_error_has_no_line() -> Result<()> {
        let error = LoadError::Player(PlayerError::Program {
            pass: 0,
            message: "link: undefined symbol".to_string(),
        });
        let text = scene_load("x", Path::new("x"), &error, false).to_string();
        assert!(text.contains("phase=link"), "phase:\n{text}");
        assert!(!text.contains("line="), "no user line:\n{text}");
        assert!(text.contains("nothing is playing"), "action:\n{text}");
        Ok(())
    }

    #[test]
    fn a_missing_shader_is_a_read_failure_with_the_path() -> Result<()> {
        let error = LoadError::Scene(SceneError::MissingShader(PathBuf::from("scenes/none")));
        let text = scene_load("none", Path::new("scenes/none"), &error, true).to_string();
        assert!(text.starts_with("IO\n"), "subsystem:\n{text}");
        assert!(text.contains("phase=read"), "phase:\n{text}");
        assert!(text.contains("path=scenes/none"), "path:\n{text}");
        Ok(())
    }

    #[test]
    fn preamble_warnings_are_dropped_but_errors_are_kept() -> Result<()> {
        let log = "(generated): 0:3(5): warning: `_uv' used uninitialized\n\
                   (generated): 0:9(1): error: broken preamble\n\
                   main.glsl:2: 1:2(19): error: undeclared";
        let kept = without_preamble_warnings(log);
        assert!(!kept.contains("warning"), "warning kept:\n{kept}");
        assert!(kept.contains("broken preamble"), "generated error lost");
        assert!(kept.contains("main.glsl:2"), "user error lost");
        Ok(())
    }

    #[test]
    fn a_script_that_did_not_load_says_the_scene_runs_without_it() -> Result<()> {
        let error = ScriptError::Load {
            message: "unexpected token (script.js:3)".to_string(),
        };
        let text = script_load("11-script-orbits", &error).to_string();
        for needle in [
            "SCRIPT",
            "scene=11-script-orbits",
            "asset=script.js",
            "phase=load",
            "unexpected token",
            "runs without its script",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        Ok(())
    }

    fn lost_after_three_frames(failover: bool) -> Rebuild {
        Rebuild {
            backend: if failover {
                Backend::Angle
            } else {
                Backend::Native
            },
            failover,
            cause: Cause::FailedFrames {
                window: Surface::Output,
                count: 3,
                error: "make current: [6] The handle is invalid. (os error 6)".to_string(),
            },
        }
    }

    #[test]
    fn a_loss_says_which_window_what_error_which_backend_and_what_happens_next() -> Result<()> {
        let text = context_lost(
            &lost_after_three_frames(false),
            Backend::Native,
            Some("09-sdf"),
        )
        .to_string();
        for needle in [
            "CONTEXT",
            "scene=09-sdf",
            "phase=lost",
            "window=output",
            "reset_status=not queried",
            "backend=native",
            "The handle is invalid",
            "rebuilding the context on native",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        Ok(())
    }

    #[test]
    fn a_fail_over_loss_says_so() -> Result<()> {
        let text = context_lost(&lost_after_three_frames(true), Backend::Native, None).to_string();
        assert!(text.contains("failing over to angle"), "action:\n{text}");
        assert!(!text.contains("scene="), "no scene was playing:\n{text}");
        Ok(())
    }

    #[test]
    fn a_loss_with_a_driver_reset_carries_its_status() -> Result<()> {
        let rebuild = Rebuild {
            backend: Backend::Native,
            failover: false,
            cause: Cause::Reset {
                window: Surface::Output,
                status: ResetStatus::Innocent,
                error: None,
            },
        };
        let text = context_lost(&rebuild, Backend::Native, None).to_string();
        assert!(
            text.contains("reset_status=innocent context reset"),
            "status:\n{text}"
        );
        Ok(())
    }

    #[test]
    fn a_recovery_records_the_backends_the_time_and_the_scene() -> Result<()> {
        let text = context_rebuilt(
            &lost_after_three_frames(true),
            Backend::Native,
            412,
            Some("09-sdf"),
            true,
        )
        .to_string();
        for needle in [
            "phase=rebuild",
            "backend_before=native",
            "backend_after=angle",
            "elapsed_ms=412",
            "outcome=recovered",
            "output recovered; 09-sdf reloaded",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        Ok(())
    }

    #[test]
    fn a_recovery_without_a_scene_or_with_a_failed_reload_says_so() -> Result<()> {
        let rebuild = lost_after_three_frames(false);
        let none = context_rebuilt(&rebuild, Backend::Native, 5, None, false).to_string();
        assert!(none.contains("no scene was playing"), "no scene:\n{none}");
        let failed = context_rebuilt(&rebuild, Backend::Native, 5, Some("x"), false).to_string();
        assert!(
            failed.contains("could not be reloaded"),
            "failed:\n{failed}"
        );
        Ok(())
    }

    #[test]
    fn a_failed_rebuild_names_the_next_step() -> Result<()> {
        let next = Decision::Rebuild(lost_after_three_frames(true));
        let text =
            context_rebuild_failed(Backend::Native, "context: refused", Backend::Native, &next)
                .to_string();
        for needle in [
            "outcome=failed",
            "backend_attempted=native",
            "context: refused",
            "failing over to angle",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        let give_up = Decision::GiveUp("ANGLE failed too".to_string());
        let text =
            context_rebuild_failed(Backend::Angle, "display: none", Backend::Native, &give_up)
                .to_string();
        assert!(
            text.contains("giving up: ANGLE failed too"),
            "give up:\n{text}"
        );
        Ok(())
    }

    #[test]
    fn the_site_parser_reads_the_first_user_reference() -> Result<()> {
        assert_eq!(
            user_site("x\nmain.glsl:7: a\nmain.glsl:9: b"),
            Some(("main.glsl", 7)),
            "first reference"
        );
        assert_eq!(user_site("(generated): oops"), None, "none present");
        assert_eq!(
            user_site("error near the .glsl: file\npasses/buffer_a.glsl:14: 0:14(7): bad"),
            Some(("passes/buffer_a.glsl", 14)),
            "prose that mentions .glsl: is not a site"
        );
        Ok(())
    }

    #[test]
    fn a_buffer_pass_failure_names_its_own_file_and_line() -> Result<()> {
        let error = LoadError::Player(PlayerError::Program {
            pass: 0,
            message: "(generated): 0:3(5): warning: `_uv' used uninitialized\n\
                      passes/buffer_a.glsl:14: 0:14(7): error: syntax error"
                .to_string(),
        });
        let text = scene_load("tunnel-XsXXDn", Path::new("x"), &error, true).to_string();
        for needle in [
            "phase=compile",
            "pass=0",
            "file=passes/buffer_a.glsl",
            "line=14",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        assert!(
            !text.contains("file=main.glsl"),
            "must not claim main.glsl:\n{text}"
        );
        Ok(())
    }
}
