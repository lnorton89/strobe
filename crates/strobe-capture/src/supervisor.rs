// ============================================================================
// Module:       strobe_capture::supervisor
// Description:  The capture threads: a reader that lists, opens, reads and
//               converts, reopening by name after a failure, and a watchdog
//               that leaves a reader stuck in a call behind and starts another.
//
// Dependencies: crate::backend, crate::cause, crate::convert; std::thread
// ============================================================================

//! The capture threads (M-04 proposal, steps 1 and 5).
//!
//! A **reader** thread loops: is the camera listed? open it, then read,
//! convert and hand over pictures until a read fails; wait
//! [`Timing::retry`] and start again. It is the only thread that touches
//! the open camera, which Media Foundation's objects need.
//!
//! A read has no timeout, so a **watchdog** thread (the supervisor) looks
//! every [`Timing::tick`] at how long the reader has been inside a backend
//! call. Past [`Timing::stall`] it says the camera stopped sending, marks
//! that reader abandoned (if its call ever returns it exits without
//! touching the sink) and starts a fresh one. At most [`MAX_ABANDONED`]
//! readers are left behind at once; past that the watchdog waits for one
//! to come back before starting another, so a driver that hangs every
//! open cannot pile up threads.
//!
//! A failure is said once per outage, through [`Sink::failed`]: the first
//! failure after a picture (or after start), not every retry. The next
//! picture says it is over. Both threads stop once [`Sink::wanted`] is
//! false; nothing ever joins them.

use crate::Picture;
use crate::Sink;
use crate::backend::{Backend, Problem};
use crate::cause::{Cause, Failure};
use crate::convert::to_rgba;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Readers left behind in a call at once, at most.
pub(crate) const MAX_ABANDONED: usize = 3;

/// How often a streaming camera logs its conversion cost.
const COST_EVERY: Duration = Duration::from_secs(60);

/// The conversion cost since the last log line, for the camera checklist's
/// capture-thread CPU (no other measurement runs in the app).
struct Cost {
    since: Instant,
    pictures: u32,
    total: Duration,
    max: Duration,
}

impl Cost {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            pictures: 0,
            total: Duration::ZERO,
            max: Duration::ZERO,
        }
    }

    fn add(&mut self, name: &str, took: Duration) {
        self.pictures += 1;
        self.total += took;
        self.max = self.max.max(took);
        if self.since.elapsed() >= COST_EVERY {
            let mean = self.total.as_secs_f64() * 1000.0 / f64::from(self.pictures);
            log::info!(
                "camera \"{name}\": {} pictures in {:.0} s, converting {mean:.2} ms each (max {:.2} ms)",
                self.pictures,
                self.since.elapsed().as_secs_f64(),
                self.max.as_secs_f64() * 1000.0
            );
            *self = Self::new();
        }
    }
}

/// How the capture threads pace themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How often the watchdog looks, and how soon a camera no one wants
    /// closes.
    pub tick: Duration,
    /// The wait between attempts to open a camera that failed.
    pub retry: Duration,
    /// How long one backend call may take before the camera counts as
    /// stopped. A camera's first picture can take a second or two.
    pub stall: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            tick: Duration::from_millis(100),
            retry: Duration::from_secs(1),
            stall: Duration::from_secs(5),
        }
    }
}

/// Says failures once per outage.
struct Reporter {
    camera: String,
    sink: Arc<dyn Sink>,
    down: AtomicBool,
}

impl Reporter {
    fn fail(&self, problem: &Problem) {
        let failure = Failure {
            camera: self.camera.clone(),
            cause: problem.cause,
            detail: problem.detail.clone(),
        };
        if self.down.swap(true, Ordering::AcqRel) {
            log::debug!("{failure} (still)");
        } else {
            log::warn!("{failure}");
            self.sink.failed(&failure);
        }
    }

    fn up(&self) {
        if self.down.swap(false, Ordering::AcqRel) {
            log::info!("camera \"{}\" is back", self.camera);
            self.sink.recovered(&self.camera);
        }
    }

    fn wanted(&self) -> bool {
        self.sink.wanted()
    }
}

/// What one reader shares with the watchdog.
#[derive(Default)]
struct Reader {
    /// Set by the watchdog: exit at the next chance, touch nothing.
    abandoned: AtomicBool,
    /// Set by the reader as it exits.
    finished: AtomicBool,
    /// When the backend call in progress began, if one is.
    busy_since: Mutex<Option<Instant>>,
}

impl Reader {
    fn set_busy(&self, since: Option<Instant>) {
        *self
            .busy_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = since;
    }

