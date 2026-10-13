// ============================================================================
// Module:       strobe_app::import
// Description:  Shadertoy import inside the app: a worker thread that runs the
//               converter, the UI's import state, and the localhost listener's
//               handler.
//
// Dependencies: strobe-shadertoy (import, acquire, listener); std::sync::mpsc
// ============================================================================

//! Importing a Shadertoy shader into the running app.
//!
//! ## Threads
//!
//! Three threads touch an import, and only one of them has a GL context:
//!
//! ```text
//! event loop  --Job-->  import worker  --Done-->  event loop
//!                          (convert, write,         (rescan, select,
//!                           download textures)       load: GL, main thread)
//! listener thread --Job(+reply)--> import worker --Reply--> listener thread
//! ```
//!
//! Everything slow and blocking (reading the file, the REST call, texture
//! downloads of up to ten seconds each, conversion, writing the folder)
//! runs on the **worker**. The event loop only ever does a non-blocking
//! `try_recv` on the results, once per wake-up, and compiles the new scene
//! itself, because shader compilation needs the context that thread owns.
//!
//! The queue is bounded ([`QUEUE`]); a full queue refuses the job with a
//! message instead of growing. Jobs run one at a time, in order.
//!
//! ## Sources
//!
//! A [`Job`] carries an [`Input`]: text the person typed or dropped (a file
//! path, a Shadertoy id or URL; the worker classifies it), or JSON that
//! arrived over the listener. Both end in `strobe_shadertoy::import`.
//!
//! ## Re-importing
//!
//! A scene already in the library is not replaced silently: the worker
//! answers [`ImportError::Exists`], the panel shows "Replace / Cancel", and
//! Replace sends the same input again with `replace` set (D113). The
//! userscript gets a 409 and asks the same question in the browser.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;
use strobe_shadertoy::acquire::{ApiKey, AssetCache, Endpoints, UreqFetch};
use strobe_shadertoy::import::{
    Downloader, ImportError, ImportOptions, Imported, Source, import_file, import_id, import_json,
};
use strobe_shadertoy::listener::{Handler, Reply, Submission};

/// Jobs that may wait for the worker.
pub const QUEUE: usize = 16;

/// How long a browser request waits for its import before the listener
/// gives up on it. An import with several textures on a slow link can take
/// a minute.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// What to import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A file path, shader id or URL, as typed or dropped.
    Text(String),
    /// A shader's JSON, from the browser userscript.
    Json {
        /// The shader JSON.
        text: String,
        /// The page it came from, for the log.
        origin: Option<String>,
    },
}

impl Input {
    /// A short name for messages: the typed text, or where the JSON came from.
    pub fn label(&self) -> String {
        // The last path or URL segment, whichever separator the person's
        // system uses (a Windows path is not a path on every host).
        let shorten = |text: &str| -> String {
            let text = text
                .trim()
                .trim_matches(['"', '\''])
                .trim_end_matches(['/', '\\']);
            let name = text.rsplit(['/', '\\']).next().unwrap_or(text);
            name.chars().take(60).collect()
        };
        match self {
            Self::Text(text) => shorten(text),
            Self::Json {
                origin: Some(origin),
                ..
            } => shorten(origin),
            Self::Json { origin: None, .. } => "a shader from the browser".to_string(),
        }
    }
}

/// One unit of work for the worker.
pub struct Job {
    /// What to import.
    pub input: Input,
    /// Replace an existing scene of the same name.
    pub replace: bool,
    /// Where to send the outcome for a waiting browser request.
    pub reply: Option<SyncSender<Reply>>,
}

/// A finished job.
pub struct Done {
    /// The input, so a "Replace" can send it again.
    pub input: Input,
    /// What happened.
    pub result: Result<Imported, ImportError>,
}

/// What the worker needs to know.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// The scene library imports are written into.
    pub library: PathBuf,
    /// The data directory: the API key, the texture cache.
    pub data_dir: PathBuf,
    /// Fetch missing textures from Shadertoy.
    pub download_textures: bool,
}

/// Why a job was not queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubmitError {
    /// The worker is busy and its queue is full.
    #[error("{QUEUE} imports are already waiting; try again in a moment")]
    Full,
    /// The worker has stopped (the app is closing).
    #[error("the importer has stopped")]
    Stopped,
}

/// The worker thread and its two channels.
pub struct Service {
    jobs: SyncSender<Job>,
    done: Receiver<Done>,
    _thread: JoinHandle<()>,
}

