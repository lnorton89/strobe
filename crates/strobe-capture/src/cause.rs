// ============================================================================
// Module:       strobe_capture::cause
// Description:  Why a camera shows nothing, read from the backend's wording,
//               and the sentence that tells a person what to do about it.
//
// Dependencies: std
// ============================================================================

//! The likely cause of a camera failure (D528).
//!
//! nokhwa keeps an HRESULT's text, not its code, so "unplugged", "in use by
//! another app" and "blocked by the privacy setting" can only be told apart
//! by their wording. [`Cause::from_text`] does that with a short list of
//! phrases and codes. The list is a best guess from Media Foundation's
//! documented errors; the error text a real camera gives in each case is on
//! M-04's camera checklist, and a wrong guess only changes the advice, never
//! what Strobe does (it shows black and keeps retrying either way).

use std::fmt;

/// Why a camera shows nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    /// No camera of that name is listed.
    NotFound,
    /// The camera went away while showing: unplugged, most likely.
    Unplugged,
    /// Open but no picture came for the watchdog's time.
    Stalled,
    /// Another app has the camera.
    Busy,
    /// Windows' camera privacy setting blocks desktop apps.
    Blocked,
    /// It sends pictures this build cannot read.
    Unreadable,
    /// No camera input on this system.
    Unsupported,
    /// Anything else.
    Other,
}

/// Phrases and codes, lower case, that point at each cause. Checked in
/// this order, so the more specific causes come first.
const PHRASES: &[(Cause, &[&str])] = &[
    (
        Cause::Blocked,
        &[
            "access is denied",
            "access denied",
            "0x80070005",
            "e_accessdenied",
            "privacy",
        ],
    ),
    (
        Cause::Busy,
        &[
            "in use",
            "being used",
            "preempted",
            "0x800700aa",
            "0xc00d3ea3",
            "sharing violation",
            "resource is busy",
        ],
    ),
    (
        Cause::Unplugged,
        &[
            "invalidated",
            "0xc00d3ea2",
            "not connected",
            "0x8007048f",
            "no longer available",
            "removed",
            "stopped sending",
            "end of stream",
        ],
    ),
    (
        Cause::NotFound,
        &[
            "no device",
            "not found",
            "could not find",
            "no such",
            "0x80070002",
        ],
    ),
];

impl Cause {
    /// The cause a backend's error text most likely means.
    pub fn from_text(text: &str) -> Self {
        let text = text.to_lowercase();
        PHRASES
            .iter()
            .find(|(_, phrases)| phrases.iter().any(|phrase| text.contains(phrase)))
            .map_or(Self::Other, |(cause, _)| *cause)
    }

    /// What the person is told happens and what to do, for the failure
    /// record's action line.
    pub fn action(self) -> &'static str {
        match self {
            Self::NotFound => {
                "syn_Media is black until the camera is plugged in; check that it is connected and \
                 listed under Media > Camera"
            }
            Self::Unplugged => {
                "syn_Media is black; the camera was probably unplugged, and it comes back by itself \
                 when it is plugged in again"
            }
            Self::Stalled => {
                "syn_Media is black; the camera stopped sending pictures (a driver or USB fault), \
                 and Strobe keeps reopening it; replugging it usually helps"
            }
            Self::Busy => {
                "syn_Media is black; another app (Teams, Zoom, the Camera app) is probably using \
                 the camera, and it comes back by itself once that app lets it go"
            }
            Self::Blocked => {
                "syn_Media is black; Windows' camera privacy setting probably blocks desktop apps: \
                 Settings > Privacy > Camera, allow desktop apps to access your camera"
            }
            Self::Unreadable => {
                "syn_Media is black; the camera sends pictures Strobe cannot read, so try another \
                 camera"
            }
            Self::Unsupported => {
                "syn_Media is black; this build has no camera input on this system"
            }
            Self::Other => {
                "syn_Media is black, and Strobe keeps trying the camera; replugging it or closing \
                 other apps that use it may help"
            }
        }
    }

    /// A short name for logs.
    pub fn label(self) -> &'static str {
        match self {
            Self::NotFound => "not found",
            Self::Unplugged => "unplugged",
            Self::Stalled => "stalled",
            Self::Busy => "in use",
            Self::Blocked => "blocked",
            Self::Unreadable => "unreadable",
            Self::Unsupported => "unsupported",
            Self::Other => "failed",
        }
    }
}

/// A camera that stopped, or never started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The camera's name.
    pub camera: String,
    /// The likely cause.
    pub cause: Cause,
    /// The backend's own words.
    pub detail: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "camera \"{}\" {}: {}",
            self.camera,
            self.cause.label(),
            self.detail
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wording_picks_the_cause() {
        let cases = [
            ("Access is denied. (0x80070005)", Cause::Blocked),
            (
                "The video recording device is in use by another app",
                Cause::Busy,
            ),
            ("HRESULT 0xC00D3EA2 device invalidated", Cause::Unplugged),
            (
                "the camera stopped sending (stream flags 0x2)",
                Cause::Unplugged,
            ),
            ("Could not find device: No Device", Cause::NotFound),
            ("Something odd", Cause::Other),
        ];
        for (text, cause) in cases {
            assert_eq!(Cause::from_text(text), cause, "{text}");
        }
    }

    #[test]
    fn every_action_says_the_picture_is_black() {
        let causes = [
            Cause::NotFound,
            Cause::Unplugged,
            Cause::Stalled,
            Cause::Busy,
            Cause::Blocked,
            Cause::Unreadable,
            Cause::Unsupported,
            Cause::Other,
        ];
        for cause in causes {
            assert!(
                cause.action().starts_with("syn_Media is black"),
                "{cause:?}: {}",
                cause.action()
            );
        }
    }
}
