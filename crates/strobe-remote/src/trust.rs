// ============================================================================
// Module:       strobe_remote::trust
// Description:  Venue mode's shared secret: the trust file, the HMAC-SHA256
//               challenge-response that proves a controller holds it without
//               sending it, a single-use challenge, and the failed-attempt
//               limiter.
//
// Dependencies: hmac, sha2; crate::secret; std::fs, std::collections
// ============================================================================

//! Trusted pairing for an unattended host (D288).
//!
//! A venue's show machine has no screen at the console, so nobody can click
//! Approve. Instead the owner makes a **trust file** once
//! (`strobe --make-remote-trust FILE`: 32 random bytes from the operating
//! system, as hex), copies it to the controlling laptop, and starts the host
//! with `--remote-trust FILE`. A controller that proves it holds the same
//! secret is paired for full desktop control without an approval.
//!
//! ## The proof never carries the secret
//!
//! The link is plain HTTP, so everything on it is visible to the network.
//! The secret therefore never crosses it. The host sends a fresh random
//! 32-byte **nonce**; the controller answers with
//!
//! ```text
//! HMAC-SHA256(secret, field(context) || field(nonce) || field(host id)
//!                     || field(controller id))      field(x) = len16(x) || x
//! ```
//!
//! - *context* is [`CONTEXT`], so a proof made for something else (a later
//!   protocol, another feature that reuses the file) cannot be replayed here;
//! - *host id* is the `Host` header the controller dialled (`ip:port`), so a
//!   proof made for one host is no use at another address;
//! - *controller id* is the name the controller gave, so the host's list and
//!   its revocation refer to what was proved;
//! - every field is length-prefixed, so no two different tuples encode the
//!   same bytes.
//!
//! A [`Challenge`] is spent by [`Challenge::verify`], which takes it by value:
//! right or wrong, a nonce answers once. It also expires after
//! [`CHALLENGE_TTL`]. The comparison is the `hmac` crate's `verify_slice`,
//! which compares in constant time. A sniffer sees a nonce and a MAC bound
//! to it: it learns nothing it can reuse on the next connection, and
//! recovering a 256-bit key from HMAC outputs is not feasible.
//!
//! What this does not give: confidentiality (frames and input stay in the
//! clear), nor protection from an attacker who can take over an established
//! connection. See D288's limitations.
//!
//! ## Failed attempts
//!
//! [`Limiter`] counts failed proofs per peer address, with exponential
//! backoff after a few free attempts, and across all peers, so a flood from
//! many addresses is slowed too. Both are in memory and reset on restart.

use crate::secret;
use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use std::collections::HashMap;
use std::fmt;
use std::io::Read as _;
use std::net::IpAddr;
use std::path::Path;
use std::time::{Duration, Instant};

type HmacSha256 = Hmac<Sha256>;

/// The first meaningful line of a trust file: its format and version.
pub const VERSION_LINE: &str = "strobe-remote-trust 1";

/// Bytes in the secret.
pub const SECRET_BYTES: usize = 32;

/// Names the use of a proof; the first field of every MAC input.
pub const CONTEXT: &str = "strobe-remote trust v1: full desktop control";

/// How long a challenge may wait for its answer.
pub const CHALLENGE_TTL: Duration = Duration::from_secs(10);

/// The longest controller id, in bytes.
pub const MAX_CONTROLLER_ID: usize = 48;

/// Hex characters in a nonce (32 bytes) and in a proof (a SHA-256 tag).
pub const HEX_LEN: usize = 64;

/// A trust file is a few lines; anything bigger is not one.
const MAX_FILE_BYTES: u64 = 4096;

/// Fewer distinct byte values than this in 32 bytes is not random.
const MIN_DISTINCT: usize = 8;

/// Why a trust file could not be made or used. The text says what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustError(String);

impl TrustError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TrustError {}

/// The 32-byte secret in a trust file. It prints as `TrustSecret(***)`, has
/// no `Display` or `Serialize`, and is compared only through
/// [`Challenge::verify`].
///
/// The bytes are not wiped on drop: zeroing memory the compiler may elide
/// needs `unsafe` or a crate for it, and the same bytes are in the file on
/// disk. D288 records that.
#[derive(Clone)]
pub struct TrustSecret([u8; SECRET_BYTES]);

impl fmt::Debug for TrustSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TrustSecret(***)")
    }
}

/// Whether `id` is a usable controller id: 1 to [`MAX_CONTROLLER_ID`] of
/// letters, digits, `.`, `_` and `-`. Printable and free of anything a log
/// line or a list could be confused by.
pub fn valid_controller_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_CONTROLLER_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Hex digits of random suffix in a generated controller id.
const SUFFIX_HEX: usize = 8;