impl Service {
    /// Starts the worker.
    pub fn start(config: WorkerConfig) -> std::io::Result<Self> {
        let (jobs, queue) = sync_channel::<Job>(QUEUE);
        let (finished, done) = std::sync::mpsc::channel::<Done>();
        let thread = std::thread::Builder::new()
            .name("strobe-import".to_string())
            .spawn(move || work(&queue, &finished, &config))?;
        Ok(Self {
            jobs,
            done,
            _thread: thread,
        })
    }

    /// Queues `input` without blocking.
    pub fn submit(&self, input: Input, replace: bool) -> Result<(), SubmitError> {
        self.jobs
            .try_send(Job {
                input,
                replace,
                reply: None,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => SubmitError::Full,
                TrySendError::Disconnected(_) => SubmitError::Stopped,
            })
    }

    /// A finished job, if one is waiting. Never blocks.
    pub fn try_next(&self) -> Option<Done> {
        self.done.try_recv().ok()
    }

    /// The listener's handler: queues the browser's shader and waits for the
    /// worker's answer, which it returns to the browser.
    pub fn listener_handler(&self) -> Arc<Handler> {
        let jobs = self.jobs.clone();
        Arc::new(move |submission: Submission| {
            let (reply, answer) = sync_channel(1);
            let job = Job {
                input: Input::Json {
                    text: submission.shader,
                    origin: submission.page_url,
                },
                replace: submission.replace,
                reply: Some(reply),
            };
            match jobs.try_send(job) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    return Reply::Busy(SubmitError::Full.to_string());
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Reply::Busy(SubmitError::Stopped.to_string());
                }
            }
            answer.recv_timeout(REPLY_TIMEOUT).unwrap_or_else(|_| {
                Reply::Failed(
                    "Strobe did not finish the import in time; check its console".to_string(),
                )
            })
        })
    }
}

/// The worker's loop: one job at a time until the queue closes.
fn work(queue: &Receiver<Job>, finished: &std::sync::mpsc::Sender<Done>, config: &WorkerConfig) {
    let fetch = UreqFetch::new();
    let endpoints = Endpoints::from_env();
    let cache = AssetCache::in_data_dir(&config.data_dir);
    while let Ok(job) = queue.recv() {
        let options = ImportOptions {
            replace: job.replace,
            downloader: config.download_textures.then_some(Downloader {
                fetch: &fetch,
                endpoints: &endpoints,
                cache: &cache,
            }),
            ..ImportOptions::offline(&config.library)
        };
        let result = run(&job.input, &options, &fetch, &endpoints, config);
        if let Some(reply) = &job.reply {
            let _ = reply.send(Reply::from_import(&result));
        }
        if finished
            .send(Done {
                input: job.input,
                result,
            })
            .is_err()
        {
            return;
        }
    }
}

/// Runs one import with the route its input implies.
fn run(
    input: &Input,
    options: &ImportOptions<'_>,
    fetch: &UreqFetch,
    endpoints: &Endpoints,
    config: &WorkerConfig,
) -> Result<Imported, ImportError> {
    match input {
        Input::Json { text, origin } => {
            if let Some(origin) = origin {
                log::info!("importing a shader sent from {origin}");
            }
            import_json(text, options)
        }
        Input::Text(text) => match Source::classify(text)? {
            Source::File(path) => import_file(&path, options),
            Source::Id(id) => {
                let key = ApiKey::find(&config.data_dir)?;
                import_id(&id, &key, fetch, endpoints, options)
            }
        },
    }
}

/// What the import panel's status line shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    /// Nothing has been imported this session.
    #[default]
    Idle,
    /// A job is queued or running.
    Working(String),
    /// The last import succeeded.
    Done(String),
    /// The last import succeeded with warnings (listed in the console).
    Warned(String),
    /// The last import failed.
    Failed(String),
}

/// A re-import waiting for the person's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacePrompt {
    /// The input to send again with `replace`.
    pub input: Input,
    /// The scene already in the library.
    pub title: String,
}

/// The import panel's state and the rules for updating it.
#[derive(Debug, Default)]
pub struct State {
    /// The path/id field's text.
    pub text: String,
    /// The status line.
    pub status: Status,
    /// A pending "replace?" question.
    pub prompt: Option<ReplacePrompt>,
    /// Jobs queued or running.
    pub in_flight: usize,
    /// A file is being dragged over the window.
    pub drop_hover: bool,
    /// The folder of the last scene this pane imported and loaded, so the
    /// pane can offer to generate controls for it (U-03, D246).
    pub last: Option<PathBuf>,
}

/// What finishing a job means for the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A scene was written at this folder: rescan and load it.
    Imported(PathBuf),
    /// The scene is already in the library, at this folder; the panel
    /// asks whether to replace it. A caller that only needs the scene (a
    /// cue being added, D229) can use it as it is.
    Exists(PathBuf),
    /// Nothing changed in the library.
    Unchanged,
}

