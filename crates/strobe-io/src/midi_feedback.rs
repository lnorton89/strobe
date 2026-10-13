// ============================================================================
// Module:       strobe_io::midi_feedback
// Description:  MIDI feedback: what each MIDI mapping's fader, knob ring or
//               pad LED should show for its control, sent only when it
//               changes and no faster than a rate. Pure: the bytes go to a
//               sender in `midi` (feature `midi`).
//
// Dependencies: crate::{mapping, macros, target, event}; strobe-scene
//               (ControlBank, ControlKind)
// ============================================================================

//! MIDI feedback (backlog P-05, D257).
//!
//! A motorised fader, an LED ring or a pad's light shows the show's state
//! only if Strobe sends it. For every enabled mapping whose source is a MIDI
//! control change or note on a **named device**, and whose target is a
//! control (directly or through a macro), [`MidiDiffer::collect`] works out
//! what that control would look like on the controller:
//!
//! - **The position.** The control's target as a fraction of its range,
//!   run backwards through the mapping's range, curve and invert
//!   ([`crate::mapping::Cfg::unshape`]), so a fader limited to half the
//!   range, or reversed, lands where moving it would have put the control.
//! - **A CC** is that travel as `0..=127`. **Pitch bend**, which motorised
//!   faders on Mackie-protocol surfaces use, is the same travel in 14 bits.
//!   **A note** is a pad's light: on (velocity 127) from half way up, off
//!   below, sent as note on with velocity 0, which every controller reads
//!   as off. A program change has no position and is not fed back.
//!
//! Only changed values are sent, at most [`RATE_HZ`] times a second and
//! [`MAX_PER_SEND`] at a time, as with OSC feedback (D209). A mapping on
//! "any device" is not fed back: there is no one device to send it to, and
//! guessing would send fader positions to a synthesiser. Bangs, commands,
//! cues, fav slots, presets and actions have no position and are skipped.
//!
//! ## No loop
//!
//! A controller that echoes what it is sent moves nothing: the echo is the
//! travel of the value the control already has, and an unchanged value is
//! not sent again. Soft takeover (pickup) sees the fader already at the
//! control, so it never has to wait.

pub use crate::midi_out::Sender;

use crate::event::Source;
use crate::macros::MacroSet;
use crate::mapping::Mapping;
use crate::target::{Target, flatten, locate_control};
use std::sync::Arc;
use std::time::{Duration, Instant};
use strobe_scene::control::{ControlBank, ControlKind};

/// The most sends a second.
pub const RATE_HZ: u32 = 30;

/// The most messages in one send; the rest go next time.
pub const MAX_PER_SEND: usize = 64;

/// One MIDI message for one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiOut {
    /// The device's key, as mappings name it (`message::device_key`).
    pub device: Arc<str>,
    /// The three bytes: status, data 1, data 2.
    pub bytes: [u8; 3],
}

/// Works out what to send (see the module docs).
#[derive(Debug, Default)]
pub struct MidiDiffer {
    /// Per mapping index: the last value sent (a 7-bit byte, or 14 bits for
    /// pitch bend).
    sent: Vec<Option<u16>>,
    /// Per mapping index: its device as a shared string, built when the
    /// mappings change.
    devices: Vec<Option<Arc<str>>>,
    /// Per mapping index: the source the slot was built for, so an edited
    /// mapping is sent again.
    sources: Vec<Source>,
    next: Option<Instant>,
}

impl MidiDiffer {
    /// Forgets what was sent, so everything is sent again: the mappings or
    /// the devices changed.
    pub fn resend_all(&mut self) {
        *self = Self::default();
    }

