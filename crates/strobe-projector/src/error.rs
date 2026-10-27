// ============================================================================
// Module:       strobe_projector::error
// Description:  What can go wrong talking to a projector: a refused
//               connection, no usable answer, a PJLink or NTCONTROL error
//               code, or a sequence that did not finish.
//
// Dependencies: thiserror
// ============================================================================

//! Errors.
//!
//! The kinds matter, not only the words: the retry policy retries
//! [`Error::Refused`] and PJLink's ERR3, and the NTCONTROL fallback takes
//! over on [`Error::Transport`], [`Error::Refused`] and PJLink's ERRA and
//! ERR1 ([`Error::pjlink_unusable`]).

/// A PJLink error code (the PJLink Class 1 specification, section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PjLinkCode {
    /// Authentication failed: the password is wrong.
    Erra,
    /// Undefined command: the unit does not support it.
    Err1,
    /// Out of parameter: an invalid argument.
    Err2,
    /// Unavailable time: the unit is warming up or cooling down.
    Err3,
    /// Projector failure: a hardware fault.
    Err4,
}

impl PjLinkCode {
    /// Every code.
    pub const ALL: [Self; 5] = [Self::Erra, Self::Err1, Self::Err2, Self::Err3, Self::Err4];

    /// The code as the projector sends it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Erra => "ERRA",
            Self::Err1 => "ERR1",
            Self::Err2 => "ERR2",
            Self::Err3 => "ERR3",
            Self::Err4 => "ERR4",
        }
    }

    /// The code `text` spells, if it is one.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.as_str() == text)
    }

    /// What the code means, in words an operator can act on.
    pub const fn meaning(self) -> &'static str {
        match self {
            Self::Erra => "authentication failed (wrong PJLink password)",
            Self::Err1 => "undefined command (projector does not support it)",
            Self::Err2 => "out of parameter (invalid argument)",
            Self::Err3 => "unavailable time (projector is warming up or cooling down)",
            Self::Err4 => "projector failure (hardware fault, check the unit)",
        }
    }
}

/// Any failure talking to a projector.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Nothing listens on the port (the unit is booting, or its network
    /// control is off).
    #[error("{0}: connection refused")]
    Refused(String),
    /// No connection, no answer in time, or an answer that is not the
    /// protocol.
    #[error("{0}")]
    Transport(String),
    /// The unit answered with a PJLink error code.
    #[error("{command}: {} = {}", code.as_str(), code.meaning())]
    PjLink {
        /// The code.
        code: PjLinkCode,
        /// The command that drew it.
        command: String,
    },
    /// The unit answered with an NTCONTROL error.
    #[error("NTCONTROL {command}: {code} = {meaning}")]
    NtControl {
        /// The code, as sent (`ER401`, `ERRA`, ...).
        code: &'static str,
        /// What it means.
        meaning: &'static str,
        /// The command that drew it.
        command: String,
    },
    /// The sequence did not finish: a unit stuck warming, a state never
    /// reached, an answer that makes no sense.
    #[error("{0}")]
    Failed(String),
}

impl Error {
    /// The PJLink code, when this is one.
    pub const fn pjlink_code(&self) -> Option<PjLinkCode> {
        match self {
            Self::PjLink { code, .. } => Some(*code),
            Self::Refused(_) | Self::Transport(_) | Self::NtControl { .. } | Self::Failed(_) => {
                None
            }
        }
    }

    /// Whether this failure means PJLink does not work on the unit at all
    /// (as opposed to a state answer such as ERR3), so the NTCONTROL
    /// fallback should take over.
    pub const fn pjlink_unusable(&self) -> bool {
        match self {
            Self::PjLink { code, .. } => matches!(code, PjLinkCode::Erra | PjLinkCode::Err1),
            Self::Refused(_) | Self::Transport(_) => true,
            Self::NtControl { .. } | Self::Failed(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_message_naming_it() {
        for code in PjLinkCode::ALL {
            let error = Error::PjLink {
                code,
                command: "%1POWR".to_string(),
            };
            let text = error.to_string();
            assert!(text.contains(code.as_str()), "{text}");
            assert!(!text.contains("unknown"), "{text}");
            assert_eq!(PjLinkCode::parse(code.as_str()), Some(code), "round trip");
        }
        assert_eq!(PjLinkCode::parse("OK"), None, "OK is not an error");
    }

    #[test]
    fn only_a_broken_pjlink_falls_back() {
        let code = |code| Error::PjLink {
            code,
            command: String::new(),
        };
        assert!(code(PjLinkCode::Erra).pjlink_unusable(), "ERRA");
        assert!(code(PjLinkCode::Err1).pjlink_unusable(), "ERR1");
        assert!(!code(PjLinkCode::Err3).pjlink_unusable(), "ERR3 is a state");
        assert!(Error::Refused(String::new()).pjlink_unusable(), "refused");
        assert!(Error::Transport(String::new()).pjlink_unusable(), "silent");
        assert!(!Error::Failed(String::new()).pjlink_unusable(), "failed");
    }
}
