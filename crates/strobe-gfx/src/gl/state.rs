// ============================================================================
// Module:       strobe_gfx::gl::state
// Description:  D296 experiment (throwaway): a snapshot of the GL state that
//               decides whether a draw writes pixels, taken on demand or just
//               before the next full-screen draw, and a diff of two.
//
// Dependencies: glow; the parent module's Gl
// ============================================================================

//! The draw-time state dump for the AMD "draws never land above 16384
//! pixels" bisect (D296).
//!
//! On the JaJa mini PC the probe's trivial draw lands in a target of any
//! size while every scene draw into a target over 16384 pixels writes
//! nothing (clears land). The first question is whether the two draws run
//! under different GL state. [`Gl::state_snapshot`] reads every piece of
//! state that can stop a fragment reaching the colour attachment, and
//! [`Gl::arm_state_capture`] takes one just before the next
//! [`Gl::draw_fullscreen_triangle`], wherever that draw comes from, so the
//! player's real first draw is captured without touching the player.
//!
//! Diagnostics only: it queries the driver freely and runs only when a
//! caller arms it. When unarmed, a draw pays one `Cell` read.

use super::{Gl, Program};
use glow::HasContext;
use std::cell::{Cell, RefCell};

/// One captured state: named values, in a fixed order, so two snapshots
/// line up for [`GlState::diff`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlState {
    /// `(name, value)` pairs.
    pub entries: Vec<(String, String)>,
}

impl GlState {
    fn push(&mut self, name: &str, value: impl std::fmt::Display) {
        self.entries.push((name.to_owned(), value.to_string()));
    }

    /// The value recorded under `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Every entry whose value differs between `self` and `other` (or that
    /// only one has), as `name: self | other` lines.
    pub fn diff(&self, other: &Self) -> Vec<String> {
        let mut lines = Vec::new();
        for (name, value) in &self.entries {
            let theirs = other.get(name).unwrap_or("(absent)");
            if theirs != value {
                lines.push(format!("{name}: {value} | {theirs}"));
            }
        }
        for (name, value) in &other.entries {
            if self.get(name).is_none() {
                lines.push(format!("{name}: (absent) | {value}"));
            }
        }
        lines
    }
}

impl std::fmt::Display for GlState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (name, value) in &self.entries {
            writeln!(f, "  {name:<34} {value}")?;
        }
        Ok(())
    }
}

/// The arm-and-take slot shared by every clone of a [`Gl`].
#[derive(Debug, Default)]
pub(super) struct Capture {
    armed: Cell<bool>,
    taken: RefCell<Option<GlState>>,
}

impl Capture {
    /// Whether a capture is armed; a single `Cell` read.
    pub(super) fn armed(&self) -> bool {
        self.armed.get()
    }
}

/// A GL enum as hex, or a name for the few values that matter here.
fn enum_name(value: i32) -> String {
    let named = match value as u32 {
        glow::NONE => "NONE",
        glow::BACK => "BACK",
        glow::FRONT => "FRONT",
        glow::BACK_LEFT => "BACK_LEFT",
        glow::FRONT_LEFT => "FRONT_LEFT",
        glow::COLOR_ATTACHMENT0 => "COLOR_ATTACHMENT0",
        glow::COLOR_ATTACHMENT1 => "COLOR_ATTACHMENT1",
        glow::TEXTURE => "TEXTURE",
        glow::RENDERBUFFER => "RENDERBUFFER",
        glow::FRAMEBUFFER_DEFAULT => "FRAMEBUFFER_DEFAULT",
        glow::FRAMEBUFFER_COMPLETE => "FRAMEBUFFER_COMPLETE",
        glow::FILL => "FILL",
        glow::LINE => "LINE",
        glow::POINT => "POINT",
        glow::LINEAR => "LINEAR",
        glow::SRGB => "SRGB",
        glow::UNSIGNED_NORMALIZED => "UNSIGNED_NORMALIZED",
        glow::FLOAT => "FLOAT",
        glow::TEXTURE0 => "TEXTURE0",
        _ => return format!("0x{value:x}"),
    };
    named.to_owned()
}

impl Gl {
    /// Takes a snapshot just before the next full-screen draw (D296).
    pub fn arm_state_capture(&self) {
        self.capture.taken.replace(None);
        self.capture.armed.set(true);
    }

    /// The snapshot the armed capture took, if a draw has happened since.
    pub fn take_state_capture(&self) -> Option<GlState> {
        self.capture.taken.take()
    }

    /// Called by the draw wrapper when a capture is armed.
    pub(super) fn capture_now(&self) {
        self.capture.armed.set(false);
        let state = self.state_snapshot();
        self.capture.taken.replace(Some(state));
    }