/// A controller id built from a computer's `name` and four random bytes:
/// `name-1a2b3c4d`. Anything outside `[A-Za-z0-9.]` in the name becomes
/// `-` and runs of `-` collapse, the name is cut so the whole id fits
/// [`MAX_CONTROLLER_ID`], and an empty name becomes `strobe`. The result
/// always satisfies [`valid_controller_id`]. Pure, so it is tested without
/// randomness; [`new_controller_id`] supplies the bytes.
pub fn controller_id_for(name: &str, suffix: [u8; 4]) -> String {
    let room = MAX_CONTROLLER_ID - SUFFIX_HEX - 1;
    let mut base = String::new();
    for ch in name.trim().chars() {
        let keep = if ch.is_ascii_alphanumeric() || ch == '.' {
            ch
        } else {
            '-'
        };
        if keep == '-' && (base.is_empty() || base.ends_with('-')) {
            continue;
        }
        if base.len() >= room {
            break;
        }
        base.push(keep);
    }
    let base = base.trim_end_matches('-');
    let base = if base.is_empty() { "strobe" } else { base };
    format!("{base}-{}", secret::hex(&suffix))
}

/// A fresh controller id for this computer, from its `name` and the
/// operating system's random source. Keep the result: a controller that
/// changes its id on every launch fills the host's list and cannot be
/// revoked by name.
pub fn new_controller_id(name: &str) -> Result<String, TrustError> {
    secret::random::<4>()
        .map(|suffix| controller_id_for(name, suffix))
        .map_err(|error| {
            TrustError::new(format!(
                "the operating system gave no random bytes ({error}); no controller id was made"
            ))
        })
}

/// Whether `text` is [`HEX_LEN`] hex digits: the shape of a nonce or proof.
pub fn valid_hex_block(text: &str) -> bool {
    text.len() == HEX_LEN && text.bytes().all(|b| b.is_ascii_hexdigit())
}

impl TrustSecret {
    /// A fresh secret from the operating system's random source.
    pub fn generate() -> Result<Self, TrustError> {
        secret::random::<SECRET_BYTES>().map(Self).map_err(|error| {
            TrustError::new(format!(
                "the operating system gave no random bytes ({error}); no trust file was made"
            ))
        })
    }

    /// The file's text: comments saying what it is, the version line, the
    /// secret as 64 hex digits.
    pub fn to_file_text(&self) -> String {
        format!(
            "# Strobe remote trust file (venue mode, D288).\n\
             # Whoever can read this file can fully control any Strobe that trusts it:\n\
             # scenes, files, quitting. Keep it private to your user account, copy it only\n\
             # to the laptop you control from, and never post it or commit it.\n\
             # Make a new one (and stop using this one) if it may have leaked.\n\
             {VERSION_LINE}\n\
             secret {}\n",
            secret::hex(&self.0)
        )
    }

    /// Reads a trust file's text. Blank lines and `#` comments are skipped;
    /// then comes exactly the version line and a `secret` line of 64 hex
    /// digits, and nothing else.
    pub fn parse(text: &str) -> Result<Self, TrustError> {
        let mut lines = text
            .trim_start_matches('\u{feff}')
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'));
        match lines.next() {
            Some(VERSION_LINE) => {}
            Some(_) => {
                return Err(TrustError::new(format!(
                    "not a Strobe trust file: its first line must be \"{VERSION_LINE}\" \
                     (a newer Strobe may have made it)"
                )));
            }
            None => return Err(TrustError::new("the trust file is empty")),
        }
        let value = lines
            .next()
            .and_then(|line| line.strip_prefix("secret"))
            .map(str::trim)
            .ok_or_else(|| TrustError::new("the trust file has no \"secret\" line"))?;
        if lines.next().is_some() {
            return Err(TrustError::new("the trust file has lines after the secret"));
        }
        let bytes = secret::unhex(value)
            .filter(|bytes| bytes.len() == SECRET_BYTES)
            .ok_or_else(|| {
                TrustError::new(format!(
                    "the secret must be {HEX_LEN} hex digits (32 bytes); make a file with \
                     --make-remote-trust rather than writing one"
                ))
            })?;
        let mut secret = [0_u8; SECRET_BYTES];
        secret.copy_from_slice(&bytes);
        let mut seen = [false; 256];
        for byte in secret {
            if let Some(slot) = seen.get_mut(usize::from(byte)) {
                *slot = true;
            }
        }
        if seen.iter().filter(|&&present| present).count() < MIN_DISTINCT {
            return Err(TrustError::new(
                "the secret does not look random; make a file with --make-remote-trust",
            ));
        }
        Ok(Self(secret))
    }

