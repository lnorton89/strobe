// ============================================================================
// Module:       strobe_app::perf_watch
// Description:  `strobe --watch-perf HOST:PORT --remote-trust FILE`: watch
//               another Strobe's live performance feed with no window,
//               printing a line a second and appending CSV rows.
//
// Dependencies: strobe-remote (client, perf, protocol); crate::trusted (the
//               trust-file proof, backoff and refusal texts)
// ============================================================================

//! Watching a show machine's frame timing from another computer (D567).
//!
//! The watcher pairs with the host the way a venue laptop does (D288,
//! D294): it proves the trust file, so nobody at the host clicks anything.
//! It then asks for the performance feed and nothing else. It never asks for
//! the desktop lease or the picture of the host's window, which cost the host
//! time of their own, so what it prints is the show as it runs unwatched,
//! give or take one small message a second.
//!
//! Each sample is a line on stdout in the words of the host's `perf:` log
//! line, and, with `--perf-csv FILE`, a row appended to that file (the header
//! is written when the file is empty), flushed at once so a soak's file is
//! whole up to the last second however the watcher ends. A dropped
//! connection is retried on the venue laptop's schedule (1 s, 2 s ... 15 s);
//! a refusal ends the watch and says why.
//!
//! The Windows release build has no console: run it with its output
//! redirected (`strobe --watch-perf ... > perf.txt`), or with `--perf-csv`.

use std::fs::File;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use strobe_remote::client::{Connection, host_header};
use strobe_remote::perf::PerfSample;
use strobe_remote::{ClientMessage, ErrorCode, ServerMessage};

use crate::trusted::{self, Handshake, Plan};

/// What to watch, and for how long.
#[derive(Debug)]
pub struct Watch {
    /// The host's address and port.
    pub host: SocketAddr,
    /// The trust file's secret and this computer's controller id.
    pub plan: Plan,
    /// Where to append CSV rows, if anywhere.
    pub csv: Option<PathBuf>,
    /// Stop after this long; `None` watches until stopped.
    pub limit: Option<Duration>,
}

/// How long the host may say nothing before the connection counts as lost.
/// It sends a sample a second and answers every ping.
const SILENCE: Duration = Duration::from_secs(15);

/// How often the watcher pings, so the host's idle timeout never fires.
const PING_EVERY: Duration = Duration::from_secs(5);

/// How the watch ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    /// The time limit passed, or it was stopped.
    Done {
        /// Samples received.
        samples: u64,
    },
    /// The host refused, and retrying will not change that: why, in a
    /// sentence that says what to change.
    Refused(String),
}

/// Why one connection ended.
enum Outcome {
    /// Retry later, after logging this.
    Lost(String),
    /// Stop: see [`Ended::Refused`].
    Refused(String),
    /// The time limit passed, or the watch was stopped.
    Done,
}

impl From<io::Error> for Outcome {
    fn from(error: io::Error) -> Self {
        Self::Lost(error.to_string())
    }
}

/// Where the samples go.
struct Sink<'a> {
    out: &'a mut dyn Write,
    csv: Option<File>,
    began: Instant,
    samples: u64,
    /// A printed line has failed once, which was logged.
    print_failed: bool,
}

impl Sink<'_> {
    /// Prints and records one sample. Only the CSV file's failure is an
    /// error: a printed line that goes nowhere (a closed pipe, no console)
    /// must not end a soak whose file is still being written.
    fn sample(&mut self, sample: &PerfSample) -> io::Result<()> {
        let seconds = self.began.elapsed().as_secs_f64();
        let printed = writeln!(self.out, "{seconds:8.1} s  {}", sample.line())
            .and_then(|()| self.out.flush());
        if let Err(error) = printed
            && !std::mem::replace(&mut self.print_failed, true)
        {
            log::warn!("watch-perf: printing a sample failed ({error}); still recording");
        }
        if let Some(csv) = self.csv.as_mut() {
            writeln!(csv, "{}", sample.csv_row(seconds))?;
            csv.flush()?;
        }
        self.samples += 1;
        Ok(())
    }

    fn note(&mut self, text: &str) {
        log::info!("watch-perf: {text}");
        // A note that cannot be printed is still in the log.
        let _ = writeln!(self.out, "{text}");
    }
}

