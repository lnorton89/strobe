// ============================================================================
// Module:       strobe_io::event
// Description:  What the worker threads hand the app: typed input events and
//               status reports, and the Source/Signal pair a mapping works
//               on.
//
// Dependencies: crate::message; serde
// ============================================================================

//! Events.
//!
//! The workers produce [`Event`]s: an [`InputEvent`] (a parsed MIDI message
//! with the device it came from, or an OSC message) or a [`Status`] (a
//! device appeared or went away, the listener is up or could not bind).
//!
//! The mapper does not care what kind of message it was. It cares *where it
//! came from* ([`Source`]: a CC number on a channel of a device, a note, an
//! OSC address) and *what it said* ([`Signal`]: a position between 0 and 1,
//! or a button going down or up). [`Source::of`] reduces an input event to
//! that pair:
//!
//! | Message | Source | Signal |
//! |---|---|---|
//! | control change | `MidiCc` | `Analog(value / 127)` |
//! | note on / off | `MidiNote` | `Button(true / false)` |
//! | pitch bend | `MidiPitchBend` | `Analog(value / 16383)` |
//! | program change | `MidiProgram` | `Button(true)` |
//! | OSC float, double | `Osc` | `Analog(x)` clamped to 0..1 |
//! | OSC int | `Osc` | `Analog(x)` clamped to 0..1 (so 0 and 1 are a button) |
//! | OSC true, false | `Osc` | `Button(b)` |
//! | OSC nil, impulse, no arguments | `Osc` | `Button(true)` |
//!
//! A *mapping's* source is a [`Source`] too, where a `device` of `None`
//! means any device. [`Source::matches`] compares a mapping's source with a
//! concrete event's, and [`Source::overlaps`] says whether two mappings'
//! sources would both fire for some event (a conflict).
//!
//! MIDI channels are `1..=16` in a source, as controllers label them.

use crate::message::MidiMessage;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

/// An OSC argument Strobe understands. Strings, blobs, colours and the rest
/// are dropped when the message is decoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arg {
    /// A 32-bit or 64-bit float. Always finite: NaN and infinity are
    /// dropped on decode, and a double beyond the f32 range is clamped to it.
    Float(f32),
    /// A 32-bit or 64-bit integer.
    Int(i32),
    /// `T` or `F`.
    Bool(bool),
    /// `N` or `I`: a message that is just "now".
    Impulse,
}

/// A decoded OSC message.
#[derive(Clone, Debug, PartialEq)]
pub struct OscMessage {
    /// The address pattern, such as `/strobe/control/speed`.
    pub address: String,
    /// The arguments Strobe understands, in order.
    pub args: Vec<Arg>,
}

impl OscMessage {
    /// One line for the activity monitor.
    pub fn describe(&self) -> String {
        let mut text = self.address.clone();
        for arg in &self.args {
            match arg {
                Arg::Float(x) => text.push_str(&format!(" {x:.3}")),
                Arg::Int(x) => text.push_str(&format!(" {x}")),
                Arg::Bool(x) => text.push_str(if *x { " true" } else { " false" }),
                Arg::Impulse => text.push_str(" !"),
            }
        }
        text
    }
}

/// A message that came in.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// MIDI from a device (its [`crate::message::device_key`]).
    Midi {
        /// The device's key.
        device: Arc<str>,
        /// What it sent.
        message: MidiMessage,
    },
    /// An OSC message.
    Osc(OscMessage),
}

impl InputEvent {
    /// One line for the activity monitor.
    pub fn describe(&self) -> String {
        match self {
            Self::Midi { device, message } => format!("{}  [{device}]", message.describe()),
            Self::Osc(message) => message.describe(),
        }
    }
}

/// One MIDI device as the workers last saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The key mappings use ([`crate::message::device_key`]).
    pub key: String,
    /// The port's full name.
    pub name: String,
    /// It is open and delivering.
    pub connected: bool,
    /// The operator switched it off.
    pub disabled: bool,
    /// Why it could not be opened, if that was tried and failed.
    pub error: Option<String>,
}

/// A report from a worker.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// MIDI cannot run at all (no sequencer, no driver, built without it).
    MidiUnavailable(String),
    /// The set of MIDI devices, or one's state, changed.
    Devices(Vec<DeviceInfo>),
    /// A device was opened.
    DeviceFound(String),
    /// A device that was open went away (unplugged, or its driver died).
    DeviceLost(String),
    /// The OSC listener is bound here.
    OscListening(SocketAddr),
    /// The OSC listener is off.
    OscStopped,
    /// The OSC listener could not bind or stopped receiving.
    OscError(String),
    /// Messages were dropped because the app was not draining fast enough.
    Dropped(u64),
}

