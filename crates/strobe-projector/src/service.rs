// ============================================================================
// Module:       strobe_projector::service
// Description:  Keeps the web page listening on the right address: waits for
//               one, binds it, checks it every 30 s, and moves when the
//               address, port or bind mode changes. Strobe and the headless
//               binary both run it.
//
// Dependencies: std::thread, std::fs; crate::{bind, config, server}
// ============================================================================

//! The supervisor (the Python tool's `server.py` main loop, D502).
//!
//! [`Service::start`] runs a thread that:
//!
//! 1. refuses to serve without a usable password (`api.password`, at least
//!    [`crate::config::MIN_PASSWORD`] characters, D516), and says so;
//! 2. asks [`bind::resolve`] for the address, and waits [`RETRY`] while
//!    there is none or it is not [`bind::safe`];
//! 3. binds it, or waits [`RETRY`] when that fails, which is what happens
//!    when another process (the headless service, or Strobe) already holds
//!    the port: one configuration, and whoever binds first serves it;
//! 4. checks every [`CHECK`] that the address, port and bind mode are
//!    still what it bound, and rebinds when they are not (a DHCP renewal,
//!    Tailscale coming up, a change in Settings).
//!
//! [`Service::state`] is what Settings shows.
//!
//! The headless service also [`follow_file`]s its config: Strobe's Settings
//! saves the same file, and a new password or port reaches the
//! running service within [`RELOAD`], as long as the file still loads.

use crate::bind::{self, Kind};
use crate::config::Config;
use crate::server::{Server, Shared};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;
use std::time::{Duration, Instant};

/// How long to wait before trying again when there is no address, or the
/// bind failed.
pub const RETRY: Duration = Duration::from_secs(15);

/// How often a listening service checks its address.
pub const CHECK: Duration = Duration::from_secs(30);

/// How often a followed config file is checked for a change.
pub const RELOAD: Duration = Duration::from_secs(10);

/// What the service is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Not started, or stopped.
    Stopped,
    /// Waiting to bind, and why.
    Waiting(String),
    /// Listening.
    Listening {
        /// The address and port.
        address: SocketAddr,
        /// Where the address came from.
        kind: Kind,
    },
}

impl State {
    /// The page's address, while listening.
    pub fn url(&self) -> Option<String> {
        match self {
            Self::Listening { address, .. } => Some(format!("http://{address}/")),
            Self::Stopped | Self::Waiting(_) => None,
        }
    }
}

/// Finds the address to bind; [`bind::resolve`] outside tests.
pub type Resolver = fn(&Config) -> (Option<Ipv4Addr>, Kind);

/// The supervisor thread and what it reports.
#[derive(Debug)]
pub struct Service {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Service {
    /// Starts supervising the page for `shared`'s configuration.
    pub fn start(shared: Arc<Shared>) -> Self {
        Self::start_with(shared, bind::resolve)
    }

    /// [`Service::start`] with another address lookup (tests).
    pub fn start_with(shared: Arc<Shared>, resolve: Resolver) -> Self {
        let state = Arc::new(Mutex::new(State::Waiting("starting".into())));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let state = Arc::clone(&state);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("projector-service".into())
                .spawn(move || supervise(&shared, resolve, &state, &stop))
        };
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                set(
                    &state,
                    State::Waiting(format!("cannot start a thread: {error}")),
                );
                None
            }
        };
        Self {
            state,
            stop,
            thread,
        }
    }

    /// What it is doing now.
    pub fn state(&self) -> State {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Stops listening and waits for the supervisor to end.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            // A join error would mean the thread panicked, and the release
            // build aborts first; there is nothing left to stop either way.
            thread.join().ok();
        }
        set(&self.state, State::Stopped);
    }

    /// Runs until the process ends (the headless binary).
    pub fn run_forever(mut self) {
        if let Some(thread) = self.thread.take() {
            // The supervisor only returns when stopped, which nothing does
            // here; a panic would already have aborted the process.
            thread.join().ok();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set(state: &Mutex<State>, new: State) {
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    if *state != new {
        match &new {
            State::Listening { address, kind } => {
                log::info!(
                    "projector page: listening on http://{address}/ ({})",
                    kind.name()
                );
            }
            State::Waiting(why) => log::warn!("projector page: {why}"),
            State::Stopped => log::info!("projector page: stopped"),
        }
    }
    *state = new;
}

/// Sleeps `duration` or until stopped; returns whether stopped.
fn pause(stop: &AtomicBool, duration: Duration) -> bool {
    let end = Instant::now() + duration;
    while !stop.load(Ordering::Acquire) {
        let Some(left) = end
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
        else {
            return false;
        };
        thread::park_timeout(left);
    }
    true
}

/// What a listening service watches for change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bound {
    ip: Option<Ipv4Addr>,
    port: u16,
    mode: crate::config::Bind,
}

fn supervise(shared: &Arc<Shared>, resolve: Resolver, state: &Mutex<State>, stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        let config = shared.config();
        if let Some(problem) = config.api_problem() {
            set(state, State::Waiting(format!("{problem}; not serving")));
            if pause(stop, RETRY) {
                break;
            }
            continue;
        }
        let (ip, kind) = resolve(&config);
        let Some(ip) = ip.filter(|ip| bind::safe(*ip, &config)) else {
            let shown = ip.map_or_else(|| "none".to_string(), |ip| ip.to_string());
            set(
                state,
                State::Waiting(format!(
                    "no usable {} address yet ({shown}); retrying in {}s",
                    kind.name(),
                    RETRY.as_secs()
                )),
            );
            if pause(stop, RETRY) {
                break;
            }
            continue;
        };
        let address = SocketAddr::from((ip, config.api.port));
        let mut server = match Server::start(address, Arc::clone(shared)) {
            Ok(server) => server,
            Err(error) => {
                let why = if error.kind() == std::io::ErrorKind::AddrInUse {
                    format!(
                        "{address} is in use (the projector service or another Strobe is \
                         serving the page)"
                    )
                } else {
                    format!("bind {address} failed: {error}")
                };
                set(
                    state,
                    State::Waiting(format!("{why}; retrying in {}s", RETRY.as_secs())),
                );
                if pause(stop, RETRY) {
                    break;
                }
                continue;
            }
        };
        set(state, State::Listening { address, kind });
        let bound = Bound {
            ip: Some(ip),
            port: config.api.port,
            mode: config.api.bind,
        };
        loop {
            if pause(stop, CHECK) {
                server.stop();
                return;
            }
            let now = shared.config();
            let current = Bound {
                ip: resolve(&now).0,
                port: now.api.port,
                mode: now.api.bind,
            };
            if current != bound || now.api_problem().is_some() {
                log::warn!("projector page: {bound:?} changed to {current:?}; rebinding");
                server.stop();
                break;
            }
        }
    }
    set(state, State::Stopped);
}

