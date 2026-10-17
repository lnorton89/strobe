// ============================================================================
// Module:       strobe_io::osc
// Description:  OSC packets decoded into typed messages with a hard bound on
//               bundle nesting, and the built-in /strobe/... address space.
//
// Dependencies: rosc; crate::{event, macros, target}; strobe-scene (names)
// ============================================================================

//! OSC (D171, D172).
//!
//! ## Decoding
//!
//! [`decode`] takes one UDP datagram and returns the messages in it, bundles
//! flattened. Bundle time tags are ignored: a message is acted on when it
//! arrives, which is what every controller app expects.
//!
//! The datagram came from the network, so it is treated as hostile. Before
//! `rosc` sees it, [`nesting_ok`] walks the bundle structure with an explicit
//! depth limit ([`MAX_DEPTH`]); `rosc` decodes bundles recursively, a 64 KB
//! packet can nest thousands deep, and release builds abort on a stack
//! overflow. The same limit applies to arrays in a message's type tags
//! (`,[[[...]]]`): `rosc` builds those without recursing, but dropping
//! 30,000 nested arrays recurses once per level and overflows the OSC
//! thread's stack just the same. The result is also bounded: at most [`MAX_MESSAGES`] messages per
//! datagram, [`MAX_ARGS`] arguments per message, and addresses of at most
//! [`MAX_ADDRESS`] bytes. Arguments Strobe cannot use (strings, blobs,
//! colours, MIDI, timestamps, arrays) are dropped; a message that *had*
//! arguments and kept none is dropped whole, so `/strobe/cmd/blackout
//! "hello"` is not read as a bare "go".
//!
//! ## The built-in address space
//!
//! With no mapping at all, a sender can reach everything. [`builtin_target`]
//! turns an address into a [`Target`]:
//!
//! | Address | Argument | Does |
//! |---|---|---|
//! | `/strobe/control/<name>` | float 0..1 | sets the scene control, across its range |
//! | `/strobe/control/<name>/<x\|y\|z\|r\|g\|b>` | float 0..1 | one component of a pad or colour |
//! | `/strobe/meta/<name>` | float 0..1 | sets the meta control |
//! | `/strobe/cmd/<id>` | none, `1`, `true` | runs the command (`next_scene`, `toggle_hold`...) |
//! | `/strobe/cue/<n>` | none, `1` | jumps to cue `n` (from 1) |
//! | `/strobe/fav/<n>` | none, `1` | recalls fav slot `n` (1 to 8) |
//! | `/strobe/preset/<name>` | none, `1` | recalls a preset of the playing scene (`_` reads as a space) |
//! | `/strobe/macro/<id or name>` | float 0..1 | drives a Performance Macro |
//!
//! `<name>` is matched by the script API's rule (case, `_`, `-` and spaces
//! ignored) and may be `group/name`. A mapping for an address always wins
//! over the built-in meaning of the same address.
//!
//! ## Synesthesia's addresses (P-08, D234)
//!
//! A controller layout made for Synesthesia works unchanged for the parts
//! that mean the same thing in Strobe. Values are normalised 0..1 there
//! too, so these are aliases of the table above:
//!
//! | Synesthesia address | Strobe meaning |
//! |---|---|
//! | `/controls/scene/<name>[/x\|y\|r\|g\|b]` | `/strobe/control/<name>[/...]` |
//! | `/controls/meta/<name>[/...]` | `/strobe/meta/<name>[/...]` |
//! | `/controls/global/<type>/<n>[/...]` | the `n`th control of that type (`slider`, `knob`, `toggle`, `bang`, `xy`, `color`, `dropdown`), from 1, in the playing scene |
//! | `/favslots/<n>` | `/strobe/fav/<n>` |
//! | `/playlist/next`, `/playlist/previous` | `next_scene`, `prev_scene` |
//! | `/playlist/toggleplay` | `toggle_hold` |
//! | `/controls/{scene\|meta}/<name>/{default\|random}`, and the same after `/controls/global/<type>/<n>` | that control to its default, or a random value (D247) |
//! | `/controls/groups/{scene\|meta}/<group>/{default\|random}` | every control of that `UI_GROUP` |
//! | `/controls/banks/{scene\|meta}/{default\|random}` | every control of the bank |
//!
//! Anything else in Synesthesia's space is ignored rather than guessed at:
//! the `/raw`, `/preset` and `/lock` suffixes, `/controls/banks/.../undo`
//! and `.../lock`,
//! scene-specific `/controls/<scenename>/...`, and everything that takes a
//! string argument (`/scenes`, `/presets`, `/media/name`), which the
//! decoder drops. A message carrying two or three numbers to a whole xy or
//! colour control sets its components, as Synesthesia's do
//! ([`crate::engine::Engine::components`]).

