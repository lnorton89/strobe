// ============================================================================
// Module:       strobe_app::trusted
// Description:  The controller side of venue mode: the proof a laptop makes
//               for a trust file, the retry schedule after a drop, and what
//               to say when the host refuses.
//
// Dependencies: strobe-remote (trust, ClientMessage, ErrorCode); std::time
// ============================================================================

//! Controlling a venue's show machine from a laptop (D294), the half of D288
//! that `venue` (the host's launch flags) does not cover.
//!
//! Three pieces, all pure so they are tested without a socket or a window:
//!
//! - [`Handshake`] makes the proof. It is a small state machine owned by one
//!   connection: it sends `trust_start` once, and answers only the challenge
//!   that follows. A challenge nobody asked for, or a second one, is refused,
//!   so a hostile host cannot use this controller to sign a nonce it got from
//!   somewhere else (a signing oracle).
//! - [`backoff`] is the retry schedule for a dropped trusted connection.
//! - [`refusal_text`] says what a refusal means and what to do about it.
//!
//! Nothing here logs or returns the secret, a nonce or a proof: the only
//! thing that leaves [`Handshake`] is the [`ClientMessage`] to send.

use std::path::Path;
use std::time::Duration;
use strobe_remote::trust::{self, TrustSecret};
use strobe_remote::{ClientMessage, ErrorCode};

/// The wait before the first retry; each further attempt doubles it.
const FIRST_RETRY: Duration = Duration::from_secs(1);

/// The longest wait between attempts. A four-hour show keeps trying at this
/// pace for as long as it takes the host to come back.
pub const MAX_RETRY: Duration = Duration::from_secs(15);

/// The wait before retry number `attempt` (1 for the first retry): 1 s, 2 s,
/// 4 s, 8 s, then [`MAX_RETRY`] for ever. Attempt 0 is treated as the first.
pub fn backoff(attempt: u32) -> Duration {
    // 2^4 = 16 s is already past the cap, so the shift stays tiny.
    let doublings = attempt.saturating_sub(1).min(4);
    FIRST_RETRY.saturating_mul(1 << doublings).min(MAX_RETRY)
}

/// Everything a worker needs to prove a trust file, and nothing printable.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The shared secret, read from the file when Connect was pressed.
    pub secret: TrustSecret,
    /// This laptop's stable id on the host's list.
    pub controller_id: String,
}

/// A typed or pasted path without the spaces around it or the quotes
/// Windows' "Copy as path" puts on it.
pub fn clean_path(path: &str) -> &str {
    path.trim().trim_matches('"').trim()
}

/// A typed path as [`clean_path`] leaves it, made whole against the folder
/// Strobe was started in, so the form shows, and remembers, the file it
/// will really read rather than a relative name a later launch from
/// another folder would look for elsewhere (D463). Empty stays empty.
pub fn whole_path(path: &str) -> String {
    let path = clean_path(path);
    if path.is_empty() {
        return String::new();
    }
    std::path::absolute(path).map_or_else(|_| path.to_string(), |whole| whole.display().to_string())
}

/// Reads the trust file at `path` for a connection as `controller_id`. The
/// error is a sentence for the form; it names the path, never the contents.
pub fn load_plan(path: &str, controller_id: &str) -> Result<Plan, String> {
    let path = clean_path(path);
    if path.is_empty() {
        return Err(
            "Enter the trust file's path: the file made with strobe --make-remote-trust.".into(),
        );
    }
    if !trust::valid_controller_id(controller_id) {
        return Err("This computer has no valid controller id; restart Strobe.".into());
    }
    let secret = trust::load_file(Path::new(path)).map_err(|error| error.to_string())?;
    Ok(Plan {
        secret,
        controller_id: controller_id.to_owned(),
    })
}

/// Why a challenge was not answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unanswered {
    /// No `trust_start` of ours is waiting for one: the host volunteered it
    /// (or this connection pairs by code), or it already answered one.
    Unsolicited,
    /// The nonce is not 64 hex digits.
    Malformed,
}

impl std::fmt::Display for Unanswered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsolicited => "the host sent a trust challenge nobody asked for",
            Self::Malformed => "the host sent a malformed trust challenge",
        })
    }
}

impl std::error::Error for Unanswered {}

