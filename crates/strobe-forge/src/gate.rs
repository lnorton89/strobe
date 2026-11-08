// ============================================================================
// Module:       strobe_forge::gate
// Description:  The quality gates a generated scene must pass, measured on
//               its own frames: black, flat, blown out, frozen, flashing.
//
// Dependencies: serde
// ============================================================================

//! The gates, ported from the website's `site/src/lib/play/image/gate.ts`
//! (D560) so a scene made by the skill or by `strobe-forge make` is held to
//! the same rules as one made in the browser (D562).
//!
//! The flash rule follows WCAG 2.3.1 and ITU-R BT.1702: a flash is a pair of
//! opposing changes in relative luminance of at least 0.10 with the darker
//! state under 0.80, and no more than three a second over a quarter of the
//! screen. It is a screen, not a certificate.
//!
//! Frames are RGBA8, top row first, all the same size, evenly spaced at
//! [`FrameSet::rate`] frames a second.

use serde::Serialize;

/// RGBA frames of one size, evenly spaced in time.
pub(crate) struct FrameSet<'a> {
    /// Width in pixels.
    pub(crate) width: usize,
    /// Height in pixels.
    pub(crate) height: usize,
    /// Frames a second they were taken at.
    pub(crate) rate: f32,
    /// The frames, RGBA8.
    pub(crate) frames: &'a [Vec<u8>],
}

/// What the gates measured, and which failed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct GateResult {
    /// Mean luma (`0..1`) over every pixel of every frame.
    pub(crate) mean: f32,
    /// Standard deviation of luma over the same pixels.
    pub(crate) spread: f32,
    /// Fraction of pixels brighter than 0.97.
    pub(crate) blown: f32,
    /// The most any frame's green channel differs from the first's, `0..1`.
    pub(crate) change: f32,
    /// The most flashes a second a quarter of the picture reaches.
    pub(crate) flashes_per_second: f32,
    /// Mean under [`BLACK`].
    pub(crate) black: bool,
    /// Lit but one flat colour.
    pub(crate) flat: bool,
    /// More than half the pixels blown out.
    pub(crate) blown_out: bool,
    /// Nothing moves.
    pub(crate) frozen: bool,
    /// Over the flash limit.
    pub(crate) flashing: bool,
}

/// Mean luma under which a picture is almost black.
pub(crate) const BLACK: f32 = 0.012;
/// Luma spread under which a lit picture is one flat colour.
const FLAT: f32 = 0.012;
/// Luma over which a pixel is blown out.
const BLOWN_PIXEL: f32 = 0.97;
/// Fraction of blown pixels over which the picture is blown out.
pub(crate) const BLOWN_SHARE: f32 = 0.5;
/// Largest change from the first frame under which the scene is frozen.
const FROZEN: f32 = 0.0008;
/// The most flashes a second allowed over a quarter of the screen.
const FLASH_LIMIT: f32 = 3.0;

impl GateResult {
    /// No hard gate failed: not black, not frozen, not flashing (flat and
    /// blown out are warnings).
    #[cfg(test)]
    fn hard_pass(&self) -> bool {
        !self.black && !self.frozen && !self.flashing
    }

    /// The hard failures, as a person is told them.
    pub(crate) fn hard_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.flashing {
            out.push(format!(
                "flashes {:.1} times a second over a quarter of the picture",
                self.flashes_per_second
            ));
        }
        if self.black {
            out.push("almost black".to_owned());
        }
        if self.frozen {
            out.push("does not move".to_owned());
        }
        out
    }

    /// The warnings: flat and blown out.
    pub(crate) fn soft_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.flat {
            out.push("one flat colour".to_owned());
        }
        if self.blown_out {
            out.push("blown out".to_owned());
        }
        out
    }

    /// Every problem, hard first.
    #[cfg(test)]
    fn problems(&self) -> Vec<String> {
        let mut out = self.hard_problems();
        out.extend(self.soft_problems());
        out
    }
}