    /// Everything that decides whether a draw writes the colour
    /// attachment, read from the driver now. Leaves the state as it found
    /// it (the active texture unit is restored), and reports how many GL
    /// errors the queries themselves raised.
    pub fn state_snapshot(&self) -> GlState {
        let mut s = GlState::default();
        let es = self.raw.version().is_embedded;
        s.push(
            "error pending before snapshot",
            self.error()
                .map_or("none".to_owned(), |code| format!("0x{code:x}")),
        );
        s.push("viewport", format!("{:?}", self.viewport_box()));
        s.push("scissor test", self.enabled(glow::SCISSOR_TEST));
        s.push("scissor box", format!("{:?}", self.scissor_box()));
        s.push(
            "color writemask",
            format!("{:?}", self.bools4(glow::COLOR_WRITEMASK)),
        );
        for (name, cap) in [
            ("blend", glow::BLEND),
            ("depth test", glow::DEPTH_TEST),
            ("stencil test", glow::STENCIL_TEST),
            ("cull face", glow::CULL_FACE),
            ("rasterizer discard", glow::RASTERIZER_DISCARD),
            ("polygon offset fill", glow::POLYGON_OFFSET_FILL),
            ("sample alpha to coverage", glow::SAMPLE_ALPHA_TO_COVERAGE),
            ("sample coverage", glow::SAMPLE_COVERAGE),
            ("dither", glow::DITHER),
        ] {
            s.push(name, self.enabled(cap));
        }
        if es {
            s.push(
                "primitive restart fixed index",
                self.enabled(glow::PRIMITIVE_RESTART_FIXED_INDEX),
            );
        } else {
            for (name, cap) in [
                ("primitive restart", glow::PRIMITIVE_RESTART),
                ("sample mask", glow::SAMPLE_MASK),
                ("multisample", glow::MULTISAMPLE),
                ("sample alpha to one", glow::SAMPLE_ALPHA_TO_ONE),
                ("depth clamp", glow::DEPTH_CLAMP),
                ("framebuffer srgb", glow::FRAMEBUFFER_SRGB),
                ("color logic op", glow::COLOR_LOGIC_OP),
                ("program point size", glow::PROGRAM_POINT_SIZE),
                ("clip distance 0", glow::CLIP_DISTANCE0),
            ] {
                s.push(name, self.enabled(cap));
            }
            s.push("polygon mode", enum_name(self.get_i32(glow::POLYGON_MODE)));
            s.push(
                "sample mask value 0",
                format!("0x{:x}", self.indexed_i32(glow::SAMPLE_MASK_VALUE, 0)),
            );
        }
        s.push("depth writemask", self.bool1(glow::DEPTH_WRITEMASK));
        s.push("samples", self.get_i32(glow::SAMPLES));
        s.push("sample buffers", self.get_i32(glow::SAMPLE_BUFFERS));
        s.push(
            "draw framebuffer",
            self.get_i32(glow::DRAW_FRAMEBUFFER_BINDING),
        );
        s.push(
            "read framebuffer",
            self.get_i32(glow::READ_FRAMEBUFFER_BINDING),
        );
        s.push(
            "draw framebuffer status",
            enum_name(self.draw_status() as i32),
        );
        for index in 0..4_u32 {
            s.push(
                &format!("draw buffer {index}"),
                enum_name(self.get_i32(glow::DRAW_BUFFER0 + index)),
            );
        }
        s.push("read buffer", enum_name(self.get_i32(glow::READ_BUFFER)));
        let attachment = |pname: u32| self.attachment_param(pname);
        s.push(
            "attachment 0 type",
            enum_name(attachment(glow::FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE)),
        );
        s.push(
            "attachment 0 name",
            attachment(glow::FRAMEBUFFER_ATTACHMENT_OBJECT_NAME),
        );
        s.push(
            "attachment 0 level",
            attachment(glow::FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL),
        );
        s.push(
            "attachment 0 layer",
            attachment(glow::FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER),
        );
        s.push(
            "attachment 0 component type",
            enum_name(attachment(glow::FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE)),
        );
        s.push(
            "attachment 0 encoding",
            enum_name(attachment(glow::FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING)),
        );
        s.push(
            "attachment 0 rgba bits",
            format!(
                "{} {} {} {}",
                attachment(glow::FRAMEBUFFER_ATTACHMENT_RED_SIZE),
                attachment(glow::FRAMEBUFFER_ATTACHMENT_GREEN_SIZE),
                attachment(glow::FRAMEBUFFER_ATTACHMENT_BLUE_SIZE),
                attachment(glow::FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE),
            ),
        );
        s.push("vertex array", self.get_i32(glow::VERTEX_ARRAY_BINDING));
        s.push("array buffer", self.get_i32(glow::ARRAY_BUFFER_BINDING));
        s.push("current program", self.get_i32(glow::CURRENT_PROGRAM));
        match self.current_program() {
            Some(program) => {
                self.validate(program);
                s.push("program validate status", self.validated(program));
                s.push(
                    "program validate log",
                    self.program_log(program).trim().replace('\n', " / "),
                );
                s.push(
                    "program active uniforms",
                    self.program_param(program, glow::ACTIVE_UNIFORMS),
                );
                s.push(
                    "program active attributes",
                    self.program_param(program, glow::ACTIVE_ATTRIBUTES),
                );
                for name in ["strobe_FragColor", "color"] {
                    s.push(
                        &format!("frag data location {name}"),
                        self.frag_location(program, name),
                    );
                }
            }
            None => s.push("program validate status", "(no program)"),
        }
        let active = self.get_i32(glow::ACTIVE_TEXTURE);
        s.push("active texture", enum_name(active));
        for unit in 0..6_u32 {
            self.active_texture(unit);
            s.push(
                &format!("unit {unit} 2D / 1D / cube"),
                format!(
                    "{} / {} / {}",
                    self.get_i32(glow::TEXTURE_BINDING_2D),
                    if es {
                        0
                    } else {
                        self.get_i32(glow::TEXTURE_BINDING_1D)
                    },
                    self.get_i32(glow::TEXTURE_BINDING_CUBE_MAP),
                ),
            );
        }
        let unit = (active as u32).saturating_sub(glow::TEXTURE0);
        self.active_texture(unit);
        s.push("errors raised by the snapshot", self.clear_errors());
        s
    }

