// ============================================================================
// Module:       strobe_app::remote
// Description:  Show control as the app holds it: the MIDI/OSC workers, the
//               mapper with its mappings and macros, the fav slots, the
//               activity log, and the saved file. Drained once per frame.
//
// Dependencies: strobe-io (Hub, Engine, ShowControl); crate::prefs (the
//               debounced atomic writer); strobe-scene (FavSlots)
// ============================================================================

//! The app's side of show control (D170 to D177).
//!
//! [`Remote`] owns everything `strobe-io` provides and the little the app
//! adds to it:
//!
//! - **The workers**, started once. `--no-midi` leaves MIDI out; OSC is off
//!   until `--osc` or the MAP pane's switch turns it on.
//! - **The drain.** [`Remote::drain`] runs once per scene frame, before the
//!   scene advances (step 1 of the frame in `docs/ARCHITECTURE.md`). It takes
//!   at most [`MAX_PER_FRAME`] events, never blocks, runs each through the
//!   mapper (which writes the control banks directly) and returns the
//!   non-control results as [`Output`]s for the app to carry out through the
//!   same actions its buttons use.
//! - **The activity log**, the last [`LOG_LINES`] things heard and what each
//!   did, for the MAP pane's monitor. A fader sweep is one line with a count,
//!   not a hundred lines.
//! - **The file**, `show-control.json` in the data directory, written by the
//!   same debounced atomic writer as the UI preferences, never from the
//!   paint path, and never in screenshot mode.
//!
//! ## Command-line overrides are not saved
//!
//! `--osc`, `--osc-port` and `--osc-all-interfaces` change what runs *this
//! time*. The saved settings stay as the operator left them: a run with
//! `--osc-all-interfaces` must not leave the next launch open to the network
//! (D172). Changing the setting in the MAP pane is an explicit choice and
//! replaces both.

use crate::commands::CommandId;
use crate::prefs::{Debounce, Saver};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use strobe_io::config::{FILE, MidiSettings, OscSettings, ShowControl};
use strobe_io::mapping::conflicting;
use strobe_io::target::{Resolution, Scope, State, resolve};
use strobe_io::{
    DeviceInfo, Engine, Event, Hub, InputEvent, Mapping, Outcome, Output, Source, Status, Target,
};
use strobe_scene::control::ControlBank;
use strobe_scene::preset::FavSlots;

/// The most events taken from the queue in one frame. A fader sweep sends
/// hundreds a second; thirty-odd per frame at 60 fps is plenty and a flood
/// beyond that waits for the next frame (or is dropped by the queue).
pub const MAX_PER_FRAME: usize = 256;

/// Lines the activity monitor keeps.
pub const LOG_LINES: usize = 64;

/// How long to wait for the writer at shutdown.
const FINISH_LIMIT: Duration = Duration::from_secs(2);

/// What the command line asked for this run.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    /// Start the workers at all (false in screenshot mode).
    pub workers: bool,
    /// `--osc`: listen for OSC this run.
    pub osc: bool,
    /// `--osc-port`.
    pub osc_port: Option<u16>,
    /// `--osc-all-interfaces`.
    pub osc_all_interfaces: bool,
    /// `--no-midi`.
    pub no_midi: bool,
    /// The file to read (and write, unless `read_only`), instead of the data
    /// directory's.
    pub file: Option<PathBuf>,
    /// Never write the file (screenshot mode).
    pub read_only: bool,
}

/// What the OSC listener is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscState {
    /// Not listening.
    Off,
    /// Listening here.
    Listening(SocketAddr),
    /// Could not listen.
    Error(String),
}

/// One line of the activity monitor.
#[derive(Clone, Debug)]
pub struct Line {
    /// Seconds since the app started when it was last heard.
    pub at: f32,
    /// What was heard.
    pub text: String,
    /// What it did.
    pub result: String,
    /// How many times in a row.
    pub count: u32,
    /// Where it came from; `None` for a system line (a device came or went).
    pub source: Option<Source>,
}