    /// The prepared MAC for these fields, or `None` if a field is too long
    /// for its 16-bit length (none that arrive here can be).
    fn mac(&self, nonce: &[u8], host_id: &str, controller_id: &str) -> Option<HmacSha256> {
        let mut mac = HmacSha256::new_from_slice(&self.0).ok()?;
        for field in [
            CONTEXT.as_bytes(),
            nonce,
            host_id.as_bytes(),
            controller_id.as_bytes(),
        ] {
            let length = u16::try_from(field.len()).ok()?;
            mac.update(&length.to_be_bytes());
            mac.update(field);
        }
        Some(mac)
    }

    /// The controller's answer to the host's `nonce_hex`: 64 hex digits, or
    /// `None` if the nonce is not 64 hex digits or a field is unusable.
    pub fn respond(&self, nonce_hex: &str, host_id: &str, controller_id: &str) -> Option<String> {
        if !valid_hex_block(nonce_hex) || !valid_controller_id(controller_id) {
            return None;
        }
        let nonce = secret::unhex(nonce_hex)?;
        let tag = self
            .mac(&nonce, host_id, controller_id)?
            .finalize()
            .into_bytes();
        Some(secret::hex(&tag))
    }
}

/// Makes a new trust file at `path`, which must not exist, and returns the
/// secret it holds.
///
/// `create_new` fails if anything is at the path, a symlink included, and
/// does not follow one, so an existing file is never overwritten and a
/// planted link is never written through. On Unix the file is created with
/// mode 0600. On Windows it takes the directory's access list: put it in
/// your user profile, where only you (and administrators) can read it. The
/// parent directory is not created.
pub fn create_file(path: &Path) -> Result<TrustSecret, TrustError> {
    use std::io::Write as _;
    let secret = TrustSecret::generate()?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        let kind = error.kind();
        let advice = if kind == std::io::ErrorKind::AlreadyExists {
            " (it already exists and is not overwritten; choose another name, or delete it \
             yourself if you mean to replace it)"
        } else if kind == std::io::ErrorKind::NotFound {
            " (is the folder there?)"
        } else if kind == std::io::ErrorKind::PermissionDenied {
            " (is the folder yours to write to?)"
        } else {
            ""
        };
        TrustError::new(format!("cannot create {}: {error}{advice}", path.display()))
    })?;
    file.write_all(secret.to_file_text().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            TrustError::new(format!(
                "cannot write {}: {error}; delete the partial file and try again",
                path.display()
            ))
        })?;
    Ok(secret)
}

/// Reads the trust file at `path`.
///
/// The file is opened once and checked through the open handle. At most
/// 4 KiB is read. On Unix a file that group or others can read is refused,
/// as `ssh` refuses a private key: fix it with `chmod 600`. On Windows there
/// is no such check; keep the file in your user profile.
pub fn load_file(path: &Path) -> Result<TrustSecret, TrustError> {
    let fail = |what: &str, error: &dyn fmt::Display| {
        TrustError::new(format!(
            "cannot {what} the trust file {}: {error}",
            path.display()
        ))
    };
    let file = std::fs::File::open(path).map_err(|error| fail("open", &error))?;
    let metadata = file.metadata().map_err(|error| fail("inspect", &error))?;
    if !metadata.is_file() {
        return Err(fail("use", &"it is not a regular file"));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(fail("use", &"it is too large to be one"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(fail(
                "use",
                &format!(
                    "other users can read it (mode {:03o}); run chmod 600 on it",
                    mode & 0o777
                ),
            ));
        }
    }
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| fail("read", &error))?;
    TrustSecret::parse(&text).map_err(|error| fail("use", &error))
}

/// How a challenge's answer was judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The controller holds the secret.
    Accepted,
    /// It does not (or sent something that is not a proof).
    Wrong,
    /// It answered after [`CHALLENGE_TTL`].
    Expired,
}

/// One outstanding challenge on one connection. [`Challenge::verify`] takes
/// it by value: a nonce can be answered once.
#[derive(Debug)]
pub struct Challenge {
    nonce: [u8; 32],
    host_id: String,
    controller_id: String,
    issued: Instant,
}

impl Challenge {
    /// A new challenge for `controller_id` at `host_id`, with a fresh nonce.
    pub fn issue(host_id: &str, controller_id: &str, now: Instant) -> std::io::Result<Self> {
        Ok(Self {
            nonce: secret::random::<32>()?,
            host_id: host_id.to_owned(),
            controller_id: controller_id.to_owned(),
            issued: now,
        })
    }

    /// The nonce as the `trust_challenge` message carries it.
    pub fn nonce_hex(&self) -> String {
        secret::hex(&self.nonce)
    }

