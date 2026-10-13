// ============================================================================
// Module:       strobe_io::config
// Description:  The show-control file: OSC and MIDI settings, the mappings,
//               the Performance Macros, the fav slots and the hotkeys, as one
//               tolerant JSON document in the data directory.
//
// Dependencies: serde, serde_json; crate::{mapping, macros}; strobe-scene
//               (FavSlots)
// ============================================================================

//! `show-control.json` (D170, D172).
//!
//! One file, written whole and atomically by the app's saver thread, read
//! once at start. It is *tolerant*, because it is the kind of file people
//! edit while a show is being built:
//!
//! - every field has a default, so a file may hold only what it changes;
//! - unknown keys are ignored, so an older build reads a newer file;
//! - a **mapping that does not parse** (an unknown source kind, a typo in a
//!   target) is skipped and reported, and the rest still load; the app keeps
//!   a `.bak` copy of the original before it ever writes over a file that
//!   had problems;
//! - a file that is not JSON at all loads as the defaults, with the reason.
//!
//! OSC is **off** until switched on (here, in the MAP pane or with
//! `--osc`), and binds to the loopback interface unless `all_interfaces`
//! says otherwise (D172).

use crate::macros::MacroSet;
use crate::mapping::Mapping;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use strobe_scene::preset::FavSlots;

/// The file's name in the data directory.
pub const FILE: &str = "show-control.json";

/// The default OSC port.
pub const DEFAULT_OSC_PORT: u16 = 9000;

/// The default port OSC feedback is sent to (TouchOSC's usual pairing with
/// a listener on 9000).
pub const DEFAULT_FEEDBACK_PORT: u16 = 9001;

/// The file format version.
const VERSION: u32 = 1;

/// The OSC listener's settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OscSettings {
    /// Listen at all.
    pub enabled: bool,
    /// The UDP port. `0` lets the system choose (tests do).
    pub port: u16,
    /// Listen on every interface instead of only this machine (D172).
    pub all_interfaces: bool,
    /// Answer the built-in `/strobe/...` addresses.
    pub builtin: bool,
    /// Send the show's state back out (D209). Off until switched on.
    pub feedback: bool,
    /// Where feedback goes: a host name or address.
    pub feedback_host: String,
    /// The port feedback goes to.
    pub feedback_port: u16,
}

impl Default for OscSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_OSC_PORT,
            all_interfaces: false,
            builtin: true,
            feedback: false,
            feedback_host: "127.0.0.1".to_string(),
            feedback_port: DEFAULT_FEEDBACK_PORT,
        }
    }
}

impl OscSettings {
    /// The address to bind: `127.0.0.1:port`, or `0.0.0.0:port` when
    /// `all_interfaces`.
    pub fn bind_addr(&self) -> SocketAddr {
        let ip = if self.all_interfaces {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        SocketAddr::new(IpAddr::V4(ip), self.port)
    }
}

/// The MIDI side's settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MidiSettings {
    /// Open MIDI input devices at all.
    pub enabled: bool,
    /// Device keys the operator switched off.
    pub disabled_devices: Vec<String>,
}

impl Default for MidiSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled_devices: Vec::new(),
        }
    }
}

/// Everything saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShowControl {
    /// The file format version.
    pub version: u32,
    /// OSC.
    pub osc: OscSettings,
    /// MIDI.
    pub midi: MidiSettings,
    /// The mappings.
    pub mappings: Vec<Mapping>,
    /// The Performance Macros.
    pub macros: MacroSet,
    /// The fav slots.
    pub favs: FavSlots,
    /// Keyboard shortcut overrides: command id to chord text.
    pub hotkeys: BTreeMap<String, String>,
}

impl Default for ShowControl {
    fn default() -> Self {
        Self {
            version: VERSION,
            osc: OscSettings::default(),
            midi: MidiSettings::default(),
            mappings: Vec::new(),
            macros: MacroSet::default(),
            favs: FavSlots::default(),
            hotkeys: BTreeMap::new(),
        }
    }
}

/// A file read, with what was wrong with it.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    /// What was usable.
    pub config: ShowControl,
    /// Why parts were not (empty for a clean file).
    pub problems: Vec<String>,
}

