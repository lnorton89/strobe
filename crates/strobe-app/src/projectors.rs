// ============================================================================
// Module:       strobe_app::projectors
// Description:  Projector power as the app holds it: projectors.json, the
//               Connections form, power and status on worker threads, the
//               web page's service, and what Settings > Projectors shows.
//
// Dependencies: strobe-projector (config, unit, server, service, audit)
// ============================================================================

//! The app's side of projector control (D502).
//!
//! [`Projectors`] owns `projectors.json` in the data directory, the form
//! Settings edits, and the web page's [`Service`] while it runs. Nothing
//! talks to a projector until the operator switches projector control on;
//! then the units' status is read every [`POLL`] and on demand.
//!
//! A projector answers in seconds and a power command can wait out a
//! 90 s cool-down, so nothing here ever runs on the frame thread: a
//! command or a status read is a short-lived worker thread, and
//! [`Projectors::maintain`] takes its result off a channel without
//! waiting, once a frame.
//!
//! The two switches (control on, serve the web page) are saved in the file
//! and come back at launch, so a show PC that serves the page keeps
//! serving it after a restart, as the Python tool's service did. The
//! headless `strobe-projector serve` reads the same file; when it already
//! holds the port, Strobe's page says so and keeps trying (one config,
//! whoever binds first serves it).
//!
//! The form is a copy: typing never changes what is saved or served until
//! Save, which checks the whole configuration first.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, PoisonError, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use strobe_projector::audit::Audit;
use strobe_projector::config::{self, Bind, Config, Projector};
use strobe_projector::server::Shared;
use strobe_projector::service::{Service, State};
use strobe_projector::unit::{self, PowerResult, Status, Timing};

/// How often the units' status is read while control is on.
pub const POLL: Duration = Duration::from_secs(30);

/// The audit log's name for a command from this window.
const SOURCE: &str = "strobe";

/// One projector in the Connections form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftUnit {
    /// Its key: the name the web page's API, `pj` and the log use.
    pub key: String,
    /// Its settings.
    pub projector: Projector,
}

/// The Connections form: an editable copy of the saved projectors and the
/// page's bind mode and port.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    /// The projectors, in the file's order.
    pub units: Vec<DraftUnit>,
    /// The page's bind mode.
    pub bind: Bind,
    /// The page's port, as typed.
    pub port: String,
    /// The page's password (D516), as typed.
    pub password: String,
    /// The projector whose settings are open below its row.
    pub open: Option<usize>,
    /// Off was pressed once for this target (`all` or a key); the second
    /// press sends it.
    pub confirm_off: Option<String>,
}

impl Form {
    /// The form for `config`.
    pub fn from_config(config: &Config) -> Self {
        Self {
            units: config
                .projectors
                .iter()
                .map(|(key, projector)| DraftUnit {
                    key: key.clone(),
                    projector: projector.clone(),
                })
                .collect(),
            bind: config.api.bind,
            port: config.api.port.to_string(),
            password: config.api.password.clone(),
            open: None,
            confirm_off: None,
        }
    }

    /// Whether the form holds the same settings as `config`.
    pub fn matches(&self, config: &Config) -> bool {
        let saved = Self::from_config(config);
        self.units == saved.units
            && self.bind == saved.bind
            && self.port.trim() == saved.port
            && self.password == saved.password
    }