use crate::event::{Arg, OscMessage};
use crate::macros::MacroSet;
use crate::target::{ActionBank, ControlAction, Target, Verb};
use strobe_scene::names::normalize;

/// The deepest bundle nesting accepted (a bundle in a bundle in a bundle).
pub const MAX_DEPTH: usize = 4;

/// The most messages taken from one datagram.
pub const MAX_MESSAGES: usize = 256;

/// The most arguments kept from one message.
pub const MAX_ARGS: usize = 8;

/// The longest address accepted, in bytes.
pub const MAX_ADDRESS: usize = 256;

/// The prefix of the built-in address space.
pub const PREFIX: &str = "/strobe/";

/// Why a datagram was not used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// Bundles nest deeper than [`MAX_DEPTH`], or a bundle's sizes do not add
    /// up.
    #[error("malformed or over-nested OSC bundle")]
    Nesting,
    /// `rosc` could not make sense of it.
    #[error("not an OSC packet: {0}")]
    Packet(String),
}

/// Whether `bytes` (a message or a bundle) is well-formed to a depth of
/// [`MAX_DEPTH`], for bundles and for arrays in a message's type tags.
/// Needs no allocation and no recursion deeper than that.
pub fn nesting_ok(bytes: &[u8]) -> bool {
    walk(bytes, 0, 0)
}

/// `bytes` starts `offset` bytes into the datagram: `rosc` pads strings to
/// four bytes counted from the datagram's start, so the type tags are found
/// where it will look for them.
fn walk(bytes: &[u8], offset: usize, depth: usize) -> bool {
    const HEADER: &[u8] = b"#bundle\0";
    if !bytes.starts_with(HEADER) {
        return arrays_ok(bytes, offset);
    }
    if depth >= MAX_DEPTH {
        return false;
    }
    // 8 bytes of "#bundle\0", then an 8-byte time tag, then elements, each
    // a big-endian i32 size and that many bytes.
    let mut at = 16;
    let Some(mut rest) = bytes.get(at..) else {
        return false;
    };
    while !rest.is_empty() {
        let Some((size, tail)) = rest.split_first_chunk::<4>() else {
            return false;
        };
        let Ok(size) = usize::try_from(i32::from_be_bytes(*size)) else {
            return false;
        };
        let Some((element, after)) = tail.split_at_checked(size) else {
            return false;
        };
        if !walk(element, offset + at + 4, depth + 1) {
            return false;
        }
        at += 4 + size;
        rest = after;
    }
    true
}

/// Whether the message in `bytes` (`offset` into the datagram) opens no
/// more than [`MAX_DEPTH`] arrays inside one another. Anything that is not
/// laid out as a message is left for `rosc` to refuse.
fn arrays_ok(bytes: &[u8], offset: usize) -> bool {
    let Some(address_end) = bytes.iter().position(|b| *b == 0) else {
        return true;
    };
    let after = address_end + 1;
    let tags_start = after + (4 - (offset + after) % 4) % 4;
    let Some(tags) = bytes.get(tags_start..) else {
        return true;
    };
    let tags = tags.split(|b| *b == 0).next().unwrap_or_default();
    // `rosc` skips the first character (the comma) whatever it is.
    let mut open = 0_usize;
    for tag in tags.iter().skip(1) {
        match tag {
            b'[' => {
                open += 1;
                if open > MAX_DEPTH {
                    return false;
                }
            }
            b']' => open = open.saturating_sub(1),
            _ => {}
        }
    }
    true
}