    /// Fills `out` (cleared first) with what changed, if a send is due at
    /// `now`. Allocates only when a mapping's source changes, and then sends
    /// every mapping again.
    pub fn collect(
        &mut self,
        now: Instant,
        mappings: &[Mapping],
        macros: &MacroSet,
        scene: Option<&ControlBank>,
        meta: Option<&ControlBank>,
        out: &mut Vec<MidiOut>,
    ) {
        out.clear();
        if self.next.is_some_and(|next| now < next) {
            return;
        }
        self.next = Some(now + Duration::from_secs(1) / RATE_HZ);
        let edited = self.sources.len() != mappings.len()
            || self
                .sources
                .iter()
                .zip(mappings)
                .any(|(source, mapping)| *source != mapping.source);
        if edited {
            self.sent = vec![None; mappings.len()];
            self.devices = mappings
                .iter()
                .map(|m| device_of(&m.source).map(Arc::from))
                .collect();
            self.sources = mappings.iter().map(|m| m.source.clone()).collect();
        }
        for (index, mapping) in mappings.iter().enumerate() {
            if out.len() >= MAX_PER_SEND {
                return;
            }
            if !mapping.enabled {
                continue;
            }
            let Some(Some(device)) = self.devices.get(index) else {
                continue;
            };
            let Some(message) = message_for(mapping, macros, scene, meta) else {
                continue;
            };
            let Some(last) = self.sent.get_mut(index) else {
                continue;
            };
            let value = u16::from(message[1]) | (u16::from(message[2]) << 7);
            if *last != Some(value) {
                *last = Some(value);
                out.push(MidiOut {
                    device: Arc::clone(device),
                    bytes: message,
                });
            }
        }
    }
}

/// The named device a MIDI source listens to; `None` for any device or a
/// source that is not MIDI.
fn device_of(source: &Source) -> Option<&str> {
    match source {
        Source::MidiCc { device, .. }
        | Source::MidiNote { device, .. }
        | Source::MidiPitchBend { device, .. } => device.as_deref(),
        Source::MidiProgram { .. } | Source::Osc { .. } => None,
    }
}

/// The message that shows `mapping`'s control as it is now, or `None` when
/// it has nothing to show.
fn message_for(
    mapping: &Mapping,
    macros: &MacroSet,
    scene: Option<&ControlBank>,
    meta: Option<&ControlBank>,
) -> Option<[u8; 3]> {
    let flat = flatten(&mapping.target, macros).ok()?;
    match flat.target {
        Target::Control { .. } | Target::Meta { .. } => {}
        Target::Command { .. }
        | Target::Cue { .. }
        | Target::FavSlot { .. }
        | Target::Preset { .. }
        | Target::Macro { .. }
        | Target::Action(_) => return None,
    }
    let found = locate_control(&flat, scene, meta).ok()?;
    let bank = match found.bank {
        crate::target::BankId::Scene => scene?,
        crate::target::BankId::Meta => meta?,
    };
    let def = bank.defs().get(found.index)?;
    if matches!(def.kind, ControlKind::Bang(_)) {
        return None;
    }
    let component = found.component.unwrap_or(0);
    let raw = *bank.target(found.index)?.get(component)?;
    let position = def.normalize(component, raw);
    if !position.is_finite() {
        return None;
    }
    let travel = mapping.cfg().unshape(position, flat.range)?;
    match &mapping.source {
        Source::MidiCc {
            channel,
            controller,
            ..
        } => Some([
            0xB0 | channel.saturating_sub(1).min(15),
            *controller & 0x7F,
            seven_bit(travel),
        ]),
        Source::MidiNote { channel, note, .. } => Some([
            0x90 | channel.saturating_sub(1).min(15),
            *note & 0x7F,
            if travel >= 0.5 { 127 } else { 0 },
        ]),
        Source::MidiPitchBend { channel, .. } => {
            let value = fourteen_bit(travel);
            Some([
                0xE0 | channel.saturating_sub(1).min(15),
                (value & 0x7F) as u8,
                (value >> 7) as u8,
            ])
        }
        Source::MidiProgram { .. } | Source::Osc { .. } => None,
    }
}

/// `0..=1` as `0..=16383`, rounded.
fn fourteen_bit(travel: f32) -> u16 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=16383 first"
    )]
    let value = (travel.clamp(0.0, 1.0) * 16383.0).round() as u16;
    value
}