    /// `base` with the form's projectors, bind, port and page password, or
    /// what is wrong.
    pub fn apply(&self, base: &Config) -> Result<Config, String> {
        let mut config = base.clone();
        config.projectors.clear();
        for unit in &self.units {
            let key = unit.key.trim().to_string();
            if config.projectors.contains_key(&key) {
                return Err(format!("two projectors are called '{key}'"));
            }
            let mut projector = unit.projector.clone();
            projector.name = projector.name.trim().to_string();
            projector.ip = projector.ip.trim().to_string();
            config.projectors.insert(key, projector);
        }
        config.api.bind = self.bind;
        config.api.port = self
            .port
            .trim()
            .parse()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| format!("port '{}' is not a port number", self.port.trim()))?;
        // An empty password keeps the page shut; a short one is refused.
        if !self.password.is_empty() && self.password.chars().count() < config::MIN_PASSWORD {
            return Err(format!(
                "the page's password needs at least {} characters",
                config::MIN_PASSWORD
            ));
        }
        config.api.password.clone_from(&self.password);
        let problems = config.problems();
        if problems.is_empty() {
            Ok(config)
        } else {
            Err(problems.join("; "))
        }
    }

    /// Adds an empty projector with a key no other has, and opens it.
    pub fn add(&mut self) {
        let key = (1..)
            .map(|n| format!("projector{n}"))
            .find(|key| !self.units.iter().any(|unit| unit.key == *key))
            .unwrap_or_default();
        self.units.push(DraftUnit {
            key,
            projector: Projector::new("", ""),
        });
        self.open = Some(self.units.len() - 1);
    }

    /// Removes the projector at `index`.
    pub fn remove(&mut self, index: usize) {
        if index < self.units.len() {
            self.units.remove(index);
            self.open = None;
        }
    }
}

/// One projector as Settings shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitView {
    /// Its key.
    pub key: String,
    /// Its name.
    pub name: String,
    /// Its address.
    pub ip: String,
    /// The last status read, if any.
    pub status: Option<Status>,
    /// A command is running on it.
    pub busy: bool,
}

/// What Settings > Projectors shows, made once per control-window frame.
#[derive(Clone, Debug, Default)]
pub struct ProjectorView {
    /// Projector control is on.
    pub enabled: bool,
    /// The web page is wanted.
    pub serve_web: bool,
    /// The web page is being served from here.
    pub listening: bool,
    /// The page's address while served.
    pub url: Option<String>,
    /// What the page's service is doing, in words.
    pub web_state: String,
    /// The saved projectors.
    pub units: Vec<UnitView>,
    /// The last thing that happened, and whether it was a failure.
    pub message: Option<(String, bool)>,
    /// Why the file could not be read, if it could not.
    pub load_error: Option<String>,
    /// The form differs from what is saved.
    pub dirty: bool,
    /// A status read is running.
    pub polling: bool,
    /// Where the file is.
    pub path: String,
}

/// A worker's result.
enum Outcome {
    /// Every unit's status.
    Status(Vec<Status>),
    /// A power command's results, per unit: key, name, result.
    Power {
        on: bool,
        results: Vec<(String, String, Result<PowerResult, String>)>,
    },
}

/// Projector control in the app.
pub struct Projectors {
    path: PathBuf,
    config: Arc<RwLock<Config>>,
    /// The form Settings edits.
    pub form: Form,
    load_error: Option<String>,
    service: Option<Service>,
    audit: Audit,
    timing: Timing,
    sender: Sender<Outcome>,
    receiver: Receiver<Outcome>,
    statuses: Vec<Status>,
    busy: BTreeSet<String>,
    polling: bool,
    last_poll: Option<Instant>,
    message: Option<(String, bool)>,
}

impl std::fmt::Debug for Projectors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Projectors")
            .field("path", &self.path)
            .field("serving", &self.service.is_some())
            .finish_non_exhaustive()
    }
}

impl Projectors {
    /// Reads `projectors.json` from `data_dir`, and starts the web page if
    /// it was left on (and `start_web`: never in a screenshot run).
    pub fn new(data_dir: &std::path::Path, start_web: bool) -> Self {
        let path = data_dir.join(config::FILE_NAME);
        let (config, load_error) = match Config::read_or_default(&path) {
            Ok(config) => (config, None),
            Err(error) => {
                log::warn!("projectors: {error}");
                (Config::default(), Some(error.to_string()))
            }
        };
        let (sender, receiver) = mpsc::channel();
        let mut projectors = Self {
            audit: Audit::beside(&path),
            path,
            form: Form::from_config(&config),
            config: Arc::new(RwLock::new(config)),
            load_error,
            service: None,
            timing: Timing::default(),
            sender,
            receiver,
            statuses: Vec::new(),
            busy: BTreeSet::new(),
            polling: false,
            last_poll: None,
            message: None,
        };
        let switches = projectors.saved().strobe;
        if start_web && switches.enabled && switches.serve_web {
            projectors.start_service();
        }
        projectors
    }

