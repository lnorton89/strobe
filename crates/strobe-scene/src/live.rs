// ============================================================================
// Module:       strobe_scene::live
// Description:  How a media choice names a live source instead of a file: a
//               camera, a Spout sender or an NDI source, each a path with its
//               own prefix.
//
// Dependencies: std only
// ============================================================================

//! Live sources as media choices (backlog M-04, D528; Spout and NDI input,
//! D575).
//!
//! Everywhere a media choice is a path (the operator's, a cue's `media`, a
//! preset's media channel), a live source is a path with a prefix naming its
//! kind: `camera:<name>` for a webcam, `spout:<name>` for a Spout sender on
//! this machine, `ndi:<name>` for an NDI source on the network. `<name>` is
//! what that system lists the source as. One spelling per kind keeps every
//! store of a media choice, and the precedence between them (D217), as it
//! is. No file is looked for at such a path, and it is never joined to a
//! folder.
//!
//! A scene's own `MEDIA` key cannot name a live source; it names a file in
//! the scene's folder, as in Synesthesia.

use std::fmt;
use std::path::{Path, PathBuf};

/// What kind of live source a media choice names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A webcam, through the system's camera API (D528).
    Camera,
    /// A Spout sender: another program's texture on this machine (D575).
    Spout,
    /// An NDI source: another program's picture over the network (D575).
    Ndi,
}

impl Kind {
    /// Every kind, in the order the Media row offers them.
    pub const ALL: [Self; 3] = [Self::Camera, Self::Spout, Self::Ndi];

    /// The prefix that makes a media path this kind.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Camera => "camera:",
            Self::Spout => "spout:",
            Self::Ndi => "ndi:",
        }
    }

    /// What one source of this kind is called in a sentence.
    pub fn noun(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Spout => "Spout sender",
            Self::Ndi => "NDI source",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.noun())
    }
}

/// A live source a media path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Source<'a> {
    /// Its kind.
    pub kind: Kind,
    /// Its name, as its system lists it.
    pub name: &'a str,
}

impl fmt::Display for Source<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} \"{}\"", self.kind, self.name)
    }
}

/// The live source `path` names, or `None` for a file or folder.
pub fn source_of(path: &Path) -> Option<Source<'_>> {
    let text = path.to_str()?;
    Kind::ALL.into_iter().find_map(|kind| {
        let name = text.strip_prefix(kind.prefix())?.trim();
        (!name.is_empty()).then_some(Source { kind, name })
    })
}

/// The media path for the source of `kind` called `name`.
pub fn path_for(kind: Kind, name: &str) -> PathBuf {
    PathBuf::from(format!("{}{}", kind.prefix(), name.trim()))
}

/// `relative` resolved against `dir`, unless it names a live source, which
/// is returned as it is.
pub fn resolve(dir: &Path, relative: &Path) -> PathBuf {
    match source_of(relative) {
        Some(_) => relative.to_path_buf(),
        None => dir.join(relative),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_round_trips_and_a_file_is_none() {
        for kind in Kind::ALL {
            let path = path_for(kind, " Feed One ");
            assert_eq!(
                source_of(&path),
                Some(Source {
                    kind,
                    name: "Feed One"
                }),
                "{path:?}"
            );
            assert_eq!(source_of(Path::new(kind.prefix())), None, "{kind}: no name");
        }
        assert_eq!(source_of(Path::new("loops/rain")), None, "a folder");
        assert_eq!(
            source_of(Path::new("spouts/x.png")),
            None,
            "a folder called spouts"
        );
    }

    #[test]
    fn an_ndi_name_keeps_its_machine_and_brackets() {
        // NDI lists a source as `MACHINE (Name)`; the whole text is the name.
        let path = path_for(Kind::Ndi, "STUDIO-PC (OBS Program)");
        assert_eq!(
            source_of(&path).map(|source| source.name),
            Some("STUDIO-PC (OBS Program)"),
            "{path:?}"
        );
    }

    #[test]
    fn a_live_source_is_never_joined_to_a_folder() {
        let dir = Path::new("scenes/a");
        for text in ["camera:USB Cam", "spout:Resolume", "ndi:HOST (Feed)"] {
            assert_eq!(
                resolve(dir, Path::new(text)),
                PathBuf::from(text),
                "{text} as it is"
            );
        }
        assert_eq!(
            resolve(dir, Path::new("pic.png")),
            dir.join("pic.png"),
            "a file is"
        );
    }
}
