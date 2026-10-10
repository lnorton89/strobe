// ============================================================================
// Module:       strobe_gfx::readback
// Description:  The final pass, downsampled to 64x36 and read back to the CPU
//               a few frames late through a pixel-buffer ring, never a stall.
//
// Dependencies: crate::gl (Gl, Texture, Framebuffer, Buffer, Fence)
// ============================================================================

//! Asynchronous final-pass readback.
//!
//! Scripts can call `textureFinalPass(x, y)` to read the picture's colour.
//! Getting pixels from the GPU to the CPU is the classic way to stall a
//! pipeline: `glReadPixels` into client memory waits until every queued
//! draw has finished. On a 2012 Intel HD 4000 that is the difference between
//! 60 fps and 30. So this module never does that. Each frame it:
//!
//! 1. **Downsamples** the final pass to [`WIDTH`] x [`HEIGHT`] with a chain
//!    of linear blits, each at most 2:1, so every step is an exact 2x2 box
//!    average rather than a four-tap sample of a huge image. The chain's
//!    textures are allocated once, when the reader is built.
//! 2. **Starts** a `glReadPixels` of that small image into the next
//!    pixel-pack buffer of a ring of [`RING`], and puts a fence behind it.
//!    The call returns at once; the GPU copies when it gets there.
//! 3. **Collects** the oldest buffer in the ring, issued `RING - 1` frames
//!    ago, but only if its fence has already signalled (a zero-timeout
//!    poll). If it has not, it is skipped: the script keeps the last picture
//!    it had and the buffer is recycled on the ring's next lap. A slow GPU
//!    costs freshness, never frame time.
//!
//! **Latency.** After frame `k` has been rendered and [`FinalPassReader::capture`]
//! has run, [`FinalPassReader::pixels`] holds frame `k - (RING - 1)`: two
//! frames older than the picture just drawn, so three older than the
//! one a script is about to draw. Until the ring has filled once, `pixels`
//! is `None` and the script reads black.
//!
//! Only GL that both desktop 3.3 and GLES 3.0 have is used (pixel-pack
//! buffers, `glMapBufferRange`, sync objects, `glBlitFramebuffer`), and
//! nothing here queries the driver with `glGet*`.
//!
//! **Lost contexts.** The owning GL types skip their deletes once the
//! context is marked lost (D66), and [`FinalPassReader::capture`] does
//! nothing at all on a lost context.

use crate::gl::{Buffer, Fence, FenceStatus, Format, Framebuffer, Gl, Sampling, Texture};

/// Width of the downsample.
pub const WIDTH: i32 = 64;

/// Height of the downsample.
pub const HEIGHT: i32 = 36;

/// Length in bytes of the RGB8 downsample, rows from the bottom up.
pub const RGB_BYTES: usize = (WIDTH as usize) * (HEIGHT as usize) * 3;

/// Pixel-pack buffers in the ring. Three because the oldest is then two
/// frames old, which an ANGLE/D3D11 pipeline that queues up to three frames
/// has usually finished.
pub const RING: usize = 3;

/// Bytes of the RGBA8 image a readback writes.
const RGBA_BYTES: usize = (WIDTH as usize) * (HEIGHT as usize) * 4;

/// One step of the downsample chain.
#[derive(Debug)]
struct Step {
    /// Kept alive for the framebuffer.
    _texture: Texture,
    fbo: Framebuffer,
    size: [i32; 2],
}

/// One buffer of the ring, and the fence behind its pending read.
#[derive(Debug)]
struct Slot {
    buffer: Buffer,
    pending: Option<Fence>,
}

/// The sizes the downsample passes through on the way from `source` to
/// [`WIDTH`] x [`HEIGHT`], ending with that size. Each axis halves (rounding
/// up) while it is more than twice the target and then jumps to the target,
/// so no step shrinks by more than 2:1 on either axis.
pub fn chain_sizes(source: [i32; 2]) -> Vec<[i32; 2]> {
    let next = |size: i32, target: i32| {
        if size > 2 * target {
            (size + 1) / 2
        } else {
            target
        }
    };
    let mut sizes = Vec::new();
    let mut current = [source[0].max(1), source[1].max(1)];
    while current != [WIDTH, HEIGHT] {
        current = [next(current[0], WIDTH), next(current[1], HEIGHT)];
        sizes.push(current);
    }
    if sizes.is_empty() {
        // A source that is already 64x36 still goes through one blit, so
        // the readback never reads the live final pass directly.
        sizes.push([WIDTH, HEIGHT]);
    }
    sizes
}

/// The downsampling, readback ring and latest picture for one player.
#[derive(Debug)]
pub struct FinalPassReader {
    gl: Gl,
    source_size: [i32; 2],
    steps: Vec<Step>,
    slots: Vec<Slot>,
    /// The slot the next capture writes.
    next: usize,
    rgba: Vec<u8>,
    rgb: Vec<u8>,
    ready: bool,
    /// Reads that were not collected before their buffer was needed again.
    dropped: u64,
    /// Whether a failure has been logged (once, not every frame).
    reported: bool,
}