impl State {
    /// Records that `input` was queued.
    pub fn queued(&mut self, input: &Input) {
        self.in_flight += 1;
        self.prompt = None;
        self.status = Status::Working(format!("Importing {}…", input.label()));
    }

    /// Records a job that could not be queued.
    pub fn refused(&mut self, error: SubmitError) {
        self.status = Status::Failed(error.to_string());
    }

    /// Applies a finished job: updates the status and prompt, and logs what
    /// a person should see in the console. Returns what to do to the library.
    pub fn finished(&mut self, done: Done) -> Outcome {
        self.in_flight = self.in_flight.saturating_sub(1);
        match done.result {
            Ok(imported) => {
                let verb = if imported.replaced {
                    "Replaced"
                } else {
                    "Imported"
                };
                log::info!(
                    "{verb} \"{}\" as {} ({} file(s), {} texture(s), {} missing)",
                    imported.title,
                    imported.folder,
                    imported.files,
                    imported.textures_found,
                    imported.textures_missing
                );
                for warning in &imported.warnings {
                    log::warn!("import \"{}\": {warning}", imported.title);
                }
                self.status = if imported.warnings.is_empty() {
                    Status::Done(format!("{verb} \"{}\".", imported.title))
                } else {
                    Status::Warned(format!(
                        "{verb} \"{}\" with {} warning(s); see the console.",
                        imported.title,
                        imported.warnings.len()
                    ))
                };
                self.prompt = None;
                Outcome::Imported(imported.dir)
            }
            Err(ImportError::Exists { title, dir, .. }) => {
                self.status = Status::Failed(format!("\"{title}\" is already in the library."));
                self.prompt = Some(ReplacePrompt {
                    input: done.input,
                    title,
                });
                Outcome::Exists(dir)
            }
            Err(error) => {
                self.status = Status::Failed(error.to_string());
                Outcome::Unchanged
            }
        }
    }

    /// The person chose Replace: the input to send again.
    pub fn confirm_replace(&mut self) -> Option<Input> {
        self.prompt.take().map(|prompt| prompt.input)
    }

