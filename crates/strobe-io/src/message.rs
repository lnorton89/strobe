// ============================================================================
// Module:       strobe_io::message
// Description:  Raw MIDI bytes parsed into typed messages, and a device name
//               reduced to a key that survives a replug.
//
// Dependencies: std only
// ============================================================================

//! MIDI messages.
//!
//! [`parse`] turns the bytes one callback delivers into a [`MidiMessage`].
//! It knows the five message kinds a show cares about (note on, note off,
//! control change, pitch bend, program change) and returns `None` for
//! everything else: system real-time (the 24-per-beat clock would flood the
//! queue), sysex, aftertouch, and the channel-mode controllers 120 to 127
//! (all notes off and friends, which some controllers send on connect and
//! which are not a fader). Anything malformed, such as a data byte with its
//! high bit set or a short message, is also `None`; a device sending noise
//! must never reach the mapper.
//!
//! Channels are `0..=15` here, as on the wire. [`crate::event::Source`]
//! stores `1..=16`, as every controller's label says.
//!
//! A note on with velocity 0 is a note off, as the MIDI specification asks.
//!
//! [`device_key`] is the stable part of a port's name. ALSA appends the
//! client and port numbers (`Launchpad Mini MIDI 1 20:0`), which change when
//! the device is replugged, and WinMM prefixes a duplicate's index
//! (`2- Launchpad Mini`). A mapping that named the raw string would stop
//! working at the next replug.

/// A parsed MIDI channel message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMessage {
    /// A key or pad pressed.
    NoteOn {
        /// Channel, `0..=15`.
        channel: u8,
        /// Note number.
        note: u8,
        /// Velocity, `1..=127`.
        velocity: u8,
    },
    /// A key or pad released (or a note on with velocity 0).
    NoteOff {
        /// Channel, `0..=15`.
        channel: u8,
        /// Note number.
        note: u8,
    },
    /// A knob or fader.
    ControlChange {
        /// Channel, `0..=15`.
        channel: u8,
        /// Controller number, `0..=119`.
        controller: u8,
        /// Value, `0..=127`.
        value: u8,
    },
    /// The pitch wheel or a touch strip, 14 bits.
    PitchBend {
        /// Channel, `0..=15`.
        channel: u8,
        /// `0..=16383`, centre 8192.
        value: u16,
    },
    /// A program (patch) selected.
    ProgramChange {
        /// Channel, `0..=15`.
        channel: u8,
        /// Program number, `0..=127`.
        program: u8,
    },
}

impl MidiMessage {
    /// The channel, `0..=15`.
    pub fn channel(self) -> u8 {
        match self {
            Self::NoteOn { channel, .. }
            | Self::NoteOff { channel, .. }
            | Self::ControlChange { channel, .. }
            | Self::PitchBend { channel, .. }
            | Self::ProgramChange { channel, .. } => channel,
        }
    }

    /// One line for the activity monitor: `CC 21 = 64  ch 1`.
    pub fn describe(self) -> String {
        let ch = self.channel() + 1;
        match self {
            Self::NoteOn { note, velocity, .. } => {
                format!("Note {note} on  vel {velocity}  ch {ch}")
            }
            Self::NoteOff { note, .. } => format!("Note {note} off  ch {ch}"),
            Self::ControlChange {
                controller, value, ..
            } => format!("CC {controller} = {value}  ch {ch}"),
            Self::PitchBend { value, .. } => format!("Pitch bend {value}  ch {ch}"),
            Self::ProgramChange { program, .. } => format!("Program {program}  ch {ch}"),
        }
    }
}

/// Parses one MIDI message, or `None` if it is not one Strobe uses.
pub fn parse(bytes: &[u8]) -> Option<MidiMessage> {
    let (&status, data) = bytes.split_first()?;
    if !(0x80..0xF0).contains(&status) {
        return None;
    }
    let channel = status & 0x0F;
    let data_byte = |i: usize| data.get(i).copied().filter(|b| *b < 0x80);
    match status & 0xF0 {
        0x80 => Some(MidiMessage::NoteOff {
            channel,
            note: data_byte(0)?,
        }),
        0x90 => {
            let note = data_byte(0)?;
            let velocity = data_byte(1)?;
            Some(if velocity == 0 {
                MidiMessage::NoteOff { channel, note }
            } else {
                MidiMessage::NoteOn {
                    channel,
                    note,
                    velocity,
                }
            })
        }
        0xB0 => {
            let controller = data_byte(0)?;
            let value = data_byte(1)?;
            (controller < 120).then_some(MidiMessage::ControlChange {
                channel,
                controller,
                value,
            })
        }
        0xC0 => Some(MidiMessage::ProgramChange {
            channel,
            program: data_byte(0)?,
        }),
        0xE0 => {
            let lsb = u16::from(data_byte(0)?);
            let msb = u16::from(data_byte(1)?);
            Some(MidiMessage::PitchBend {
                channel,
                value: (msb << 7) | lsb,
            })
        }
        _ => None,
    }
}

/// The part of a port's name that identifies the device across replugs.
pub fn device_key(port_name: &str) -> String {
    let mut name = port_name.trim();
    // WinMM: "2- Launchpad Mini".
    if let Some((prefix, rest)) = name.split_once("- ")
        && !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_digit())
    {
        name = rest.trim_start();
    }
    // ALSA: "... 20:0".
    if let Some((head, tail)) = name.rsplit_once(' ')
        && let Some((client, port)) = tail.split_once(':')
        && !client.is_empty()
        && !port.is_empty()
        && client.chars().all(|c| c.is_ascii_digit())
        && port.chars().all(|c| c.is_ascii_digit())
    {
        name = head.trim_end();
    }
    name.to_string()
}