impl FinalPassReader {
    /// Allocates the downsample chain for a `source_size` final pass and the
    /// ring of buffers. Do this at scene load or resize, never per frame.
    pub fn new(gl: &Gl, source_size: [i32; 2]) -> Result<Self, String> {
        let mut steps = Vec::new();
        for size in chain_sizes(source_size) {
            let texture = gl.texture_2d(
                size[0],
                size[1],
                Format::Rgba8,
                Sampling::LINEAR_CLAMP,
                None,
            )?;
            let fbo = gl.framebuffer(&texture)?;
            steps.push(Step {
                _texture: texture,
                fbo,
                size,
            });
        }
        let mut slots = Vec::with_capacity(RING);
        for _ in 0..RING {
            slots.push(Slot {
                buffer: gl.pack_buffer(RGBA_BYTES)?,
                pending: None,
            });
        }
        Ok(Self {
            gl: gl.clone(),
            source_size,
            steps,
            slots,
            next: 0,
            rgba: vec![0; RGBA_BYTES],
            rgb: vec![0; RGB_BYTES],
            ready: false,
            dropped: 0,
            reported: false,
        })
    }

    /// The final pass size this reader was built for.
    pub fn source_size(&self) -> [i32; 2] {
        self.source_size
    }

    /// The newest downsample that has arrived: [`WIDTH`] x [`HEIGHT`] RGB8,
    /// rows from the bottom up (the orientation of `_uv`). `None` until the
    /// first read lands.
    pub fn pixels(&self) -> Option<&[u8]> {
        self.ready.then_some(self.rgb.as_slice())
    }

    /// Reads that were abandoned because the GPU had not finished them by
    /// the time their buffer came round again.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Downsamples `source` (the final pass just drawn, `source_size`),
    /// starts its readback, and collects the oldest finished one. Call once
    /// per frame after the last pass. Allocates nothing; waits for nothing.
    pub fn capture(&mut self, source: &Framebuffer) {
        if self.gl.is_lost() {
            return;
        }
        if let Err(error) = self.issue(source) {
            self.report(&error);
            return;
        }
        self.collect();
    }

    /// Downsamples and starts the read into the next slot.
    fn issue(&mut self, source: &Framebuffer) -> Result<(), String> {
        let mut from = source;
        let mut from_size = self.source_size;
        for step in &self.steps {
            self.gl.blit_between(from, from_size, &step.fbo, step.size);
            from = &step.fbo;
            from_size = step.size;
        }
        let slot_index = self.next;
        self.next = (self.next + 1) % RING;
        let Some(slot) = self.slots.get_mut(slot_index) else {
            return Ok(());
        };
        if slot.pending.take().is_some() {
            // The GPU never finished this one in RING - 1 frames; its fence
            // is dropped and the buffer reused.
            self.dropped += 1;
        }
        self.gl
            .read_pixels_into(from, &slot.buffer, WIDTH, HEIGHT)?;
        slot.pending = Some(self.gl.fence()?);
        Ok(())
    }

    /// Takes the oldest slot's picture if its read has finished.
    fn collect(&mut self) {
        let oldest = self.next % RING;
        let Some(slot) = self.slots.get_mut(oldest) else {
            return;
        };
        let Some(fence) = slot.pending.as_ref() else {
            return;
        };
        match self.gl.fence_status(fence) {
            FenceStatus::Pending => {}
            FenceStatus::Failed => {
                slot.pending = None;
                self.dropped += 1;
            }
            FenceStatus::Signaled => {
                slot.pending = None;
                if self.gl.read_buffer(&slot.buffer, &mut self.rgba) {
                    let (texels, _) = self.rgba.as_chunks::<4>();
                    let (out, _) = self.rgb.as_chunks_mut::<3>();
                    for (rgb, [r, g, b, _]) in out.iter_mut().zip(texels) {
                        *rgb = [*r, *g, *b];
                    }
                    self.ready = true;
                } else {
                    self.dropped += 1;
                }
            }
        }
    }

    /// Logs a failure once; the picture simply stops updating.
    fn report(&mut self, error: &str) {
        if !self.reported {
            self.reported = true;
            log::warn!("textureFinalPass readback failed (reported once): {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_1280x720_final_pass_halves_down_to_64x36() {
        assert_eq!(
            chain_sizes([1280, 720]),
            vec![[640, 360], [320, 180], [160, 90], [80, 45], [64, 36]],
            "four exact halvings, then the odd last step"
        );
    }

    #[test]
    fn no_step_shrinks_an_axis_by_more_than_two() {
        for source in [[1920, 1080], [641, 361], [200, 3000], [65, 37], [1, 1]] {
            let mut previous = source;
            for size in chain_sizes(source) {
                for axis in 0..2 {
                    assert!(
                        previous[axis] <= 2 * size[axis] || size[axis] >= previous[axis],
                        "{source:?}: {previous:?} -> {size:?} loses more than 2:1"
                    );
                }
                previous = size;
            }
            assert_eq!(previous, [WIDTH, HEIGHT], "{source:?} ends at the target");
        }
    }

    #[test]
    fn a_source_at_the_target_size_still_gets_one_step() {
        assert_eq!(
            chain_sizes([WIDTH, HEIGHT]),
            vec![[WIDTH, HEIGHT]],
            "one blit"
        );
    }

    #[test]
    fn a_small_source_is_upscaled_in_one_step() {
        assert_eq!(chain_sizes([32, 18]), vec![[WIDTH, HEIGHT]], "one blit up");
    }
}
