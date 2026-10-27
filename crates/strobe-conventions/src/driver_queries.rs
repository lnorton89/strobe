// ============================================================================
// Module:       strobe_conventions::driver_queries
// Description:  Enforces that every GL state query goes through a counted
//               leaf in strobe-gfx's gl module, so a test can prove the frame
//               loop makes none.
//
// Dependencies: crate::sources (the workspace walk and function lookup)
// ============================================================================

//! No `glGet*` in the frame loop (D497).
//!
//! A state query (`glGetIntegerv`, `glGetError`, `glIsEnabled`, a compile
//! status or log, a uniform location) is a round trip to the driver that
//! can stall the pipeline until the GPU catches up. The renderer resolves
//! everything it needs at load time (`player::build_program`, `caps`), and
//! the rule is that a playing scene asks the driver nothing.
//!
//! The proof is in two halves. At run time, `strobe_gfx::gl::Gl` counts
//! every state query (`Gl::state_queries`), and `strobe-gfx`'s tests
//! (`render.rs`'s `a_playing_scene_asks_the_driver_nothing`,
//! `compositor.rs`'s custom transition) assert that the count stops moving
//! once a scene plays. That catches a query however deeply the frame path
//! reaches it. Allocation is not a steady frame: a resize, or a playlist
//! warming its next scene, makes framebuffers and checks them, by design.
//! This module is the other half: it makes the counter trustworthy, by
//! failing the build for a query that does not count itself.
//!
//! - In `crates/strobe-gfx/src/gl/`, every function that calls a glow query
//!   method must call `self.count_query()` before it, or be one of the
//!   [`EXEMPT`] leaves, each with its reason.
//! - Anywhere else, a glow query method is a failure: GL calls go through
//!   the `gl` module's wrappers, never through `glow` directly.

use crate::sources::{enclosing_function, function_name};

/// The glow `HasContext` methods that ask the driver for state. A call is
/// spelled `.<prefix>` followed by the rest of the method name.
pub const QUERIES: [&str; 19] = [
    "check_framebuffer_status",
    "get_parameter",
    "get_error(",
    "get_uniform",
    "get_shader_",
    "get_program_",
    "get_query_parameter",
    "get_active_",
    "get_attrib_location",
    "get_frag_data",
    "get_tex_",
    "get_buffer_",
    "get_framebuffer_",
    "get_renderbuffer_",
    "get_sync_",
    "get_debug_",
    "get_internal_format",
    "get_vertex_attrib",
    "is_enabled(",
];

/// The one query name other APIs share (egui's `Ui::is_enabled`), so it is
/// only judged inside the `gl` module, where it can only be glow's.
const OUTSIDE_AMBIGUOUS: &str = "is_enabled(";

/// The module that holds every GL call.
const GL_MODULE: &str = "crates/strobe-gfx/src/gl/";

/// This crate's own sources, which spell the method names in their checks.
const SELF: &str = "crates/strobe-conventions/";

/// The leaves allowed to query the driver without counting: `(file, fn,
/// why)`. Each is part of how a frame avoids waiting, or runs before a
/// `Gl` exists.
pub const EXEMPT: [(&str, &str, &str); 3] = [
    (
        "crates/strobe-gfx/src/gl/mod.rs",
        "query_u32",
        "a timer query's result, read only once it says it is available",
    ),
    (
        "crates/strobe-gfx/src/gl/mod.rs",
        "disjoint_flag",
        "GL_GPU_DISJOINT_EXT, read only alongside an available timer result",
    ),
    (
        "crates/strobe-gfx/src/gl/debug.rs",
        "is_enabled",
        "debug output set-up on the raw context, before a `Gl` exists",
    ),
];

/// The query method a line calls, if any (comments stripped).
fn query_on(line: &str) -> Option<&'static str> {
    let code = line.split("//").next().unwrap_or_default();
    QUERIES
        .iter()
        .copied()
        .find(|query| code.contains(&format!(".{query}")))
}

