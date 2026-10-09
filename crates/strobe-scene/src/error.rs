// ============================================================================
// Module:       strobe_scene::error
// Description:  Every way a scene can fail to load, each naming what is at
//               fault so the UI can show it rather than a generic failure.
//
// Dependencies: thiserror
// ============================================================================

//! Scene load errors.
//!
//! A malformed scene is ordinary input, not an exceptional event: a scene
//! folder is something a person edits by hand mid-show. Every variant
//! therefore carries enough to point at the problem — the control's name,
//! the offending token, the file — and none of them is a panic.

use std::path::{Path, PathBuf};

/// Why a scene could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    /// A file in the scene folder could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The file that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },

    /// `scene.json` is not valid JSON, or not the shape SSF describes.
    #[error("scene.json: {0}")]
    Json(#[from] serde_json::Error),

    /// A control's `TYPE` has a token SSF does not define.
    #[error("control `{name}`: unknown type `{ty}`")]
    UnknownControlType {
        /// The control's `NAME`.
        name: String,
        /// The whole `TYPE` string as written.
        ty: String,
    },

    /// A control's `TYPE` combines tokens that do not go together, such as
    /// `toggle speed`.
    #[error("control `{name}`: `{modifier}` cannot modify `{base}`")]
    InvalidModifier {
        /// The control's `NAME`.
        name: String,
        /// The base type.
        base: String,
        /// The modifier that does not apply to it.
        modifier: String,
    },

    /// A control's name is empty or is not a valid GLSL identifier.
    #[error("control name `{0}` is not a valid GLSL identifier")]
    InvalidName(String),

    /// Two names normalise to the same key, so the JavaScript API could not
    /// tell them apart.
    #[error(
        "`{first}` and `{second}` are the same name once case, spaces, `_` and `-` are ignored"
    )]
    NameCollision {
        /// The name declared first.
        first: String,
        /// The name that collides with it.
        second: String,
    },

    /// A control or pass reuses a name Strobe already declares as a uniform.
    #[error("`{0}` is a built-in uniform and cannot be redeclared")]
    ReservedName(String),

    /// A value array has the wrong number of components for its control.
    #[error("control `{name}`: `{key}` has {found} component(s), expected {expected}")]
    Arity {
        /// The control's `NAME`.
        name: String,
        /// The JSON key (`DEFAULT`, `MIN`, `MAX`).
        key: &'static str,
        /// Components the control type needs.
        expected: usize,
        /// Components found.
        found: usize,
    },

    /// A dropdown has neither `LABELS` nor `VALUES`, or they disagree in
    /// length.
    #[error("dropdown `{0}` needs LABELS or VALUES, of equal length when both are given")]
    Dropdown(String),

    /// A pass size could not be understood.
    #[error("pass {index}: cannot read size `{value}`")]
    PassSize {
        /// The pass's position in `PASSES`.
        index: usize,
        /// The value as written.
        value: String,
    },

    /// A pass option has a value SSF does not define.
    #[error("pass {index}: unknown {key} `{value}`")]
    PassOption {
        /// The pass's position in `PASSES`.
        index: usize,
        /// The JSON key (`WRAP`, `FILTER`).
        key: &'static str,
        /// The value as written.
        value: String,
    },

    /// The folder has no `main.glsl`.
    #[error("{0} has no main.glsl")]
    MissingShader(PathBuf),
}

impl SceneError {
    /// A `scene.json` value that parsed but means something invalid.
    ///
    /// It is reported as a [`SceneError::Json`] with a message of its own
    /// rather than as a new variant: `PASS_SOURCES` and image-option
    /// mistakes are errors in `scene.json`, and the app matches this enum
    /// exhaustively, so a variant added here would break a crate this one
    /// does not own (D80).
    pub fn invalid(message: impl std::fmt::Display) -> Self {
        Self::Json(<serde_json::Error as serde::de::Error>::custom(message))
    }

    /// A file the scene names that is not in its folder.
    pub fn missing_file(dir: &Path, file: &str) -> Self {
        Self::Io {
            path: dir.join(file),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
        }
    }
}
