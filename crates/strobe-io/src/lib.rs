// ============================================================================
// Module:       strobe_io (library crate root)
// Description:  Show control for Strobe: MIDI and OSC input on worker
//               threads, the mapping model with learn and soft takeover, and
//               the Performance Macro set. No GL, no windows.
//
// Dependencies: strobe-scene, serde, serde_json, rosc, midir (feature midi)
// ============================================================================

//! Show control (Phase 6, D170 to D177).
//!
//! A controller (a fader bank, a pad grid, a phone running TouchOSC) moves
//! controls, recalls presets and runs transport commands. This crate is
//! everything about that which is not the UI and not the renderer:
//!
//! | Part | Module | What it is |
//! |---|---|---|
//! | MIDI bytes | [`message`] | parse note, CC, pitch bend, program change; a device name reduced to a key that survives a replug |
//! | OSC bytes | [`osc`] | decode a datagram, bounded; the built-in `/strobe/...` address space |
//! | events | [`event`] | what the workers deliver; [`Source`] and [`Signal`], the pair a mapping works on |
//! | workers | [`hub`], [`midi`], [`listener`] | a MIDI thread (poll, open, hot-plug) and an OSC thread, one bounded queue |
//! | targets | [`target`] | what a mapping acts on, and the six states (resolved, unavailable, broken, disabled, learning, conflict) |
//! | mappings | [`mapping`], [`curve`], [`takeover`] | range, curve, invert, button mode, soft takeover |
//! | the mapper | [`engine`] | message in, `ControlBank` write or command out; learn and conflicts |
//! | macros | [`macros`] | the curated, ordered, persistent Performance Macro set |
//! | persistence | [`config`] | `show-control.json`, tolerant of hand edits |
//!
//! # How the app uses it
//!
//! ```text
//! start     Hub::start(&midi, &osc)                       (two worker threads)
//! per scene frame:
//!           hub.drain(MAX, &mut events)                   (never blocks)
//!           for each Event::Input: Source::of -> (source, signal)
//!                                  engine.handle(&source, signal, &mut banks, &mut out)
//!           for each Output: the same Action a button would send
//! ```
//!
//! `strobe-io` depends on `strobe-scene` (for [`ControlBank`] and presets)
//! and on nothing else of Strobe's; in particular **not** on `strobe-gfx`, so
//! every rule here is tested with no GPU, no device and no window. MIDI is
//! tested from recorded byte sequences and OSC over loopback UDP.
//!
//! [`ControlBank`]: strobe_scene::control::ControlBank

pub mod config;
pub mod curve;
pub mod engine;
pub mod event;
pub mod hub;
mod listener;
pub mod macros;
pub mod mapping;
pub mod message;
mod midi;
pub mod osc;
pub mod takeover;
pub mod target;

pub use config::{MidiSettings, OscSettings, ShowControl};
pub use curve::Curve;
pub use engine::{Armed, Banks, Conflict, Engine, Outcome, Output};
pub use event::{Arg, DeviceInfo, Event, InputEvent, OscMessage, Signal, Source, Status};
pub use hub::Hub;
pub use macros::{Macro, MacroSet};
pub use mapping::{ButtonMode, Mapping};
pub use message::MidiMessage;
pub use takeover::Takeover;
pub use target::{Resolution, Scope, State, Target};