    /// The saved configuration.
    fn saved(&self) -> Config {
        self.config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the saved configuration and writes it.
    fn store(&mut self, config: Config) -> bool {
        let written = config.save(&self.path);
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = config;
        match written {
            Ok(()) => {
                self.load_error = None;
                true
            }
            Err(error) => {
                self.say(
                    format!("Could not save {}: {error}", self.path.display()),
                    true,
                );
                false
            }
        }
    }

    fn say(&mut self, text: String, failure: bool) {
        if failure {
            log::warn!("projectors: {text}");
        }
        self.message = Some((text, failure));
    }

    /// Once a frame: takes finished work off the channel and starts the
    /// next status read when it is due. Never waits.
    pub fn maintain(&mut self, now: Instant) {
        while let Ok(outcome) = self.receiver.try_recv() {
            self.finished(outcome);
        }
        let due = self
            .last_poll
            .is_none_or(|last| now.duration_since(last) >= POLL);
        if self.saved().strobe.enabled && due && !self.polling {
            self.refresh_at(now);
        }
    }

    fn finished(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Status(statuses) => {
                self.polling = false;
                self.statuses = statuses;
            }
            Outcome::Power { on, results } => {
                let action = if on { "on" } else { "off" };
                let mut failed = false;
                let lines: Vec<String> = results
                    .into_iter()
                    .map(|(key, name, result)| {
                        self.busy.remove(&key);
                        match result {
                            Ok(result) if result.sent => format!("{name}: {action} sent"),
                            Ok(result) => format!("{name}: already {}", result.previous),
                            Err(error) => {
                                failed = true;
                                format!("{name}: FAILED - {error}")
                            }
                        }
                    })
                    .collect();
                self.say(lines.join("\n"), failed);
                // Read the new state now rather than at the next poll.
                self.last_poll = None;
            }
        }
    }

    /// Turns projector control on or off. Off stops the web page too.
    pub fn set_enabled(&mut self, on: bool) {
        let mut config = self.saved();
        config.strobe.enabled = on;
        self.store(config);
        if on {
            self.last_poll = None;
            if self.saved().strobe.serve_web {
                self.start_service();
            }
        } else {
            self.stop_service();
            self.statuses.clear();
        }
    }

    /// Serves the web page from Strobe, or stops. Without a password the
    /// switch stays on and the page waits for one, and says so (D516).
    pub fn set_web(&mut self, on: bool) {
        let mut config = self.saved();
        config.strobe.serve_web = on;
        if on && let Some(problem) = config.api_problem() {
            self.say(
                format!("Set the page's password under Connections and Save: {problem}"),
                true,
            );
        }
        self.store(config);
        if on && self.saved().strobe.enabled {
            self.start_service();
        } else {
            self.stop_service();
        }
    }

    fn start_service(&mut self) {
        if self.service.is_none() {
            let shared = Arc::new(Shared {
                config: Arc::clone(&self.config),
                audit: self.audit.clone(),
                timing: self.timing,
            });
            self.service = Some(Service::start(shared));
        }
    }

    fn stop_service(&mut self) {
        if let Some(mut service) = self.service.take() {
            service.stop();
        }
    }