/// Where a connection stands in the proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// `trust_start` not sent yet.
    Idle,
    /// Sent; the host's one challenge may arrive.
    Asked,
    /// The challenge was answered. No other is welcome.
    Answered,
}

/// One connection's proof of the trust file (D288's challenge-response).
///
/// Build one per connection, call [`start`](Self::start) when the host's
/// `hello` arrives and [`answer`](Self::answer) with each `trust_challenge`.
#[derive(Debug)]
pub struct Handshake {
    plan: Plan,
    /// The `Host` header this connection sent, bound into the proof.
    host: String,
    phase: Phase,
}

impl Handshake {
    /// A handshake for a connection whose `Host` header was `host`
    /// (`strobe_remote::client::host_header`).
    pub fn new(plan: Plan, host: String) -> Self {
        Self {
            plan,
            host,
            phase: Phase::Idle,
        }
    }

    /// The `trust_start` to send, once. `None` if it was already sent.
    pub fn start(&mut self) -> Option<ClientMessage> {
        if self.phase != Phase::Idle {
            return None;
        }
        self.phase = Phase::Asked;
        Some(ClientMessage::TrustStart {
            controller_id: self.plan.controller_id.clone(),
        })
    }

    /// The `trust_proof` for `nonce`, only if it answers the `trust_start`
    /// this connection sent and has not been answered already.
    pub fn answer(&mut self, nonce: &str) -> Result<ClientMessage, Unanswered> {
        if self.phase != Phase::Asked {
            return Err(Unanswered::Unsolicited);
        }
        let response = self
            .plan
            .secret
            .respond(nonce, &self.host, &self.plan.controller_id)
            .ok_or(Unanswered::Malformed)?;
        // One challenge, one answer: a second never gets a proof, right or
        // wrong.
        self.phase = Phase::Answered;
        Ok(ClientMessage::TrustProof { response })
    }
}

/// Whether a refusal before the session starts is the host asking for
/// patience rather than saying no. Only a paused peer is: the host answers
/// `busy` while it slows failed proofs, and the pause ends by itself.
pub fn is_transient(code: ErrorCode) -> bool {
    code == ErrorCode::Busy
}

/// What the viewer says when the host refuses a trusted connection. These
/// stop the retries, so each one says what to change.
pub fn refusal_text(code: ErrorCode, host_message: &str) -> String {
    const START: &str = "On the host, Strobe must be started with --remote \
                         --allow-desktop-control --remote-trust FILE.";
    match code {
        ErrorCode::PairingRejected => "The host does not accept this trust file. It was made for \
             another show machine, or the host was started with a different one. Copy the file \
             the host was started with to this computer, or make a new one with strobe \
             --make-remote-trust and relaunch both ends."
            .to_string(),
        ErrorCode::Unauthorized => "The host revoked this controller. It stays refused until \
             Strobe is restarted on the host (Settings > Remote > Revoke)."
            .to_string(),
        ErrorCode::Unavailable => format!(
            "The host does not offer trusted pairing right now ({host_message}). {START} Full \
             desktop control must also be on."
        ),
        ErrorCode::InvalidRequest | ErrorCode::InvalidState => format!(
            "The host refused the request ({host_message}). Connect to its own IP address and \
             port, as shown in its Settings > Remote."
        ),
        ErrorCode::VersionMismatch => format!(
            "The host speaks a different protocol version ({host_message}). Update Strobe on \
             both computers."
        ),
        ErrorCode::Busy => format!("The host is pausing attempts ({host_message})."),
    }
}

/// The text of the card while a trusted connection is down: why, and what
/// happens next. `attempt` is the retry about to be made, which waits
/// [`backoff`]`(attempt)`. Made once per attempt, never per frame.
pub fn retry_text(reason: &str, attempt: u32) -> String {
    let wait = backoff(attempt);
    let cap = MAX_RETRY.as_secs();
    if wait >= MAX_RETRY {
        format!(
            "{} Trying again in {cap} s (attempt {attempt}), and every {cap} s until you \
             press Disconnect.",
            reason.trim()
        )
    } else {
        format!(
            "{} Trying again in {} s (attempt {attempt}); the wait grows to {cap} s.",
            reason.trim(),
            wait.as_secs()
        )
    }
}