    /// The controller id this challenge was issued to.
    pub fn controller_id(&self) -> &str {
        &self.controller_id
    }

    /// Judges `response_hex` against `secret` and spends the challenge.
    /// Expiry is decided first, so a late right answer is still refused.
    pub fn verify(self, secret: &TrustSecret, response_hex: &str, now: Instant) -> Verdict {
        if now.saturating_duration_since(self.issued) > CHALLENGE_TTL {
            return Verdict::Expired;
        }
        let Some(tag) = secret::unhex(response_hex) else {
            return Verdict::Wrong;
        };
        let Some(mac) = secret.mac(&self.nonce, &self.host_id, &self.controller_id) else {
            return Verdict::Wrong;
        };
        // `verify_slice` refuses a tag of the wrong length and compares the
        // rest in constant time.
        if mac.verify_slice(&tag).is_ok() {
            Verdict::Accepted
        } else {
            Verdict::Wrong
        }
    }
}

/// Failed proofs tolerated from one address before backoff starts.
const FREE_FAILURES: u32 = 3;
/// The first per-address backoff; each further failure doubles it.
const PEER_BACKOFF: Duration = Duration::from_secs(2);
/// The longest per-address backoff.
const PEER_BACKOFF_MAX: Duration = Duration::from_secs(300);
/// An address is forgotten this long after its last failure.
const PEER_FORGET: Duration = Duration::from_secs(600);
/// The window in which failures from all addresses are counted together.
const GLOBAL_WINDOW: Duration = Duration::from_secs(60);
/// Failures in that window that pause every address.
const GLOBAL_LIMIT: u32 = 20;
/// The first global pause; each repeat doubles it.
const GLOBAL_BACKOFF: Duration = Duration::from_secs(30);
/// The longest global pause.
const GLOBAL_BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Addresses remembered at once.
const MAX_PEERS: usize = 256;

/// What a recorded failure caused, for the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    /// Failures from this address since it last succeeded.
    pub failures: u32,
    /// The pause now imposed on this address, if any.
    pub peer_pause: Option<Duration>,
    /// The pause now imposed on every address, if this failure tripped it.
    pub global_pause: Option<Duration>,
}

#[derive(Debug)]
struct Peer {
    failures: u32,
    blocked_until: Option<Instant>,
    last: Instant,
}

/// `base` doubled `strikes - 1` times, at most `max`.
fn backoff(base: Duration, strikes: u32, max: Duration) -> Duration {
    let doublings = strikes.saturating_sub(1).min(16);
    base.saturating_mul(1_u32 << doublings).min(max)
}

/// Failed trusted-pairing attempts, per peer address and overall.
///
/// Brute-forcing a 256-bit secret is not the threat; the limiter exists so
/// that a misconfigured or hostile peer cannot make the host spend its time
/// on proofs or fill its log, and so a guess costs more the more of them
/// there are. The price is that a flood from the LAN also pauses the real
/// controller's *new* attempts for up to [`GLOBAL_BACKOFF`] (growing to five
/// minutes while it continues); a session already open is not affected, and
/// the controller keeps retrying with its own backoff.
///
/// It takes the time as an argument and never reads a clock, so tests drive
/// it with made-up instants.
#[derive(Debug, Default)]
pub struct Limiter {
    peers: HashMap<IpAddr, Peer>,
    window_start: Option<Instant>,
    window_failures: u32,
    global_until: Option<Instant>,
    global_strikes: u32,
}

impl Limiter {
    /// Whether `peer` may start a proof now; if not, how long to wait.
    pub fn admit(&self, peer: IpAddr, now: Instant) -> Result<(), Duration> {
        let waits = [
            self.global_until,
            self.peers
                .get(&peer.to_canonical())
                .and_then(|p| p.blocked_until),
        ];
        match waits
            .into_iter()
            .flatten()
            .filter(|until| *until > now)
            .max()
        {
            Some(until) => Err(until.saturating_duration_since(now)),
            None => Ok(()),
        }
    }