/// Decodes one datagram into messages.
pub fn decode(datagram: &[u8]) -> Result<Vec<OscMessage>, DecodeError> {
    if !nesting_ok(datagram) {
        return Err(DecodeError::Nesting);
    }
    let (_, packet) =
        rosc::decoder::decode_udp(datagram).map_err(|e| DecodeError::Packet(e.to_string()))?;
    let mut out = Vec::new();
    flatten(&packet, &mut out);
    Ok(out)
}

fn flatten(packet: &rosc::OscPacket, out: &mut Vec<OscMessage>) {
    if out.len() >= MAX_MESSAGES {
        return;
    }
    match packet {
        rosc::OscPacket::Message(message) => {
            if let Some(message) = convert(message) {
                out.push(message);
            }
        }
        rosc::OscPacket::Bundle(bundle) => {
            for inner in &bundle.content {
                flatten(inner, out);
            }
        }
    }
}

fn convert(message: &rosc::OscMessage) -> Option<OscMessage> {
    if message.addr.len() > MAX_ADDRESS || !message.addr.starts_with('/') {
        return None;
    }
    let args: Vec<Arg> = message.args.iter().filter_map(arg).take(MAX_ARGS).collect();
    if args.is_empty() && !message.args.is_empty() {
        return None;
    }
    Some(OscMessage {
        address: message.addr.clone(),
        args,
    })
}

fn arg(value: &rosc::OscType) -> Option<Arg> {
    match value {
        rosc::OscType::Float(x) => x.is_finite().then_some(Arg::Float(*x)),
        // A finite double can be too big for a float, and `as` would make it
        // infinity: clamp it to the f32 range, as a long is to the i32's.
        rosc::OscType::Double(x) => x.is_finite().then_some(Arg::Float(
            x.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32,
        )),
        rosc::OscType::Int(x) => Some(Arg::Int(*x)),
        rosc::OscType::Long(x) => Some(Arg::Int(
            (*x).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        )),
        rosc::OscType::Bool(x) => Some(Arg::Bool(*x)),
        rosc::OscType::Nil | rosc::OscType::Inf => Some(Arg::Impulse),
        rosc::OscType::String(_)
        | rosc::OscType::Blob(_)
        | rosc::OscType::Time(_)
        | rosc::OscType::Char(_)
        | rosc::OscType::Color(_)
        | rosc::OscType::Midi(_)
        | rosc::OscType::Array(_) => None,
    }
}

/// Encodes a message for a test or a feedback sender.
pub fn encode(address: &str, args: &[Arg]) -> Option<Vec<u8>> {
    let args = args
        .iter()
        .map(|a| match a {
            Arg::Float(x) => rosc::OscType::Float(*x),
            Arg::Int(x) => rosc::OscType::Int(*x),
            Arg::Bool(x) => rosc::OscType::Bool(*x),
            Arg::Impulse => rosc::OscType::Nil,
        })
        .collect();
    rosc::encoder::encode(&rosc::OscPacket::Message(rosc::OscMessage {
        addr: address.to_string(),
        args,
    }))
    .ok()
}

/// What the built-in address space makes of `address`, if it is one of
/// ours or one of Synesthesia's that means the same. `macros` resolves a
/// macro given by name.
pub fn builtin_target(address: &str, macros: &MacroSet) -> Option<Target> {
    if let Some(target) = synesthesia_target(address) {
        return Some(target);
    }
    let rest = address.get(..PREFIX.len()).and_then(|head| {
        head.eq_ignore_ascii_case(PREFIX)
            .then(|| address.get(PREFIX.len()..))
            .flatten()
    })?;
    let (kind, tail) = rest.split_once('/')?;
    if tail.is_empty() || tail.starts_with('/') || tail.ends_with('/') || tail.contains("//") {
        return None;
    }
    let number = |text: &str| text.parse::<u32>().ok();
    match kind.to_ascii_lowercase().as_str() {
        "control" => {
            let (name, component) = split_component(tail);
            Some(Target::Control { name, component })
        }
        "meta" => {
            let (name, component) = split_component(tail);
            Some(Target::Meta { name, component })
        }
        "cmd" => (!tail.contains('/')).then(|| Target::command(tail)),
        "cue" => number(tail)
            .filter(|n| *n >= 1)
            .map(|number| Target::Cue { number }),
        "fav" => number(tail)
            .and_then(|n| u8::try_from(n).ok())
            .map(|slot| Target::FavSlot { slot }),
        "preset" => (!tail.contains('/')).then(|| Target::Preset {
            name: tail.to_string(),
        }),
        "macro" => {
            if let Some(id) = number(tail) {
                return Some(Target::Macro { id });
            }
            let wanted = normalize(tail);
            macros
                .all()
                .iter()
                .find(|m| normalize(&m.name) == wanted)
                .map(|m| Target::Macro { id: m.id })
        }
        _ => None,
    }
}