/// Opens `path` for appending CSV rows, writing the header if it is empty.
fn open_csv(path: &std::path::Path) -> io::Result<File> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if file.metadata()?.len() == 0 {
        writeln!(file, "{}", PerfSample::CSV_HEADER)?;
    }
    Ok(file)
}

/// Watches until the limit, a stop, or a refusal, writing each sample's line
/// to `out`. A CSV file that cannot be opened is a refusal of its own.
pub fn run(watch: &Watch, out: &mut dyn Write, stop: &AtomicBool) -> Ended {
    let csv = match watch.csv.as_deref().map(open_csv).transpose() {
        Ok(csv) => csv,
        Err(error) => {
            let path = watch.csv.as_deref().unwrap_or(std::path::Path::new(""));
            return Ended::Refused(format!("Cannot write {}: {error}", path.display()));
        }
    };
    let began = Instant::now();
    let deadline = watch.limit.map(|limit| began + limit);
    let mut sink = Sink {
        out,
        csv,
        began,
        samples: 0,
        print_failed: false,
    };
    let mut attempt = 0_u32;
    loop {
        match session(watch, &mut sink, stop, deadline)
            .err()
            .unwrap_or(Outcome::Done)
        {
            Outcome::Done => {
                return Ended::Done {
                    samples: sink.samples,
                };
            }
            Outcome::Refused(why) => {
                log::warn!("watch-perf: refused: {why}");
                return Ended::Refused(why);
            }
            Outcome::Lost(why) => {
                attempt = attempt.saturating_add(1);
                let wait = trusted::backoff(attempt);
                sink.note(&format!(
                    "Connection to {} lost ({why}); trying again in {} s (attempt {attempt}).",
                    watch.host,
                    wait.as_secs()
                ));
                if !pause(stop, deadline, wait) {
                    return Ended::Done {
                        samples: sink.samples,
                    };
                }
            }
        }
    }
}