    /// Records a failed proof from `peer`.
    pub fn failure(&mut self, peer: IpAddr, now: Instant) -> Recorded {
        let entry = self.peers.entry(peer.to_canonical()).or_insert(Peer {
            failures: 0,
            blocked_until: None,
            last: now,
        });
        if now.saturating_duration_since(entry.last) > PEER_FORGET {
            entry.failures = 0;
            entry.blocked_until = None;
        }
        entry.failures = entry.failures.saturating_add(1);
        entry.last = now;
        let peer_pause = entry
            .failures
            .checked_sub(FREE_FAILURES)
            .filter(|over| *over > 0)
            .map(|over| backoff(PEER_BACKOFF, over, PEER_BACKOFF_MAX));
        entry.blocked_until = peer_pause.map(|pause| now + pause);
        let failures = entry.failures;

        // Strikes are forgiven after a quiet spell as long as the longest
        // pause since the last one ended; until then a repeat pauses longer.
        if self
            .global_until
            .is_some_and(|until| now.saturating_duration_since(until) > GLOBAL_BACKOFF_MAX)
        {
            self.global_strikes = 0;
            self.global_until = None;
        }
        let fresh = self
            .window_start
            .is_none_or(|start| now.saturating_duration_since(start) > GLOBAL_WINDOW);
        if fresh {
            self.window_start = Some(now);
            self.window_failures = 0;
        }
        self.window_failures = self.window_failures.saturating_add(1);
        let global_pause = (self.window_failures >= GLOBAL_LIMIT).then(|| {
            self.global_strikes = self.global_strikes.saturating_add(1);
            self.window_failures = 0;
            self.window_start = None;
            let pause = backoff(GLOBAL_BACKOFF, self.global_strikes, GLOBAL_BACKOFF_MAX);
            self.global_until = Some(now + pause);
            pause
        });
        self.trim(now);
        Recorded {
            failures,
            peer_pause,
            global_pause,
        }
    }

    /// Records a proof that succeeded: that address starts again.
    pub fn success(&mut self, peer: IpAddr) {
        self.peers.remove(&peer.to_canonical());
    }

    /// Keeps the table bounded: drop what is old, then the oldest.
    fn trim(&mut self, now: Instant) {
        if self.peers.len() <= MAX_PEERS {
            return;
        }
        self.peers
            .retain(|_, p| now.saturating_duration_since(p.last) <= PEER_FORGET);
        while self.peers.len() > MAX_PEERS {
            let Some(oldest) = self
                .peers
                .iter()
                .min_by_key(|(_, p)| p.last)
                .map(|(ip, _)| *ip)
            else {
                break;
            };
            self.peers.remove(&oldest);
        }
    }

    /// Addresses remembered (tests watch the bound).
    pub fn tracked(&self) -> usize {
        self.peers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};
    use std::net::Ipv4Addr;

    /// 0x00..0x1f, a fixed key for the vectors below.
    fn fixed_secret() -> Result<TrustSecret> {
        let text = format!(
            "{VERSION_LINE}\nsecret {}\n",
            secret::hex(&(0..32).collect::<Vec<u8>>())
        );
        Ok(TrustSecret::parse(&text)?)
    }

    fn challenge_for(nonce: [u8; 32], host: &str, controller: &str, at: Instant) -> Challenge {
        Challenge {
            nonce,
            host_id: host.to_owned(),
            controller_id: controller.to_owned(),
            issued: at,
        }
    }

