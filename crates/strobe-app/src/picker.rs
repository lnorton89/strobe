// ============================================================================
// Module:       strobe_app::picker
// Description:  Native file and folder pickers (Browse buttons), run off the
//               frame loop's thread and polled for their answer.
//
// Dependencies: rfd; std::sync::mpsc, std::thread
// ============================================================================

//! Browse (D470).
//!
//! Every path field (a playlist, a Shadertoy export, the watched folder, a
//! venue trust file) can also be filled from the system's own file or
//! folder dialog, with typed paths and drag-and-drop still working.
//!
//! ## Never on the frame loop
//!
//! A system dialog is modal to the thread that opens it: opened from the
//! frame loop, the output on the projector would stop for as long as the
//! dialog stayed up. So [`Picker::open`] runs it on a thread of its own and
//! returns at once; the app calls [`Picker::poll`] each frame and acts on
//! the answer when it comes. One dialog at a time: Browse is disabled while
//! one is open ([`Picker::busy`]).
//!
//! macOS is the exception, because its panels must be opened on the main
//! thread: there the dialog runs inline and the frame loop waits. Strobe
//! ships for Windows first, and this is stated rather than hidden.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// What a Browse button is for: which dialog, and where the answer goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A playlist file, opened as soon as it is chosen.
    Playlist,
    /// An exported Shadertoy `.json`, imported as soon as it is chosen.
    Shadertoy,
    /// The folder new scenes are dropped into (D265), watched at once.
    WatchFolder,
    /// A venue trust file (D294), put in the connect form's field.
    TrustFile,
}

impl Purpose {
    /// The dialog for this purpose, starting in `start` when given.
    fn dialog(self, start: Option<&PathBuf>) -> rfd::FileDialog {
        let mut dialog = rfd::FileDialog::new();
        if let Some(start) = start.filter(|dir| dir.is_dir()) {
            dialog = dialog.set_directory(start);
        }
        match self {
            Self::Playlist => dialog
                .set_title("Open a playlist")
                .add_filter("Strobe playlist", &["toml"]),
            Self::Shadertoy => dialog
                .set_title("Import a Shadertoy export")
                .add_filter("Shadertoy export", &["json"]),
            Self::WatchFolder => dialog.set_title("Choose the folder to watch for new scenes"),
            Self::TrustFile => dialog
                .set_title("Choose the venue trust file")
                .add_filter("Strobe trust file", &["trust"])
                .add_filter("Any file", &["*"]),
        }
    }

    /// Whether this purpose picks a folder rather than a file.
    const fn folder(self) -> bool {
        match self {
            Self::WatchFolder => true,
            Self::Playlist | Self::Shadertoy | Self::TrustFile => false,
        }
    }
}

/// Shows `purpose`'s dialog and waits for it: a path, or `None` if it was
/// cancelled.
fn ask(purpose: Purpose, start: Option<&PathBuf>) -> Option<PathBuf> {
    let dialog = purpose.dialog(start);
    if purpose.folder() {
        dialog.pick_folder()
    } else {
        dialog.pick_file()
    }
}

/// The dialog in progress, if any.
#[derive(Debug, Default)]
pub struct Picker {
    pending: Option<(Purpose, Receiver<Option<PathBuf>>)>,
    /// An answer from an inline dialog (macOS), handed out by the next
    /// poll like a threaded one.
    ready: Option<(Purpose, Option<PathBuf>)>,
}

impl Picker {
    /// Whether a dialog is open now.
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Opens `purpose`'s dialog, starting in `start`. Returns at once
    /// (except on macOS, see the module docs); does nothing while another
    /// dialog is open.
    pub fn open(&mut self, purpose: Purpose, start: Option<PathBuf>) {
        if self.busy() {
            return;
        }
        if cfg!(target_os = "macos") {
            self.ready = Some((purpose, ask(purpose, start.as_ref())));
            return;
        }
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("file picker".to_string())
            .spawn(move || {
                // The app may have stopped listening; nothing to do then.
                let _ = sender.send(ask(purpose, start.as_ref()));
            });
        match spawned {
            Ok(_) => self.pending = Some((purpose, receiver)),
            Err(error) => log::warn!("picker: could not open the dialog: {error}"),
        }
    }

    /// The answer, once the dialog has closed: what it was for and the
    /// path chosen. `None` while it is open, when it was cancelled, and
    /// when no dialog was asked for.
    pub fn poll(&mut self) -> Option<(Purpose, PathBuf)> {
        if let Some((purpose, path)) = self.ready.take() {
            return path.map(|path| (purpose, path));
        }
        let (purpose, receiver) = self.pending.as_ref()?;
        let purpose = *purpose;
        match receiver.try_recv() {
            Ok(answer) => {
                self.pending = None;
                answer.map(|path| (purpose, path))
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                // The dialog's thread ended without answering.
                self.pending = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_watched_folder_picks_a_folder() {
        assert!(Purpose::WatchFolder.folder(), "a folder");
        for purpose in [Purpose::Playlist, Purpose::Shadertoy, Purpose::TrustFile] {
            assert!(!purpose.folder(), "{purpose:?} picks a file");
        }
    }

    #[test]
    fn a_picker_with_nothing_open_answers_nothing() {
        let mut picker = Picker::default();
        assert!(!picker.busy(), "nothing open");
        assert_eq!(picker.poll(), None, "no answer");
    }

    #[test]
    fn an_answer_is_handed_out_once_and_frees_the_picker() {
        let (sender, receiver) = channel();
        let mut picker = Picker {
            pending: Some((Purpose::Playlist, receiver)),
            ready: None,
        };
        assert!(picker.busy(), "open");
        assert_eq!(picker.poll(), None, "still open");
        let chosen = PathBuf::from("show.toml");
        let _ = sender.send(Some(chosen.clone()));
        assert_eq!(
            picker.poll(),
            Some((Purpose::Playlist, chosen)),
            "the path, with what it was for"
        );
        assert!(!picker.busy(), "free again");
        assert_eq!(picker.poll(), None, "handed out once");
    }

    #[test]
    fn a_cancelled_or_abandoned_dialog_frees_the_picker() {
        let (sender, receiver) = channel();
        let mut picker = Picker {
            pending: Some((Purpose::Shadertoy, receiver)),
            ready: None,
        };
        let _ = sender.send(None);
        assert_eq!(picker.poll(), None, "cancelled");
        assert!(!picker.busy(), "free after a cancel");
        let (sender, receiver) = channel::<Option<PathBuf>>();
        picker.pending = Some((Purpose::TrustFile, receiver));
        drop(sender);
        assert_eq!(picker.poll(), None, "no answer");
        assert!(!picker.busy(), "free after the thread went away");
    }
}
