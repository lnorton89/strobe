// ============================================================================
// Module:       strobe_app::ui::testing
// Description:  Test helper: one headless egui pass whose output is dropped
//               safely. Test code only.
//
// Dependencies: egui (headless context)
// ============================================================================

//! Running egui without a window.
//!
//! `Context::run_ui` returns a `FullOutput` whose texture deltas must be
//! applied or cleared, or dropping it panics (it is how egui catches an
//! integration that forgets to upload its font atlas). A test has nothing to
//! upload to, so every test pass goes through [`run`], which clears them and
//! hands back only what a test looks at.

use egui::{Context, RawInput, Rect, Ui, Vec2, epaint::ClippedShape};

/// What one pass produced.
pub(super) struct Pass {
    /// The tessellation input, in paint order.
    pub(super) shapes: Vec<ClippedShape>,
}

/// An input for a screen of `size` with the window focused.
pub(super) fn input(size: Vec2, events: Vec<egui::Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        focused: true,
        ..Default::default()
    }
}

/// Runs one pass of `add_contents` with `raw_input`.
pub(super) fn run(ctx: &Context, raw_input: RawInput, add_contents: impl FnMut(&mut Ui)) -> Pass {
    let mut output = ctx.run_ui(raw_input, add_contents);
    output.textures_delta.clear();
    Pass {
        shapes: output.shapes,
    }
}
