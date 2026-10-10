// ============================================================================
// Module:       strobe_app::screenshot
// Description:  The --screenshot mode: when to capture each window, and
//               writing the read-back pixels to PNG.
//
// Dependencies: image
// ============================================================================

//! Screenshot mode.
//!
//! This is how the UI is checked with no human and no screen-capture tool:
//! the app reads each window's own back buffer with `glReadPixels` after
//! drawing and before the swap, so the result is what that window drew,
//! whatever else is on the desktop.
//!
//! The output capture has two sources. A *shown* output is read from its own
//! back buffer, so the run checks what the projector would get, letterboxing
//! and all. A *hidden* one (the default, D96) is read from the scene's output
//! texture instead: the default framebuffer of a window nobody can see has
//! undefined contents, and the texture does not depend on the window at all.
//!
//! The output window is captured after `--frames` frames have been
//! rendered. The control window is captured after that, once it has also
//! drawn a few frames of its own: egui lays a window out over two passes,
//! and the first frames after a scene load are not representative.

use std::path::PathBuf;

/// Control-window frames drawn after the output capture before the control
/// capture, so egui's layout has settled.
const CONTROL_SETTLE_FRAMES: u32 = 4;

/// Which window a capture is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The control window, with the egui UI.
    Control,
    /// The output window, with the letterboxed scene.
    Output,
}

impl Target {
    fn suffix(self) -> &'static str {
        match self {
            Self::Control => "-control.png",
            Self::Output => "-output.png",
        }
    }
}

/// The state of one screenshot run.
#[derive(Debug)]
pub struct Screenshot {
    base: PathBuf,
    frames: u32,
    output_done: bool,
    control_done: bool,
    control_frames_since: u32,
}

impl Screenshot {
    /// A run that captures after `frames` output frames, writing to
    /// `<base>-control.png` and `<base>-output.png`.
    pub fn new(base: PathBuf, frames: u32) -> Self {
        Self {
            base,
            frames,
            output_done: false,
            control_done: false,
            control_frames_since: 0,
        }
    }

    /// Whether the output window should be captured now, given how many
    /// output frames have been presented.
    pub fn output_due(&self, presented: u64) -> bool {
        !self.output_done && presented >= u64::from(self.frames)
    }

    /// Whether the control window should be captured on this frame.
    /// Counts the call as one control frame once the output is captured.
    pub fn control_due(&mut self) -> bool {
        if !self.output_done || self.control_done {
            return false;
        }
        self.control_frames_since += 1;
        self.control_frames_since > CONTROL_SETTLE_FRAMES
    }

    /// Whether the *next* call to [`Self::control_due`] will say yes: the
    /// frame before the control capture, which is when a state that must be
    /// on screen during the capture (a load in progress) has to be asked for.
    pub fn control_capture_next(&self) -> bool {
        self.output_done
            && !self.control_done
            && self.control_frames_since + 1 > CONTROL_SETTLE_FRAMES
    }

    /// Whether both captures are written.
    pub fn finished(&self) -> bool {
        self.output_done && self.control_done
    }

    /// The file a capture of `target` is written to.
    pub fn path(&self, target: Target) -> PathBuf {
        let mut name = self.base.clone().into_os_string();
        name.push(target.suffix());
        PathBuf::from(name)
    }

    /// Writes a capture of `target`: `pixels` are RGBA8, bottom row first,
    /// as `glReadPixels` returns them.
    pub fn save(
        &mut self,
        target: Target,
        size: [u32; 2],
        pixels: Vec<u8>,
    ) -> Result<PathBuf, String> {
        self.write(target, size, pixels, true)
    }

    /// Writes a capture of `target` whose `pixels` are RGBA8, top row
    /// first, as `ScenePlayer::read_output` returns them.
    pub fn save_top_down(
        &mut self,
        target: Target,
        size: [u32; 2],
        pixels: Vec<u8>,
    ) -> Result<PathBuf, String> {
        self.write(target, size, pixels, false)
    }

    fn write(
        &mut self,
        target: Target,
        size: [u32; 2],
        mut pixels: Vec<u8>,
        flip: bool,
    ) -> Result<PathBuf, String> {
        // The window has no alpha channel; read-back alpha is not meaningful.
        let (texels, _) = pixels.as_chunks_mut::<4>();
        for [_, _, _, alpha] in texels {
            *alpha = u8::MAX;
        }
        let mut image = image::RgbaImage::from_raw(size[0], size[1], pixels)
            .ok_or_else(|| format!("{target:?} read-back has the wrong size for {size:?}"))?;
        if flip {
            image::imageops::flip_vertical_in_place(&mut image);
        }
        let path = self.path(target);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        image
            .save(&path)
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
        match target {
            Target::Control => self.control_done = true,
            Target::Output => self.output_done = true,
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_append_the_window_name_to_the_base() {
        let shot = Screenshot::new(PathBuf::from("target/shot"), 10);
        assert_eq!(
            shot.path(Target::Control),
            PathBuf::from("target/shot-control.png"),
            "control"
        );
        assert_eq!(
            shot.path(Target::Output),
            PathBuf::from("target/shot-output.png"),
            "output"
        );
    }

    #[test]
    fn the_frame_before_the_control_capture_is_announced() {
        let mut shot = Screenshot::new(PathBuf::from("x"), 3);
        assert!(!shot.control_capture_next(), "output not captured yet");
        shot.output_done = true;
        let mut announced_at = None;
        let mut captured_at = None;
        // As the app does it: a control frame draws (and asks whether to
        // capture), then, at its end, asks whether the next one will.
        for frame in 1..=CONTROL_SETTLE_FRAMES + 3 {
            if shot.control_due() {
                captured_at = Some(frame);
                break;
            }
            if announced_at.is_none() && shot.control_capture_next() {
                announced_at = Some(frame);
            }
        }
        assert_eq!(
            announced_at.map(|f| f + 1),
            captured_at,
            "announced exactly one frame before the capture"
        );
    }

    #[test]
    fn the_control_capture_waits_for_the_output_and_for_layout_to_settle() {
        let mut shot = Screenshot::new(PathBuf::from("x"), 3);
        assert!(!shot.output_due(2), "too early");
        assert!(shot.output_due(3), "due");
        assert!(!shot.control_due(), "output not captured yet");
        shot.output_done = true;
        let due_at = (1..=CONTROL_SETTLE_FRAMES + 2).find(|_| shot.control_due());
        assert_eq!(due_at, Some(CONTROL_SETTLE_FRAMES + 1), "after settling");
    }
}