/// `0..=1` as `0..=127`, rounded.
fn seven_bit(travel: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=127 first"
    )]
    let byte = (travel.clamp(0.0, 1.0) * 127.0).round() as u8;
    byte
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::Curve;
    use crate::macros::Macro;
    use anyhow::{Result, anyhow};
    use strobe_scene::control::ControlDef;
    use strobe_scene::json::RawControl;

    fn bank(controls: &[&str]) -> Result<ControlBank> {
        let mut defs = Vec::new();
        for json in controls {
            let raw: RawControl = serde_json::from_str(json)?;
            defs.push(ControlDef::from_raw(&raw)?);
        }
        Ok(ControlBank::new(defs)?)
    }

    fn scene() -> Result<ControlBank> {
        bank(&[
            r#"{ "NAME": "speed", "TYPE": "slider", "MIN": 0, "MAX": 2, "DEFAULT": 1 }"#,
            r#"{ "NAME": "strobe", "TYPE": "toggle", "DEFAULT": 0 }"#,
            r#"{ "NAME": "hit", "TYPE": "bang" }"#,
        ])
    }

    fn cc(device: Option<&str>, controller: u8, target: Target) -> Mapping {
        Mapping::new(
            Source::MidiCc {
                device: device.map(str::to_string),
                channel: 2,
                controller,
            },
            target,
        )
    }

    fn note(note: u8, target: Target) -> Mapping {
        Mapping::new(
            Source::MidiNote {
                device: Some("pads".into()),
                channel: 1,
                note,
            },
            target,
        )
    }

    /// One collect at `now`, its output.
    fn run(
        differ: &mut MidiDiffer,
        now: Instant,
        mappings: &[Mapping],
        scene: &ControlBank,
    ) -> Vec<MidiOut> {
        let mut out = Vec::new();
        differ.collect(
            now,
            mappings,
            &MacroSet::default(),
            Some(scene),
            None,
            &mut out,
        );
        out
    }

    #[test]
    fn a_fader_shows_its_control_once_and_then_only_changes() -> Result<()> {
        let mut scene = scene()?;
        let mappings = vec![cc(Some("faders"), 7, Target::control("speed"))];
        let mut differ = MidiDiffer::default();
        let start = Instant::now();
        let first = run(&mut differ, start, &mappings, &scene);
        assert_eq!(
            first,
            vec![MidiOut {
                device: Arc::from("faders"),
                bytes: [0xB1, 7, 64],
            }],
            "speed 1 of 0..2 is half way: CC 7 on channel 2 = 64"
        );
        let later = start + Duration::from_millis(100);
        assert!(
            run(&mut differ, later, &mappings, &scene).is_empty(),
            "nothing changed, nothing sent"
        );
        let index = scene.find("speed").ok_or_else(|| anyhow!("speed"))?;
        scene.set(index, None, &[2.0]);
        let moved = run(
            &mut differ,
            later + Duration::from_millis(100),
            &mappings,
            &scene,
        );
        assert_eq!(moved.len(), 1, "one change");
        assert_eq!(moved[0].bytes[2], 127, "the top of the range");
        scene.set(index, None, &[0.0]);
        assert!(
            run(
                &mut differ,
                later + Duration::from_millis(110),
                &mappings,
                &scene
            )
            .is_empty(),
            "a second send inside a thirtieth of a second waits"
        );
        Ok(())
    }

    #[test]
    fn range_curve_and_invert_are_undone() -> Result<()> {
        let scene = scene()?;
        let mut mapping = cc(Some("faders"), 7, Target::control("speed"));
        mapping.invert = true;
        mapping.curve = Curve::Exp;
        mapping.range = [0.0, 0.5];
        let mappings = vec![mapping.clone()];
        let mut differ = MidiDiffer::default();
        let out = run(&mut differ, Instant::now(), &mappings, &scene);
        let byte = out.first().ok_or_else(|| anyhow!("one message"))?.bytes[2];
        // Moving the fader to that byte must put speed back where it is.
        let travel = f32::from(byte) / 127.0;
        let position = mapping.cfg().shape(travel, None);
        assert!(
            (position - 0.5).abs() < 0.01,
            "byte {byte} reproduces the control's position, got {position}"
        );
        Ok(())
    }

    #[test]
    fn a_pad_lights_for_a_toggle_and_skips_what_has_no_position() -> Result<()> {
        let mut scene = scene()?;
        let mappings = vec![
            note(36, Target::control("strobe")),
            note(37, Target::control("hit")),
            note(38, Target::command("next_scene")),
            cc(None, 9, Target::control("speed")),
        ];
        let mut differ = MidiDiffer::default();
        let start = Instant::now();
        let out = run(&mut differ, start, &mappings, &scene);
        assert_eq!(
            out,
            vec![MidiOut {
                device: Arc::from("pads"),
                bytes: [0x90, 36, 0],
            }],
            "the toggle's light, off; no bang, no command, nothing for any device"
        );
        let index = scene.find("strobe").ok_or_else(|| anyhow!("strobe"))?;
        scene.set(index, None, &[1.0]);
        let out = run(
            &mut differ,
            start + Duration::from_millis(50),
            &mappings,
            &scene,
        );
        assert_eq!(out.first().map(|m| m.bytes), Some([0x90, 36, 127]), "on");
        Ok(())
    }

    #[test]
    fn a_macro_is_shown_through_its_target_and_its_reach() -> Result<()> {
        let scene = scene()?;
        let mut macros = MacroSet::default();
        let mut found = Macro::new("drive", Target::control("speed"));
        found.range = Some([0.5, 1.0]);
        let id = macros.add(found);
        let mappings = vec![cc(Some("faders"), 1, Target::Macro { id })];
        let mut differ = MidiDiffer::default();
        let mut out = Vec::new();
        differ.collect(
            Instant::now(),
            &mappings,
            &macros,
            Some(&scene),
            None,
            &mut out,
        );
        // speed is half way, the bottom of the macro's reach: the fader's 0.
        assert_eq!(out.first().map(|m| m.bytes[2]), Some(0), "{out:?}");
        Ok(())
    }

    #[test]
    fn an_edited_mapping_is_sent_again() -> Result<()> {
        let scene = scene()?;
        let mut mappings = vec![cc(Some("faders"), 7, Target::control("speed"))];
        let mut differ = MidiDiffer::default();
        let start = Instant::now();
        assert_eq!(
            run(&mut differ, start, &mappings, &scene).len(),
            1,
            "first send"
        );
        mappings[0].source = Source::MidiCc {
            device: Some("faders".into()),
            channel: 2,
            controller: 8,
        };
        let out = run(
            &mut differ,
            start + Duration::from_millis(50),
            &mappings,
            &scene,
        );
        assert_eq!(
            out.first().map(|m| m.bytes),
            Some([0xB1, 8, 64]),
            "the same value, on the new controller"
        );
        Ok(())
    }

    #[test]
    fn pitch_bend_carries_fourteen_bits() -> Result<()> {
        let scene = scene()?;
        let mappings = vec![Mapping::new(
            Source::MidiPitchBend {
                device: Some("motors".into()),
                channel: 1,
            },
            Target::control("speed"),
        )];
        let mut differ = MidiDiffer::default();
        let out = run(&mut differ, Instant::now(), &mappings, &scene);
        // Half way: 8192 rounds from 8191.5, LSB 0, MSB 64.
        assert_eq!(out.first().map(|m| m.bytes), Some([0xE0, 0, 64]), "{out:?}");
        Ok(())
    }

    #[test]
    fn a_disabled_mapping_and_a_missing_control_send_nothing() -> Result<()> {
        let scene = scene()?;
        let mut off = cc(Some("faders"), 7, Target::control("speed"));
        off.enabled = false;
        let mappings = vec![off, cc(Some("faders"), 8, Target::control("not_here"))];
        let mut differ = MidiDiffer::default();
        assert!(
            run(&mut differ, Instant::now(), &mappings, &scene).is_empty(),
            "nothing to show"
        );
        Ok(())
    }
}
