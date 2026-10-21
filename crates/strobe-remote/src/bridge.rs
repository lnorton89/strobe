// ============================================================================
// Module:       strobe_remote::bridge
// Description:  The meeting point between a network adapter (any thread) and
//               the app's event-loop thread: a bounded command queue with a
//               reply per request, and a revisioned state snapshot.
//
// Dependencies: crate (protocol types); std::sync (mpsc, Mutex, Arc)
// ============================================================================

//! The app-thread bridge (backlog L-05, D232).
//!
//! A network adapter must never touch the renderer, the playlist or the
//! UI. It hands each authenticated command to [`Remote::submit`] and waits
//! on the reply it gets back; the app takes commands off the other end
//! with [`AppEnd::next`] once a frame, on its own thread, decides whether
//! each is valid *now*, carries it out through its ordinary command path,
//! and answers it. The other direction is a state snapshot the app
//! publishes with [`AppEnd::publish`] and adapters read with
//! [`Remote::snapshot`].
//!
//! The rules the research note sets (`docs/REMOTE_CONTROL_PWA_RESEARCH.md`)
//! are properties of this type, not of whoever uses it:
//!
//! - **Bounded.** The queue holds at most the capacity it was made with; a
//!   submit to a full queue is answered `Busy` at once. Nothing an adapter
//!   does can grow memory or make the frame wait.
//! - **Never blocking the app.** The app end only ever `try`s: it takes
//!   what is there and answers without waiting on the adapter, whose reply
//!   slot always has room for its one answer.
//! - **Revisioned state.** The revision goes up only when the snapshot
//!   changes, so an adapter can tell a client "nothing new" cheaply, and a
//!   reconnecting client always gets the whole current state.
//! - **A gone app is `Unavailable`.** When the app end is dropped (the
//!   remote switched off, the app closing), every submit and every pending
//!   reply says so instead of hanging.
//! - **`Busy` means not done, ever.** A command whose adapter stopped
//!   waiting is withdrawn: [`AppEnd::next`] skips it, so a phone told
//!   "busy, try again" that tries again runs the command once, not twice
//!   (D460). Whichever side claims the command first wins; once the app has
//!   taken it, the adapter waits for the app's real answer.
//!
//! No socket is opened here, or anywhere yet: the listener (L-02) and
//! pairing (L-03) come with their own decisions.

use crate::{AudioStatus, ErrorCode, RemoteCommand, ServerMessage, valid_request_id};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// A command still in the queue: either side may claim it.
const WAITING: u8 = 0;
/// The app took it and will run and answer it.
const TAKEN: u8 = 1;
/// Its adapter stopped waiting and told the client `Busy`: never run.
const WITHDRAWN: u8 = 2;

/// What the remote shows: the fields of [`ServerMessage::State`] without
/// the revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// The scene or cue playing.
    pub current: Option<String>,
    /// The next one, if known.
    pub next: Option<String>,
    /// Milliseconds left in a timed cue.
    pub remaining_ms: Option<u64>,
    /// Whether a playlist is held.
    pub held: bool,
    /// Whether the output is blacked out.
    pub blackout: bool,
    /// Whether the output window is shown.
    pub output_visible: bool,
    /// The audio input, in a word.
    pub audio_status: AudioStatus,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            current: None,
            next: None,
            remaining_ms: None,
            held: false,
            blackout: false,
            output_visible: false,
            audio_status: AudioStatus::Unknown,
        }
    }
}

impl Snapshot {
    /// This snapshot as the message a client receives.
    pub fn to_message(&self, revision: u64) -> ServerMessage {
        ServerMessage::State {
            revision,
            current: self.current.clone(),
            next: self.next.clone(),
            remaining_ms: self.remaining_ms,
            held: self.held,
            blackout: self.blackout,
            output_visible: self.output_visible,
            audio_status: self.audio_status,
        }
    }
}

/// The app's answer to one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Carried out.
    Accepted,
    /// Not carried out, with a short reason for the phone's screen.
    Rejected(String),
}

/// A command waiting on the app's thread.
#[derive(Debug)]
pub struct Incoming {
    /// The client's id for it, already validated.
    pub request_id: String,
    /// What it asks for.
    pub command: RemoteCommand,
    reply: SyncSender<Answer>,
    /// [`WAITING`], [`TAKEN`] or [`WITHDRAWN`], shared with its [`Pending`].
    claim: Arc<AtomicU8>,
}

impl Incoming {
    /// Answers the command. Never blocks: the reply slot has room for
    /// exactly this answer, and an adapter that stopped waiting is fine.
    pub fn answer(self, answer: Answer) {
        // An adapter that stopped waiting has already answered its client
        // Busy.
        let _ = self.reply.try_send(answer);
    }
}