    /// Checks and saves the form. A running page moves to a changed port
    /// or bind mode at once.
    pub fn save_form(&mut self) {
        let saved = self.saved();
        match self.form.apply(&saved) {
            Ok(config) => {
                let moved = config.api.port != saved.api.port || config.api.bind != saved.api.bind;
                if self.store(config) {
                    self.form = Form::from_config(&self.saved());
                    self.say(format!("Saved {}.", self.path.display()), false);
                    self.last_poll = None;
                    if moved && self.service.is_some() {
                        self.stop_service();
                        self.start_service();
                    }
                }
            }
            Err(problem) => self.say(format!("Not saved: {problem}"), true),
        }
    }

    /// Puts the form back to what is saved.
    pub fn revert_form(&mut self) {
        self.form = Form::from_config(&self.saved());
        self.message = None;
    }

    /// Reads every unit's status now, on a worker.
    pub fn refresh(&mut self) {
        self.refresh_at(Instant::now());
    }

    fn refresh_at(&mut self, now: Instant) {
        self.last_poll = Some(now);
        let config = self.saved();
        if config.projectors.is_empty() || self.polling {
            return;
        }
        let Ok(units) = unit::units_for(&config, "all") else {
            return;
        };
        let timing = self.timing;
        let sender = self.sender.clone();
        let spawned = thread::Builder::new()
            .name("projector-status".into())
            .spawn(move || {
                let units = units
                    .into_iter()
                    .map(|unit| unit.with_timing(timing))
                    .collect();
                let statuses = unit::run_on(units, |unit| Ok(unit.status()))
                    .into_iter()
                    .filter_map(|(_, status)| status.ok())
                    .collect();
                // The app may have closed while the units answered.
                let _ = sender.send(Outcome::Status(statuses));
            });
        match spawned {
            Ok(_) => self.polling = true,
            Err(error) => self.say(format!("Could not read status: {error}"), true),
        }
    }

    /// Switches `target` (`all` or a key) on or off, on a worker.
    pub fn power(&mut self, target: &str, on: bool) {
        self.form.confirm_off = None;
        let config = self.saved();
        let units = match unit::units_for(&config, target) {
            Ok(units) => units,
            Err(error) => {
                self.say(error.to_string(), true);
                return;
            }
        };
        if units.iter().any(|unit| self.busy.contains(unit.key())) {
            self.say(
                "A command is already running on that projector.".into(),
                true,
            );
            return;
        }
        let keys: Vec<String> = units.iter().map(|unit| unit.key().to_string()).collect();
        let timing = self.timing;
        let audit = self.audit.clone();
        let sender = self.sender.clone();
        let action = if on { "on" } else { "off" };
        let spawned = thread::Builder::new()
            .name("projector-power".into())
            .spawn(move || {
                let units = units
                    .into_iter()
                    .map(|unit| unit.with_timing(timing))
                    .collect();
                let results = unit::run_on(units, |unit| unit.set_power(on, false))
                    .into_iter()
                    .map(|(unit, result)| {
                        let line = match &result {
                            Ok(result) => {
                                format!("OK {}", serde_json::to_string(result).unwrap_or_default())
                            }
                            Err(error) => format!("FAIL {error}"),
                        };
                        audit.record(SOURCE, unit.key(), action, &line);
                        (
                            unit.key().to_string(),
                            unit.name().to_string(),
                            result.map_err(|error| error.to_string()),
                        )
                    })
                    .collect();
                // The app may have closed while the units answered.
                let _ = sender.send(Outcome::Power { on, results });
            });
        match spawned {
            Ok(_) => {
                self.busy.extend(keys);
                self.say(
                    format!(
                        "Sending {action} to {target}... (waits out warming or cooling, up to \
                         90 s)"
                    ),
                    false,
                );
            }
            Err(error) => self.say(format!("Could not send {action}: {error}"), true),
        }
    }

