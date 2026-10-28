// ============================================================================
// Module:       strobe_app::venue
// Description:  Venue mode's launch-time pieces: `--make-remote-trust`, and
//               loading `--remote-trust` so a failure is reported without
//               stopping the show.
//
// Dependencies: strobe-remote (trust); strobe-diag (failure reports)
// ============================================================================

//! Venue mode (D288): control a headless show machine without anyone
//! clicking Approve at it.
//!
//! The trust file is made once with `strobe --make-remote-trust FILE`,
//! copied to the controlling laptop, and named at both ends with
//! `--remote-trust FILE`. The host reads it **once, at launch**; there is
//! no setting, button or saved state for it, and so nothing a remote
//! controller's UI pass could be asked to change (the same reasoning as
//! D275's `host_only`: the trust file widens who can reach the host, so it
//! is only ever set on the command line, at the host).
//!
//! A trust file that cannot be used must not stop a show that is about to
//! start with nobody at the machine: [`load`] reports it and the host runs
//! without trusted pairing (fail closed), as if the flag were absent.

use std::path::Path;
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_remote::trust::{self, TrustSecret};

/// What to tell the person who just made a trust file.
pub fn made_text(path: &Path) -> String {
    format!(
        "Trust file made: {path}\n\n\
         Whoever can read this file has full control of any Strobe that trusts it. Keep it in \
         your user profile, copy it only to the laptop you will control from, and never post \
         it.\n\n\
         On the show machine:\n  strobe --remote --allow-desktop-control --remote-trust \"{path}\"\n\n\
         Copy the file to the laptop you will control from, then on the laptop (HOST is the \
         show machine's address, shown in its Settings > Remote; the port is 47812 unless you \
         changed it):\n  strobe --control HOST:47812 --remote-trust \"PATH-TO-THE-COPY\"\n\n\
         It opens straight into the show machine's window and reconnects by itself if the \
         machine restarts or the network drops, until you press Disconnect. The same \
         option is in Settings > Control another Strobe > Use a trust file.\n\n\
         The secret is never sent over the network; a controller proves it holds the file.",
        path = path.display()
    )
}

/// `strobe --make-remote-trust FILE`: writes a new trust file, refusing to
/// replace one, and returns what to tell the person: what to do next, or
/// why it was not made. The caller says it, in a box where there is no
/// console; this does not, so a test of it puts nothing on the screen.
pub fn make(path: &Path) -> Result<String, String> {
    trust::create_file(path)
        .map(|_| made_text(path))
        .map_err(|error| error.to_string())
}

/// Reads the trust file named by `--remote-trust` for the host. A file that
/// cannot be used is reported (log, failure list) and the host starts
/// without trusted pairing; the show goes on.
pub fn load(path: &Path) -> Option<TrustSecret> {
    match trust::load_file(path) {
        Ok(secret) => {
            log::info!(
                "venue mode: trust file {} loaded; a controller that proves it may take full \
                 desktop control without approval while full desktop access is on",
                path.display()
            );
            Some(secret)
        }
        Err(error) => {
            failure::report(
                Failure::new(Subsystem::Io, "remote trust file")
                    .path(path)
                    .message(error.to_string())
                    .action("trusted pairing is off; every pairing needs approval at the host"),
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};

    fn scratch(tag: &str) -> Result<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("strobe-venue-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn making_a_file_works_once_and_never_overwrites() -> Result<()> {
        let dir = scratch("make")?;
        let path = dir.join("venue.trust");
        assert!(make(&path).is_ok(), "made");
        let first = std::fs::read(&path)?;
        assert!(make(&path).is_err(), "a second make is refused");
        assert_eq!(std::fs::read(&path)?, first, "and leaves the file alone");
        assert!(load(&path).is_some(), "what was made loads");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_bad_trust_file_is_reported_and_the_host_runs_without_it() -> Result<()> {
        let dir = scratch("bad")?;
        assert!(load(&dir.join("missing.trust")).is_none(), "missing");
        let path = dir.join("junk.trust");
        std::fs::write(&path, "not a trust file")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        assert!(load(&path).is_none(), "junk");
        let recent = failure::recent(8);
        assert!(
            recent
                .iter()
                .any(|f| f.to_string().contains("remote trust file")),
            "the failure was reported: {recent:?}"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_instructions_name_both_commands_and_the_file() -> Result<()> {
        let text = made_text(Path::new("C:/Users/me/venue.trust"));
        for needle in [
            "--remote --allow-desktop-control --remote-trust",
            "strobe --control HOST:47812 --remote-trust",
            "reconnects by itself",
            "venue.trust",
            "never sent",
        ] {
            assert!(text.contains(needle), "missing {needle}: {text}");
        }
        let _ = text.lines().next().context("a first line")?;
        Ok(())
    }
}