/// A submitted command's reply, for the adapter to wait on.
#[derive(Debug)]
pub struct Pending {
    reply: Receiver<Answer>,
    claim: Arc<AtomicU8>,
}

impl Pending {
    /// Waits up to `timeout` for the app's answer. An app that went away
    /// is [`ErrorCode::Unavailable`]. One that has not taken the command
    /// in time is [`ErrorCode::Busy`], and the command is withdrawn: the
    /// app never runs it (D460). One that took it just as the time ran out
    /// is running it, so this waits on for that answer rather than report
    /// a command it is carrying out as not done.
    pub fn wait(self, timeout: Duration) -> Result<Answer, ErrorCode> {
        match self.reply.recv_timeout(timeout) {
            Ok(answer) => Ok(answer),
            Err(RecvTimeoutError::Disconnected) => Err(ErrorCode::Unavailable),
            Err(RecvTimeoutError::Timeout) => {
                let withdrawn = self
                    .claim
                    .compare_exchange(WAITING, WITHDRAWN, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok();
                if withdrawn {
                    return Err(ErrorCode::Busy);
                }
                // The app took it: it answers in the same pass that runs it,
                // or drops it (gone), which ends this wait too.
                self.reply.recv().map_err(|_| ErrorCode::Unavailable)
            }
        }
    }
}

#[derive(Debug)]
struct Shared {
    revision: u64,
    snapshot: Snapshot,
}

/// The adapter's end: cheap to clone, one per connection if need be.
#[derive(Clone, Debug)]
pub struct Remote {
    /// Optional full desktop bridge, disabled until the host opts in.
    pub desktop: crate::desktop::Desktop,
    commands: SyncSender<Incoming>,
    state: Arc<Mutex<Shared>>,
}

/// The app's end, kept on its event-loop thread.
#[derive(Debug)]
pub struct AppEnd {
    /// Optional full desktop bridge, shared with authenticated connections.
    pub desktop: crate::desktop::Desktop,
    commands: Receiver<Incoming>,
    state: Arc<Mutex<Shared>>,
}

impl Drop for AppEnd {
    fn drop(&mut self) {
        // A gone app cannot receive UI input or publish trustworthy frames.
        self.desktop.enable(false);
    }
}

/// A bridge whose queue holds `capacity` commands (at least one).
pub fn bridge(capacity: usize) -> (Remote, AppEnd) {
    let desktop = crate::desktop::Desktop::default();
    let (commands, receiver) = sync_channel(capacity.max(1));
    let state = Arc::new(Mutex::new(Shared {
        revision: 0,
        snapshot: Snapshot::default(),
    }));
    (
        Remote {
            desktop: desktop.clone(),
            commands,
            state: Arc::clone(&state),
        },
        AppEnd {
            desktop,
            commands: receiver,
            state,
        },
    )
}

impl Remote {
    /// Queues `command` for the app. Refuses an invalid request id
    /// ([`ErrorCode::InvalidRequest`]), a full queue ([`ErrorCode::Busy`])
    /// and a gone app ([`ErrorCode::Unavailable`]) at once.
    pub fn submit(&self, request_id: &str, command: RemoteCommand) -> Result<Pending, ErrorCode> {
        if !valid_request_id(request_id) {
            return Err(ErrorCode::InvalidRequest);
        }
        let (reply, answer) = sync_channel(1);
        let claim = Arc::new(AtomicU8::new(WAITING));
        let incoming = Incoming {
            request_id: request_id.to_string(),
            command,
            reply,
            claim: Arc::clone(&claim),
        };
        match self.commands.try_send(incoming) {
            Ok(()) => Ok(Pending {
                reply: answer,
                claim,
            }),
            Err(TrySendError::Full(_)) => Err(ErrorCode::Busy),
            Err(TrySendError::Disconnected(_)) => Err(ErrorCode::Unavailable),
        }
    }

