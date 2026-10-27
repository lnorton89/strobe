// ============================================================================
// Module:       strobe_projector::ntcontrol
// Description:  Panasonic's NTCONTROL protocol on TCP 1024, for power only:
//               the fallback when PJLink does not work on a Panasonic.
//
// Dependencies: crate::{error, wire}
// ============================================================================

//! NTCONTROL (Panasonic projectors' native network protocol).
//!
//! The unit greets with `NTCONTROL 0` or `NTCONTROL 1 <nonce>`. A command
//! is `00` followed by the command and CR, prefixed in the second case by
//! `md5("user:password:nonce")` in lowercase hex; the user and password are
//! the web interface's. The answer is `00` followed by the data, or an
//! error code ([`ERRORS`]).
//!
//! Only power is used: `QPW` (`000` standby, `001` on), `PON` and `POF`.
//! [`NtControl::power`] maps the answer to PJLink's `0` and `1` so the
//! power sequence reads both protocols the same way.

use crate::error::Error;
use crate::wire::{self, Greeting};

/// The NTCONTROL TCP port.
pub const PORT: u16 = 1024;

/// The default user of a Panasonic's web interface.
pub const DEFAULT_USER: &str = "admin1";

/// Error codes, checked in this order against the answer, with their
/// meanings.
pub const ERRORS: [(&str, &str); 8] = [
    ("ERRA", "authentication failed (web UI user/password)"),
    (
        "ER401",
        "command cannot be executed now (warming/cooling or wrong state)",
    ),
    ("ER402", "invalid parameter"),
    ("ERR1", "undefined command"),
    ("ERR2", "out of parameter"),
    ("ERR3", "busy"),
    ("ERR4", "timeout"),
    ("ERR5", "wrong data length"),
];

/// An NTCONTROL connection's settings. Holds no socket.
#[derive(Clone, Debug)]
pub struct NtControl {
    host: String,
    user: String,
    password: String,
    port: u16,
}

impl NtControl {
    /// A client for `host` on the NTCONTROL port.
    pub fn new(host: &str, user: &str, password: &str) -> Self {
        Self {
            host: host.to_string(),
            user: user.to_string(),
            password: password.to_string(),
            port: PORT,
        }
    }

    /// The same client on another port (a test's fake projector).
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sends `command` (`QPW`, `PON`, ...) and returns the data after `00`.
    pub fn raw(&self, command: &str) -> Result<String, Error> {
        let mut stream = wire::connect(&self.host, self.port)?;
        let greeting = wire::read_line(&mut stream)?;
        let prefix = match wire::greeting(&greeting, "NTCONTROL") {
            Some(Greeting::Open) => String::new(),
            Some(Greeting::Challenge(nonce)) => {
                wire::md5_hex(&format!("{}:{}:{nonce}", self.user, self.password))
            }
            None => {
                return Err(Error::Transport(format!(
                    "unexpected NTCONTROL greeting {greeting:?}"
                )));
            }
        };
        wire::send_line(&mut stream, &format!("{prefix}00{command}"))?;
        let answer = wire::read_line(&mut stream)?;
        drop(stream);
        parse_answer(command, &answer)
    }

    /// `QPW` as a PJLink power code: `1` on, `0` standby.
    pub fn power(&self) -> Result<String, Error> {
        let value = self.raw("QPW")?;
        power_code(&value)
    }

    /// `PON` or `POF`.
    pub fn set_power(&self, on: bool) -> Result<(), Error> {
        self.raw(if on { "PON" } else { "POF" }).map(drop)
    }
}

/// The data in `answer` to `command`, or its error.
fn parse_answer(command: &str, answer: &str) -> Result<String, Error> {
    if let Some((code, meaning)) = ERRORS.iter().find(|(code, _)| answer.contains(code)) {
        return Err(Error::NtControl {
            code,
            meaning,
            command: command.to_string(),
        });
    }
    Ok(answer.strip_prefix("00").unwrap_or(answer).to_string())
}

/// `QPW`'s data (`000`, `001`) as `0` or `1`.
fn power_code(value: &str) -> Result<String, Error> {
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return Err(Error::Failed(format!(
            "NTCONTROL QPW: unexpected reply {value:?}"
        )));
    }
    Ok(if digits.ends_with('1') { "1" } else { "0" }.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn qpw_maps_to_pjlink_codes() -> Result<()> {
        assert_eq!(power_code("001")?, "1", "on");
        assert_eq!(power_code("000")?, "0", "standby");
        assert!(power_code("").is_err(), "nothing");
        assert!(power_code("abc").is_err(), "no digits");
        Ok(())
    }

    #[test]
    fn errors_are_found_anywhere_in_the_answer() -> Result<()> {
        assert_eq!(parse_answer("QPW", "00001")?, "001", "data");
        assert!(parse_answer("PON", "00ER401").is_err(), "ER401");
        assert!(parse_answer("PON", "ERRA").is_err(), "ERRA");
        Ok(())
    }
}