/// The keys of a list of ports, in order: each port's [`device_key`], and
/// a second port with the same key gets ` #2`, a third ` #3`. Input and
/// output ports are keyed alike, so a mapping's device names both.
pub fn device_keys<'a>(port_names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    port_names
        .into_iter()
        .map(|name| {
            let base = device_key(name);
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                base
            } else {
                format!("{base} #{count}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_port_with_the_same_key_is_numbered() {
        assert_eq!(
            device_keys(["2- Launchpad Mini", "Launchpad Mini 24:0", "nanoKONTROL2"]),
            vec!["Launchpad Mini", "Launchpad Mini #2", "nanoKONTROL2"],
            "keys in order, the repeat numbered"
        );
    }

    #[test]
    fn recorded_bytes_from_a_controller_parse_to_the_right_messages() {
        // A recorded session: a fader sweep, a pad hit and release, a wheel,
        // a patch change, and the clock tick a keyboard sends 24 times a beat.
        let session: [(&[u8], Option<MidiMessage>); 9] = [
            (
                &[0xB0, 21, 0],
                Some(MidiMessage::ControlChange {
                    channel: 0,
                    controller: 21,
                    value: 0,
                }),
            ),
            (
                &[0xB0, 21, 127],
                Some(MidiMessage::ControlChange {
                    channel: 0,
                    controller: 21,
                    value: 127,
                }),
            ),
            (
                &[0x99, 36, 100],
                Some(MidiMessage::NoteOn {
                    channel: 9,
                    note: 36,
                    velocity: 100,
                }),
            ),
            (
                &[0x89, 36, 64],
                Some(MidiMessage::NoteOff {
                    channel: 9,
                    note: 36,
                }),
            ),
            (
                &[0xE2, 0x00, 0x40],
                Some(MidiMessage::PitchBend {
                    channel: 2,
                    value: 8192,
                }),
            ),
            (
                &[0xCF, 5],
                Some(MidiMessage::ProgramChange {
                    channel: 15,
                    program: 5,
                }),
            ),
            (&[0xF8], None),
            (&[0xFE], None),
            (&[0xF0, 1, 2, 0xF7], None),
        ];
        for (bytes, expected) in session {
            assert_eq!(parse(bytes), expected, "{bytes:02X?}");
        }
    }

    #[test]
    fn a_note_on_with_zero_velocity_is_a_note_off() {
        assert_eq!(
            parse(&[0x90, 60, 0]),
            Some(MidiMessage::NoteOff {
                channel: 0,
                note: 60
            }),
            "running-status style release"
        );
    }

    #[test]
    fn pitch_bend_is_fourteen_bits_least_significant_first() {
        let at = |lsb, msb| parse(&[0xE0, lsb, msb]);
        assert_eq!(
            at(0, 0),
            Some(MidiMessage::PitchBend {
                channel: 0,
                value: 0
            }),
            "bottom"
        );
        assert_eq!(
            at(0x7F, 0x7F),
            Some(MidiMessage::PitchBend {
                channel: 0,
                value: 16383
            }),
            "top"
        );
        assert_eq!(
            at(0x01, 0x00),
            Some(MidiMessage::PitchBend {
                channel: 0,
                value: 1
            }),
            "lsb is the low seven"
        );
    }

    #[test]
    fn malformed_and_unused_messages_are_dropped_not_guessed() {
        let bad: [&[u8]; 12] = [
            &[],
            &[0x90],
            &[0x90, 60],
            &[0xB0, 21],
            &[0x7F, 1, 2],
            &[0xB0, 0x80, 1],
            &[0xB0, 21, 0x80],
            &[0xB0, 123, 0],
            &[0xB0, 120, 0],
            &[0xA0, 60, 10],
            &[0xD0, 10],
            &[0xE0, 0x80, 0],
        ];
        for bytes in bad {
            assert_eq!(parse(bytes), None, "{bytes:02X?}");
        }
        assert!(
            parse(&[0xB0, 119, 1]).is_some(),
            "119 is the last real controller"
        );
    }

    #[test]
    fn every_channel_is_kept() {
        for channel in 0..16_u8 {
            assert_eq!(
                parse(&[0xB0 | channel, 1, 1]).map(MidiMessage::channel),
                Some(channel),
                "channel {channel}"
            );
        }
    }

    #[test]
    fn device_keys_drop_the_parts_that_change_on_replug() {
        assert_eq!(
            device_key("Launchpad Mini MIDI 1 20:0"),
            "Launchpad Mini MIDI 1"
        );
        assert_eq!(
            device_key("Launchpad Mini MIDI 1 24:0"),
            "Launchpad Mini MIDI 1"
        );
        assert_eq!(device_key("2- Launchpad Mini"), "Launchpad Mini");
        assert_eq!(device_key("nanoKONTROL2"), "nanoKONTROL2");
        assert_eq!(device_key("  padded  "), "padded");
        assert_eq!(device_key("Port 3 is fine"), "Port 3 is fine");
        assert_eq!(
            device_key("Midi Through:Midi Through Port-0 14:0"),
            "Midi Through:Midi Through Port-0"
        );
    }

    #[test]
    fn descriptions_count_channels_from_one() {
        assert_eq!(
            MidiMessage::ControlChange {
                channel: 0,
                controller: 21,
                value: 64
            }
            .describe(),
            "CC 21 = 64  ch 1"
        );
    }
}