/// Everything a worker sends.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A message from a controller.
    Input(InputEvent),
    /// A report about the workers themselves.
    Status(Status),
}

/// What a message said, for the mapper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Signal {
    /// A position, `0..=1`.
    Analog(f32),
    /// A button going down (`true`) or up.
    Button(bool),
}

/// Where a message came from, or which messages a mapping answers to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A knob or fader.
    MidiCc {
        /// The device, or any device.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Channel, `1..=16`.
        channel: u8,
        /// Controller number.
        controller: u8,
    },
    /// A key or pad (both its press and its release).
    MidiNote {
        /// The device, or any device.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Channel, `1..=16`.
        channel: u8,
        /// Note number.
        note: u8,
    },
    /// The pitch wheel or a touch strip.
    MidiPitchBend {
        /// The device, or any device.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Channel, `1..=16`.
        channel: u8,
    },
    /// One program number (a patch-change button).
    MidiProgram {
        /// The device, or any device.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Channel, `1..=16`.
        channel: u8,
        /// Program number.
        program: u8,
    },
    /// An OSC address, matched exactly.
    Osc {
        /// The address, such as `/1/fader1`.
        address: String,
    },
}

impl Source {
    /// The source and signal an input event amounts to, or `None` for an
    /// OSC message with nothing a control could use (only strings, say).
    pub fn of(event: &InputEvent) -> Option<(Self, Signal)> {
        match event {
            InputEvent::Midi { device, message } => {
                let device = Some(device.to_string());
                Some(match *message {
                    MidiMessage::ControlChange {
                        channel,
                        controller,
                        value,
                    } => (
                        Self::MidiCc {
                            device,
                            channel: channel + 1,
                            controller,
                        },
                        Signal::Analog(f32::from(value) / 127.0),
                    ),
                    MidiMessage::NoteOn { channel, note, .. } => (
                        Self::MidiNote {
                            device,
                            channel: channel + 1,
                            note,
                        },
                        Signal::Button(true),
                    ),
                    MidiMessage::NoteOff { channel, note } => (
                        Self::MidiNote {
                            device,
                            channel: channel + 1,
                            note,
                        },
                        Signal::Button(false),
                    ),
                    MidiMessage::PitchBend { channel, value } => (
                        Self::MidiPitchBend {
                            device,
                            channel: channel + 1,
                        },
                        Signal::Analog(f32::from(value) / 16383.0),
                    ),
                    MidiMessage::ProgramChange { channel, program } => (
                        Self::MidiProgram {
                            device,
                            channel: channel + 1,
                            program,
                        },
                        Signal::Button(true),
                    ),
                })
            }
            InputEvent::Osc(message) => {
                let signal = match message.args.first() {
                    None | Some(Arg::Impulse) => Signal::Button(true),
                    Some(Arg::Bool(down)) => Signal::Button(*down),
                    Some(Arg::Float(x)) => Signal::Analog(x.clamp(0.0, 1.0)),
                    Some(Arg::Int(x)) => Signal::Analog((*x as f32).clamp(0.0, 1.0)),
                };
                Some((
                    Self::Osc {
                        address: message.address.clone(),
                    },
                    signal,
                ))
            }
        }
    }

    /// Whether a mapping with this source answers to an event from `event`
    /// (a concrete source, with its device filled in).
    pub fn matches(&self, event: &Source) -> bool {
        self.overlaps(event)
    }

