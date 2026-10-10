// ============================================================================
// Module:       strobe_autocontrol (library crate root)
// Description:  Auto-generated controls: finds the literals that shape a
//               scene, rewrites them into uniforms and keeps the ones that
//               visibly change the image.
//
// Dependencies: strobe-scene, strobe-shadertoy (tokenizer), strobe-gfx (scoring)
// ============================================================================

//! Auto-generated controls.

pub mod analyze;
pub mod candidate;
pub mod naming;
pub mod rewrite;
pub mod source;