    /// The latest snapshot and its revision.
    pub fn snapshot(&self) -> (u64, Snapshot) {
        let shared = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        (shared.revision, shared.snapshot.clone())
    }
}

impl AppEnd {
    /// The next waiting command, if any. Never blocks. A command whose
    /// adapter already gave up on it (and told its client `Busy`) is
    /// dropped here, never returned (D460). The one returned is the app's:
    /// it must answer it, or drop it, in the same pass, since its adapter
    /// now waits for that answer.
    pub fn next(&self) -> Option<Incoming> {
        while let Ok(incoming) = self.commands.try_recv() {
            let taken = incoming
                .claim
                .compare_exchange(WAITING, TAKEN, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();
            if taken {
                return Some(incoming);
            }
            log::info!(
                "remote: skipped {} ({}): its client was already told Strobe was busy",
                incoming.command.id(),
                incoming.request_id
            );
        }
        None
    }

    /// Publishes the app's state; the revision goes up only if it changed.
    /// Returns whether it did.
    pub fn publish(&self, snapshot: &Snapshot) -> bool {
        let mut shared = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if shared.snapshot == *snapshot {
            return false;
        }
        shared.snapshot.clone_from(snapshot);
        shared.revision += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_command_goes_to_the_app_and_its_answer_comes_back() -> Result<()> {
        let (remote, app) = bridge(4);
        let pending = remote
            .submit("phone-1", RemoteCommand::ToggleBlackout)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let Some(incoming) = app.next() else {
            anyhow::bail!("the app saw nothing");
        };
        assert_eq!(
            incoming.command,
            RemoteCommand::ToggleBlackout,
            "the command"
        );
        assert_eq!(incoming.request_id, "phone-1", "and its id");
        incoming.answer(Answer::Rejected("needs a playlist".into()));
        assert_eq!(
            pending.wait(Duration::from_millis(100)),
            Ok(Answer::Rejected("needs a playlist".into())),
            "the answer reaches the adapter"
        );
        assert!(app.next().is_none(), "nothing else is waiting");
        Ok(())
    }

    #[test]
    fn a_full_queue_is_busy_at_once_and_a_bad_id_is_refused() {
        let (remote, _app) = bridge(2);
        assert!(remote.submit("a", RemoteCommand::TapTempo).is_ok(), "one");
        assert!(remote.submit("b", RemoteCommand::TapTempo).is_ok(), "two");
        assert_eq!(
            remote.submit("c", RemoteCommand::TapTempo).err(),
            Some(ErrorCode::Busy),
            "the third waits for nobody: busy"
        );
        assert_eq!(
            remote.submit("has space", RemoteCommand::TapTempo).err(),
            Some(ErrorCode::InvalidRequest),
            "an id the protocol forbids"
        );
    }

    #[test]
    fn a_gone_app_is_unavailable_not_a_hang() -> Result<()> {
        let (remote, app) = bridge(2);
        let pending = remote
            .submit("a", RemoteCommand::NextScene)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        drop(app);
        assert_eq!(
            pending.wait(Duration::from_secs(5)),
            Err(ErrorCode::Unavailable),
            "the pending reply says so at once"
        );
        assert_eq!(
            remote.submit("b", RemoteCommand::NextScene).err(),
            Some(ErrorCode::Unavailable),
            "and so does the next submit"
        );
        Ok(())
    }

    #[test]
    fn an_unanswered_command_times_out_as_busy() -> Result<()> {
        let (remote, app) = bridge(1);
        let pending = remote
            .submit("a", RemoteCommand::NextScene)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(
            pending.wait(Duration::from_millis(20)),
            Err(ErrorCode::Busy),
            "the adapter stops waiting"
        );
        assert!(
            app.next().is_none(),
            "and a command its client was told is busy is never run (D460)"
        );
        Ok(())
    }

    #[test]
    fn a_command_the_app_took_is_answered_truly_even_past_the_timeout() -> Result<()> {
        // The app takes the command just as the adapter's time runs out: it
        // is running it, so "busy" would be a lie the phone acts on.
        let (remote, app) = bridge(1);
        let pending = remote
            .submit("a", RemoteCommand::NextScene)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let Some(incoming) = app.next() else {
            anyhow::bail!("the app saw nothing");
        };
        let answering = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            incoming.answer(Answer::Accepted);
        });
        assert_eq!(
            pending.wait(Duration::from_millis(10)),
            Ok(Answer::Accepted),
            "the adapter waits for the answer of a command being run"
        );
        answering
            .join()
            .map_err(|_| anyhow::anyhow!("the answering thread panicked"))?;
        Ok(())
    }

    #[test]
    fn the_revision_moves_only_when_the_state_does() {
        let (remote, app) = bridge(1);
        assert_eq!(remote.snapshot().0, 0, "nothing published");
        let mut state = Snapshot {
            current: Some("Aurora".into()),
            blackout: true,
            ..Snapshot::default()
        };
        assert!(app.publish(&state), "new state");
        assert!(!app.publish(&state), "the same again is not news");
        state.blackout = false;
        assert!(app.publish(&state), "a change is");
        let (revision, seen) = remote.snapshot();
        assert_eq!(
            (revision, seen),
            (2, state.clone()),
            "two revisions, the latest state"
        );
        assert!(
            matches!(
                state.to_message(2),
                ServerMessage::State {
                    revision: 2,
                    blackout: false,
                    ..
                }
            ),
            "and it is the protocol's message"
        );
    }
}
