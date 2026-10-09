// ============================================================================
// Module:       strobe_app::mouse
// Description:  The pointer over the output window, turned into SSF's mouse
//               uniforms (_mouse, _click, _muv, _muvc) in render coordinates.
//
// Dependencies: strobe-gfx (Mouse)
// ============================================================================

//! The mouse uniforms.
//!
//! The output window letterboxes the render target (see
//! `ScenePlayer::present`), so a pointer position in window pixels has to
//! be mapped through the same rectangle to land on the scene pixel under it.
//! Positions outside the picture clamp to its edge.
//!
//! | Uniform | Value |
//! |---|---|
//! | `_mouse` | `xy` pointer in render pixels, origin bottom-left; `zw` the last click, positive while a button is held and negative after (Shadertoy's `iMouse` convention, without its once-per-frame `w` pulse) |
//! | `_click` | left, right, middle held (`0` or `1`) |
//! | `_muv` | pointer, `0..1` across the picture |
//! | `_muvc` | pointer in `_uvc` space: centred, scaled by height |
//!
//! Before the pointer has ever entered the window every value is zero
//! (`docs/OPEN_QUESTIONS.md`, Q2).

use strobe_gfx::Mouse;

/// A mouse button the uniforms report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Left.
    Left,
    /// Right.
    Right,
    /// Middle.
    Middle,
}

/// What is known about the pointer.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MouseTracker {
    /// Window pixels, origin top-left, as winit reports them.
    position: Option<[f32; 2]>,
    held: [bool; 3],
    /// The window position of the last press.
    clicked_at: Option<[f32; 2]>,
}

impl MouseTracker {
    /// The pointer moved to `position` (window pixels, origin top-left).
    pub fn moved(&mut self, position: [f32; 2]) {
        self.position = Some(position);
    }

    /// A button went down or up at the last known position.
    pub fn button(&mut self, button: Button, down: bool) {
        let slot = match button {
            Button::Left => 0,
            Button::Right => 1,
            Button::Middle => 2,
        };
        if let Some(held) = self.held.get_mut(slot) {
            *held = down;
        }
        if down {
            self.clicked_at = self.position;
        }
    }

    /// The uniforms for a `window` of pixels showing a render target of
    /// `render` pixels letterboxed to fit.
    pub fn uniforms(&self, window: [u32; 2], render: [i32; 2]) -> Mouse {
        let Some(position) = self.position else {
            return Mouse::default();
        };
        let to_render = |window_position: [f32; 2]| to_render(window_position, window, render);
        let [x, y] = to_render(position);
        let (w, h) = (render[0].max(1) as f32, render[1].max(1) as f32);
        let any_held = self.held.iter().any(|held| *held);
        let [cx, cy] = self.clicked_at.map_or([0.0, 0.0], to_render);
        let sign = if any_held { 1.0 } else { -1.0 };
        let flag = |held: bool| if held { 1.0 } else { 0.0 };
        Mouse {
            mouse: [x, y, cx * sign, cy * sign],
            click: [flag(self.held[0]), flag(self.held[1]), flag(self.held[2])],
            muv: [x / w, y / h],
            muvc: [(x - 0.5 * w) / h, (y - 0.5 * h) / h],
        }
    }
}

/// A window position in render pixels (origin bottom-left), clamped to the
/// picture. The picture is the render target scaled to fit and centred, as
/// `ScenePlayer::present` draws it.
fn to_render(position: [f32; 2], window: [u32; 2], render: [i32; 2]) -> [f32; 2] {
    let (ww, wh) = (window[0].max(1) as f32, window[1].max(1) as f32);
    let (w, h) = (render[0].max(1) as f32, render[1].max(1) as f32);
    let scale = (ww / w).min(wh / h);
    let (left, bottom) = ((ww - w * scale) / 2.0, (wh - h * scale) / 2.0);
    // winit's y grows downward, GL's upward.
    let gl_y = wh - position[1];
    [
        ((position[0] - left) / scale).clamp(0.0, w),
        ((gl_y - bottom) / scale).clamp(0.0, h),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn before_the_pointer_arrives_everything_is_zero() {
        let tracker = MouseTracker::default();
        assert_eq!(tracker.uniforms([800, 600], [400, 300]), Mouse::default());
    }

    #[test]
    fn the_centre_of_the_window_is_the_centre_of_the_picture() {
        let mut tracker = MouseTracker::default();
        tracker.moved([400.0, 300.0]);
        let mouse = tracker.uniforms([800, 600], [400, 300]);
        assert!(
            near(mouse.muv[0], 0.5) && near(mouse.muv[1], 0.5),
            "{mouse:?}"
        );
        assert!(
            near(mouse.muvc[0], 0.0) && near(mouse.muvc[1], 0.0),
            "{mouse:?}"
        );
        assert!(
            near(mouse.mouse[0], 200.0) && near(mouse.mouse[1], 150.0),
            "{mouse:?}"
        );
    }

    #[test]
    fn the_top_left_of_the_window_is_the_top_left_of_the_picture() {
        let mut tracker = MouseTracker::default();
        tracker.moved([0.0, 0.0]);
        let mouse = tracker.uniforms([800, 600], [400, 300]);
        assert!(near(mouse.muv[0], 0.0), "left edge: {mouse:?}");
        assert!(
            near(mouse.muv[1], 1.0),
            "winit's top is GL's top: {mouse:?}"
        );
    }

    #[test]
    fn letterbox_bars_clamp_to_the_picture_edge() {
        // A 16:9 picture in a square window has bars above and below.
        let mut tracker = MouseTracker::default();
        tracker.moved([500.0, 0.0]);
        let mouse = tracker.uniforms([1000, 1000], [160, 90]);
        assert!(
            near(mouse.muv[1], 1.0),
            "above the picture clamps: {mouse:?}"
        );
        assert!(near(mouse.muv[0], 0.5), "x is unaffected: {mouse:?}");
    }

    #[test]
    fn a_held_button_makes_the_click_position_positive() {
        let mut tracker = MouseTracker::default();
        tracker.moved([400.0, 300.0]);
        tracker.button(Button::Left, true);
        let down = tracker.uniforms([800, 600], [400, 300]);
        assert!(down.mouse[2] > 0.0 && near(down.click[0], 1.0), "{down:?}");
        tracker.button(Button::Left, false);
        let up = tracker.uniforms([800, 600], [400, 300]);
        assert!(up.mouse[2] < 0.0 && near(up.click[0], 0.0), "{up:?}");
    }
}