/// The Synesthesia types `/controls/global/<type>/<n>` may name.
const GLOBAL_TYPES: [&str; 7] = [
    "slider", "knob", "toggle", "bang", "xy", "color", "dropdown",
];

/// What Synesthesia's address `address` means in Strobe, if it means the
/// same thing (see the module docs).
fn synesthesia_target(address: &str) -> Option<Target> {
    let lower = address.to_ascii_lowercase();
    let parts: Vec<&str> = lower.strip_prefix('/')?.split('/').collect();
    if parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let position = |n: &str| n.parse::<u32>().ok().filter(|n| *n >= 1);
    let control = |name: &str, component: Option<&&str>| {
        let component = match component {
            Some(word) => Some(component_of(word)?),
            None => None,
        };
        Some(Target::Control {
            name: name.to_string(),
            component,
        })
    };
    let action = |verb: &str, bank: ActionBank, group: Option<&str>, name: Option<String>| {
        let verb = match verb {
            "default" => Verb::Default,
            "random" => Verb::Random,
            _ => return None,
        };
        Some(Target::Action(ControlAction {
            verb,
            bank,
            group: group.map(str::to_string),
            name,
        }))
    };
    let bank_of = |word: &str| match word {
        "scene" => Some(ActionBank::Scene),
        "meta" => Some(ActionBank::Meta),
        _ => None,
    };
    match parts.as_slice() {
        // Default and random (D247): a control, a group, a whole bank.
        ["controls", "scene", name, verb @ ("default" | "random")] => {
            action(verb, ActionBank::Scene, None, Some((*name).to_string()))
        }
        ["controls", "meta", name, verb @ ("default" | "random")] => {
            action(verb, ActionBank::Meta, None, Some((*name).to_string()))
        }
        ["controls", "global", kind, n, verb @ ("default" | "random")] => {
            let kind = if *kind == "colour" { "color" } else { kind };
            if !GLOBAL_TYPES.contains(&kind) {
                return None;
            }
            let n = position(n)?;
            let name = format!("{}{kind}/{n}", crate::target::BY_POSITION);
            action(verb, ActionBank::Scene, None, Some(name))
        }
        ["controls", "banks", bank, verb] => action(verb, bank_of(bank)?, None, None),
        ["controls", "groups", bank, group, verb] => {
            action(verb, bank_of(bank)?, Some(group), None)
        }
        ["controls", "scene", name, rest @ ..] if rest.len() <= 1 => control(name, rest.first()),
        ["controls", "meta", name, rest @ ..] if rest.len() <= 1 => {
            let component = match rest.first() {
                Some(word) => Some(component_of(word)?),
                None => None,
            };
            Some(Target::Meta {
                name: (*name).to_string(),
                component,
            })
        }
        ["controls", "global", kind, n, rest @ ..] if rest.len() <= 1 => {
            let kind = if *kind == "colour" { "color" } else { kind };
            if !GLOBAL_TYPES.contains(&kind) {
                return None;
            }
            let n = position(n)?;
            control(
                &format!("{}{kind}/{n}", crate::target::BY_POSITION),
                rest.first(),
            )
        }
        ["favslots", n] => position(n)
            .and_then(|n| u8::try_from(n).ok())
            .map(|slot| Target::FavSlot { slot }),
        ["playlist", "next"] => Some(Target::command("next_scene")),
        ["playlist", "previous"] => Some(Target::command("prev_scene")),
        ["playlist", "toggleplay"] => Some(Target::command("toggle_hold")),
        _ => None,
    }
}

/// A component word (`x`, `y`, `z`, `r`, `g`, `b`) as its index.
fn component_of(word: &str) -> Option<u8> {
    match word.to_ascii_lowercase().as_str() {
        "x" | "r" => Some(0),
        "y" | "g" => Some(1),
        "z" | "b" => Some(2),
        _ => None,
    }
}

