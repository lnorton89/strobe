// ============================================================================
// Module:       strobe_remote::pairing
// Description:  The short-lived pairing code shown on the desktop, its
//               attempt limit, and the requests waiting for the operator to
//               approve or deny them.
//
// Dependencies: crate::secret (random source, comparison); std::time
// ============================================================================

//! Pairing (L-03, D251).
//!
//! A phone is paired in two steps, and both need the person at the laptop:
//!
//! 1. **The code.** The operator opens pairing; the desktop shows a
//!    six-digit code from the OS random source, good for
//!    [`CODE_LIFETIME`] and for one successful use. The phone sends it in a
//!    `pair` message. A wrong code costs one of [`CODE_ATTEMPTS`] shared by
//!    every connection, and the last one burns the code, so guessing is
//!    `5 / 1 000 000` per code the operator chooses to open. An expired,
//!    burned or used code is gone; a replay of it is refused like a wrong
//!    one.
//! 2. **The approval.** A right code does not pair anything. It makes a
//!    [`Request`] with the phone's address and kind, which the desktop
//!    lists with Approve and Deny. Only Approve issues a token. A request
//!    nobody decides expires after [`APPROVAL_WAIT`].
//!
//! A desktop controller may skip the code (D468): [`Pairing::request`]
//! puts it straight in front of the operator. What the code bounded (who
//! can ask at all) is bounded instead by [`MAX_REQUESTS`], one waiting
//! request per address, and [`DENY_COOLDOWN`] after a Deny, so a machine on
//! the LAN can neither fill the list nor ask again the moment it is told
//! no. The request names the connection's real address, which a TCP peer
//! cannot fake.
//!
//! Nothing here persists: a restart forgets the code and every request,
//! which is the safe direction. Paired clients persist in
//! [`crate::clients`].

use crate::secret;
use std::fmt;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// How long a pairing code is good for.
pub const CODE_LIFETIME: Duration = Duration::from_secs(120);

/// Wrong codes allowed, across every connection, before the code is burned.
pub const CODE_ATTEMPTS: u32 = 5;

/// How long a request waits for the operator before it expires.
pub const APPROVAL_WAIT: Duration = Duration::from_secs(90);

/// Requests waiting at once; a phone beyond this is refused.
pub const MAX_REQUESTS: usize = 4;

/// How long an address that was denied waits before it may ask again
/// without a code (D468).
pub const DENY_COOLDOWN: Duration = Duration::from_secs(30);

/// Digits in a code.
const DIGITS: usize = 6;

/// A request's state as the connection waiting on it sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The operator has not answered.
    Waiting,
    /// Approved: the connection may issue a token.
    Approved,
    /// Denied on the desktop.
    Denied,
    /// Not answered in time, or forgotten.
    Expired,
}

/// Why a code was refused. Each is said to the phone as `pairing_rejected`;
/// the variants are for the desktop's log and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Pairing is not open on the desktop: never opened, closed, expired,
    /// burned, or used by another phone already.
    NotOpen,
    /// Wrong; this many attempts remain on the code.
    Wrong(u32),
    /// Wrong, and that was the last attempt: the code is burned.
    Burned,
    /// Too many phones are already waiting for approval.
    Full,
    /// This address already has a request waiting (D468).
    Pending,
    /// This address was denied within [`DENY_COOLDOWN`] (D468).
    Cooling,
}

/// A phone that sent the right code, waiting for the operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Its id, for Approve and Deny.
    pub id: u64,
    /// What the desktop shows: the phone's kind and address.
    pub label: String,
    /// When it arrived.
    pub since: Instant,
    /// Where it stands.
    pub decision: Decision,
}

/// A code on the desktop.
struct Code {
    digits: String,
    expires: Instant,
    attempts_left: u32,
}

/// The pairing state the listener and the desktop share.
#[derive(Default)]
pub struct Pairing {
    code: Option<Code>,
    requests: Vec<Request>,
    next_id: u64,
    /// The address each code-free request came from (D468).
    origins: Vec<(u64, IpAddr)>,
    /// Addresses denied, and when, for [`DENY_COOLDOWN`].
    denied: Vec<(IpAddr, Instant)>,
}

impl fmt::Debug for Pairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pairing")
            .field("code", &self.code.as_ref().map(|_| "***"))
            .field("requests", &self.requests)
            .finish()
    }
}