    /// What Settings shows this frame.
    pub fn view(&self) -> ProjectorView {
        let config = self.saved();
        let state = self.service.as_ref().map_or(State::Stopped, Service::state);
        let (listening, web_state) = match &state {
            State::Listening { kind, .. } => {
                (true, format!("Serving on the {} address", kind.name()))
            }
            State::Waiting(why) => (false, why.clone()),
            State::Stopped => (false, "Off".to_string()),
        };
        let url = state.url();
        let units = config
            .projectors
            .iter()
            .map(|(key, projector)| UnitView {
                key: key.clone(),
                name: projector.name.clone(),
                ip: projector.ip.clone(),
                status: self
                    .statuses
                    .iter()
                    .find(|status| status.unit == *key)
                    .cloned(),
                busy: self.busy.contains(key),
            })
            .collect();
        ProjectorView {
            enabled: config.strobe.enabled,
            serve_web: config.strobe.serve_web,
            listening,
            url,
            web_state,
            units,
            message: self.message.clone(),
            load_error: self.load_error.clone(),
            dirty: !self.form.matches(&config),
            polling: self.polling,
            path: self.path.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    #[test]
    fn the_form_round_trips_and_checks_what_it_saves() -> Result<()> {
        let mut config = Config::default();
        config
            .projectors
            .insert("sony".into(), Projector::new("Sony", "192.168.50.11"));
        let mut form = Form::from_config(&config);
        assert!(form.matches(&config), "fresh form");
        form.add();
        assert_eq!(form.open, Some(1), "the new one opens");
        assert!(!form.matches(&config), "dirty");
        assert!(
            form.apply(&config).is_err(),
            "an empty projector is refused"
        );
        let added = form.units.get_mut(1).ok_or_else(|| anyhow!("added"))?;
        added.projector.name = " Panasonic ".into();
        added.projector.ip = "192.168.50.10".into();
        let saved = form.apply(&config).map_err(|e| anyhow!(e))?;
        let panasonic = saved
            .projectors
            .get("projector1")
            .ok_or_else(|| anyhow!("projector1"))?;
        assert_eq!(panasonic.name, "Panasonic", "trimmed");
        form.port = "nope".into();
        assert!(form.apply(&config).is_err(), "a bad port");
        form.port = "8780".into();
        if let Some(unit) = form.units.get_mut(1) {
            unit.key = "sony".into();
        }
        assert!(form.apply(&config).is_err(), "a duplicate key");
        form.remove(1);
        assert!(form.matches(&config), "back to saved");
        Ok(())
    }

    #[test]
    fn switches_persist_and_the_page_needs_control_on() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("strobe-app-projectors-{}", std::process::id()));
        let mut projectors = Projectors::new(&dir, false);
        assert!(!projectors.view().enabled, "off by default");
        projectors.set_web(true);
        let view = projectors.view();
        assert!(view.serve_web, "wanted");
        assert!(!view.listening, "but control is off");
        assert!(
            projectors.service.is_none(),
            "no listener while control is off"
        );
        let saved = Config::read_or_default(&dir.join(config::FILE_NAME))?;
        assert!(
            saved.api_problem().is_some(),
            "no password is made up: the page waits for one (D516)"
        );
        assert!(
            projectors
                .view()
                .message
                .is_some_and(|(text, _)| text.contains("password")),
            "and says so"
        );
        assert!(saved.strobe.serve_web, "saved");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_page_password_is_saved_with_the_form_and_a_short_one_is_refused() -> Result<()> {
        let mut config = Config::default();
        config
            .projectors
            .insert("sony".into(), config::Projector::new("Sony", "192.0.2.1"));
        let mut form = Form::from_config(&config);
        form.password = "short".into();
        assert!(form.apply(&config).is_err(), "too short");
        form.password = "jazz-hands".into();
        let saved = form.apply(&config).map_err(|why| anyhow::anyhow!(why))?;
        assert_eq!(saved.api.password, "jazz-hands", "saved");
        assert!(saved.api_problem().is_none(), "the page may serve");
        assert!(!form.matches(&config), "a changed password is unsaved");
        assert!(form.matches(&saved), "and saved, it is not");
        Ok(())
    }
}