/// The last things heard.
#[derive(Clone, Debug, Default)]
pub struct Activity {
    lines: VecDeque<Line>,
}

impl Activity {
    /// Adds a line, or folds it into the previous one when it is the same
    /// source with the same result (a fader moving).
    pub fn note(&mut self, at: f32, text: String, result: String, source: Option<Source>) {
        if let (Some(last), Some(source)) = (self.lines.back_mut(), source.as_ref())
            && last.source.as_ref() == Some(source)
            && last.result == result
        {
            last.at = at;
            last.text = text;
            last.count = last.count.saturating_add(1);
            return;
        }
        if self.lines.len() >= LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(Line {
            at,
            text,
            result,
            count: 1,
            source,
        });
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &Line> {
        self.lines.iter()
    }

    /// How many lines there are.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether nothing has been heard.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Forgets everything.
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

/// The state of every mapping and macro against what is playing now.
#[derive(Clone, Debug, Default)]
pub struct Evaluated {
    /// One per mapping, in order.
    pub mappings: Vec<Resolution>,
    /// One per macro (pinned or not), in order.
    pub macros: Vec<Resolution>,
}

/// Show control.
pub struct Remote {
    hub: Option<Hub>,
    /// The mapper: mappings, macros, learn.
    pub engine: Engine,
    /// OSC as the operator saved it.
    pub osc_saved: OscSettings,
    /// OSC as it is running (the saved settings plus command-line overrides).
    pub osc: OscSettings,
    /// MIDI settings.
    pub midi: MidiSettings,
    /// The fav slots.
    pub favs: FavSlots,
    /// Keyboard shortcut overrides, by command id.
    pub hotkeys: std::collections::BTreeMap<String, String>,
    /// MIDI inputs as the worker last listed them.
    pub devices: Vec<DeviceInfo>,
    /// Why MIDI cannot run at all, if it cannot.
    pub midi_note: Option<String>,
    /// The OSC listener.
    pub osc_state: OscState,
    /// Messages dropped because the app did not drain fast enough.
    pub dropped: u64,
    /// What was wrong with the file when it was read.
    pub problems: Vec<String>,
    /// The last things heard.
    pub activity: Activity,
    /// The mapping learn last bound, for the pane to scroll to.
    pub learned: Option<usize>,
    started: Instant,
    path: Option<PathBuf>,
    snapshot: ShowControl,
    debounce: Debounce,
    saver: Option<Saver>,
    events: Vec<Event>,
}

impl Remote {
    /// Reads the file, applies the overrides and starts the workers.
    pub fn new(data_dir: &Path, overrides: &Overrides, now: Instant) -> Self {
        let path = overrides
            .file
            .clone()
            .unwrap_or_else(|| data_dir.join(FILE));
        let loaded = ShowControl::load(&path);
        if !loaded.problems.is_empty() {
            for problem in &loaded.problems {
                log::warn!("{}: {problem}", path.display());
            }
            if !overrides.read_only {
                keep_a_copy(&path);
            }
        }
        let config = loaded.config;
        let mut osc = config.osc.clone();
        if overrides.osc {
            osc.enabled = true;
        }
        if let Some(port) = overrides.osc_port {
            osc.port = port;
        }
        if overrides.osc_all_interfaces {
            osc.all_interfaces = true;
        }
        let mut midi = config.midi.clone();
        if overrides.no_midi {
            midi.enabled = false;
        }
        let hub = overrides.workers.then(|| Hub::start(&midi, &osc));
        if osc.enabled && osc.all_interfaces {
            log::warn!(
                "OSC will listen on every network interface (port {}). Anyone on the network can send it commands.",
                osc.port
            );
        }
        let mut engine = Engine::default();
        engine.mappings = config.mappings.clone();
        engine.macros = config.macros.clone();
        engine.builtin_osc = osc.builtin;
        let writes = !overrides.read_only;
        Self {
            hub,
            engine,
            osc_saved: config.osc.clone(),
            osc,
            midi,
            favs: config.favs.clone(),
            hotkeys: config.hotkeys.clone(),
            devices: Vec::new(),
            midi_note: None,
            osc_state: OscState::Off,
            dropped: 0,
            problems: loaded.problems,
            activity: Activity::default(),
            learned: None,
            started: now,
            saver: writes.then(|| Saver::start(path.clone())),
            path: writes.then_some(path),
            snapshot: config,
            debounce: Debounce::default(),
            events: Vec::with_capacity(MAX_PER_FRAME),
        }
    }

    /// Whether the worker threads are running (they are not in screenshot
    /// mode).
    pub fn workers_running(&self) -> bool {
        self.hub.is_some()
    }

    /// The file this reads, and writes unless it is read-only.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    // ------------------------------------------------------------ the drain

    /// Runs the queued events through the mapper. Control writes land in
    /// `banks`; everything else is appended to `out`.
    pub fn drain(&mut self, banks: &mut strobe_io::Banks<'_>, now: Instant, out: &mut Vec<Output>) {
        let Some(hub) = self.hub.as_mut() else {
            return;
        };
        let mut events = std::mem::take(&mut self.events);
        events.clear();
        hub.drain(MAX_PER_FRAME, &mut events);
        let at = now.duration_since(self.started).as_secs_f32();
        for event in events.drain(..) {
            match event {
                Event::Status(status) => self.status(status, at),
                Event::Input(input) => self.input(&input, banks, at, out),
            }
        }
        self.events = events;
    }

    fn input(
        &mut self,
        input: &InputEvent,
        banks: &mut strobe_io::Banks<'_>,
        at: f32,
        out: &mut Vec<Output>,
    ) {
        let Some((source, signal)) = Source::of(input) else {
            return;
        };
        let outcome = self.engine.handle(&source, signal, banks, out);
        match &outcome {
            Outcome::Learned(index) => {
                self.learned = Some(*index);
                log::info!("learned {}", self.describe_mapping(*index));
            }
            Outcome::Conflict => log::info!(
                "learn: {} is already mapped; waiting for an answer",
                source.label()
            ),
            Outcome::Unmapped
            | Outcome::Applied(_)
            | Outcome::Waiting
            | Outcome::Unavailable(_)
            | Outcome::Ignored => {}
        }
        self.activity
            .note(at, input.describe(), outcome.label(), Some(source));
    }

    fn describe_mapping(&self, index: usize) -> String {
        self.engine
            .mappings
            .get(index)
            .map_or_else(String::new, strobe_io::Mapping::summary)
    }

    fn status(&mut self, status: Status, at: f32) {
        match status {
            Status::Devices(list) => self.devices = list,
            Status::DeviceFound(key) => {
                log::info!("MIDI: {key} connected");
                self.activity
                    .note(at, format!("MIDI: {key} connected"), String::new(), None);
            }
            Status::DeviceLost(key) => {
                log::warn!("MIDI: {key} disconnected");
                self.activity
                    .note(at, format!("MIDI: {key} disconnected"), String::new(), None);
            }
            Status::MidiUnavailable(why) => {
                if self.midi_note.as_deref() != Some(why.as_str()) {
                    log::info!("MIDI unavailable: {why}");
                }
                self.midi_note = Some(why);
            }
            Status::OscListening(addr) => {
                log::info!("OSC: listening on {addr}");
                self.osc_state = OscState::Listening(addr);
            }
            Status::OscStopped => self.osc_state = OscState::Off,
            Status::OscError(why) => {
                log::warn!("OSC: {why}");
                self.osc_state = OscState::Error(why);
            }
            Status::Dropped(total) => {
                if total != self.dropped {
                    log::warn!(
                        "show control: {total} input message(s) dropped so far (queue full)"
                    );
                }
                self.dropped = total;
            }
        }
        // A device list means MIDI works, whatever was said before.
        if !self.devices.is_empty() {
            self.midi_note = None;
        }
    }

    // ------------------------------------------------------------- settings

    /// Applies OSC settings the operator chose. They are saved, and replace
    /// any command-line override.
    pub fn set_osc(&mut self, settings: OscSettings) {
        if settings.enabled && settings.all_interfaces && !self.osc.all_interfaces {
            log::warn!(
                "OSC will listen on every network interface (port {}). Anyone on the network can send it commands.",
                settings.port
            );
        }
        self.engine.builtin_osc = settings.builtin;
        self.osc_saved = settings.clone();
        self.osc = settings;
        if let Some(hub) = &self.hub {
            hub.set_osc(&self.osc);
        }
    }

    /// Applies MIDI settings.
    pub fn set_midi(&mut self, settings: MidiSettings) {
        self.midi = settings;
        if let Some(hub) = &self.hub {
            hub.set_midi(&self.midi);
        }
    }

    /// Switches one device on or off.
    pub fn set_device_enabled(&mut self, key: &str, enabled: bool) {
        let mut settings = self.midi.clone();
        settings.disabled_devices.retain(|d| d != key);
        if !enabled {
            settings.disabled_devices.push(key.to_string());
        }
        self.set_midi(settings);
    }

    /// Looks at the MIDI ports again now.
    pub fn rescan_midi(&self) {
        if let Some(hub) = &self.hub {
            hub.rescan_midi();
        }
    }

    // ------------------------------------------------------------- states

    /// Resolves every mapping and macro against `scene` and `meta` (the
    /// controls that are playing now) and layers the mapper's own states on
    /// top: disabled, learning, conflict. Nothing is guessed: a name that is
    /// not there is unavailable.
    pub fn evaluate(
        &self,
        scene: Option<&ControlBank>,
        meta: Option<&ControlBank>,
        presets: &[String],
    ) -> Evaluated {
        let known = |id: &str| CommandId::from_id(id).is_some_and(CommandId::mappable);
        let scope = Scope {
            scene,
            meta,
            macros: &self.engine.macros,
            favs: &self.favs,
            presets,
            command_known: &known,
        };
        let conflicts = conflicting(&self.engine.mappings);
        let mappings = self
            .engine
            .mappings
            .iter()
            .enumerate()
            .map(|(index, mapping)| {
                if !mapping.enabled {
                    return layer(State::Disabled, "switched off");
                }
                if self.engine.is_relearning(index) {
                    return layer(State::Learning, "move a control or send a message");
                }
                if conflicts.get(index).copied().unwrap_or(false) {
                    return layer(
                        State::Conflict,
                        "another mapping answers to the same source",
                    );
                }
                resolve(&mapping.target, &scope)
            })
            .collect();
        let macros = self
            .engine
            .macros
            .all()
            .iter()
            .map(|found| {
                let target = Target::Macro { id: found.id };
                if !found.enabled {
                    return layer(State::Disabled, "switched off");
                }
                if self.engine.is_learning_target(&target) {
                    return layer(State::Learning, "move a control or send a message");
                }
                let clash = self.engine.mappings.iter().enumerate().any(|(index, m)| {
                    conflicts.get(index).copied().unwrap_or(false)
                        && (m.target == target || m.target == found.target)
                });
                if clash {
                    return layer(
                        State::Conflict,
                        "its source is also mapped to something else",
                    );
                }
                resolve(&target, &scope)
            })
            .collect();
        Evaluated { mappings, macros }
    }

    /// The mappings that drive macro `id`, directly or by naming the same
    /// thing the macro does.
    pub fn mappings_for_macro(&self, id: u32) -> impl Iterator<Item = &Mapping> {
        let own = self.engine.macros.get(id).map(|m| m.target.clone());
        self.engine
            .mappings
            .iter()
            .filter(move |m| m.target == Target::Macro { id } || Some(&m.target) == own.as_ref())
    }

    // ---------------------------------------------------------- persistence

    /// The settings as they would be written.
    fn current(&self) -> ShowControl {
        ShowControl {
            version: self.snapshot.version,
            osc: self.osc_saved.clone(),
            midi: self.midi.clone(),
            mappings: self.engine.mappings.clone(),
            macros: self.engine.macros.clone(),
            favs: self.favs.clone(),
            hotkeys: self.hotkeys.clone(),
        }
    }

    /// Whether what is live differs from what was last written, compared
    /// without building a copy.
    fn changed(&self) -> bool {
        self.snapshot.osc != self.osc_saved
            || self.snapshot.midi != self.midi
            || self.snapshot.mappings != self.engine.mappings
            || self.snapshot.macros != self.engine.macros
            || self.snapshot.favs != self.favs
            || self.snapshot.hotkeys != self.hotkeys
    }

    /// Called once per control-window frame: writes the file, off this
    /// thread, when changes have settled.
    pub fn persist(&mut self, now: Instant) {
        if self.saver.is_none() {
            return;
        }
        let changed = self.changed();
        // `changed` stays true until the write, so the debounce is told only
        // about the moment it first became true (otherwise it would never
        // be quiet).
        let newly = changed && !self.debounce.pending();
        if self.debounce.note(newly, now) {
            self.write();
        }
    }

    fn write(&mut self) {
        let current = self.current();
        match current.to_text() {
            Ok(text) => {
                if let Some(saver) = &self.saver {
                    saver.save(text);
                }
                self.snapshot = current;
            }
            Err(error) => log::warn!("could not serialise show control: {error}"),
        }
    }

    /// Writes anything pending, stops the workers and waits briefly for the
    /// writer.
    pub fn finish(mut self) {
        if self.saver.is_some() && self.changed() {
            self.write();
        }
        drop(self.hub.take());
        if let Some(saver) = self.saver.take() {
            saver.finish(FINISH_LIMIT);
        }
    }
}

fn layer(state: State, reason: &str) -> Resolution {
    Resolution {
        state,
        reason: reason.to_string(),
    }
}

/// Keeps `path` as `path.bak` before a file that had unreadable parts is
/// written over, so a hand edit is never lost to a typo.
fn keep_a_copy(path: &Path) {
    let mut backup = path.as_os_str().to_owned();
    backup.push(".bak");
    let backup = PathBuf::from(backup);
    if path.exists() && !backup.exists() {
        match std::fs::copy(path, &backup) {
            Ok(_) => log::warn!(
                "{} had parts that could not be read; the original is kept as {}",
                path.display(),
                backup.display()
            ),
            Err(error) => log::warn!("could not keep a copy of {}: {error}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use strobe_io::{Mapping, Target};

    fn dir(tag: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("strobe-remote-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn quiet() -> Overrides {
        Overrides {
            workers: false,
            ..Overrides::default()
        }
    }

    #[test]
    fn the_activity_log_folds_a_sweep_into_one_line_and_is_bounded() {
        let mut log = Activity::default();
        let cc = Source::MidiCc {
            device: Some("pad".into()),
            channel: 1,
            controller: 7,
        };
        for value in 0..50 {
            log.note(
                1.0,
                format!("CC 7 = {value}"),
                "applied".into(),
                Some(cc.clone()),
            );
        }
        assert_eq!(log.len(), 1, "one line");
        assert_eq!(log.lines().next().map(|l| l.count), Some(50), "counted");
        assert_eq!(
            log.lines().next().map(|l| l.text.as_str()),
            Some("CC 7 = 49"),
            "latest value"
        );
        log.note(2.0, "CC 7 = 3".into(), "unmapped".into(), Some(cc));
        assert_eq!(log.len(), 2, "a different result starts a new line");
        for n in 0..(LOG_LINES * 2) {
            log.note(3.0, format!("system {n}"), String::new(), None);
        }
        assert_eq!(log.len(), LOG_LINES, "bounded");
    }

    #[test]
    fn command_line_osc_settings_are_not_saved_but_the_panes_are() -> Result<()> {
        let dir = dir("overrides")?;
        let overrides = Overrides {
            osc: true,
            osc_port: Some(9123),
            osc_all_interfaces: true,
            read_only: true,
            ..quiet()
        };
        let remote = Remote::new(&dir, &overrides, Instant::now());
        assert!(
            remote.osc.enabled && remote.osc.all_interfaces,
            "running with the flags"
        );
        assert_eq!(remote.osc.port, 9123, "port");
        assert!(
            !remote.osc_saved.enabled && !remote.osc_saved.all_interfaces,
            "saved stays safe"
        );
        assert!(!remote.changed(), "and nothing is waiting to be written");
        let mut remote = remote;
        let mut chosen = remote.osc.clone();
        chosen.all_interfaces = false;
        remote.set_osc(chosen);
        assert!(remote.changed(), "an explicit choice is saved");
        assert!(
            !remote.osc_saved.all_interfaces && remote.osc_saved.enabled,
            "as chosen"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn edits_are_written_once_they_settle_and_read_back() -> Result<()> {
        let dir = dir("persist")?;
        let start = Instant::now();
        let mut remote = Remote::new(&dir, &quiet(), start);
        remote.persist(start);
        remote.engine.mappings.push(Mapping::new(
            Source::Osc {
                address: "/a".into(),
            },
            Target::meta("hue"),
        ));
        remote.persist(start + Duration::from_millis(10));
        assert!(
            !dir.join(FILE).exists(),
            "not yet: the edit has not settled"
        );
        remote.persist(start + Duration::from_secs(2));
        remote.finish();
        let loaded = ShowControl::load(&dir.join(FILE));
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        assert_eq!(loaded.config.mappings.len(), 1, "the mapping was saved");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_file_with_a_bad_mapping_is_kept_as_a_backup_before_it_is_rewritten() -> Result<()> {
        let dir = dir("backup")?;
        let text = r#"{ "mappings": [ { "source": { "kind": "bogus" }, "target": { "kind": "cue", "number": 1 } } ] }"#;
        std::fs::write(dir.join(FILE), text)?;
        let remote = Remote::new(&dir, &quiet(), Instant::now());
        assert_eq!(remote.problems.len(), 1, "reported");
        let backup = dir.join(format!("{FILE}.bak"));
        assert_eq!(
            std::fs::read_to_string(&backup)?,
            text,
            "the original is kept"
        );
        remote.finish();
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_read_only_remote_never_writes() -> Result<()> {
        let dir = dir("readonly")?;
        let overrides = Overrides {
            read_only: true,
            ..quiet()
        };
        let start = Instant::now();
        let mut remote = Remote::new(&dir, &overrides, start);
        remote.engine.mappings.push(Mapping::new(
            Source::Osc {
                address: "/a".into(),
            },
            Target::meta("hue"),
        ));
        remote.persist(start + Duration::from_secs(10));
        remote.finish();
        assert!(!dir.join(FILE).exists(), "nothing written");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_remote_without_workers_drains_nothing_and_does_not_fail() -> Result<()> {
        let dir = dir("noworkers")?;
        let mut remote = Remote::new(&dir, &quiet(), Instant::now());
        assert!(!remote.workers_running(), "no workers");
        let mut banks = strobe_io::Banks {
            scene: None,
            meta: None,
        };
        let mut out = Vec::new();
        remote.drain(&mut banks, Instant::now(), &mut out);
        assert!(out.is_empty(), "nothing");
        remote.finish();
        std::fs::remove_dir_all(&dir).map_err(|e| anyhow!(e))?;
        Ok(())
    }
}