/// A uniformly random code of [`DIGITS`] digits.
fn fresh_code() -> std::io::Result<String> {
    /// The largest multiple of 10^6 below 2^32: values at or above it are
    /// drawn again, so every code is equally likely.
    const LIMIT: u32 = u32::MAX - u32::MAX % 1_000_000;
    loop {
        let value = u32::from_le_bytes(secret::random::<4>()?);
        if value < LIMIT {
            return Ok(format!("{:06}", value % 1_000_000));
        }
    }
}

/// The digits of what a person typed: spaces and dashes are allowed
/// between them.
fn digits_of(typed: &str) -> Option<String> {
    let digits: String = typed.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    (digits.len() == DIGITS && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

impl Pairing {
    /// Opens pairing with a fresh code, replacing any open one. Returns the
    /// code for the desktop, as `123 456`.
    pub fn open(&mut self, now: Instant) -> std::io::Result<String> {
        let digits = fresh_code()?;
        let shown = spaced(&digits);
        self.code = Some(Code {
            digits,
            expires: now + CODE_LIFETIME,
            attempts_left: CODE_ATTEMPTS,
        });
        Ok(shown)
    }

    /// Closes pairing: the code is gone. Waiting requests stay for the
    /// operator to decide.
    pub fn close(&mut self) {
        self.code = None;
    }

    /// The open code, as `123 456`, and how long it has left.
    pub fn code(&mut self, now: Instant) -> Option<(String, Duration)> {
        self.expire(now);
        self.code.as_ref().map(|code| {
            (
                spaced(&code.digits),
                code.expires.saturating_duration_since(now),
            )
        })
    }

    /// Drops an expired code and expires undecided requests.
    fn expire(&mut self, now: Instant) {
        if self.code.as_ref().is_some_and(|code| now >= code.expires) {
            self.code = None;
        }
        for request in &mut self.requests {
            if request.decision == Decision::Waiting
                && now.saturating_duration_since(request.since) >= APPROVAL_WAIT
            {
                request.decision = Decision::Expired;
            }
        }
    }

    /// Checks a code a phone sent. Right: the code is used up and a request
    /// labelled `label` waits for the operator; its id is returned.
    pub fn try_code(&mut self, typed: &str, label: String, now: Instant) -> Result<u64, Refusal> {
        self.expire(now);
        let Some(code) = self.code.as_mut() else {
            return Err(Refusal::NotOpen);
        };
        let right = digits_of(typed)
            .is_some_and(|digits| secret::same(digits.as_bytes(), code.digits.as_bytes()));
        if !right {
            code.attempts_left = code.attempts_left.saturating_sub(1);
            if code.attempts_left == 0 {
                self.code = None;
                return Err(Refusal::Burned);
            }
            return Err(Refusal::Wrong(code.attempts_left));
        }
        if self.waiting_count() >= MAX_REQUESTS {
            return Err(Refusal::Full);
        }
        // One use: the next phone needs a new code.
        self.code = None;
        // An answered request stays until its connection reads it and
        // forgets it; an expired one reads the same whether kept or not.
        self.requests.retain(|r| r.decision != Decision::Expired);
        self.next_id += 1;
        let id = self.next_id;
        self.requests.push(Request {
            id,
            label,
            since: now,
            decision: Decision::Waiting,
        });
        Ok(id)
    }

    /// A desktop's request with no code (D468): labelled `label`, from
    /// `origin`, waiting for the operator; its id is returned. Refused when
    /// the list is full, when `origin` already waits, or when `origin` was
    /// denied within [`DENY_COOLDOWN`].
    pub fn request(&mut self, label: String, origin: IpAddr, now: Instant) -> Result<u64, Refusal> {
        self.expire(now);
        self.denied
            .retain(|(_, at)| now.saturating_duration_since(*at) < DENY_COOLDOWN);
        if self.denied.iter().any(|(ip, _)| *ip == origin) {
            return Err(Refusal::Cooling);
        }
        let waiting_here = self.origins.iter().any(|(id, ip)| {
            *ip == origin
                && self
                    .requests
                    .iter()
                    .any(|r| r.id == *id && r.decision == Decision::Waiting)
        });
        if waiting_here {
            return Err(Refusal::Pending);
        }
        if self.waiting_count() >= MAX_REQUESTS {
            return Err(Refusal::Full);
        }
        self.requests.retain(|r| r.decision != Decision::Expired);
        self.next_id += 1;
        let id = self.next_id;
        self.requests.push(Request {
            id,
            label,
            since: now,
            decision: Decision::Waiting,
        });
        self.origins.push((id, origin));
        Ok(id)
    }

    fn waiting_count(&self) -> usize {
        self.requests
            .iter()
            .filter(|r| r.decision == Decision::Waiting)
            .count()
    }

    /// The operator's answer to request `id`. Returns whether it was
    /// waiting.
    pub fn decide(&mut self, id: u64, approve: bool, now: Instant) -> bool {
        self.expire(now);
        let Some(request) = self
            .requests
            .iter_mut()
            .find(|r| r.id == id && r.decision == Decision::Waiting)
        else {
            return false;
        };
        request.decision = if approve {
            Decision::Approved
        } else {
            Decision::Denied
        };
        if !approve && let Some((_, ip)) = self.origins.iter().find(|(origin, _)| *origin == id) {
            self.denied.push((*ip, now));
        }
        true
    }

    /// Where request `id` stands. A request that is not known is expired.
    pub fn decision(&mut self, id: u64, now: Instant) -> Decision {
        self.expire(now);
        self.requests
            .iter()
            .find(|r| r.id == id)
            .map_or(Decision::Expired, |r| r.decision)
    }

    /// Forgets request `id`: its connection has its answer, or closed.
    pub fn forget(&mut self, id: u64) {
        self.requests.retain(|r| r.id != id);
        self.origins.retain(|(origin, _)| *origin != id);
    }

    /// The requests waiting for the operator, oldest first.
    pub fn waiting(&mut self, now: Instant) -> Vec<Request> {
        self.expire(now);
        self.requests
            .iter()
            .filter(|r| r.decision == Decision::Waiting)
            .cloned()
            .collect()
    }
}

/// `123456` as `123 456`.
fn spaced(digits: &str) -> String {
    let (a, b) = digits.split_at(digits.len() / 2);
    format!("{a} {b}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn opened() -> Result<(Pairing, String, Instant)> {
        let mut pairing = Pairing::default();
        let now = Instant::now();
        let code = pairing.open(now)?;
        Ok((pairing, code, now))
    }

    #[test]
    fn a_code_is_six_digits_shown_in_two_halves() -> Result<()> {
        let (mut pairing, code, now) = opened()?;
        assert_eq!(code.len(), 7, "`123 456`: {code}");
        assert!(digits_of(&code).is_some(), "its digits read back");
        let (shown, left) = pairing
            .code(now)
            .ok_or_else(|| anyhow::anyhow!("no code"))?;
        assert_eq!(shown, code, "the desktop shows it");
        assert_eq!(left, CODE_LIFETIME, "for its whole lifetime");
        Ok(())
    }

    #[test]
    fn the_right_code_waits_for_approval_and_is_used_up() -> Result<()> {
        let (mut pairing, code, now) = opened()?;
        let id = pairing
            .try_code(&code.replace(' ', "-"), "iPhone at 10.0.0.7".into(), now)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(pairing.decision(id, now), Decision::Waiting, "waiting");
        assert_eq!(pairing.waiting(now).len(), 1, "listed for the operator");
        assert_eq!(
            pairing.try_code(&code, "replay".into(), now),
            Err(Refusal::NotOpen),
            "a replayed code finds nothing open"
        );
        assert!(pairing.decide(id, true, now), "approve");
        assert_eq!(pairing.decision(id, now), Decision::Approved, "approved");
        assert!(!pairing.decide(id, false, now), "decided once only");
        pairing.forget(id);
        assert_eq!(pairing.decision(id, now), Decision::Expired, "forgotten");
        Ok(())
    }

    #[test]
    fn a_denied_request_is_denied() -> Result<()> {
        let (mut pairing, code, now) = opened()?;
        let id = pairing
            .try_code(&code, "x".into(), now)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert!(pairing.decide(id, false, now), "deny");
        assert_eq!(pairing.decision(id, now), Decision::Denied, "denied");
        Ok(())
    }

    #[test]
    fn an_expired_code_and_an_undecided_request_expire() -> Result<()> {
        let (mut pairing, code, now) = opened()?;
        let late = now + CODE_LIFETIME;
        assert_eq!(
            pairing.try_code(&code, "x".into(), late),
            Err(Refusal::NotOpen),
            "an expired code is gone"
        );
        assert!(pairing.code(late).is_none(), "and not shown");
        let code = pairing.open(now)?;
        let id = pairing
            .try_code(&code, "x".into(), now)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(
            pairing.decision(id, now + APPROVAL_WAIT),
            Decision::Expired,
            "nobody approved it in time"
        );
        assert!(
            !pairing.decide(id, true, now + APPROVAL_WAIT),
            "too late to approve"
        );
        Ok(())
    }

    #[test]
    fn guessing_burns_the_code_after_the_attempt_limit() -> Result<()> {
        let (mut pairing, code, now) = opened()?;
        let wrong = if code.starts_with('0') {
            "111 111"
        } else {
            "000 000"
        };
        for left in (1..CODE_ATTEMPTS).rev() {
            assert_eq!(
                pairing.try_code(wrong, "x".into(), now),
                Err(Refusal::Wrong(left)),
                "attempt with {left} left"
            );
        }
        assert_eq!(
            pairing.try_code(wrong, "x".into(), now),
            Err(Refusal::Burned),
            "the last wrong guess burns it"
        );
        assert_eq!(
            pairing.try_code(&code, "x".into(), now),
            Err(Refusal::NotOpen),
            "and the right code is no use after that"
        );
        for junk in ["", "12345", "1234567", "abcdef", "١٢٣٤٥٦"] {
            let mut pairing = Pairing::default();
            pairing.open(now)?;
            assert!(
                matches!(
                    pairing.try_code(junk, "x".into(), now),
                    Err(Refusal::Wrong(_))
                ),
                "junk {junk:?} is a wrong code"
            );
        }
        Ok(())
    }

    #[test]
    fn only_a_few_phones_can_wait_at_once() -> Result<()> {
        let mut pairing = Pairing::default();
        let now = Instant::now();
        for _ in 0..MAX_REQUESTS {
            let code = pairing.open(now)?;
            pairing
                .try_code(&code, "x".into(), now)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
        let code = pairing.open(now)?;
        assert_eq!(
            pairing.try_code(&code, "x".into(), now),
            Err(Refusal::Full),
            "one more is refused"
        );
        Ok(())
    }

    #[test]
    fn a_desktop_asks_without_a_code_once_at_a_time_and_waits_after_a_deny() -> Result<()> {
        let mut pairing = Pairing::default();
        let now = Instant::now();
        let laptop = IpAddr::from([192, 168, 1, 40]);
        let id = pairing
            .request("laptop".into(), laptop, now)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert_eq!(
            pairing.decision(id, now),
            Decision::Waiting,
            "in front of the operator"
        );
        assert_eq!(
            pairing.request("again".into(), laptop, now),
            Err(Refusal::Pending),
            "one waiting request per address"
        );
        assert!(pairing.decide(id, false, now), "denied");
        pairing.forget(id);
        assert_eq!(
            pairing.request("again".into(), laptop, now),
            Err(Refusal::Cooling),
            "a denied address waits"
        );
        assert!(
            pairing
                .request("later".into(), laptop, now + DENY_COOLDOWN)
                .is_ok(),
            "and may ask again after the cooldown"
        );
        let mut full = Pairing::default();
        for last in 1..=MAX_REQUESTS {
            let ip = IpAddr::from([10, 0, 0, u8::try_from(last)?]);
            full.request("x".into(), ip, now)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
        assert_eq!(
            full.request("x".into(), IpAddr::from([10, 0, 0, 99]), now),
            Err(Refusal::Full),
            "the list has a limit with or without codes"
        );
        Ok(())
    }

    #[test]
    fn codes_are_spread_and_never_print() -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            seen.insert(fresh_code()?);
        }
        assert!(
            seen.len() > 45,
            "fifty draws barely collide: {}",
            seen.len()
        );
        let (pairing, code, _) = opened()?;
        let printed = format!("{pairing:?}");
        assert!(
            !printed.contains(&code) && !printed.contains(&code.replace(' ', "")),
            "debug: {printed}"
        );
        Ok(())
    }
}
