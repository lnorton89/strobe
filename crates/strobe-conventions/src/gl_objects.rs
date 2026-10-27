// ============================================================================
// Module:       strobe_conventions::gl_objects
// Description:  Enforces that every GL object has an owning type with a Drop
//               in strobe-gfx's gl module, and that no raw object handle is
//               stored anywhere else.
//
// Dependencies: crate::sources (the workspace walk)
// ============================================================================

//! Every GL object has an owning type with a `Drop` (D497).
//!
//! A GL object (a texture, a framebuffer, a program) is GPU memory that
//! Rust cannot see. If the handle is a plain value, an early return or a
//! `?` leaks it, and a show that switches scenes for four hours leaks it
//! hundreds of times. So `strobe-gfx`'s `gl` module wraps each one in an
//! owning type whose `Drop` deletes it (`Texture`, `Program`, ...), and
//! nothing outside the module ever holds the raw handle. Three checks:
//!
//! - In `crates/strobe-gfx/src/gl/`, a struct with a field whose type is a
//!   raw glow object (`raw: glow::Texture,`, or an `Option` of one) has an
//!   `impl Drop for` it.
//! - Every glow call there that creates an object (`create_texture`,
//!   `fence_sync`, ...) makes a kind some such struct owns.
//! - Anywhere else, a struct field of a raw glow object type is a failure.
//!   Handles passed to a library that manages its own (egui's painter, which
//!   takes a texture to draw) are arguments and map keys, not fields.

use std::collections::BTreeSet;

/// glow's object handle types, and the call that creates each.
pub const OBJECTS: [(&str, &str); 11] = [
    ("Shader", "create_shader("),
    ("Program", "create_program("),
    ("Texture", "create_texture("),
    ("Framebuffer", "create_framebuffer("),
    ("Renderbuffer", "create_renderbuffer("),
    ("Query", "create_query("),
    ("Buffer", "create_buffer("),
    ("Fence", "fence_sync("),
    ("VertexArray", "create_vertex_array("),
    ("Sampler", "create_sampler("),
    ("TransformFeedback", "create_transform_feedback("),
];

/// The module that owns every GL object.
const GL_MODULE: &str = "crates/strobe-gfx/src/gl/";

/// This crate's own sources, which spell the patterns in their checks.
const SELF: &str = "crates/strobe-conventions/";

/// The raw glow object a struct field line holds, if any:
/// `    raw: glow::Texture,` and `Option<glow::Texture>` hold `Texture`; a
/// map keyed by one does not (the key names a handle someone else owns).
fn field_object(line: &str) -> Option<&'static str> {
    let code = line.split("//").next().unwrap_or_default().trim();
    let code = code.strip_prefix("pub ").unwrap_or(code);
    let (name, kind) = code.split_once(": ")?;
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let kind = kind.strip_suffix(',')?.trim();
    let kind = kind
        .strip_prefix("Option<")
        .and_then(|inner| inner.strip_suffix('>'))
        .unwrap_or(kind);
    let kind = kind.strip_prefix("egui_glow::").unwrap_or(kind);
    OBJECTS
        .iter()
        .map(|(object, _)| *object)
        .find(|object| kind == format!("glow::{object}"))
}

/// The name a line declares a struct of (`pub struct Texture {`).
fn struct_name(line: &str) -> Option<&str> {
    let code = line.trim_start();
    let code = [
        "pub struct ",
        "pub(crate) struct ",
        "pub(super) struct ",
        "struct ",
    ]
    .iter()
    .find_map(|prefix| code.strip_prefix(prefix))?;
    let end = code
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(code.len());
    code.get(..end).filter(|name| !name.is_empty())
}