/// `tint/r` into (`tint`, component 0); anything else is all name.
fn split_component(tail: &str) -> (String, Option<u8>) {
    if let Some((name, last)) = tail.rsplit_once('/') {
        let component = component_of(last);
        if component.is_some() {
            return (name.to_string(), component);
        }
    }
    (tail.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::Macro;
    use anyhow::{Result, anyhow};

    fn packet(address: &str, args: Vec<rosc::OscType>) -> Result<Vec<u8>> {
        Ok(rosc::encoder::encode(&rosc::OscPacket::Message(
            rosc::OscMessage {
                addr: address.to_string(),
                args,
            },
        ))?)
    }

    fn bundle(content: Vec<rosc::OscPacket>) -> Result<Vec<u8>> {
        Ok(rosc::encoder::encode(&rosc::OscPacket::Bundle(
            rosc::OscBundle {
                timetag: rosc::OscTime {
                    seconds: 0,
                    fractional: 1,
                },
                content,
            },
        ))?)
    }

    fn message(address: &str, args: Vec<rosc::OscType>) -> rosc::OscPacket {
        rosc::OscPacket::Message(rosc::OscMessage {
            addr: address.to_string(),
            args,
        })
    }

    #[test]
    fn a_float_message_decodes_with_its_address_and_argument() -> Result<()> {
        let bytes = packet("/strobe/control/speed", vec![rosc::OscType::Float(0.5)])?;
        let messages = decode(&bytes)?;
        assert_eq!(messages.len(), 1, "one message");
        assert_eq!(messages[0].address, "/strobe/control/speed", "address");
        assert_eq!(messages[0].args, vec![Arg::Float(0.5)], "args");
        Ok(())
    }

    #[test]
    fn our_own_encoder_round_trips_every_argument_kind() -> Result<()> {
        let args = [Arg::Float(0.25), Arg::Int(3), Arg::Bool(true), Arg::Impulse];
        let bytes = encode("/x", &args).ok_or_else(|| anyhow!("encode"))?;
        let messages = decode(&bytes)?;
        assert_eq!(messages[0].args, args.to_vec(), "round trip");
        Ok(())
    }

    #[test]
    fn a_hand_built_datagram_decodes() -> Result<()> {
        // "/a" padded to 4 bytes, type tag ",f" padded to 4, then 1.0 as a
        // big-endian float: what `python3 -c` sends in the end-to-end test.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"/a\0\0,f\0\0");
        bytes.extend_from_slice(&1.0_f32.to_be_bytes());
        let messages = decode(&bytes)?;
        assert_eq!(messages[0].address, "/a", "address");
        assert_eq!(messages[0].args, vec![Arg::Float(1.0)], "argument");
        Ok(())
    }

    #[test]
    fn bundles_are_flattened_and_their_time_tags_ignored() -> Result<()> {
        let inner = rosc::OscPacket::Bundle(rosc::OscBundle {
            timetag: rosc::OscTime {
                seconds: 5,
                fractional: 0,
            },
            content: vec![message("/c", vec![])],
        });
        let bytes = bundle(vec![
            message("/a", vec![rosc::OscType::Int(1)]),
            message("/b", vec![rosc::OscType::Bool(false)]),
            inner,
        ])?;
        let addresses: Vec<String> = decode(&bytes)?.into_iter().map(|m| m.address).collect();
        assert_eq!(addresses, vec!["/a", "/b", "/c"], "in order");
        Ok(())
    }

    #[test]
    fn deeply_nested_bundles_are_refused_before_rosc_recurses() -> Result<()> {
        let mut nested = message("/x", vec![]);
        for _ in 0..(MAX_DEPTH + 1) {
            nested = rosc::OscPacket::Bundle(rosc::OscBundle {
                timetag: rosc::OscTime {
                    seconds: 0,
                    fractional: 1,
                },
                content: vec![nested],
            });
        }
        let bytes = rosc::encoder::encode(&nested)?;
        assert_eq!(decode(&bytes), Err(DecodeError::Nesting), "too deep");
        Ok(())
    }

    #[test]
    fn a_bundle_of_thousands_of_levels_is_cheap_to_refuse() {
        // 20 bytes a level: "#bundle\0", a time tag and a size that points
        // at the next level. A 64 KB datagram holds thousands.
        let levels = 3000_usize;
        let mut bytes = vec![0_u8; 0];
        for i in 0..levels {
            bytes.extend_from_slice(b"#bundle\0");
            bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
            let remaining = (levels - i - 1) * 20;
            bytes.extend_from_slice(&(remaining as i32).to_be_bytes());
        }
        assert!(!nesting_ok(&bytes), "rejected by the guard");
        assert!(decode(&bytes).is_err(), "and by decode");
    }

    /// A message whose type tags open `depth` arrays and close them again:
    /// `,[[[...]]]` with no data, which `rosc` builds into `depth` nested
    /// arrays.
    fn nested_arrays(depth: usize) -> Vec<u8> {
        let mut bytes = b"/x\0\0".to_vec();
        bytes.push(b',');
        bytes.extend(std::iter::repeat_n(b'[', depth));
        bytes.extend(std::iter::repeat_n(b']', depth));
        bytes.push(0);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn deeply_nested_arrays_are_refused_on_a_listener_sized_stack() -> Result<()> {
        // 32,000 levels fit one datagram. `rosc` builds them without
        // recursing, but dropping the result recurses once per level, which
        // overflows the OSC thread's stack and aborts the app: anyone who
        // can reach the port could take the projector down.
        let bytes = nested_arrays(32_000);
        assert!(bytes.len() <= 65_507, "fits one UDP datagram");
        let worker = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || decode(&bytes).is_err())?;
        let refused = worker
            .join()
            .map_err(|_| anyhow!("the decoder thread died"))?;
        assert!(refused, "refused before rosc builds it");
        Ok(())
    }

    #[test]
    fn shallow_arrays_still_decode_and_are_dropped_as_arguments() -> Result<()> {
        let mut bytes = b"/x\0\0".to_vec();
        bytes.extend_from_slice(b",f[f]\0\0\0");
        bytes.extend_from_slice(&0.5_f32.to_be_bytes());
        bytes.extend_from_slice(&0.25_f32.to_be_bytes());
        assert_eq!(
            decode(&bytes)?[0].args,
            vec![Arg::Float(0.5)],
            "the float outside the array is kept"
        );
        assert!(
            decode(&nested_arrays(MAX_DEPTH))?.is_empty(),
            "arrays as deep as bundles may go decode, and carry nothing usable"
        );
        assert!(
            decode(&nested_arrays(MAX_DEPTH + 1)).is_err(),
            "one deeper is refused"
        );
        let inner = nested_arrays(MAX_DEPTH + 1);
        let mut wrapped = b"#bundle\0".to_vec();
        wrapped.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        wrapped.extend_from_slice(&i32::try_from(inner.len())?.to_be_bytes());
        wrapped.extend_from_slice(&inner);
        assert!(!nesting_ok(&wrapped), "and inside a bundle");
        Ok(())
    }

    #[test]
    fn lying_bundle_sizes_are_malformed_not_a_panic() {
        let mut bytes = b"#bundle\0".to_vec();
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&1000_i32.to_be_bytes());
        bytes.extend_from_slice(b"/x\0\0");
        assert!(!nesting_ok(&bytes), "size past the end");
        let mut negative = b"#bundle\0".to_vec();
        negative.extend_from_slice(&[0; 8]);
        negative.extend_from_slice(&(-4_i32).to_be_bytes());
        assert!(!nesting_ok(&negative), "negative size");
        assert!(!nesting_ok(b"#bundle\0\0\0"), "truncated header");
    }

    #[test]
    fn garbage_is_an_error_and_never_a_panic() {
        for garbage in [
            &b""[..],
            b"hello",
            b"/x",
            b"\xff\xff\xff\xff",
            b"/x\0\0,z\0\0",
        ] {
            assert!(decode(garbage).is_err(), "{garbage:?}");
        }
    }

    #[test]
    fn unusable_arguments_are_dropped_and_an_argument_less_leftover_drops_the_message() -> Result<()>
    {
        let only_text = packet(
            "/strobe/cmd/blackout",
            vec![rosc::OscType::String("go".into())],
        )?;
        assert!(decode(&only_text)?.is_empty(), "text alone is not a press");
        let mixed = packet(
            "/x",
            vec![rosc::OscType::String("a".into()), rosc::OscType::Float(0.5)],
        )?;
        assert_eq!(
            decode(&mixed)?[0].args,
            vec![Arg::Float(0.5)],
            "the float is kept"
        );
        let nan = packet("/x", vec![rosc::OscType::Float(f32::NAN)])?;
        assert!(decode(&nan)?.is_empty(), "NaN is not a value");
        Ok(())
    }

    #[test]
    fn arguments_addresses_and_message_counts_are_bounded() -> Result<()> {
        let many = packet("/x", (0..20).map(rosc::OscType::Int).collect())?;
        assert_eq!(decode(&many)?[0].args.len(), MAX_ARGS, "args");
        let long = packet(&format!("/{}", "a".repeat(MAX_ADDRESS)), vec![])?;
        assert!(decode(&long)?.is_empty(), "address");
        let flood = bundle(
            (0..(MAX_MESSAGES + 50))
                .map(|_| message("/x", vec![]))
                .collect(),
        )?;
        assert_eq!(decode(&flood)?.len(), MAX_MESSAGES, "messages");
        let relative = packet("x", vec![])?;
        assert!(
            decode(&relative).map(|m| m.is_empty()).unwrap_or(true),
            "no leading slash"
        );
        Ok(())
    }

    #[test]
    fn longs_and_doubles_are_narrowed_without_overflow() -> Result<()> {
        let bytes = packet(
            "/x",
            vec![
                rosc::OscType::Long(i64::MAX),
                rosc::OscType::Double(0.5),
                rosc::OscType::Inf,
            ],
        )?;
        assert_eq!(
            decode(&bytes)?[0].args,
            vec![Arg::Int(i32::MAX), Arg::Float(0.5), Arg::Impulse],
            "narrowed"
        );
        Ok(())
    }

    #[test]
    fn a_double_too_big_for_a_float_is_the_biggest_float_not_infinity() -> Result<()> {
        let bytes = packet(
            "/x",
            vec![
                rosc::OscType::Double(1e300),
                rosc::OscType::Double(-1e300),
                rosc::OscType::Double(f64::from(f32::MAX)),
            ],
        )?;
        let args = decode(&bytes)?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("the message was dropped"))?
            .args;
        assert_eq!(
            args,
            vec![
                Arg::Float(f32::MAX),
                Arg::Float(-f32::MAX),
                Arg::Float(f32::MAX)
            ],
            "clamped to the f32 range, every one finite"
        );
        Ok(())
    }

    #[test]
    fn what_is_not_a_number_is_dropped_whatever_its_width() -> Result<()> {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let double = packet("/x", vec![rosc::OscType::Double(value)])?;
            assert!(
                decode(&double)?.is_empty(),
                "a double {value} is dropped, with its message"
            );
            let single = packet("/x", vec![rosc::OscType::Float(value as f32)])?;
            assert!(
                decode(&single)?.is_empty(),
                "a float {value} is dropped, with its message"
            );
        }
        Ok(())
    }

    fn target(address: &str) -> Option<Target> {
        builtin_target(address, &MacroSet::default())
    }

    #[test]
    fn synesthesias_addresses_are_aliases_and_the_rest_is_ignored() {
        let component = |name: &str, c: u8| Target::Control {
            name: name.into(),
            component: Some(c),
        };
        assert_eq!(
            target("/controls/scene/speed"),
            Some(Target::control("speed")),
            "scene control"
        );
        assert_eq!(
            target("/controls/scene/tint/g"),
            Some(component("tint", 1)),
            "a component"
        );
        assert_eq!(
            target("/controls/meta/hue"),
            Some(Target::meta("hue")),
            "meta"
        );
        assert_eq!(
            target("/Controls/Global/Colour/2/b"),
            Some(component("@color/2", 2)),
            "global, any case, British colour"
        );
        assert_eq!(
            target("/favslots/3"),
            Some(Target::FavSlot { slot: 3 }),
            "fav"
        );
        assert_eq!(
            target("/playlist/toggleplay"),
            Some(Target::command("toggle_hold")),
            "hold"
        );
        let act = |verb, bank, group: Option<&str>, name: Option<&str>| {
            Some(Target::Action(ControlAction {
                verb,
                bank,
                group: group.map(str::to_string),
                name: name.map(str::to_string),
            }))
        };
        assert_eq!(
            target("/controls/scene/speed/default"),
            act(Verb::Default, ActionBank::Scene, None, Some("speed")),
            "one control to its default"
        );
        assert_eq!(
            target("/controls/meta/hue/random"),
            act(Verb::Random, ActionBank::Meta, None, Some("hue")),
            "one meta control, random"
        );
        assert_eq!(
            target("/controls/global/slider/2/random"),
            act(Verb::Random, ActionBank::Scene, None, Some("@slider/2")),
            "a global control"
        );
        assert_eq!(
            target("/controls/groups/meta/color/random"),
            act(Verb::Random, ActionBank::Meta, Some("color"), None),
            "a group, as Synesthesia's manual shows"
        );
        assert_eq!(
            target("/controls/banks/scene/default"),
            act(Verb::Default, ActionBank::Scene, None, None),
            "a whole bank"
        );
        for ignored in [
            "/controls/scene/speed/raw",
            "/controls/banks/scene/undo",
            "/controls/banks/meta/lock",
            "/controls/banks/media/default",
            "/controls/groups/scene/color/lock",
            "/controls/meta/hue/lock",
            "/controls/hexarray/separation",
            "/controls/global/fader/1",
            "/controls/global/slider/0",
            "/controls/scene//speed",
            "/scenes/hexarray",
            "/playlist/position",
            "/favslots/300",
        ] {
            assert_eq!(target(ignored), None, "{ignored}");
        }
    }

    #[test]
    fn the_builtin_space_reaches_controls_meta_commands_cues_favs_and_presets() {
        assert_eq!(
            target("/strobe/control/speed"),
            Some(Target::control("speed")),
            "control"
        );
        assert_eq!(
            target("/strobe/control/Motion/Speed"),
            Some(Target::control("Motion/Speed")),
            "group/name"
        );
        assert_eq!(
            target("/strobe/control/tint/g"),
            Some(Target::Control {
                name: "tint".into(),
                component: Some(1)
            }),
            "component"
        );
        assert_eq!(
            target("/strobe/meta/hue"),
            Some(Target::meta("hue")),
            "meta"
        );
        assert_eq!(
            target("/strobe/cmd/next_scene"),
            Some(Target::command("next_scene")),
            "command"
        );
        assert_eq!(
            target("/strobe/cue/3"),
            Some(Target::Cue { number: 3 }),
            "cue"
        );
        assert_eq!(
            target("/strobe/fav/2"),
            Some(Target::FavSlot { slot: 2 }),
            "fav"
        );
        assert_eq!(
            target("/strobe/preset/Slow_Burn"),
            Some(Target::Preset {
                name: "Slow_Burn".into()
            }),
            "preset"
        );
        assert_eq!(
            target("/STROBE/CONTROL/speed"),
            Some(Target::control("speed")),
            "prefix and kind ignore case"
        );
    }

    #[test]
    fn addresses_that_are_not_ours_or_not_well_formed_are_nothing() {
        for address in [
            "/other/control/speed",
            "/strobe",
            "/strobe/",
            "/strobe/control",
            "/strobe/control/",
            "/strobe/control//x",
            "/strobe/control/speed/",
            "/strobe/cue/0",
            "/strobe/cue/x",
            "/strobe/fav/300",
            "/strobe/cmd/a/b",
            "/strobe/preset/a/b",
            "/strobe/unknown/x",
            "strobe/control/x",
        ] {
            assert_eq!(target(address), None, "{address}");
        }
    }

    #[test]
    fn a_macro_can_be_addressed_by_id_or_by_name() {
        let mut macros = MacroSet::default();
        let id = macros.add(Macro::new("Big Zoom", Target::control("zoom")));
        assert_eq!(
            builtin_target("/strobe/macro/1", &macros),
            Some(Target::Macro { id: 1 }),
            "id"
        );
        assert_eq!(
            builtin_target("/strobe/macro/big_zoom", &macros),
            Some(Target::Macro { id }),
            "name by the script rule"
        );
        assert_eq!(
            builtin_target("/strobe/macro/nope", &macros),
            None,
            "unknown name"
        );
    }
}