/// Waits `wait`, or less if stopped or past the deadline; `false` then.
fn pause(stop: &AtomicBool, deadline: Option<Instant>, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) || deadline.is_some_and(|at| Instant::now() >= at) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// One connection: prove the file, ask for the feed, print what comes.
/// `Ok` when the limit passed or the watch was stopped.
fn session(
    watch: &Watch,
    sink: &mut Sink<'_>,
    stop: &AtomicBool,
    deadline: Option<Instant>,
) -> Result<(), Outcome> {
    let mut handshake = Handshake::new(watch.plan.clone(), host_header(watch.host));
    let mut connection = Connection::connect(watch.host)?;
    connection.send(&ClientMessage::Hello {
        protocol_version: strobe_remote::PROTOCOL_VERSION,
    })?;
    let mut authenticated = false;
    let mut seen = false;
    let mut heard = Instant::now();
    let mut next_ping = Instant::now() + PING_EVERY;
    loop {
        let now = Instant::now();
        if stop.load(Ordering::Relaxed) || deadline.is_some_and(|at| now >= at) {
            // Say goodbye; the host forgets a trusted pairing as it closes.
            let _ = connection.send(&ClientMessage::PerfClose {});
            return Ok(());
        }
        if now.duration_since(heard) > SILENCE {
            return Err(Outcome::Lost("the host stopped answering".into()));
        }
        if now >= next_ping {
            connection.send(&ClientMessage::Ping {})?;
            next_ping = now + PING_EVERY;
        }
        let Some(message) = connection.receive()? else {
            continue;
        };
        heard = Instant::now();
        match message {
            ServerMessage::Hello {
                protocol_version, ..
            } => {
                if strobe_remote::negotiate(protocol_version).is_err() {
                    return Err(Outcome::Refused(trusted::refusal_text(
                        ErrorCode::VersionMismatch,
                        &format!("it speaks protocol version {protocol_version}"),
                    )));
                }
                if let Some(start) = handshake.start() {
                    connection.send(&start)?;
                }
            }
            ServerMessage::TrustChallenge { nonce } => match handshake.answer(&nonce) {
                Ok(proof) => connection.send(&proof)?,
                Err(why) => return Err(Outcome::Refused(format!("Not answered: {why}."))),
            },
            ServerMessage::Authenticated { .. } => {
                authenticated = true;
                sink.note(&format!(
                    "Watching {}: one line a second (Ctrl+C to stop).",
                    watch.host
                ));
                connection.send(&ClientMessage::PerfOpen {})?;
            }
            ServerMessage::Perf { sample, .. } => {
                seen = true;
                sink.sample(&sample)
                    .map_err(|error| Outcome::Refused(format!("Cannot write a sample: {error}")))?;
            }
            ServerMessage::Error { code, message } => {
                // An older host could not decode `perf_open`; any other
                // `invalid_request` is a drop like the rest.
                if authenticated
                    && code == ErrorCode::InvalidRequest
                    && !seen
                    && message == strobe_remote::NOT_UNDERSTOOD
                {
                    return Err(Outcome::Refused(
                        "The host has no performance feed: it runs a Strobe from before D567. \
                         Update Strobe on the host."
                            .into(),
                    ));
                }
                if !authenticated && !trusted::is_transient(code) {
                    return Err(Outcome::Refused(trusted::refusal_text(code, &message)));
                }
                return Err(Outcome::Lost(message));
            }
            // A watcher asks for nothing else; the state, pongs and any
            // pairing message are not its business.
            ServerMessage::State { .. }
            | ServerMessage::Pong {}
            | ServerMessage::CommandResult { .. }
            | ServerMessage::Paired { .. }
            | ServerMessage::DesktopFrame { .. }
            | ServerMessage::DesktopClipboard { .. }
            | ServerMessage::DesktopControls { .. }
            | ServerMessage::DesktopLayout { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};
    use std::net::IpAddr;
    use strobe_remote::bridge::{self, AppEnd};
    use strobe_remote::clients::Clients;
    use strobe_remote::server::{Limits, Server, ServerConfig};
    use strobe_remote::trust::{self, TrustSecret};

    /// A folder of its own under the temp dir, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Result<Self> {
            let dir = std::env::temp_dir()
                .join(format!("strobe-perf-watch-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl std::ops::Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A host with full desktop access on and `secret` as its trust file.
    fn host(secret: &TrustSecret) -> Result<(Server, AppEnd)> {
        let (remote, app) = bridge::bridge(4);
        app.desktop.enable(true);
        let mut config = ServerConfig::new(IpAddr::from([127, 0, 0, 1]), 0);
        config.limits = Limits {
            tick: Duration::from_millis(5),
            ..Limits::default()
        };
        config.trust = Some(secret.clone());
        let server = Server::start(config, remote, Clients::in_memory())
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok((server, app))
    }

    /// Publishes a sample every 20 ms until stopped.
    fn publisher(app: AppEnd, stop: std::sync::Arc<AtomicBool>) -> std::thread::JoinHandle<AppEnd> {
        std::thread::spawn(move || {
            let mut frames = 0;
            while !stop.load(Ordering::Relaxed) {
                frames += 1;
                app.publish_perf(PerfSample {
                    frames,
                    fps: 59.9,
                    budget_ms: 1000.0 / 60.0,
                    ..PerfSample::default()
                });
                std::thread::sleep(Duration::from_millis(20));
            }
            app
        })
    }

    #[test]
    fn a_watch_proves_the_file_prints_samples_and_writes_whole_csv_rows() -> Result<()> {
        let scratch = Scratch::new("ok")?;
        let file = scratch.0.join("venue.trust");
        let secret = trust::create_file(&file)?;
        let (server, app) = host(&secret)?;
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let publishing = publisher(app, std::sync::Arc::clone(&stop));
        let csv = scratch.0.join("perf.csv");
        let watch = Watch {
            host: server.addr(),
            plan: trusted::load_plan(&file.to_string_lossy(), "watch-1a2b3c4d")
                .map_err(|why| anyhow::anyhow!(why))?,
            csv: Some(csv.clone()),
            limit: Some(Duration::from_millis(800)),
        };
        let mut out = Vec::new();
        let ended = run(&watch, &mut out, &AtomicBool::new(false));
        stop.store(true, Ordering::Relaxed);
        let app = publishing
            .join()
            .map_err(|_| anyhow::anyhow!("the publisher panicked"))?;
        let Ended::Done { samples } = ended else {
            anyhow::bail!("the watch was refused: {ended:?}");
        };
        assert!(samples >= 2, "samples arrived: {samples}");
        let printed = String::from_utf8(out)?;
        assert!(printed.contains("Watching "), "{printed}");
        assert!(printed.contains("59.9 fps"), "{printed}");
        let text = std::fs::read_to_string(&csv)?;
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(PerfSample::CSV_HEADER),
            "the header first"
        );
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len() as u64, samples, "a row per sample");
        let columns = PerfSample::CSV_HEADER.split(',').count();
        for row in &rows {
            assert_eq!(row.split(',').count(), columns, "a whole row: {row}");
        }
        assert!(
            server.pairing_requests().is_empty(),
            "nobody was asked to approve"
        );
        assert!(!app.desktop.active(), "the lease was never taken");
        server.stop();
        Ok(())
    }

    /// Output that always fails, like a pipe whose reader has gone.
    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn a_closed_stdout_does_not_stop_the_csv() -> Result<()> {
        let scratch = Scratch::new("closed")?;
        let file = scratch.0.join("venue.trust");
        let secret = trust::create_file(&file)?;
        let (server, app) = host(&secret)?;
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let publishing = publisher(app, std::sync::Arc::clone(&stop));
        let csv = scratch.0.join("perf.csv");
        let watch = Watch {
            host: server.addr(),
            plan: trusted::load_plan(&file.to_string_lossy(), "watch-1a2b3c4d")
                .map_err(|why| anyhow::anyhow!(why))?,
            csv: Some(csv.clone()),
            limit: Some(Duration::from_millis(800)),
        };
        let ended = run(&watch, &mut Closed, &AtomicBool::new(false));
        stop.store(true, Ordering::Relaxed);
        let _ = publishing.join();
        let Ended::Done { samples } = ended else {
            anyhow::bail!("the watch ended early: {ended:?}");
        };
        assert!(samples >= 2, "it kept recording: {samples}");
        let rows = std::fs::read_to_string(&csv)?.lines().count() as u64;
        assert_eq!(rows, samples + 1, "a row per sample after the header");
        server.stop();
        Ok(())
    }

    #[test]
    fn the_wrong_trust_file_ends_the_watch_and_says_why() -> Result<()> {
        let scratch = Scratch::new("wrong")?;
        let secret = trust::create_file(&scratch.0.join("host.trust"))?;
        let other = scratch.0.join("other.trust");
        trust::create_file(&other)?;
        let (server, _app) = host(&secret)?;
        let watch = Watch {
            host: server.addr(),
            plan: trusted::load_plan(&other.to_string_lossy(), "watch-1a2b3c4d")
                .map_err(|why| anyhow::anyhow!(why))?,
            csv: None,
            limit: Some(Duration::from_secs(10)),
        };
        let mut out = Vec::new();
        let ended = run(&watch, &mut out, &AtomicBool::new(false));
        let Ended::Refused(why) = ended else {
            anyhow::bail!("not refused: {ended:?}");
        };
        assert!(why.contains("trust file"), "{why}");
        server.stop();
        Ok(())
    }

    #[test]
    fn a_csv_that_cannot_be_written_is_said_before_connecting() -> Result<()> {
        let scratch = Scratch::new("csv")?;
        let file = scratch.0.join("venue.trust");
        trust::create_file(&file)?;
        let watch = Watch {
            host: "127.0.0.1:9".parse()?,
            plan: trusted::load_plan(&file.to_string_lossy(), "watch-1a2b3c4d")
                .map_err(|why| anyhow::anyhow!(why))?,
            // A folder, not a file.
            csv: Some(scratch.0.clone()),
            limit: Some(Duration::from_secs(1)),
        };
        let ended = run(&watch, &mut Vec::new(), &AtomicBool::new(false));
        let Ended::Refused(why) = ended else {
            anyhow::bail!("not refused: {ended:?}");
        };
        assert!(why.starts_with("Cannot write"), "{why}");
        Ok(())
    }

    #[test]
    fn an_appended_csv_keeps_one_header() -> Result<()> {
        let scratch = Scratch::new("append")?;
        let path = scratch.0.join("perf.csv");
        drop(open_csv(&path)?);
        drop(open_csv(&path)?);
        let text = std::fs::read_to_string(&path)?;
        assert_eq!(text.lines().count(), 1, "one header: {text}");
        let first = text.lines().next().context("a line")?;
        assert_eq!(first, PerfSample::CSV_HEADER, "the header");
        Ok(())
    }
}