/// When `path` was last written, if it can be read.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

/// Loads `path` into `config` if it changed since `seen`, and moves `seen`
/// on. A file that no longer loads is logged and left out: the service
/// keeps the configuration it has. Returns whether `config` changed.
pub fn reload_if_changed(
    path: &Path,
    seen: &mut Option<SystemTime>,
    config: &RwLock<Config>,
) -> bool {
    let now = modified(path);
    if now == *seen {
        return false;
    }
    *seen = now;
    match Config::load(path) {
        Ok(loaded) => {
            let mut current = config.write().unwrap_or_else(PoisonError::into_inner);
            if *current == loaded {
                return false;
            }
            *current = loaded;
            log::info!("projector config: reloaded {}", path.display());
            true
        }
        Err(error) => {
            log::warn!("projector config: {error}; keeping the one loaded");
            false
        }
    }
}

/// Follows `path` on a thread of its own for the life of the process,
/// reloading `config` within [`RELOAD`] of a change.
pub fn follow_file(path: &Path, config: Arc<RwLock<Config>>) {
    let path: PathBuf = path.to_path_buf();
    let spawned = thread::Builder::new()
        .name("projector-config".into())
        .spawn(move || {
            let mut seen = modified(&path);
            loop {
                thread::sleep(RELOAD);
                reload_if_changed(&path, &mut seen, &config);
            }
        });
    if let Err(error) = spawned {
        log::warn!("projector config: not followed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::Audit;
    use crate::unit::Timing;
    use anyhow::Result;

    fn shared(password: &str) -> Arc<Shared> {
        let mut config = Config::default();
        config.api.password = password.into();
        Arc::new(Shared {
            config: Arc::new(RwLock::new(config)),
            audit: Audit::none(),
            timing: Timing::default(),
        })
    }

    fn wait_for(service: &Service, done: impl Fn(&State) -> bool) -> State {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let state = service.state();
            if done(&state) || Instant::now() > end {
                return state;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_changed_file_is_reloaded_and_a_broken_one_is_not() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("strobe-projector-follow-{}", std::process::id()));
        let path = dir.join(crate::config::FILE_NAME);
        let mut first = Config::default();
        first.projectors.insert(
            "sony".into(),
            crate::config::Projector::new("Sony", "192.168.50.11"),
        );
        first.save(&path)?;
        let config = RwLock::new(first.clone());
        let mut seen = None;
        assert!(
            !reload_if_changed(&path, &mut seen, &config),
            "the same contents"
        );
        let mut second = first.clone();
        second.api.password = "stage-left".into();
        second.save(&path)?;
        seen = None;
        assert!(
            reload_if_changed(&path, &mut seen, &config),
            "a new password"
        );
        let password = config
            .read()
            .map_err(|_| anyhow::anyhow!("lock"))?
            .api
            .password
            .clone();
        assert_eq!(password, second.api.password, "reloaded");
        std::fs::write(&path, "{ broken")?;
        seen = None;
        assert!(
            !reload_if_changed(&path, &mut seen, &config),
            "broken file ignored"
        );
        let password = config
            .read()
            .map_err(|_| anyhow::anyhow!("lock"))?
            .api
            .password
            .clone();
        assert_eq!(password, second.api.password, "kept");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn no_password_no_page() -> Result<()> {
        let mut service = Service::start_with(shared("short"), |_| (None, Kind::Wifi));
        let state = wait_for(
            &service,
            |state| matches!(state, State::Waiting(why) if why.contains("password")),
        );
        assert!(
            matches!(&state, State::Waiting(why) if why.contains("api.password")),
            "{state:?}"
        );
        service.stop();
        assert_eq!(service.state(), State::Stopped, "stopped");
        Ok(())
    }

    #[test]
    fn loopback_and_no_address_are_never_bound() -> Result<()> {
        let password = "stage-left";
        let service = Service::start_with(shared(password), |_| {
            (Some(Ipv4Addr::LOCALHOST), Kind::Wifi)
        });
        let state = wait_for(
            &service,
            |state| matches!(state, State::Waiting(why) if why.contains("127.0.0.1")),
        );
        assert!(
            matches!(&state, State::Waiting(why) if why.contains("no usable wifi address")),
            "{state:?}"
        );
        drop(service);
        let service = Service::start_with(shared(password), |_| (None, Kind::Tailscale));
        let state = wait_for(
            &service,
            |state| matches!(state, State::Waiting(why) if why.contains("tailscale")),
        );
        assert!(
            matches!(&state, State::Waiting(why) if why.contains("no usable tailscale address yet (none)")),
            "{state:?}"
        );
        assert_eq!(state.url(), None, "no url while waiting");
        Ok(())
    }
}
