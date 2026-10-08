// ============================================================================
// Module:       strobe_scene::control
// Description:  Scene controls: their types, validated definitions, and the
//               per-frame runtime that turns UI input into uniform values.
//
// Dependencies: crate::json, crate::error, crate::names
// ============================================================================

//! Controls.
//!
//! [`kind`] parses a `TYPE` string, [`def`] validates a whole control
//! object, and [`runtime`] holds the live values a running scene reads.

pub mod def;
pub mod kind;
pub mod runtime;

pub use def::ControlDef;
pub use kind::{Bang, ControlKind, Mode};
pub use runtime::{ControlBank, UniformValue};