/// This computer's name for a controller id, from the environment or
/// `/etc/hostname`; empty if neither says.
pub fn computer_name() -> String {
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(name) = std::env::var(key)
            && !name.trim().is_empty()
        {
            return name;
        }
    }
    std::fs::read_to_string("/etc/hostname").unwrap_or_default()
}

/// A new controller id for this computer, or a fixed fallback if the
/// operating system has no random bytes (then the host cannot tell two such
/// laptops apart, which is why it is logged).
pub fn new_controller_id() -> String {
    match trust::new_controller_id(&computer_name()) {
        Ok(id) => id,
        Err(error) => {
            log::warn!("venue mode: {error}; using a fixed controller id");
            trust::controller_id_for(&computer_name(), [0; 4])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};

    /// 0x00..0x1f, the key the protocol vector in `strobe_remote::trust`
    /// was computed with (independently, in Python).
    fn fixed_plan() -> Result<Plan> {
        let text = format!(
            "{}\nsecret {}\n",
            trust::VERSION_LINE,
            (0..32).map(|b| format!("{b:02x}")).collect::<String>()
        );
        Ok(Plan {
            secret: TrustSecret::parse(&text)?,
            controller_id: "t480-1a2b3c4d".into(),
        })
    }

    const NONCE: &str = "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f";

    #[test]
    fn the_proof_is_the_protocols_published_vector() -> Result<()> {
        let mut handshake = Handshake::new(fixed_plan()?, "127.0.0.1:47812".into());
        let start = handshake.start().context("a trust_start")?;
        assert_eq!(
            serde_json::to_string(&start)?,
            r#"{"type":"trust_start","controller_id":"t480-1a2b3c4d"}"#,
            "the first message names this controller"
        );
        let proof = handshake.answer(NONCE)?;
        assert_eq!(
            serde_json::to_string(&proof)?,
            r#"{"type":"trust_proof","response":"7aea3b9895699f81047022e272038f94ed6995618c444749b1d73c6ce06e5d58"}"#,
            "byte for byte what the host's verify accepts for this nonce, host and id"
        );
        Ok(())
    }

    #[test]
    fn the_host_accepts_what_the_handshake_makes_and_only_for_the_host_dialled() -> Result<()> {
        use std::time::Instant;
        let plan = fixed_plan()?;
        let now = Instant::now();
        for (host, dialled, accepted) in [
            ("10.0.0.5:47812", "10.0.0.5:47812", true),
            ("10.0.0.5:47812", "10.0.0.6:47812", false),
            ("10.0.0.5:47812", "10.0.0.5:47813", false),
        ] {
            let challenge = trust::Challenge::issue(host, &plan.controller_id, now)?;
            let nonce = challenge.nonce_hex();
            let mut handshake = Handshake::new(plan.clone(), dialled.into());
            handshake.start();
            let ClientMessage::TrustProof { response } = handshake.answer(&nonce)? else {
                anyhow::bail!("answer must be a proof");
            };
            let verdict = challenge.verify(&plan.secret, &response, now);
            assert_eq!(
                verdict == trust::Verdict::Accepted,
                accepted,
                "host {host}, controller dialled {dialled}: {verdict:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_challenge_nobody_asked_for_is_refused_and_so_is_a_second() -> Result<()> {
        let mut handshake = Handshake::new(fixed_plan()?, "10.0.0.5:47812".into());
        assert_eq!(
            handshake.answer(NONCE).err(),
            Some(Unanswered::Unsolicited),
            "before trust_start there is nothing to answer"
        );
        handshake.start().context("start")?;
        assert!(handshake.start().is_none(), "trust_start goes out once");
        handshake.answer(NONCE)?;
        assert_eq!(
            handshake.answer(NONCE).err(),
            Some(Unanswered::Unsolicited),
            "the same nonce again"
        );
        assert_eq!(
            handshake.answer(&"a".repeat(64)).err(),
            Some(Unanswered::Unsolicited),
            "or any other"
        );
        Ok(())
    }

    #[test]
    fn a_malformed_nonce_is_not_signed_and_does_not_use_up_the_answer() -> Result<()> {
        let mut handshake = Handshake::new(fixed_plan()?, "10.0.0.5:47812".into());
        handshake.start().context("start")?;
        for bad in ["", "abcd", &"g".repeat(64), &"a".repeat(65)] {
            assert_eq!(
                handshake.answer(bad).err(),
                Some(Unanswered::Malformed),
                "{bad:?}"
            );
        }
        handshake.answer(NONCE)?;
        Ok(())
    }

    #[test]
    fn nothing_printable_holds_the_secret() -> Result<()> {
        let handshake = Handshake::new(fixed_plan()?, "10.0.0.5:47812".into());
        let shown = format!("{handshake:?}");
        assert!(shown.contains("TrustSecret(***)"), "{shown}");
        assert!(
            !shown.contains("000102030405"),
            "the secret's hex is not in the debug text: {shown}"
        );
        Ok(())
    }

    #[test]
    fn the_retry_text_says_the_real_wait_and_attempt() {
        let first = retry_text("Connection lost. ", 1);
        assert!(
            first.contains("Trying again in 1 s (attempt 1); the wait grows to 15 s."),
            "{first}"
        );
        let third = retry_text("Connection lost.", 3);
        assert!(third.contains("in 4 s (attempt 3)"), "{third}");
        let capped = retry_text("Connection lost.", 9);
        assert!(
            capped.contains("in 15 s (attempt 9), and every 15 s until you press Disconnect"),
            "{capped}"
        );
        assert!(
            !capped.contains("about every"),
            "the old, untrue wording is gone: {capped}"
        );
    }

    #[test]
    fn the_retry_schedule_doubles_to_fifteen_seconds_and_stays_there() {
        let seconds: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(
            seconds,
            [1, 2, 4, 8, 15, 15, 15, 15],
            "1 s, 2 s, 4 s ... 15 s"
        );
        assert_eq!(backoff(0), backoff(1), "attempt 0 is the first");
        assert_eq!(backoff(u32::MAX), MAX_RETRY, "for ever, without overflow");
    }

    #[test]
    fn only_a_paused_host_is_worth_retrying() {
        for code in [
            ErrorCode::PairingRejected,
            ErrorCode::Unauthorized,
            ErrorCode::Unavailable,
            ErrorCode::InvalidRequest,
            ErrorCode::VersionMismatch,
        ] {
            assert!(!is_transient(code), "{code:?} stops the retries");
            assert!(
                !refusal_text(code, "host words").is_empty(),
                "{code:?} says why"
            );
        }
        assert!(is_transient(ErrorCode::Busy), "busy ends by itself");
    }

    #[test]
    fn each_refusal_says_what_to_change() {
        let wrong = refusal_text(ErrorCode::PairingRejected, "that trust file does not match");
        assert!(
            wrong.contains("trust file") && wrong.contains("--make-remote-trust"),
            "{wrong}"
        );
        let revoked = refusal_text(ErrorCode::Unauthorized, "revoked");
        assert!(
            revoked.contains("revoked") && revoked.contains("restarted"),
            "{revoked}"
        );
        let off = refusal_text(ErrorCode::Unavailable, "Trusted pairing is not available");
        assert!(
            off.contains("--allow-desktop-control") && off.contains("--remote-trust"),
            "{off}"
        );
    }

    #[test]
    fn a_controller_id_is_valid_and_survives_odd_computer_names() {
        for name in ["", "T480", "My Laptop", "über-rechner", &"x".repeat(200)] {
            let id = trust::controller_id_for(name, [1, 2, 3, 4]);
            assert!(trust::valid_controller_id(&id), "{name:?} -> {id:?}");
        }
        assert!(
            trust::valid_controller_id(&new_controller_id()),
            "the real one is valid too"
        );
    }

    #[test]
    fn a_missing_or_junk_trust_file_is_a_sentence_that_never_holds_a_secret() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-trusted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        let id = "laptop-00000000";
        let blank = load_plan("   ", id).err().context("blank")?;
        assert!(blank.contains("trust file"), "{blank}");
        let missing = load_plan(&dir.join("nope.trust").to_string_lossy(), id)
            .err()
            .context("missing")?;
        assert!(missing.contains("nope.trust"), "names the path: {missing}");
        assert!(
            load_plan("x", "bad id").is_err(),
            "an invalid id is refused"
        );
        let made = dir.join("ok.trust");
        trust::create_file(&made)?;
        let quoted = format!("  \"{}\"  ", made.display());
        assert!(
            load_plan(&quoted, id).is_ok(),
            "a path pasted with quotes and spaces still opens"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
