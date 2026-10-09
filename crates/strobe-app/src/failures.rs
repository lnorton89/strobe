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

use crate::session::LoadError;
use std::path::Path;
use strobe_diag::failure::{Failure, Subsystem};
use strobe_gfx::PlayerError;
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
                .detail("pass", pass)
                .detail("file", "main.glsl");
            if let Some(line) = user_line(message) {
                failure = failure.detail("line", line);
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

/// A compiler log without the warnings the generated preamble itself
/// provokes (`_uv` used uninitialized, on every scene, on Mesa). They bury
/// the one error a person can act on. Errors in generated code are kept.
fn without_preamble_warnings(log: &str) -> String {
    log.lines()
        .filter(|line| !(line.starts_with("(generated):") && line.contains("warning:")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first `main.glsl:N` line number in a mapped compiler log.
fn user_line(log: &str) -> Option<u32> {
    let (_, rest) = log.split_once("main.glsl:")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::path::PathBuf;

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

    #[test]
    fn the_line_parser_reads_the_first_main_glsl_reference() -> Result<()> {
        assert_eq!(user_line("x\nmain.glsl:7: a\nmain.glsl:9: b"), Some(7));
        assert_eq!(user_line("(generated): oops"), None, "none present");
        Ok(())
    }
}