    /// The person chose Cancel.
    pub fn cancel_replace(&mut self) {
        if self.prompt.take().is_some() {
            self.status = Status::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use std::time::Instant;

    const SINGLE: &str = include_str!("../../strobe-shadertoy/tests/fixtures/single_image.json");

    fn scratch(name: &str) -> Result<PathBuf> {
        let dir =
            std::env::temp_dir().join(format!("strobe-app-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn service(name: &str) -> Result<(Service, PathBuf)> {
        let library = scratch(name)?;
        let service = Service::start(WorkerConfig {
            library: library.join("scenes"),
            data_dir: library.join("data"),
            download_textures: false,
        })?;
        Ok((service, library))
    }

    /// Waits for the next finished job (the worker is another thread).
    fn next(service: &Service) -> Result<Done> {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(20) {
            if let Some(done) = service.try_next() {
                return Ok(done);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(anyhow!("the worker produced nothing in 20 s"))
    }

    #[test]
    fn a_dropped_file_is_converted_on_the_worker_and_reported_back() -> Result<()> {
        let (service, root) = service("file")?;
        let file = root.join("shader.json");
        std::fs::write(&file, SINGLE)?;
        service.submit(Input::Text(file.display().to_string()), false)?;
        let done = next(&service)?;
        let imported = done.result.map_err(|e| anyhow!("{e}"))?;
        assert!(imported.dir.join("scene.json").is_file(), "scene written");
        assert!(
            imported.dir.starts_with(root.join("scenes")),
            "in the library"
        );
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn json_from_the_listener_gets_an_answer_for_the_browser_and_the_app() -> Result<()> {
        let (service, root) = service("listener")?;
        let handler = service.listener_handler();
        let reply = handler(Submission {
            shader: SINGLE.to_string(),
            page_url: Some("https://www.shadertoy.com/view/Tst001".to_string()),
            replace: false,
        });
        assert!(matches!(reply, Reply::Imported { .. }), "{reply:?}");
        let done = next(&service)?;
        assert!(done.result.is_ok(), "the app hears of it too");
        // Again: the browser is asked, with a 409-able reply.
        let again = handler(Submission {
            shader: SINGLE.to_string(),
            page_url: None,
            replace: false,
        });
        assert!(matches!(again, Reply::Exists { .. }), "{again:?}");
        let replaced = handler(Submission {
            shader: SINGLE.to_string(),
            page_url: None,
            replace: true,
        });
        assert!(
            matches!(replaced, Reply::Imported { replaced: true, .. }),
            "{replaced:?}"
        );
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_bad_file_and_a_bad_id_are_errors_with_words() -> Result<()> {
        let (service, root) = service("errors")?;
        let file = root.join("nope.json");
        std::fs::write(&file, "{ not json")?;
        for text in [file.display().to_string(), "gibberish words".to_string()] {
            service.submit(Input::Text(text.clone()), false)?;
            let done = next(&service)?;
            let error = done
                .result
                .err()
                .ok_or_else(|| anyhow!("`{text}` should fail"))?;
            assert!(!error.to_string().is_empty(), "`{text}` has a message");
        }
        // A well-formed id with no key anywhere says how to get one. (Skipped
        // when the developer has a real key in the environment: that would
        // make a real request.)
        if std::env::var_os(strobe_shadertoy::acquire::KEY_ENV).is_none() {
            service.submit(Input::Text("XsXXDn".to_string()), false)?;
            let message = next(&service)?
                .result
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(message.contains("STROBE_SHADERTOY_KEY"), "{message}");
        }
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    fn imported(replaced: bool, warnings: usize) -> Imported {
        Imported {
            dir: PathBuf::from("scenes/x.synScene"),
            folder: "x.synScene".to_string(),
            title: "X".to_string(),
            shader_id: None,
            replaced,
            files: 3,
            textures_found: 0,
            textures_missing: 0,
            warnings: vec!["w".to_string(); warnings],
        }
    }

    #[test]
    fn the_status_line_follows_a_job_through_its_outcomes() {
        let mut state = State::default();
        let input = Input::Text("C:\\shaders\\tunnel.json".to_string());
        state.queued(&input);
        assert_eq!(
            state.status,
            Status::Working("Importing tunnel.json…".to_string()),
            "working"
        );
        assert_eq!(state.in_flight, 1, "in flight");

        let outcome = state.finished(Done {
            input: input.clone(),
            result: Ok(imported(false, 0)),
        });
        assert_eq!(
            outcome,
            Outcome::Imported(PathBuf::from("scenes/x.synScene"))
        );
        assert_eq!(
            state.status,
            Status::Done("Imported \"X\".".to_string()),
            "done"
        );
        assert_eq!(state.in_flight, 0, "idle again");

        state.queued(&input);
        state.finished(Done {
            input: input.clone(),
            result: Ok(imported(true, 2)),
        });
        assert!(
            matches!(&state.status, Status::Warned(m) if m.contains("Replaced") && m.contains("2 warning")),
            "{:?}",
            state.status
        );
    }

    #[test]
    fn exists_asks_and_replace_resends_the_same_input() {
        let mut state = State::default();
        let input = Input::Json {
            text: "{}".to_string(),
            origin: None,
        };
        state.queued(&input);
        let exists = Err(ImportError::Exists {
            dir: PathBuf::from("d"),
            folder: "f".to_string(),
            title: "Tunnel".to_string(),
        });
        let outcome = state.finished(Done {
            input: input.clone(),
            result: exists,
        });
        assert_eq!(
            outcome,
            Outcome::Exists(PathBuf::from("d")),
            "nothing written; the existing folder named"
        );
        assert_eq!(
            state.prompt.as_ref().map(|p| p.title.as_str()),
            Some("Tunnel"),
            "asks about Tunnel"
        );
        assert_eq!(
            state.confirm_replace(),
            Some(input),
            "resends the same input"
        );
        assert!(state.prompt.is_none(), "the question is answered");

        state.prompt = Some(ReplacePrompt {
            input: Input::Text("x".into()),
            title: "T".into(),
        });
        state.cancel_replace();
        assert!(
            state.prompt.is_none() && state.status == Status::Idle,
            "cancelled"
        );
    }

    #[test]
    fn a_failure_shows_the_converters_words_and_changes_nothing() {
        let mut state = State::default();
        state.queued(&Input::Text("x".to_string()));
        let outcome = state.finished(Done {
            input: Input::Text("x".to_string()),
            result: Err(ImportError::Input("no file at x".to_string())),
        });
        assert_eq!(outcome, Outcome::Unchanged, "nothing to load");
        assert_eq!(state.status, Status::Failed("no file at x".to_string()));
    }

    #[test]
    fn labels_are_short_and_name_the_thing() {
        assert_eq!(
            Input::Text("\"C:\\Users\\me\\Downloads\\a.json\"".into()).label(),
            "a.json",
            "a quoted Windows path"
        );
        assert_eq!(
            Input::Text("https://www.shadertoy.com/view/XsXXDn".into()).label(),
            "XsXXDn",
            "a url"
        );
        assert_eq!(
            Input::Json {
                text: String::new(),
                origin: None
            }
            .label(),
            "a shader from the browser",
            "no origin"
        );
    }
}