    fn busy_since(&self) -> Option<Instant> {
        *self
            .busy_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn abandoned(&self) -> bool {
        self.abandoned.load(Ordering::Acquire)
    }

    fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

/// What every thread of one camera needs.
#[derive(Clone)]
struct Camera {
    backend: Arc<dyn Backend>,
    timing: Timing,
    reporter: Arc<Reporter>,
}

/// Starts the watchdog thread, which starts the first reader.
pub(crate) fn spawn(backend: Arc<dyn Backend>, timing: Timing, name: String, sink: Arc<dyn Sink>) {
    let camera = Camera {
        backend,
        timing,
        reporter: Arc::new(Reporter {
            camera: name,
            sink,
            down: AtomicBool::new(false),
        }),
    };
    let watched = camera.clone();
    let spawned = std::thread::Builder::new()
        .name("camera".to_string())
        .spawn(move || watch(&watched));
    if let Err(error) = spawned {
        camera.reporter.fail(&Problem {
            cause: Cause::Other,
            detail: format!("the camera thread did not start: {error}"),
        });
    }
}

fn watch(camera: &Camera) {
    log::info!("camera \"{}\": starting", camera.reporter.camera);
    let mut current = start_reader(camera);
    let mut left_behind: Vec<Arc<Reader>> = Vec::new();
    loop {
        std::thread::sleep(camera.timing.tick);
        left_behind.retain(|reader| !reader.finished());
        if !camera.reporter.wanted() {
            if let Some(reader) = &current {
                reader.abandoned.store(true, Ordering::Release);
            }
            log::info!(
                "camera \"{}\": closed, no one shows it",
                camera.reporter.camera
            );
            return;
        }
        let Some(reader) = current.take() else {
            if left_behind.len() < MAX_ABANDONED {
                current = start_reader(camera);
            }
            continue;
        };
        let stalled = reader
            .busy_since()
            .is_some_and(|since| since.elapsed() >= camera.timing.stall);
        if stalled {
            camera.reporter.fail(&Problem {
                cause: Cause::Stalled,
                detail: format!("no picture for {:.1} s", camera.timing.stall.as_secs_f32()),
            });
            reader.abandoned.store(true, Ordering::Release);
            left_behind.push(reader);
            if left_behind.len() < MAX_ABANDONED {
                current = start_reader(camera);
            }
        } else if reader.finished() {
            // A reader only finishes when told to; start over if it did
            // anyway.
            current = start_reader(camera);
        } else {
            current = Some(reader);
        }
    }
}

fn start_reader(camera: &Camera) -> Option<Arc<Reader>> {
    let reader = Arc::new(Reader::default());
    let (mine, theirs) = (reader.clone(), camera.clone());
    let spawned = std::thread::Builder::new()
        .name("camera-read".to_string())
        .spawn(move || {
            read(&theirs, &mine);
            mine.finished.store(true, Ordering::Release);
        });
    match spawned {
        Ok(_) => Some(reader),
        Err(error) => {
            camera.reporter.fail(&Problem {
                cause: Cause::Other,
                detail: format!("the camera reader did not start: {error}"),
            });
            None
        }
    }
}

fn read(camera: &Camera, reader: &Reader) {
    let name = camera.reporter.camera.as_str();
    let stop = || reader.abandoned() || !camera.reporter.wanted();
    // A buffer kept for the next picture when one could not be converted.
    let mut spare: Option<Vec<u8>> = None;
    while !stop() {
        reader.set_busy(Some(Instant::now()));
        let opened = open(camera.backend.as_ref(), name);
        reader.set_busy(None);
        if stop() {
            return;
        }
        let mut stream = match opened {
            Ok(stream) => stream,
            Err(problem) => {
                camera.reporter.fail(&problem);
                if !pause(camera.timing, &stop) {
                    return;
                }
                continue;
            }
        };
        log::info!("camera \"{name}\": open, {}", stream.describe());
        let mut cost = Cost::new();
        loop {
            reader.set_busy(Some(Instant::now()));
            let frame = stream.read();
            reader.set_busy(None);
            if stop() {
                return;
            }
            let frame = match frame {
                Ok(frame) => frame,
                Err(problem) => {
                    camera.reporter.fail(&problem);
                    break;
                }
            };
            let captured = Instant::now();
            let mut rgba = spare
                .take()
                .unwrap_or_else(|| camera.reporter.sink.buffer());
            match to_rgba(&frame, &mut rgba) {
                Ok(()) => {
                    cost.add(name, captured.elapsed());
                    camera.reporter.up();
                    camera.reporter.sink.put(Picture {
                        width: frame.width,
                        height: frame.height,
                        rgba,
                        captured: Some(captured),
                    });
                }
                Err(error) => {
                    spare = Some(rgba);
                    camera.reporter.fail(&Problem {
                        cause: Cause::Unreadable,
                        detail: format!("{} {error}", frame.format),
                    });
                }
            }
        }
        drop(stream);
        if !pause(camera.timing, &stop) {
            return;
        }
    }
}

/// Opens `name` if the backend lists it.
fn open(backend: &dyn Backend, name: &str) -> Result<Box<dyn crate::Stream>, Problem> {
    let listed = backend.list()?;
    if !listed.iter().any(|listed| listed == name) {
        return Err(Problem {
            cause: Cause::NotFound,
            detail: if listed.is_empty() {
                "no camera is listed".to_string()
            } else {
                format!("not listed; the cameras are {}", listed.join(", "))
            },
        });
    }
    backend.open(name)
}

/// Waits [`Timing::retry`] in ticks. `false` when told to stop meanwhile.
fn pause(timing: Timing, stop: &impl Fn() -> bool) -> bool {
    let until = Instant::now() + timing.retry;
    while Instant::now() < until {
        if stop() {
            return false;
        }
        std::thread::sleep(timing.tick.min(timing.retry));
    }
    !stop()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{RawFrame, Stream};
    use crate::format::PixelFormat;
    use anyhow::{Result, bail};
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    const CAMERA: &str = "Test Cam";

    fn quick() -> Timing {
        Timing {
            tick: Duration::from_millis(5),
            retry: Duration::from_millis(20),
            stall: Duration::from_millis(150),
        }
    }

    /// What a fake stream does on each read.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Behaviour {
        /// A 2x2 YUY2 picture every few milliseconds.
        Pictures,
        /// Pictures, then an error after this many.
        FailAfter(usize),
        /// Block until the test ends.
        Hang,
    }

    #[derive(Default)]
    struct State {
        listed: bool,
        /// One behaviour per open, in turn; the last repeats.
        opens: Vec<Behaviour>,
        opened: usize,
        streams_alive: usize,
    }

    #[derive(Default)]
    struct Fake {
        state: Arc<Mutex<State>>,
        /// The other ends of the streams' channels, kept so a hung read
        /// blocks until the process ends.
        keep: Mutex<Vec<mpsc::Sender<()>>>,
    }

    impl Fake {
        fn new(listed: bool, opens: &[Behaviour]) -> Arc<Self> {
            let fake = Self::default();
            if let Ok(mut state) = fake.state.lock() {
                state.listed = listed;
                state.opens = opens.to_vec();
            }
            Arc::new(fake)
        }

        fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
            f(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner))
        }
    }

    impl Backend for Fake {
        fn list(&self) -> Result<Vec<String>, Problem> {
            Ok(if self.with(|s| s.listed) {
                vec!["Other Cam".to_string(), CAMERA.to_string()]
            } else {
                vec!["Other Cam".to_string()]
            })
        }

        fn open(&self, _name: &str) -> Result<Box<dyn Stream>, Problem> {
            let behaviour = self.with(|s| {
                let b = s
                    .opens
                    .get(s.opened)
                    .or(s.opens.last())
                    .copied()
                    .unwrap_or(Behaviour::Pictures);
                s.opened += 1;
                s.streams_alive += 1;
                b
            });
            let (tx, rx) = mpsc::channel();
            self.keep
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(tx);
            Ok(Box::new(FakeStream {
                behaviour,
                reads: 0,
                data: [235, 128, 16, 128, 235, 128, 16, 128],
                state: self.state.clone(),
                block: rx,
            }))
        }
    }

    struct FakeStream {
        behaviour: Behaviour,
        reads: usize,
        data: [u8; 8],
        state: Arc<Mutex<State>>,
        block: mpsc::Receiver<()>,
    }

    impl Drop for FakeStream {
        fn drop(&mut self) {
            if let Ok(mut state) = self.state.lock() {
                state.streams_alive -= 1;
            }
        }
    }

    impl Stream for FakeStream {
        fn describe(&self) -> String {
            "2x2 YUY2 (fake)".to_string()
        }

        fn read(&mut self) -> Result<RawFrame<'_>, Problem> {
            self.reads += 1;
            match self.behaviour {
                Behaviour::Hang => {
                    let _ = self.block.recv();
                    return Err(Problem::from_text("ended"));
                }
                Behaviour::FailAfter(n) if self.reads > n => {
                    return Err(Problem::from_text("device invalidated"));
                }
                Behaviour::FailAfter(_) | Behaviour::Pictures => {}
            }
            std::thread::sleep(Duration::from_millis(2));
            Ok(RawFrame {
                format: PixelFormat::Yuy2,
                width: 2,
                height: 2,
                data: &self.data,
            })
        }
    }

    #[derive(Default)]
    struct Recorder {
        pictures: AtomicUsize,
        failures: Mutex<Vec<Failure>>,
        recovered: AtomicUsize,
        wanted: AtomicBool,
    }

    impl Recorder {
        fn new() -> Arc<Self> {
            let recorder = Self::default();
            recorder.wanted.store(true, Ordering::Release);
            Arc::new(recorder)
        }

        fn causes(&self) -> Vec<Cause> {
            self.failures
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(|f| f.cause)
                .collect()
        }
    }

    impl Sink for Recorder {
        fn buffer(&self) -> Vec<u8> {
            Vec::new()
        }

        fn put(&self, picture: Picture) {
            if picture.width == 2 && picture.rgba.len() == 16 {
                self.pictures.fetch_add(1, Ordering::AcqRel);
            }
        }

        fn failed(&self, failure: &Failure) {
            self.failures
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(failure.clone());
        }

        fn recovered(&self, _camera: &str) {
            self.recovered.fetch_add(1, Ordering::AcqRel);
        }

        fn wanted(&self) -> bool {
            self.wanted.load(Ordering::Acquire)
        }
    }

    fn wait_for(what: &str, ok: impl Fn() -> bool) -> Result<()> {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        bail!("timed out waiting for {what}")
    }

    fn pictures(r: &Recorder) -> usize {
        r.pictures.load(Ordering::Acquire)
    }

    #[test]
    fn pictures_flow_and_the_camera_closes_when_no_one_wants_it() -> Result<()> {
        let fake = Fake::new(true, &[Behaviour::Pictures]);
        let sink = Recorder::new();
        spawn(fake.clone(), quick(), CAMERA.to_string(), sink.clone());
        wait_for("pictures", || pictures(&sink) >= 3)?;
        assert!(sink.causes().is_empty(), "no failure: {:?}", sink.causes());
        sink.wanted.store(false, Ordering::Release);
        wait_for("the stream to close", || {
            fake.with(|s| s.streams_alive) == 0
        })?;
        assert_eq!(fake.with(|s| s.opened), 1, "opened once");
        Ok(())
    }

    #[test]
    fn a_missing_camera_is_said_once_and_shows_when_it_comes() -> Result<()> {
        let fake = Fake::new(false, &[Behaviour::Pictures]);
        let sink = Recorder::new();
        spawn(fake.clone(), quick(), CAMERA.to_string(), sink.clone());
        // Several retries go by.
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(sink.causes(), vec![Cause::NotFound], "said once");
        assert_eq!(pictures(&sink), 0, "nothing to show");
        fake.with(|s| s.listed = true);
        wait_for("pictures", || pictures(&sink) >= 2)?;
        assert_eq!(sink.recovered.load(Ordering::Acquire), 1, "back once");
        sink.wanted.store(false, Ordering::Release);
        Ok(())
    }

    #[test]
    fn an_unplugged_camera_is_reopened_by_name() -> Result<()> {
        let fake = Fake::new(true, &[Behaviour::FailAfter(3), Behaviour::Pictures]);
        let sink = Recorder::new();
        spawn(fake.clone(), quick(), CAMERA.to_string(), sink.clone());
        wait_for("the reopen", || fake.with(|s| s.opened) >= 2)?;
        wait_for("pictures after it", || pictures(&sink) >= 5)?;
        assert_eq!(sink.causes(), vec![Cause::Unplugged], "said once");
        assert_eq!(sink.recovered.load(Ordering::Acquire), 1, "back once");
        sink.wanted.store(false, Ordering::Release);
        Ok(())
    }

    #[test]
    fn a_read_that_never_returns_is_left_behind() -> Result<()> {
        let fake = Fake::new(true, &[Behaviour::Hang, Behaviour::Pictures]);
        let sink = Recorder::new();
        spawn(fake.clone(), quick(), CAMERA.to_string(), sink.clone());
        wait_for("pictures from a second reader", || pictures(&sink) >= 2)?;
        assert_eq!(sink.causes(), vec![Cause::Stalled], "the watchdog said it");
        assert_eq!(sink.recovered.load(Ordering::Acquire), 1, "back once");
        assert_eq!(fake.with(|s| s.opened), 2, "one fresh open");
        sink.wanted.store(false, Ordering::Release);
        Ok(())
    }

    #[test]
    fn readers_that_all_hang_do_not_pile_up() -> Result<()> {
        let fake = Fake::new(true, &[Behaviour::Hang]);
        let sink = Recorder::new();
        spawn(fake.clone(), quick(), CAMERA.to_string(), sink.clone());
        // Long enough for many stalls if nothing capped them.
        std::thread::sleep(quick().stall * 8);
        let opened = fake.with(|s| s.opened);
        assert_eq!(opened, MAX_ABANDONED, "capped at {MAX_ABANDONED}: {opened}");
        assert_eq!(sink.causes(), vec![Cause::Stalled], "said once");
        sink.wanted.store(false, Ordering::Release);
        Ok(())
    }
}