    /// Whether `program`'s fragment output `name` is at location 0, from
    /// outside a draw. For the bisect's shader rows.
    pub fn frag_data_location(&self, program: &Program, name: &str) -> i32 {
        self.frag_location(program.raw, name)
    }

    /// Draws `count` vertices as triangles with the bound program and VAO
    /// (the bisect's in-range quad: six vertices).
    pub fn draw_triangles(&self, count: i32) {
        if self.capture.armed() {
            self.capture_now();
        }
        // SAFETY: current-context invariant; no vertex buffer is read, the
        // vertex shader derives positions from gl_VertexID.
        unsafe { self.raw.draw_arrays(glow::TRIANGLES, 0, count) }
    }

    fn enabled(&self, cap: u32) -> bool {
        // SAFETY: current-context invariant; an unknown capability is a GL
        // error, not undefined behaviour.
        unsafe { self.raw.is_enabled(cap) }
    }

    fn bools4(&self, name: u32) -> [bool; 4] {
        // SAFETY: current-context invariant; glow passes a four-slot array,
        // the size `GL_COLOR_WRITEMASK` writes.
        unsafe { self.raw.get_parameter_bool_array::<4>(name) }
    }

    fn bool1(&self, name: u32) -> bool {
        // SAFETY: current-context invariant; single-valued boolean query.
        unsafe { self.raw.get_parameter_bool(name) }
    }

    fn draw_status(&self) -> u32 {
        // SAFETY: current-context invariant.
        unsafe { self.raw.check_framebuffer_status(glow::DRAW_FRAMEBUFFER) }
    }

    fn attachment_param(&self, pname: u32) -> i32 {
        // SAFETY: current-context invariant; on the default framebuffer
        // this is a GL error (counted), not undefined behaviour.
        unsafe {
            self.raw.get_framebuffer_attachment_parameter_i32(
                glow::DRAW_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                pname,
            )
        }
    }

    fn current_program(&self) -> Option<glow::Program> {
        // SAFETY: current-context invariant; single-valued integer query.
        unsafe { self.raw.get_parameter_program(glow::CURRENT_PROGRAM) }
    }

    fn validate(&self, program: glow::Program) {
        // SAFETY: current-context invariant; the program is the current one.
        unsafe { self.raw.validate_program(program) }
    }

    fn validated(&self, program: glow::Program) -> bool {
        // SAFETY: current-context invariant.
        unsafe { self.raw.get_program_validate_status(program) }
    }

    fn program_param(&self, program: glow::Program, pname: u32) -> i32 {
        // SAFETY: current-context invariant; single-valued program query.
        unsafe { self.raw.get_program_parameter_i32(program, pname) }
    }

    fn indexed_i32(&self, name: u32, index: u32) -> i32 {
        // SAFETY: current-context invariant; single-valued indexed query.
        unsafe { self.raw.get_parameter_indexed_i32(name, index) }
    }

    fn frag_location(&self, program: glow::Program, name: &str) -> i32 {
        // SAFETY: current-context invariant; glow passes a NUL-terminated
        // copy of `name`.
        unsafe { self.raw.get_frag_data_location(program, name) }
    }
}
