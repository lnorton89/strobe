// ============================================================================
// Module:       strobe_projector::pjlink
// Description:  A PJLink Class 1 client: one TCP session per command, the
//               MD5 password prefix when the unit challenges, and the
//               answer or its error code.
//
// Dependencies: crate::{error, wire}
// ============================================================================

//! PJLink Class 1 (JBMIA's PJLink specification, version 1).
//!
//! The unit greets with `PJLINK 0` (no password) or `PJLINK 1 <random>`; in
//! the second case every command is prefixed with
//! `md5(random + password)` in lowercase hex. A command is `%1XXXX param`
//! and its answer `%1XXXX=value`, where the value may be one of the error
//! codes in [`PjLinkCode`](crate::error::PjLinkCode). A wrong password
//! answers `PJLINK ERRA`, at the greeting or after the command.
//!
//! One connection per command is the simplest thing that works on every
//! unit: some close the session after one command whatever the
//! specification allows.

use crate::error::{Error, PjLinkCode};
use crate::wire::{self, Greeting};

/// The PJLink TCP port.
pub const PORT: u16 = 4352;

/// The answer a unit gives to a wrong password.
const AUTH_FAILED: &str = "PJLINK ERRA";

/// A PJLink connection's settings. Holds no socket.
#[derive(Clone, Debug)]
pub struct PjLink {
    host: String,
    password: String,
    port: u16,
}

impl PjLink {
    /// A client for `host` on the PJLink port. An empty password means none.
    pub fn new(host: &str, password: &str) -> Self {
        Self {
            host: host.to_string(),
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

    /// Sends `body` (`%1POWR ?`, or `POWR ?` with the `%1` added) and returns
    /// the value after `=`. An error code comes back as [`Error::PjLink`].
    pub fn raw(&self, body: &str) -> Result<String, Error> {
        let command = if body.starts_with("%1") {
            body.to_string()
        } else {
            format!("%1{body}")
        };
        let mut stream = wire::connect(&self.host, self.port)?;
        let greeting = wire::read_line(&mut stream)?;
        if greeting == AUTH_FAILED {
            return Err(auth_failed(&command));
        }
        let prefix = match wire::greeting(&greeting, "PJLINK") {
            Some(Greeting::Open) => String::new(),
            Some(Greeting::Challenge(random)) => {
                wire::md5_hex(&format!("{random}{}", self.password))
            }
            None => {
                return Err(Error::Transport(format!(
                    "unexpected greeting {greeting:?} (not a PJLink device?)"
                )));
            }
        };
        wire::send_line(&mut stream, &format!("{prefix}{command}"))?;
        let answer = wire::read_line(&mut stream)?;
        drop(stream);
        parse_answer(&command, &answer)
    }

    /// `POWR ?`: `0` off, `1` on, `2` cooling, `3` warming.
    pub fn power(&self) -> Result<String, Error> {
        self.raw("%1POWR ?")
    }

    /// `POWR 1` or `POWR 0`; anything but `OK` is a failure.
    pub fn set_power(&self, on: bool) -> Result<(), Error> {
        let value = self.raw(if on { "%1POWR 1" } else { "%1POWR 0" })?;
        if value == "OK" {
            Ok(())
        } else {
            Err(Error::Failed(format!("POWR set returned {value:?}")))
        }
    }

    /// `NAME ?`: the unit's name.
    pub fn name(&self) -> Result<String, Error> {
        self.raw("%1NAME ?")
    }

    /// `INF1 ?`: the manufacturer.
    pub fn inf1(&self) -> Result<String, Error> {
        self.raw("%1INF1 ?")
    }

    /// `INF2 ?`: the model.
    pub fn inf2(&self) -> Result<String, Error> {
        self.raw("%1INF2 ?")
    }

    /// `ERST ?`: six error-status digits.
    pub fn erst(&self) -> Result<String, Error> {
        self.raw("%1ERST ?")
    }

    /// `LAMP ?`: hours and on/off for each lamp.
    pub fn lamp(&self) -> Result<String, Error> {
        self.raw("%1LAMP ?")
    }

    /// `INPT ?`: the current input.
    pub fn input(&self) -> Result<String, Error> {
        self.raw("%1INPT ?")
    }
}

fn auth_failed(command: &str) -> Error {
    Error::PjLink {
        code: PjLinkCode::Erra,
        command: command.to_string(),
    }
}

/// The value in `answer` to `command`, or its error.
fn parse_answer(command: &str, answer: &str) -> Result<String, Error> {
    if answer == AUTH_FAILED {
        return Err(auth_failed(command));
    }
    // `%1POWR`: the class and the command, which the answer echoes.
    let head = command.get(..6).unwrap_or(command);
    let expected = format!("{head}=");
    let echoed = answer
        .get(..expected.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(&expected));
    if !echoed {
        return Err(Error::Transport(format!(
            "unexpected response {answer:?} to {command:?}"
        )));
    }
    let value = answer.get(expected.len()..).unwrap_or_default();
    match PjLinkCode::parse(value) {
        Some(code) => Err(Error::PjLink {
            code,
            command: command.to_string(),
        }),
        None => Ok(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, bail};

    #[test]
    fn an_answer_echoes_its_command() -> Result<()> {
        assert_eq!(parse_answer("%1POWR ?", "%1POWR=1")?, "1", "value");
        assert_eq!(parse_answer("%1POWR ?", "%1powr=1")?, "1", "any case");
        assert_eq!(
            parse_answer("%1LAMP ?", "%1LAMP=1234 1")?,
            "1234 1",
            "spaces"
        );
        assert!(parse_answer("%1POWR ?", "%1INPT=31").is_err(), "wrong echo");
        assert!(parse_answer("%1POWR ?", "garbage").is_err(), "garbage");
        Ok(())
    }

    #[test]
    fn error_codes_and_erra_are_errors() -> Result<()> {
        let Err(error) = parse_answer("%1POWR 1", "%1POWR=ERR3") else {
            bail!("ERR3 must be an error");
        };
        assert_eq!(error.pjlink_code(), Some(PjLinkCode::Err3), "{error}");
        let Err(error) = parse_answer("%1POWR ?", "PJLINK ERRA") else {
            bail!("ERRA must be an error");
        };
        assert_eq!(error.pjlink_code(), Some(PjLinkCode::Erra), "{error}");
        Ok(())
    }
}
