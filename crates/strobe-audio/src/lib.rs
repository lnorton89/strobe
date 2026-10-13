// ============================================================================
// Module:       strobe_audio (library crate root)
// Description:  Audio analysis for Strobe: FFT bands, onsets, tempo, and the
//               SSF audio uniforms and textures, plus live capture.
//
// Dependencies: realfft, thiserror, log; strobe-scene (uniform names);
//               cpal, rtrb, arc-swap (capture feature)
// ============================================================================

//! Audio analysis.
//!
//! This crate turns sound into the values an SSF scene reads: the scalar
//! `syn_*` uniforms named by [`strobe_scene::audio::SCALARS`], the
//! `syn_Spectrum` and `syn_LevelTrail` textures, and the Shadertoy audio
//! texture. It replaces the bench's synthetic stand-in behind the same
//! contract: an [`AudioFrame`] whose `scalars` follow `SCALARS` order.
//!
//! It is split so the part that has to be right is the part that is easy
//! to test:
//!
//! - [`Analyzer`] is pure DSP. No I/O, no threads, no clock. Push samples,
//!   call [`Analyzer::analyze`], read the frame. Deterministic, so every
//!   behaviour has a test built from synthetic signals.
//! - [`offline`] drives an analyser over a buffer at a fixed frame rate;
//!   the `wav-analyze` example uses it to dump every uniform to CSV for
//!   tuning by ear and by plot.
//! - `capture` (feature `capture`, on by default) opens an audio device with
//!   `cpal`, carries samples to an analysis thread over a lock-free ring,
//!   and publishes the latest frame so the render thread never waits on
//!   audio. It degrades to silence when there is no device.
//!
//! The pieces below `Analyzer` (bands, spectrum, tempo, onset, smoothing)
//! are internal, each documented with the reasoning for its numbers.
//! Values will not match Synesthesia numerically; scenes are tuned by ear,
//! so ranges and shapes are what the tests pin down.
//!
//! Texture widths are a [`Layout`] setting, not constants baked into data
//! shapes: the exact layout real Synesthesia scenes expect is still
//! unverified, and nothing downstream should depend on 256 or 512.

pub mod analyzer;
pub mod bands;
#[cfg(feature = "capture")]
pub mod capture;
pub mod events;
pub mod frame;
pub mod offline;
mod onset;
mod signal;
mod smoothing;
mod spectrum;
mod tempo;

#[cfg(test)]
mod behaviour;
#[cfg(test)]
mod fixtures;

pub use analyzer::{Analyzer, FFT_LEN, HOP};
pub use bands::{BAND_COUNT, Band, BandLayout};
pub use frame::{AudioFrame, Layout};
pub use tempo::{BPM_DEFAULT, BPM_MAX, BPM_MIN};