/// What the `gl` module declares: each struct holding a raw object, with the
/// object and where it is, and every type with an `impl Drop`.
#[derive(Debug, Default)]
pub struct Owners {
    /// `(struct, object, "path:line")` for each raw-object field.
    pub holders: Vec<(String, &'static str, String)>,
    /// Every type with an `impl Drop for`.
    pub dropped: BTreeSet<String>,
    /// `(object, "path:line")` for each object-creating call.
    pub created: Vec<(&'static str, String)>,
}

impl Owners {
    /// Adds what one file under the `gl` module declares.
    pub fn read(&mut self, relative: &str, text: &str) {
        let mut current: Option<String> = None;
        for (index, line) in text.lines().enumerate() {
            let location = format!("{relative}:{}", index + 1);
            if let Some(name) = struct_name(line) {
                current = line.trim_end().ends_with('{').then(|| name.to_string());
            } else if line.starts_with('}') {
                current = None;
            }
            if let (Some(owner), Some(object)) = (&current, field_object(line)) {
                self.holders.push((owner.clone(), object, location.clone()));
            }
            let code = line.split("//").next().unwrap_or_default();
            if let Some(rest) = code.trim_start().strip_prefix("impl Drop for ") {
                let end = rest
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                if let Some(name) = rest.get(..end) {
                    self.dropped.insert(name.to_string());
                }
            }
            for (object, call) in OBJECTS {
                if code.contains(&format!(".{call}")) {
                    self.created.push((object, location.clone()));
                }
            }
        }
    }

    /// Every broken ownership rule found.
    pub fn problems(&self) -> Vec<String> {
        let mut found = Vec::new();
        for (owner, object, location) in &self.holders {
            if !self.dropped.contains(owner) {
                found.push(format!(
                    "{location}: `{owner}` holds a raw `glow::{object}` but has no `impl Drop`"
                ));
            }
        }
        for (object, location) in &self.created {
            let owned = self
                .holders
                .iter()
                .any(|(owner, held, _)| held == object && self.dropped.contains(owner));
            if !owned {
                found.push(format!(
                    "{location}: creates a `glow::{object}` that no type with a `Drop` owns"
                ));
            }
        }
        found
    }
}

/// Every raw object stored as a struct field outside the `gl` module.
pub fn stray_handles(relative: &str, text: &str) -> Vec<String> {
    if relative.starts_with(GL_MODULE) || relative.starts_with(SELF) {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            field_object(line).map(|object| {
                format!(
                    "{relative}:{}: stores a raw `glow::{object}`; hold the owning type \
                     from strobe_gfx::gl instead",
                    index + 1
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::workspace_sources;
    use anyhow::Result;

    #[test]
    fn every_gl_object_has_an_owning_type_with_a_drop() -> Result<()> {
        let sources = workspace_sources()?;
        let mut owners = Owners::default();
        for source in sources
            .iter()
            .filter(|source| source.relative.starts_with(GL_MODULE))
        {
            owners.read(&source.relative, &source.text);
        }
        let mut failures = owners.problems();
        for source in &sources {
            failures.extend(stray_handles(&source.relative, &source.text));
        }
        assert!(
            owners.holders.len() >= 8 && owners.created.len() >= 8,
            "found {} owning fields and {} create calls; has the gl module moved?",
            owners.holders.len(),
            owners.created.len()
        );
        assert!(
            failures.is_empty(),
            "GL object ownership violation(s):\n{}",
            failures.join("\n")
        );
        Ok(())
    }

    #[test]
    fn a_holder_without_a_drop_is_reported() {
        let text = "pub struct Sampler {\n    gl: Gl,\n    raw: glow::Sampler,\n}\n\
                    fn make(&self) {\n    unsafe { self.raw.create_sampler() }\n}\n";
        let mut owners = Owners::default();
        owners.read("crates/strobe-gfx/src/gl/mod.rs", text);
        let found = owners.problems();
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("has no `impl Drop`")),
            "got {found:?}"
        );
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("no type with a `Drop` owns")),
            "got {found:?}"
        );

        owners.read(
            "crates/strobe-gfx/src/gl/mod.rs",
            "impl Drop for Sampler {\n    fn drop(&mut self) {}\n}\n",
        );
        assert!(owners.problems().is_empty(), "a Drop fixes both");
    }

    #[test]
    fn a_create_with_no_owner_at_all_is_reported() {
        let mut owners = Owners::default();
        owners.read(
            "crates/strobe-gfx/src/gl/mod.rs",
            "fn make(&self) {\n    unsafe { self.raw.create_renderbuffer() }\n}\n",
        );
        assert_eq!(owners.problems().len(), 1, "got {:?}", owners.problems());
    }

    #[test]
    fn a_raw_handle_stored_outside_the_gl_module_is_reported() {
        let text = "struct Preview {\n    texture: glow::Texture,\n    \
                    previews: HashMap<glow::Texture, TextureId>,\n    \
                    maybe: Option<glow::Texture>,\n}\n";
        let found = stray_handles("crates/strobe-app/src/ui/host.rs", text);
        assert_eq!(
            found.len(),
            2,
            "the field and the optional field: {found:?}"
        );
        assert!(
            stray_handles("crates/strobe-gfx/src/gl/mod.rs", text).is_empty(),
            "the gl module is where owners live"
        );
    }
}
