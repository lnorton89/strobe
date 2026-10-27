// ============================================================================
// Module:       strobe_app::song
// Description:  Hears a new song start, once a frame for the whole app, or
//               takes the operator's word for it (the `new_song` command).
//
// Dependencies: strobe-audio (SongDetector); crate::audio, crate::show::beat
// ============================================================================

//! A new song, heard or told (D237, D489).
//!
//! Two things listen for the next track: a playlist cue that ends on
//! `audio-song` (D237), and the playing scene's `HARD_TRANSITIONS` and
//! `SMOOTH_TRANSITIONS`, which change its look when the music does (D489).
//! A scene plays with or without a playlist, so the listening happens here,
//! once a frame for the whole app, and the app hands what it hears to
//! whichever is running.
//!
//! Only live audio counts: the synthetic track has no songs. Any change of
//! source (a device lost, the synthetic fallback, a device back) starts the
//! detector over, so a reconnect is never a song change.
//!
//! What the detector cannot hear (a DJ mix that holds one tempo through
//! the change, with no gap) the operator can say: [`SongWatch::ask`], the
//! `new_song` command, counts as a song heard on the next frame, with live
//! audio or without.

use crate::audio::{AudioInput, Source};
use crate::show::beat::BeatReader;
use strobe_audio::song::SongDetector;

/// The app's ear for a new song.
#[derive(Debug)]
pub struct SongWatch {
    detector: SongDetector,
    reader: BeatReader,
    /// The source the detector last heard; a change resets it.
    source: Option<Source>,
    /// The operator said a new song started.
    asked: bool,
}

impl Default for SongWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl SongWatch {
    /// Listening, with nothing heard yet.
    pub fn new() -> Self {
        Self {
            detector: SongDetector::new(),
            reader: BeatReader::new(),
            source: None,
            asked: false,
        }
    }

    /// The operator says a new song has started; the next
    /// [`SongWatch::step`] reports it.
    pub fn ask(&mut self) {
        self.asked = true;
    }

    /// Listens to one frame, `dt` seconds of wall time after the last.
    /// Whether a new song started: heard in live audio, or asked for.
    pub fn step(&mut self, dt: f32, audio: &mut AudioInput) -> bool {
        let asked = std::mem::take(&mut self.asked);
        let source = audio.source();
        if self.source != Some(source) {
            self.source = Some(source);
            self.detector.reset();
        }
        let heard = match source {
            // Live frames ignore the time: they are the analyser's latest.
            Source::Live => {
                let frame = audio.frame_at(0.0);
                let beat = self.reader.read(&frame.scalars, frame.downbeat);
                self.detector
                    .step(dt, frame.energy[0], beat.bpm, beat.confidence)
            }
            Source::Synthetic => false,
        };
        if heard {
            log::info!("audio: a new song");
        } else if asked {
            log::info!("a new song, says the operator");
        }
        heard || asked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_audio_hears_no_song_but_the_operator_is_believed() {
        let mut audio = AudioInput::new(false);
        let mut watch = SongWatch::new();
        for _ in 0..600 {
            assert!(
                !watch.step(0.1, &mut audio),
                "the synthetic track has no songs"
            );
        }
        watch.ask();
        assert!(watch.step(0.1, &mut audio), "asked for, so heard");
        assert!(!watch.step(0.1, &mut audio), "once");
    }
}