    fn peer(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, last))
    }

    #[test]
    fn a_generated_controller_id_is_always_valid_and_keeps_the_name_readable() {
        let suffix = [0x1a, 0x2b, 0x3c, 0x4d];
        assert_eq!(controller_id_for("T480", suffix), "T480-1a2b3c4d");
        assert_eq!(
            controller_id_for("  My Laptop (2)  ", suffix),
            "My-Laptop-2-1a2b3c4d",
            "spaces and brackets fold to one dash"
        );
        let long = "x".repeat(500);
        for odd in ["", "   ", "---", "\u{feff}", "ünïcödé", long.as_str()] {
            let id = controller_id_for(odd, suffix);
            assert!(
                valid_controller_id(&id),
                "{odd:?} gave {id:?}, which the host would refuse"
            );
            assert!(id.ends_with("-1a2b3c4d"), "{id}: the suffix always fits");
        }
        assert_eq!(controller_id_for("", suffix), "strobe-1a2b3c4d");
        assert_eq!(
            controller_id_for(&long, suffix).len(),
            MAX_CONTROLLER_ID,
            "a long name is cut, never the suffix"
        );
    }

    #[test]
    fn two_new_controller_ids_for_one_computer_differ() -> Result<()> {
        let a = new_controller_id("laptop")?;
        let b = new_controller_id("laptop")?;
        assert!(
            valid_controller_id(&a) && valid_controller_id(&b),
            "{a} {b}"
        );
        assert_ne!(
            a, b,
            "the random suffix tells two laptops of one name apart"
        );
        Ok(())
    }

    #[test]
    fn hmac_sha256_matches_the_rfc_4231_vectors() -> Result<()> {
        // The crate wiring (hmac over sha2 at digest 0.11) is right.
        let cases: [(&[u8], &[u8], &str); 3] = [
            (
                &[0x0b; 20],
                b"Hi There",
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            ),
            (
                b"Jefe",
                b"what do ya want for nothing?",
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            ),
            (
                &[0xaa; 20],
                &[0xdd; 50],
                "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
            ),
        ];
        for (key, data, expected) in cases {
            let mut mac = HmacSha256::new_from_slice(key)?;
            mac.update(data);
            assert_eq!(
                secret::hex(&mac.finalize().into_bytes()),
                expected,
                "RFC 4231 case"
            );
        }
        Ok(())
    }

    #[test]
    fn the_response_is_the_framed_hmac_of_the_documented_fields() -> Result<()> {
        // Computed independently with Python's hmac/hashlib from the layout
        // in the module docs: key 00..1f, nonce 80..9f.
        let nonce: Vec<u8> = (0x80..0xa0).collect();
        let nonce_hex = secret::hex(&nonce);
        assert_eq!(
            fixed_secret()?.respond(&nonce_hex, "127.0.0.1:47812", "t480-1a2b3c4d"),
            Some("7aea3b9895699f81047022e272038f94ed6995618c444749b1d73c6ce06e5d58".to_owned()),
            "the protocol vector"
        );
        Ok(())
    }

    #[test]
    fn the_right_proof_is_accepted_once_and_only_by_its_fields() -> Result<()> {
        let secret = fixed_secret()?;
        let now = Instant::now();
        let make = || challenge_for([7; 32], "10.0.0.2:47812", "laptop", now);
        let proof = secret
            .respond(&make().nonce_hex(), "10.0.0.2:47812", "laptop")
            .context("a proof")?;
        assert_eq!(
            make().verify(&secret, &proof, now),
            Verdict::Accepted,
            "the right secret and fields"
        );
        for (what, host, controller, nonce) in [
            ("another host", "10.0.0.3:47812", "laptop", [7; 32]),
            ("another port", "10.0.0.2:47813", "laptop", [7; 32]),
            ("another controller", "10.0.0.2:47812", "other", [7; 32]),
            ("another nonce", "10.0.0.2:47812", "laptop", [8; 32]),
        ] {
            assert_eq!(
                challenge_for(nonce, host, controller, now).verify(&secret, &proof, now),
                Verdict::Wrong,
                "a proof does not transfer to {what}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_wrong_secret_a_flipped_bit_and_a_malformed_answer_are_refused() -> Result<()> {
        let secret = fixed_secret()?;
        let other = TrustSecret::parse(&TrustSecret::generate()?.to_file_text())?;
        let now = Instant::now();
        let make = || challenge_for([1; 32], "h:1", "c", now);
        let nonce_hex = make().nonce_hex();
        let proof = secret.respond(&nonce_hex, "h:1", "c").context("proof")?;
        let wrong = other.respond(&nonce_hex, "h:1", "c").context("proof")?;
        assert_eq!(
            make().verify(&secret, &wrong, now),
            Verdict::Wrong,
            "secret"
        );
        let mut flipped = proof.clone().into_bytes();
        if let Some(last) = flipped.last_mut() {
            *last = if *last == b'0' { b'1' } else { b'0' };
        }
        assert_eq!(
            make().verify(&secret, &String::from_utf8(flipped)?, now),
            Verdict::Wrong,
            "one nibble"
        );
        for bad in ["", "zz", &proof[..62], &format!("{proof}00"), "0"] {
            assert_eq!(
                make().verify(&secret, bad, now),
                Verdict::Wrong,
                "{bad:?} is not a proof"
            );
        }
        Ok(())
    }

    #[test]
    fn a_challenge_is_spent_by_its_answer_and_expires() -> Result<()> {
        // `verify` takes the challenge by value, so a second answer to the
        // same nonce cannot be written; this checks the other half: a
        // replayed *response* against a *new* nonce fails.
        let secret = fixed_secret()?;
        let now = Instant::now();
        let first = Challenge::issue("h:1", "c", now)?;
        let second = Challenge::issue("h:1", "c", now)?;
        assert_ne!(first.nonce_hex(), second.nonce_hex(), "nonces never repeat");
        let proof = secret
            .respond(&first.nonce_hex(), "h:1", "c")
            .context("proof")?;
        assert_eq!(
            first.verify(&secret, &proof, now),
            Verdict::Accepted,
            "first"
        );
        assert_eq!(
            second.verify(&secret, &proof, now),
            Verdict::Wrong,
            "the replayed answer fails on the next nonce"
        );
        let late = Challenge::issue("h:1", "c", now)?;
        let proof = secret
            .respond(&late.nonce_hex(), "h:1", "c")
            .context("proof")?;
        assert_eq!(
            late.verify(
                &secret,
                &proof,
                now + CHALLENGE_TTL + Duration::from_millis(1)
            ),
            Verdict::Expired,
            "a late right answer is refused"
        );
        Ok(())
    }

    #[test]
    fn a_nonce_or_controller_id_that_is_not_well_formed_gets_no_response() -> Result<()> {
        let secret = fixed_secret()?;
        let nonce = "ab".repeat(32);
        assert!(secret.respond(&nonce, "h:1", "c").is_some(), "well formed");
        assert!(secret.respond(&nonce[..62], "h:1", "c").is_none(), "short");
        assert!(secret.respond(&nonce, "h:1", "").is_none(), "no id");
        assert!(secret.respond(&nonce, "h:1", "a b").is_none(), "a space");
        assert!(valid_controller_id("T480-1a2b.c_d"), "ordinary");
        assert!(
            !valid_controller_id(&"x".repeat(MAX_CONTROLLER_ID + 1)),
            "long"
        );
        assert!(!valid_controller_id("é"), "non-ASCII");
        assert!(
            valid_hex_block(&nonce) && !valid_hex_block("00"),
            "hex block"
        );
        Ok(())
    }

    #[test]
    fn the_secret_never_prints() -> Result<()> {
        let secret = fixed_secret()?;
        let printed = format!("{secret:?}");
        assert!(printed.contains("***"), "{printed}");
        assert!(
            !printed.contains("0001") && !printed.contains(&secret::hex(&secret.0)),
            "{printed}"
        );
        Ok(())
    }

    #[test]
    fn a_trust_file_round_trips_and_malformed_ones_are_refused() -> Result<()> {
        let made = TrustSecret::generate()?;
        let text = made.to_file_text();
        assert!(text.contains(VERSION_LINE), "{text}");
        assert!(
            TrustSecret::parse(&text)?.0 == made.0,
            "the text reads back as the same secret"
        );
        let crlf = text.replace('\n', "\r\n");
        assert!(
            TrustSecret::parse(&format!("\u{feff}{crlf}")).is_ok(),
            "CRLF and BOM"
        );
        let hex = secret::hex(&made.0);
        for (bad, why) in [
            (String::new(), "empty"),
            (format!("secret {hex}\n"), "no version line"),
            (
                format!("strobe-remote-trust 2\nsecret {hex}\n"),
                "future version",
            ),
            (format!("{VERSION_LINE}\n"), "no secret"),
            (format!("{VERSION_LINE}\nsecret {}\n", &hex[..62]), "short"),
            (format!("{VERSION_LINE}\nsecret {hex}00\n"), "long"),
            (
                format!("{VERSION_LINE}\nsecret {}\n", "zz".repeat(32)),
                "not hex",
            ),
            (
                format!("{VERSION_LINE}\nsecret {hex}\nsecret {hex}\n"),
                "two secrets",
            ),
            (
                format!("{VERSION_LINE}\nsecret {}\n", "ab".repeat(32)),
                "one byte repeated",
            ),
            (
                format!("{VERSION_LINE}\nsecret {}\n", "00".repeat(32)),
                "all zero",
            ),
        ] {
            assert!(TrustSecret::parse(&bad).is_err(), "{why}: {bad:?}");
        }
        Ok(())
    }

    fn scratch(tag: &str) -> Result<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("strobe-trust-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn making_a_trust_file_never_overwrites_one() -> Result<()> {
        let dir = scratch("make")?;
        let path = dir.join("venue.trust");
        let made = create_file(&path)?;
        let loaded = load_file(&path)?;
        assert!(loaded.0 == made.0, "the file holds the secret made");
        let before = std::fs::read(&path)?;
        let error = create_file(&path)
            .err()
            .context("a second make must fail")?;
        assert!(error.to_string().contains("already exists"), "{error}");
        assert_eq!(std::fs::read(&path)?, before, "and left the file alone");
        let missing = create_file(&dir.join("no-such-folder").join("x.trust"));
        assert!(missing.is_err(), "the folder is not created");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path)?.permissions().mode() & 0o777,
                0o600,
                "private to the owner"
            );
            let link = dir.join("link.trust");
            std::os::unix::fs::symlink(dir.join("planted-target"), &link)?;
            assert!(create_file(&link).is_err(), "not written through a symlink");
            assert!(!dir.join("planted-target").exists(), "nothing was created");
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn a_trust_file_other_users_can_read_is_refused_on_unix() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch("mode")?;
        let path = dir.join("venue.trust");
        create_file(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        let error = load_file(&path)
            .err()
            .context("a readable file is refused")?;
        assert!(error.to_string().contains("chmod 600"), "{error}");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        assert!(load_file(&path).is_ok(), "fixed");
        assert!(load_file(&dir).is_err(), "a directory is not a trust file");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn an_oversized_file_is_not_read() -> Result<()> {
        let dir = scratch("big")?;
        let path = dir.join("big.trust");
        std::fs::write(&path, "#".repeat(5000))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        let error = load_file(&path).err().context("refused")?;
        assert!(error.to_string().contains("too large"), "{error}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn failures_from_one_address_back_off_and_others_are_unaffected() -> Result<()> {
        let mut limiter = Limiter::default();
        let t0 = Instant::now();
        let a = peer(10);
        let b = peer(11);
        for n in 1..=FREE_FAILURES {
            let recorded = limiter.failure(a, t0);
            assert_eq!(recorded.failures, n, "counted");
            assert_eq!(recorded.peer_pause, None, "free attempt {n}");
            assert_eq!(limiter.admit(a, t0), Ok(()), "still admitted");
        }
        let fourth = limiter.failure(a, t0);
        assert_eq!(fourth.peer_pause, Some(PEER_BACKOFF), "first pause");
        assert_eq!(limiter.admit(a, t0), Err(PEER_BACKOFF), "refused during it");
        assert_eq!(
            limiter.admit(a, t0 + PEER_BACKOFF),
            Ok(()),
            "admitted once it passes"
        );
        assert_eq!(
            limiter.admit(b, t0),
            Ok(()),
            "another address is not paused"
        );
        let fifth = limiter.failure(a, t0 + PEER_BACKOFF);
        assert_eq!(fifth.peer_pause, Some(PEER_BACKOFF * 2), "doubled");
        for _ in 0..10 {
            limiter.failure(a, t0 + PEER_BACKOFF);
        }
        let capped = limiter.failure(a, t0 + PEER_BACKOFF);
        assert_eq!(capped.peer_pause, Some(PEER_BACKOFF_MAX), "capped");
        limiter.success(a);
        assert_eq!(
            limiter.admit(a, t0 + PEER_BACKOFF),
            Ok(()),
            "success clears"
        );
        assert_eq!(
            limiter.failure(a, t0).failures,
            1,
            "and starts the count over"
        );
        Ok(())
    }

    #[test]
    fn many_addresses_together_trip_a_global_pause_that_ends_and_grows() -> Result<()> {
        let mut limiter = Limiter::default();
        let t0 = Instant::now();
        let mut tripped = None;
        for n in 0..GLOBAL_LIMIT {
            // A different address each time: no per-address pause applies.
            let ip = IpAddr::V4(Ipv4Addr::new(
                10,
                0,
                u8::try_from(n / 200)?,
                u8::try_from(n % 200)?,
            ));
            let recorded = limiter.failure(ip, t0);
            assert_eq!(recorded.peer_pause, None, "one failure each");
            tripped = recorded.global_pause;
        }
        assert_eq!(tripped, Some(GLOBAL_BACKOFF), "the limit pauses everyone");
        assert_eq!(
            limiter.admit(peer(200), t0 + Duration::from_secs(1)),
            Err(GLOBAL_BACKOFF - Duration::from_secs(1)),
            "even an address that never failed"
        );
        let after = t0 + GLOBAL_BACKOFF;
        assert_eq!(limiter.admit(peer(200), after), Ok(()), "it ends");
        let mut again = None;
        for n in 0..GLOBAL_LIMIT {
            again = limiter.failure(peer(u8::try_from(n)?), after).global_pause;
        }
        assert_eq!(again, Some(GLOBAL_BACKOFF * 2), "a repeat pauses longer");
        Ok(())
    }

    #[test]
    fn the_table_of_addresses_stays_bounded() {
        let mut limiter = Limiter::default();
        let t0 = Instant::now();
        for n in 0..(MAX_PEERS * 3) {
            let high = u8::try_from(n / 250).unwrap_or(0);
            let low = u8::try_from(n % 250).unwrap_or(0);
            limiter.failure(
                IpAddr::V4(Ipv4Addr::new(10, 1, high, low)),
                t0 + Duration::from_millis(u64::try_from(n).unwrap_or(0)),
            );
        }
        assert!(limiter.tracked() <= MAX_PEERS, "{}", limiter.tracked());
    }

    #[test]
    fn a_v4_mapped_address_is_the_same_peer_as_the_v4_address() -> Result<()> {
        let mut limiter = Limiter::default();
        let t0 = Instant::now();
        let v4 = peer(5);
        let mapped: IpAddr = "::ffff:192.168.1.5".parse()?;
        for _ in 0..=FREE_FAILURES {
            limiter.failure(v4, t0);
        }
        assert!(
            limiter.admit(mapped, t0).is_err(),
            "one peer, two spellings"
        );
        Ok(())
    }
}