/// sRGB byte to linear light.
fn linear(byte: u8) -> f32 {
    let c = f32::from(byte) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Relative luminance per cell of a coarse grid, one vector per frame.
fn cell_luminance(set: &FrameSet<'_>, columns: usize, rows: usize) -> Vec<Vec<f32>> {
    let mut table = [0.0_f32; 256];
    for (byte, slot) in table.iter_mut().enumerate() {
        *slot = linear(byte as u8);
    }
    set.frames
        .iter()
        .map(|data| {
            let mut sums = vec![0.0_f32; columns * rows];
            let mut counts = vec![0_u32; columns * rows];
            for y in 0..set.height {
                let row = (y * rows / set.height.max(1)).min(rows - 1);
                for x in 0..set.width {
                    let column = (x * columns / set.width.max(1)).min(columns - 1);
                    let i = (y * set.width + x) * 4;
                    let cell = row * columns + column;
                    let (Some(r), Some(g), Some(b)) =
                        (data.get(i), data.get(i + 1), data.get(i + 2))
                    else {
                        continue;
                    };
                    sums[cell] += 0.2126 * table[usize::from(*r)]
                        + 0.7152 * table[usize::from(*g)]
                        + 0.0722 * table[usize::from(*b)];
                    counts[cell] += 1;
                }
            }
            sums.iter()
                .zip(&counts)
                .map(|(sum, count)| sum / (*count).max(1) as f32)
                .collect()
        })
        .collect()
}

/// Flashes in one cell's luminance over time: pairs of opposing large
/// changes, the turning point following a change that keeps going.
pub(crate) fn count_flashes(values: &[f32]) -> u32 {
    let large = |a: f32, b: f32| (a - b).abs() >= 0.1 && a.min(b) < 0.8;
    let mut transitions = 0_u32;
    let mut direction = 0.0_f32;
    let mut turn = values.first().copied().unwrap_or(0.0);
    for &value in values {
        if direction == 0.0 {
            if large(value, turn) {
                direction = (value - turn).signum();
                transitions += 1;
                turn = value;
            }
        } else if (value - turn) * direction > 0.0 {
            turn = value;
        } else if large(value, turn) {
            direction = -direction;
            transitions += 1;
            turn = value;
        }
    }
    transitions / 2
}

/// Runs every gate over `set`.
pub(crate) fn gate(set: &FrameSet<'_>) -> GateResult {
    const COLUMNS: usize = 8;
    const ROWS: usize = 6;
    let cells = cell_luminance(set, COLUMNS, ROWS);
    let rate = set.rate.max(1e-3);
    let seconds = (1.0 / rate).max(set.frames.len().saturating_sub(1) as f32 / rate);

    // Per cell, its flashes a second; the 75th percentile is what a quarter
    // of the screen reaches or passes.
    let mut per_cell: Vec<f32> = (0..COLUMNS * ROWS)
        .map(|cell| {
            let series: Vec<f32> = cells
                .iter()
                .map(|frame| frame.get(cell).copied().unwrap_or(0.0))
                .collect();
            count_flashes(&series) as f32 / seconds
        })
        .collect();
    per_cell.sort_by(f32::total_cmp);
    let flashes_per_second = per_cell.get(per_cell.len() * 3 / 4).copied().unwrap_or(0.0);

    let mut total = 0.0_f64;
    let mut squared = 0.0_f64;
    let mut blown = 0_u64;
    let mut count = 0_u64;
    for data in set.frames {
        for px in data.as_chunks::<4>().0 {
            let v =
                (0.2126 * f64::from(px[0]) + 0.7152 * f64::from(px[1]) + 0.0722 * f64::from(px[2]))
                    / 255.0;
            total += v;
            squared += v * v;
            if v > f64::from(BLOWN_PIXEL) {
                blown += 1;
            }
            count += 1;
        }
    }
    let n = count.max(1) as f64;
    let mean = (total / n) as f32;
    let spread = ((squared / n - (total / n).powi(2)).max(0.0)).sqrt() as f32;
    let blown = (blown as f64 / n) as f32;

    // The most any frame differs from the first, since a loop can end where
    // it began.
    let mut change = 0.0_f32;
    if let Some(first) = set.frames.first() {
        let pixels = (first.len() / 4).max(1) as f32;
        for data in set.frames {
            let sum: u64 = first
                .as_chunks::<4>()
                .0
                .iter()
                .zip(data.as_chunks::<4>().0)
                .map(|(a, b)| u64::from(a[1].abs_diff(b[1])))
                .sum();
            change = change.max(sum as f32 / 255.0 / pixels);
        }
    }

    GateResult {
        mean,
        spread,
        blown,
        change,
        flashes_per_second,
        black: mean < BLACK,
        flat: spread < FLAT && mean >= BLACK,
        blown_out: blown > BLOWN_SHARE,
        frozen: set.frames.len() > 1 && change < FROZEN,
        flashing: flashes_per_second > FLASH_LIMIT,
    }
}

/// Mean luma (`0..1`) and blown share of one RGBA8 frame.
pub(crate) fn mean_and_blown(frame: &[u8]) -> (f32, f32) {
    let mut total = 0.0_f64;
    let mut blown = 0_u64;
    let mut count = 0_u64;
    for px in frame.as_chunks::<4>().0 {
        let v = (0.2126 * f64::from(px[0]) + 0.7152 * f64::from(px[1]) + 0.0722 * f64::from(px[2]))
            / 255.0;
        total += v;
        if v > f64::from(BLOWN_PIXEL) {
            blown += 1;
        }
        count += 1;
    }
    let n = count.max(1) as f64;
    ((total / n) as f32, (blown as f64 / n) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, ensure};

    fn solid(width: usize, height: usize, value: u8) -> Vec<u8> {
        [value, value, value, 255].repeat(width * height)
    }

    #[test]
    fn a_square_wave_of_four_a_second_is_flashing() -> Result<()> {
        // 30 fps, two seconds, light and dark swapping every 4 frames: 3.75
        // full flashes a second over the whole picture.
        let frames: Vec<Vec<u8>> = (0..60)
            .map(|i| solid(16, 12, if (i / 4) % 2 == 0 { 20 } else { 230 }))
            .collect();
        let got = gate(&FrameSet {
            width: 16,
            height: 12,
            rate: 30.0,
            frames: &frames,
        });
        ensure!(got.flashing, "expected flashing: {got:?}");
        ensure!(!got.hard_pass(), "flashing is a hard failure");
        Ok(())
    }

    #[test]
    fn a_slow_fade_is_not_flashing_and_moves() -> Result<()> {
        let frames: Vec<Vec<u8>> = (0..60).map(|i| solid(16, 12, 60 + i as u8)).collect();
        let got = gate(&FrameSet {
            width: 16,
            height: 12,
            rate: 30.0,
            frames: &frames,
        });
        ensure!(!got.flashing, "a fade is not a flash: {got:?}");
        ensure!(!got.frozen, "it changes: {got:?}");
        Ok(())
    }

    #[test]
    fn black_and_frozen_are_caught() -> Result<()> {
        let frames = vec![solid(8, 8, 0); 10];
        let got = gate(&FrameSet {
            width: 8,
            height: 8,
            rate: 30.0,
            frames: &frames,
        });
        ensure!(got.black && got.frozen, "black and still: {got:?}");
        ensure!(!got.flat, "black is not also called flat: {got:?}");
        ensure!(
            got.problems().len() == 2,
            "two problems: {:?}",
            got.problems()
        );
        Ok(())
    }

    #[test]
    fn white_is_blown_out() -> Result<()> {
        let frames = vec![solid(8, 8, 255), solid(8, 8, 250)];
        let got = gate(&FrameSet {
            width: 8,
            height: 8,
            rate: 30.0,
            frames: &frames,
        });
        ensure!(got.blown_out, "white is blown: {got:?}");
        ensure!(got.hard_pass(), "blown is a warning, not a hard failure");
        Ok(())
    }

    #[test]
    fn flashes_count_pairs_of_turns() -> Result<()> {
        ensure!(
            count_flashes(&[0.0, 0.5, 0.0, 0.5, 0.0]) == 2,
            "four large turns are two flashes"
        );
        ensure!(
            count_flashes(&[0.0, 0.05, 0.0, 0.05]) == 0,
            "small changes are not flashes"
        );
        ensure!(
            count_flashes(&[0.85, 0.99, 0.85, 0.99]) == 0,
            "changes among bright states are not flashes"
        );
        Ok(())
    }
}