impl ShowControl {
    /// The file as pretty JSON.
    pub fn to_text(&self) -> Result<String, serde_json::Error> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }

    /// Reads a file's text, keeping what parses.
    pub fn from_text(text: &str) -> Loaded {
        let mut loaded = Loaded::default();
        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(value) => value,
            Err(error) => {
                loaded.problems.push(format!("not valid JSON: {error}"));
                return loaded;
            }
        };
        let serde_json::Value::Object(mut map) = value else {
            loaded
                .problems
                .push("the file is not a JSON object".to_string());
            return loaded;
        };
        let mappings = map.remove("mappings");
        let mut config: ShowControl = match serde_json::from_value(serde_json::Value::Object(map)) {
            Ok(config) => config,
            Err(error) => {
                loaded.problems.push(format!("settings: {error}"));
                ShowControl::default()
            }
        };
        match mappings {
            Some(serde_json::Value::Array(items)) => {
                for (index, item) in items.into_iter().enumerate() {
                    match serde_json::from_value::<Mapping>(item) {
                        Ok(mapping) => config.mappings.push(mapping),
                        Err(error) => loaded
                            .problems
                            .push(format!("mapping {}: {error}", index + 1)),
                    }
                }
            }
            Some(_) => loaded.problems.push("mappings: not a list".to_string()),
            None => {}
        }
        config.favs.pad();
        config.macros.repair();
        loaded.config = config;
        loaded
    }

    /// Reads the file at `path`. A missing file is the defaults with no
    /// problem; an unreadable one is the defaults with the reason.
    pub fn load(path: &Path) -> Loaded {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_text(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Loaded::default(),
            Err(error) => Loaded {
                config: Self::default(),
                problems: vec![format!("{}: {error}", path.display())],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Source;
    use crate::macros::Macro;
    use crate::target::Target;
    use anyhow::Result;
    use strobe_scene::preset::Fav;

    fn sample() -> ShowControl {
        let mut config = ShowControl::default();
        config.osc.enabled = true;
        config.osc.port = 9100;
        config.midi.disabled_devices = vec!["Thru".into()];
        config.mappings.push(Mapping::new(
            Source::MidiCc {
                device: Some("nanoKONTROL2".into()),
                channel: 1,
                controller: 0,
            },
            Target::control("speed"),
        ));
        config
            .macros
            .add(Macro::new("Speed", Target::control("speed")));
        config.favs.set(
            0,
            Fav {
                scene: "04-voronoi".into(),
                preset: Some("slow".into()),
            },
        );
        config
            .hotkeys
            .insert("toggle_blackout".into(), "Ctrl+B".into());
        config
    }

    #[test]
    fn the_whole_file_round_trips() -> Result<()> {
        let config = sample();
        let loaded = ShowControl::from_text(&config.to_text()?);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        assert_eq!(loaded.config, config, "round trip");
        Ok(())
    }

    #[test]
    fn an_empty_object_is_the_defaults_and_osc_is_off_on_loopback() {
        let loaded = ShowControl::from_text("{}");
        assert!(loaded.problems.is_empty(), "clean");
        let osc = &loaded.config.osc;
        assert!(!osc.enabled, "off until switched on");
        assert_eq!(
            osc.bind_addr().to_string(),
            "127.0.0.1:9000",
            "loopback, 9000"
        );
        assert!(loaded.config.midi.enabled, "MIDI on");
    }

    #[test]
    fn all_interfaces_is_an_explicit_choice() {
        let osc = OscSettings {
            all_interfaces: true,
            port: 9001,
            ..OscSettings::default()
        };
        assert_eq!(
            osc.bind_addr().to_string(),
            "0.0.0.0:9001",
            "every interface"
        );
    }

    #[test]
    fn a_bad_mapping_is_skipped_and_named_and_the_rest_load() -> Result<()> {
        let text = r#"{
            "mappings": [
                { "source": { "kind": "osc", "address": "/a" }, "target": { "kind": "meta", "name": "hue" } },
                { "source": { "kind": "telepathy" }, "target": { "kind": "meta", "name": "hue" } },
                { "source": { "kind": "osc", "address": "/b" }, "target": { "kind": "command", "id": "next_scene" } }
            ],
            "osc": { "enabled": true }
        }"#;
        let loaded = ShowControl::from_text(text);
        assert_eq!(loaded.config.mappings.len(), 2, "two good ones");
        assert_eq!(
            loaded.problems.len(),
            1,
            "one problem: {:?}",
            loaded.problems
        );
        assert!(
            loaded.problems[0].starts_with("mapping 2:"),
            "{}",
            loaded.problems[0]
        );
        assert!(loaded.config.osc.enabled, "other settings survive");
        Ok(())
    }

    #[test]
    fn unknown_keys_are_ignored_and_junk_loads_as_defaults_with_the_reason() {
        let newer = ShowControl::from_text(
            r#"{ "from_the_future": 1, "osc": { "port": 9500, "later": true } }"#,
        );
        assert!(newer.problems.is_empty(), "unknown keys are fine");
        assert_eq!(newer.config.osc.port, 9500, "known keys read");
        for junk in ["", "{ nope", "[1,2]", "42"] {
            let loaded = ShowControl::from_text(junk);
            assert_eq!(
                loaded.config,
                ShowControl::default(),
                "{junk:?} -> defaults"
            );
            assert!(!loaded.problems.is_empty(), "{junk:?} says why");
        }
    }

    #[test]
    fn a_wrongly_typed_setting_loses_the_settings_but_not_the_mappings() {
        let text = r#"{ "osc": { "port": "nine thousand" },
            "mappings": [ { "source": { "kind": "osc", "address": "/a" }, "target": { "kind": "cue", "number": 2 } } ] }"#;
        let loaded = ShowControl::from_text(text);
        assert_eq!(loaded.config.mappings.len(), 1, "mapping kept");
        assert!(
            loaded.problems.iter().any(|p| p.starts_with("settings:")),
            "{:?}",
            loaded.problems
        );
    }

    #[test]
    fn a_missing_file_is_quietly_the_defaults() {
        let loaded = ShowControl::load(Path::new("/definitely/not/here/show-control.json"));
        assert!(loaded.problems.is_empty(), "first run is not a problem");
        assert_eq!(loaded.config, ShowControl::default(), "defaults");
    }

    #[test]
    fn fav_slots_and_macro_ids_are_repaired_on_load() {
        let loaded = ShowControl::from_text(
            r#"{ "favs": { "slots": [] },
                 "macros": { "macros": [
                    { "name": "A", "target": { "kind": "control", "name": "a" } },
                    { "name": "B", "target": { "kind": "control", "name": "b" } } ] } }"#,
        );
        assert_eq!(
            loaded.config.favs.iter().count(),
            strobe_scene::preset::FAV_SLOTS,
            "padded"
        );
        let ids: Vec<u32> = loaded.config.macros.all().iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), 2, "two macros");
        assert!(
            ids[0] != ids[1] && ids.iter().all(|i| *i > 0),
            "unique ids: {ids:?}"
        );
    }
}