/// Every uncounted or misplaced driver query in one file.
pub fn violations(relative: &str, text: &str) -> Vec<String> {
    let mut failures = Vec::new();
    if relative.starts_with(SELF) {
        return failures;
    }
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let Some(query) = query_on(line) else {
            continue;
        };
        let location = format!("{relative}:{}", index + 1);
        if !relative.starts_with(GL_MODULE) {
            if query == OUTSIDE_AMBIGUOUS {
                continue;
            }
            failures.push(format!(
                "{location}: `{query}` queries the driver outside {GL_MODULE}; \
                 call a counted `Gl` wrapper instead"
            ));
            continue;
        }
        let Some(start) = enclosing_function(&lines, index) else {
            failures.push(format!("{location}: `{query}` outside any function"));
            continue;
        };
        let name = lines
            .get(start)
            .and_then(|declaration| function_name(declaration))
            .unwrap_or_default();
        let exempt = EXEMPT
            .iter()
            .any(|(file, function, _)| *file == relative && *function == name);
        let counted = lines
            .get(start..index)
            .is_some_and(|body| body.iter().any(|code| code.trim() == "self.count_query();"));
        if !exempt && !counted {
            failures.push(format!(
                "{location}: `{name}` calls `{query}` without `self.count_query();` first, \
                 so `Gl::state_queries` cannot see it"
            ));
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::workspace_sources;
    use anyhow::Result;

    #[test]
    fn every_driver_query_is_counted_and_in_the_gl_module() -> Result<()> {
        let sources = workspace_sources()?;
        let failures: Vec<String> = sources
            .iter()
            .flat_map(|source| violations(&source.relative, &source.text))
            .collect();
        assert!(
            failures.is_empty(),
            "driver query violation(s):\n{}",
            failures.join("\n")
        );
        let gl = sources
            .iter()
            .find(|source| source.relative == "crates/strobe-gfx/src/gl/mod.rs");
        assert!(
            gl.is_some_and(|source| source.text.contains("self.count_query();")),
            "the gl module should count its queries; has the counter moved?"
        );
        Ok(())
    }

    #[test]
    fn an_uncounted_query_is_reported() {
        let text = "    fn get_i32(&self, name: u32) -> i32 {\n\
                    \x20       // SAFETY: current-context invariant.\n\
                    \x20       unsafe { self.raw.get_parameter_i32(name) }\n\
                    \x20   }\n";
        let found = violations("crates/strobe-gfx/src/gl/mod.rs", text);
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("without `self.count_query();`")),
            "got {found:?}"
        );
        let counted = text.replacen(
            "        // SAFETY",
            "        self.count_query();\n        // SAFETY",
            1,
        );
        assert!(
            violations("crates/strobe-gfx/src/gl/mod.rs", &counted).is_empty(),
            "a counted query is allowed"
        );
    }

    #[test]
    fn a_count_in_the_function_above_does_not_cover_the_next_one() {
        let text = "    fn error(&self) -> u32 {\n\
                    \x20       self.count_query();\n\
                    \x20       0\n\
                    \x20   }\n\
                    \x20   fn get_i32(&self) -> i32 {\n\
                    \x20       unsafe { self.raw.get_parameter_i32(1) }\n\
                    \x20   }\n";
        let found = violations("crates/strobe-gfx/src/gl/mod.rs", text);
        assert_eq!(found.len(), 1, "got {found:?}");
    }

    #[test]
    fn the_exempt_leaves_need_no_count_but_only_where_they_are_named() {
        let text = "    fn query_u32(&self, query: glow::Query, parameter: u32) -> u32 {\n\
                    \x20       unsafe { self.raw.get_query_parameter_u32(query, parameter) }\n\
                    \x20   }\n";
        assert!(
            violations("crates/strobe-gfx/src/gl/mod.rs", text).is_empty(),
            "the timer result leaf is exempt"
        );
        assert!(
            !violations("crates/strobe-gfx/src/gl/window.rs", text).is_empty(),
            "the exemption names one file"
        );
    }

    #[test]
    fn a_query_outside_the_gl_module_is_reported() {
        let text = "fn frame(gl: &glow::Context) {\n    let e = gl.get_error();\n}\n";
        let found = violations("crates/strobe-gfx/src/player.rs", text);
        assert!(
            found.iter().any(|problem| problem.contains("outside")),
            "got {found:?}"
        );
        assert!(
            violations(
                "crates/strobe-gfx/src/player.rs",
                "let e = gl.error(); // not .get_error() itself\n"
            )
            .is_empty(),
            "a counted wrapper, and a comment, are fine"
        );
    }
}