    /// Whether some event would match both `self` and `other`.
    pub fn overlaps(&self, other: &Source) -> bool {
        fn devices(a: &Option<String>, b: &Option<String>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => a == b,
                (None, _) | (_, None) => true,
            }
        }
        match (self, other) {
            (
                Self::MidiCc {
                    device: da,
                    channel: ca,
                    controller: ka,
                },
                Self::MidiCc {
                    device: db,
                    channel: cb,
                    controller: kb,
                },
            ) => ca == cb && ka == kb && devices(da, db),
            (
                Self::MidiNote {
                    device: da,
                    channel: ca,
                    note: na,
                },
                Self::MidiNote {
                    device: db,
                    channel: cb,
                    note: nb,
                },
            ) => ca == cb && na == nb && devices(da, db),
            (
                Self::MidiPitchBend {
                    device: da,
                    channel: ca,
                },
                Self::MidiPitchBend {
                    device: db,
                    channel: cb,
                },
            ) => ca == cb && devices(da, db),
            (
                Self::MidiProgram {
                    device: da,
                    channel: ca,
                    program: pa,
                },
                Self::MidiProgram {
                    device: db,
                    channel: cb,
                    program: pb,
                },
            ) => ca == cb && pa == pb && devices(da, db),
            (Self::Osc { address: a }, Self::Osc { address: b }) => a == b,
            (
                Self::MidiCc { .. }
                | Self::MidiNote { .. }
                | Self::MidiPitchBend { .. }
                | Self::MidiProgram { .. }
                | Self::Osc { .. },
                _,
            ) => false,
        }
    }

    /// The device this source is restricted to, if it is a MIDI source that
    /// names one.
    pub fn device(&self) -> Option<&str> {
        match self {
            Self::MidiCc { device, .. }
            | Self::MidiNote { device, .. }
            | Self::MidiPitchBend { device, .. }
            | Self::MidiProgram { device, .. } => device.as_deref(),
            Self::Osc { .. } => None,
        }
    }

    /// The same source for any device.
    pub fn for_any_device(&self) -> Self {
        let mut any = self.clone();
        match &mut any {
            Self::MidiCc { device, .. }
            | Self::MidiNote { device, .. }
            | Self::MidiPitchBend { device, .. }
            | Self::MidiProgram { device, .. } => *device = None,
            Self::Osc { .. } => {}
        }
        any
    }

    /// The same source restricted to `key`'s device (OSC is unchanged).
    pub fn for_device(&self, key: &str) -> Self {
        let mut one = self.clone();
        match &mut one {
            Self::MidiCc { device, .. }
            | Self::MidiNote { device, .. }
            | Self::MidiPitchBend { device, .. }
            | Self::MidiProgram { device, .. } => *device = Some(key.to_string()),
            Self::Osc { .. } => {}
        }
        one
    }

    /// Whether this is an OSC source.
    pub fn is_osc(&self) -> bool {
        matches!(self, Self::Osc { .. })
    }

    /// What the source is, without the device: `CC 21 ch 1`, `/1/fader1`.
    pub fn label(&self) -> String {
        match self {
            Self::MidiCc {
                channel,
                controller,
                ..
            } => format!("CC {controller} ch {channel}"),
            Self::MidiNote { channel, note, .. } => format!("Note {note} ch {channel}"),
            Self::MidiPitchBend { channel, .. } => format!("Pitch bend ch {channel}"),
            Self::MidiProgram {
                channel, program, ..
            } => format!("Program {program} ch {channel}"),
            Self::Osc { address } => address.clone(),
        }
    }

    /// The kind of source for a badge: `MIDI` or `OSC`.
    pub fn badge(&self) -> &'static str {
        if self.is_osc() { "OSC" } else { "MIDI" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn midi(message: MidiMessage) -> InputEvent {
        InputEvent::Midi {
            device: Arc::from("pad"),
            message,
        }
    }

    fn osc(address: &str, args: Vec<Arg>) -> InputEvent {
        InputEvent::Osc(OscMessage {
            address: address.to_string(),
            args,
        })
    }

    #[test]
    fn a_cc_is_an_analog_signal_on_a_one_based_channel() {
        let event = midi(MidiMessage::ControlChange {
            channel: 0,
            controller: 21,
            value: 127,
        });
        assert_eq!(
            Source::of(&event),
            Some((
                Source::MidiCc {
                    device: Some("pad".into()),
                    channel: 1,
                    controller: 21
                },
                Signal::Analog(1.0)
            )),
            "CC"
        );
    }

    #[test]
    fn notes_and_programs_are_buttons_and_pitch_bend_is_analog() {
        let on = Source::of(&midi(MidiMessage::NoteOn {
            channel: 9,
            note: 36,
            velocity: 100,
        }));
        let off = Source::of(&midi(MidiMessage::NoteOff {
            channel: 9,
            note: 36,
        }));
        assert_eq!(on.map(|(_, s)| s), Some(Signal::Button(true)), "note on");
        assert_eq!(off.map(|(_, s)| s), Some(Signal::Button(false)), "note off");
        let program = Source::of(&midi(MidiMessage::ProgramChange {
            channel: 0,
            program: 3,
        }));
        assert_eq!(
            program.map(|(_, s)| s),
            Some(Signal::Button(true)),
            "program"
        );
        let bend = Source::of(&midi(MidiMessage::PitchBend {
            channel: 0,
            value: 16383,
        }));
        assert_eq!(bend.map(|(_, s)| s), Some(Signal::Analog(1.0)), "bend");
    }

    #[test]
    fn osc_arguments_become_the_matching_signal() {
        let signal = |args: Vec<Arg>| Source::of(&osc("/x", args)).map(|(_, s)| s);
        assert_eq!(
            signal(vec![Arg::Float(0.25)]),
            Some(Signal::Analog(0.25)),
            "float"
        );
        assert_eq!(
            signal(vec![Arg::Float(7.0)]),
            Some(Signal::Analog(1.0)),
            "clamped high"
        );
        assert_eq!(
            signal(vec![Arg::Float(-7.0)]),
            Some(Signal::Analog(0.0)),
            "clamped low"
        );
        assert_eq!(
            signal(vec![Arg::Int(1)]),
            Some(Signal::Analog(1.0)),
            "int 1"
        );
        assert_eq!(
            signal(vec![Arg::Int(0)]),
            Some(Signal::Analog(0.0)),
            "int 0"
        );
        assert_eq!(
            signal(vec![Arg::Bool(false)]),
            Some(Signal::Button(false)),
            "false"
        );
        assert_eq!(
            signal(vec![Arg::Impulse]),
            Some(Signal::Button(true)),
            "impulse"
        );
        assert_eq!(signal(vec![]), Some(Signal::Button(true)), "no arguments");
    }

    #[test]
    fn a_mapping_without_a_device_answers_to_every_device() {
        let any = Source::MidiCc {
            device: None,
            channel: 1,
            controller: 7,
        };
        let from_a = Source::MidiCc {
            device: Some("a".into()),
            channel: 1,
            controller: 7,
        };
        let from_b = Source::MidiCc {
            device: Some("b".into()),
            channel: 1,
            controller: 7,
        };
        assert!(any.matches(&from_a) && any.matches(&from_b), "any device");
        assert!(from_a.matches(&from_a), "same device");
        assert!(!from_a.matches(&from_b), "other device");
        let other_cc = Source::MidiCc {
            device: None,
            channel: 1,
            controller: 8,
        };
        let other_channel = Source::MidiCc {
            device: None,
            channel: 2,
            controller: 7,
        };
        assert!(!any.matches(&other_cc), "other controller");
        assert!(!any.matches(&other_channel), "other channel");
    }

    #[test]
    fn different_kinds_never_overlap_and_osc_matches_exactly() {
        let cc = Source::MidiCc {
            device: None,
            channel: 1,
            controller: 1,
        };
        let note = Source::MidiNote {
            device: None,
            channel: 1,
            note: 1,
        };
        assert!(!cc.overlaps(&note), "cc vs note");
        let a = Source::Osc {
            address: "/a".into(),
        };
        assert!(a.overlaps(&a.clone()), "same address");
        assert!(
            !a.overlaps(&Source::Osc {
                address: "/A".into()
            }),
            "OSC addresses are case-sensitive"
        );
        assert!(!a.overlaps(&cc), "osc vs cc");
    }

    #[test]
    fn device_helpers_restrict_and_release() {
        let cc = Source::MidiCc {
            device: Some("x".into()),
            channel: 1,
            controller: 1,
        };
        assert_eq!(cc.device(), Some("x"), "device");
        assert_eq!(cc.for_any_device().device(), None, "any");
        assert_eq!(
            cc.for_any_device().for_device("y").device(),
            Some("y"),
            "restricted"
        );
        let a = Source::Osc {
            address: "/a".into(),
        };
        assert_eq!(a.for_device("y"), a, "OSC has no device");
    }

    #[test]
    fn labels_read_the_way_a_controller_is_labelled() {
        assert_eq!(
            Source::MidiCc {
                device: None,
                channel: 3,
                controller: 21
            }
            .label(),
            "CC 21 ch 3"
        );
        assert_eq!(
            Source::Osc {
                address: "/1/fader1".into()
            }
            .label(),
            "/1/fader1"
        );
    }

    #[test]
    fn sources_survive_json_without_a_device_field() -> Result<(), serde_json::Error> {
        let source = Source::MidiNote {
            device: None,
            channel: 10,
            note: 36,
        };
        let text = serde_json::to_string(&source)?;
        assert!(!text.contains("device"), "None is left out: {text}");
        assert_eq!(serde_json::from_str::<Source>(&text)?, source, "round trip");
        Ok(())
    }
}
