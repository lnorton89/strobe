// ============================================================================
// Module:       strobe_remote::clients
// Description:  Paired clients: the list the desktop shows and revokes, kept
//               as token digests in remote-clients.json in the data
//               directory, with rotation on every authentication, and what
//               tells one device from another (name, platform, address).
//
// Dependencies: serde, serde_json; crate::secret; std::fs, std::net, std::sync
// ============================================================================

//! Paired clients and their tokens (L-03, D251).
//!
//! ## What is stored
//!
//! `remote-clients.json` in the data directory holds, per client, an id
//! (`c` and eight hex digits), a label (what the operator approved: the
//! name it asked under, its kind and address), when it paired and was last
//! seen, and the **SHA-256 of its token**, never the token. Reading the
//! file gives nobody a credential.
//!
//! Since D500 a record also keeps, apart, what the Paired devices list
//! shows to tell devices apart: the `name` the device asked under, the
//! `platform` word from its User-Agent, and the `address` it last
//! connected from; `last_seen` moves when a connection ends as well as
//! when one authenticates. Each is optional and read leniently: a file
//! without them (written before D500) fills platform and address in from
//! the label, and a value of the wrong type or an address that does not
//! parse costs that detail, never the pairing.
//! It is written whole to a temporary file and renamed over the old one,
//! readable by its owner only on Unix (on Windows it inherits
//! `%LOCALAPPDATA%`'s per-user ACL, as the import token does, D114).
//!
//! ## Rotation
//!
//! Every successful `authenticate` issues a new token, which the listener
//! sends in a `paired` message right after `authenticated`. The old token
//! keeps working only until the new one is first used: a phone that lost
//! the connection before it saved the new token can still get in with the
//! old one (and is given another), and once the new one is used the old
//! one is dead. So a copied token stops working the next time the real
//! phone connects, and at most two tokens per client are ever valid.
//!
//! ## Revocation
//!
//! [`Clients::revoke`] and [`Clients::revoke_all`] delete the records and
//! save at once; the listener also drops that client's open connection
//! (`Server::revoke`). They work with the listener off.
//!
//! A file that cannot be read is set aside as `remote-clients.json.bad`
//! and the list starts empty: every phone pairs again, which is the safe
//! failure.
//!
//! ## Full desktop grants are never saved (D275)
//!
//! A client paired with `pair_desktop` is kept in memory only. It is never
//! written to the file, a record in an older file that claims the grant is
//! dropped when the file is read, and [`Clients::revoke_desktop`] removes
//! every such client when the host switches full desktop access off or
//! stops listening. So a desktop token sniffed from the plain-HTTP link,
//! or left behind by a crash, cannot outlive the run and the switch it
//! was approved under. The native controller keeps its token in memory
//! only as well, so nothing that could use a saved grant is lost.

use crate::secret::{self, Token};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// The file's name in the data directory.
pub const CLIENTS_FILE: &str = "remote-clients.json";

/// Paired clients kept at most; pairing another asks the operator to
/// revoke one first.
pub const MAX_PAIRED: usize = 32;

/// The longest label kept, in characters.
pub const MAX_LABEL: usize = 80;

/// The longest platform word kept, in characters (D500).
const MAX_PLATFORM: usize = 32;

/// The file format's version.
const FILE_VERSION: u32 = 1;

/// One paired client as stored.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Record {
    /// Explicitly approved for full desktop UI access. Older records and
    /// ordinary phone pairings remain limited to performance commands.
    /// Such a record lives in memory only: [`Store::save`] leaves it out
    /// and [`Clients::load`] drops one it finds.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    desktop: bool,
    id: String,
    label: String,
    /// The name the device asked under (`request_pair`, `request_desktop`,
    /// or a trusted controller's id); empty for the version-1 `pair` code
    /// (D500).
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "lenient"
    )]
    name: String,
    /// The device word from its User-Agent when it paired: `iPhone`,
    /// `Windows`, `Strobe desktop` (D500).
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "lenient"
    )]
    platform: String,
    /// The address it last connected from, as text (D500).
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "lenient"
    )]
    address: String,
    paired_at: u64,
    last_seen: u64,
    /// SHA-256 of the token in force.
    token_sha256: String,
    /// SHA-256 of the token issued at the last authentication, until it is
    /// first used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_sha256: Option<String>,
    /// For a client paired by trust file (D288): the controller id it
    /// proved. Memory only, like every desktop record; the token digest of
    /// such a record is of a token nobody was given.
    #[serde(skip)]
    trusted_as: Option<String>,
}

impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Record")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("name", &self.name)
            .field("platform", &self.platform)
            .field("address", &self.address)
            .field("token_sha256", &"***")
            .finish_non_exhaustive()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    clients: Vec<Record>,
}

/// What a paired client may do, which is also what kind of client it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The controller page in a browser (a phone, usually): performance
    /// commands only.
    Phone,
    /// Another Strobe, approved for full desktop control (D275); this run
    /// only.
    Desktop,
    /// Another Strobe that proved the venue trust file (D288); this
    /// connection only.
    Trusted,
}

/// A paired client, as the desktop lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// Its id, for revoking.
    pub id: String,
    /// What the operator approved: `Ana's iPad, iPhone at 192.168.1.23`.
    pub label: String,
    /// The name it asked under, if it gave one (D500).
    pub name: Option<String>,
    /// The device word from its User-Agent, if known: `iPhone` (D500).
    pub platform: Option<String>,
    /// The address it last connected from, if known (D500).
    pub address: Option<IpAddr>,
    /// What kind of client it is.
    pub access: Access,
    /// When it paired, in seconds since 1970.
    pub paired_at: u64,
    /// When it last authenticated, or its last connection ended, in
    /// seconds since 1970.
    pub last_seen: u64,
}

/// Who is pairing, as the listener saw the connection (D500).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    /// The name it asked under; empty for the version-1 `pair` code.
    pub name: String,
    /// The device word from its User-Agent: `iPhone`, `Windows`.
    pub platform: String,
    /// The address it connected from.
    pub address: Option<IpAddr>,
}

/// A trusted pairing made (D288).
#[derive(Debug, PartialEq, Eq)]
pub struct TrustedPairing {
    /// The new client's id.
    pub client_id: String,
    /// Ids of this controller's earlier pairings, now removed.
    pub superseded: Vec<String>,
}

/// Why a trusted pairing was not made.
#[derive(Debug)]
pub enum TrustedRefusal {
    /// The operator revoked this controller (or everyone) this run.
    Revoked,
    /// [`MAX_PAIRED`] clients are paired already.
    Full,
    /// The operating system gave no random bytes.
    NoRandom(std::io::Error),
}

impl fmt::Display for TrustedRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Revoked => f.write_str("revoked on the host this run"),
            Self::Full => write!(f, "{MAX_PAIRED} remotes are paired already"),
            Self::NoRandom(error) => write!(f, "no random bytes: {error}"),
        }
    }
}

impl std::error::Error for TrustedRefusal {}

/// A successful authentication.
#[derive(Debug)]
pub struct Authenticated {
    /// Whether the operator approved full desktop UI access for this token.
    pub desktop: bool,
    /// Who.
    pub client_id: String,
    /// The token to use next time, when it could be saved; `None` keeps
    /// the one just used.
    pub rotated: Option<Token>,
}

#[derive(Debug)]
struct Store {
    path: Option<PathBuf>,
    records: Vec<Record>,
    /// Controller ids the operator revoked this run (D288). Memory only.
    denied: Vec<String>,
    /// Revoke all closes trusted pairing for the rest of the run.
    trust_closed: bool,
}

/// The paired clients, shared by the listener and the desktop.
#[derive(Clone, Debug)]
pub struct Clients(Arc<Mutex<Store>>);

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid(record: &Record) -> bool {
    record.id.len() == 9
        && record.id.starts_with('c')
        && record.id.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
        && is_digest(&record.token_sha256)
        && record.next_sha256.as_deref().is_none_or(is_digest)
}

/// `label` cut to [`MAX_LABEL`] characters, controls removed.
fn clean_label(label: &str) -> String {
    clean(label, MAX_LABEL)
}

/// `text` cut to `max` characters, controls removed.
fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// A text field added after the first file format (D500): anything but a
/// string reads as empty, so a damaged value costs that detail, never the
/// pairing.
fn lenient<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(serde_json::Value::deserialize(deserializer)?
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default())
}

/// The platform word and address in a label made before D500: the label
/// ends `KIND at ADDRESS`, after any `NAME, ` or desktop prefix.
fn from_label(label: &str) -> (String, Option<IpAddr>) {
    let Some((before, after)) = label.rsplit_once(" at ") else {
        return (String::new(), None);
    };
    let Ok(address) = after.parse::<IpAddr>() else {
        return (String::new(), None);
    };
    let platform = before.rsplit_once(", ").map_or(before, |(_, kind)| kind);
    (clean(platform, MAX_PLATFORM), Some(address))
}

/// A record read from the file, made safe to show: every text cleaned and
/// cut, an address that does not parse dropped, and a record from before
/// D500 given the platform and address its label names.
fn tidy(record: &mut Record) {
    record.label = clean_label(&record.label);
    record.name = clean_label(&record.name);
    record.platform = clean(&record.platform, MAX_PLATFORM);
    if record.address.parse::<IpAddr>().is_err() {
        record.address.clear();
    }
    if record.platform.is_empty() && record.address.is_empty() {
        let (platform, address) = from_label(&record.label);
        record.platform = platform;
        record.address = address.map(|a| a.to_string()).unwrap_or_default();
    }
}

/// `text`, or `None` when it is empty.
fn some(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

impl Clients {
    /// The clients in `data_dir`'s file, and a problem to log if the file
    /// was there but could not be used (it is set aside and the list
    /// starts empty).
    pub fn load(data_dir: &Path) -> (Self, Option<String>) {
        let path = data_dir.join(CLIENTS_FILE);
        let (records, problem) = match std::fs::read_to_string(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
            Err(error) => (
                Vec::new(),
                Some(format!("cannot read {}: {error}", path.display())),
            ),
            Ok(text) => match serde_json::from_str::<File>(&text) {
                Ok(file) if file.version == FILE_VERSION => {
                    let total = file.clients.len();
                    // A desktop grant is never saved; one in the file was not
                    // written by this version, and is not honoured.
                    let mut records: Vec<Record> = file
                        .clients
                        .into_iter()
                        .filter(|record| valid(record) && !record.desktop)
                        .collect();
                    records.truncate(MAX_PAIRED);
                    for record in &mut records {
                        tidy(record);
                    }
                    let dropped = total - records.len();
                    let problem = (dropped > 0).then(|| {
                        format!(
                            "{} has {dropped} damaged entries; they must pair again",
                            path.display()
                        )
                    });
                    (records, problem)
                }
                Ok(_) | Err(_) => {
                    let aside = path.with_extension("json.bad");
                    // The message says what really happened to the bad file:
                    // if it could not be moved, the next save overwrites it.
                    let fate = match std::fs::rename(&path, &aside) {
                        Ok(()) => format!("was moved to {}", aside.display()),
                        Err(error) => format!(
                            "could not be moved aside ({error}) and will be replaced at the next save"
                        ),
                    };
                    (
                        Vec::new(),
                        Some(format!(
                            "{} could not be read and {fate}; every remote must pair again",
                            path.display(),
                        )),
                    )
                }
            },
        };
        (
            Self(Arc::new(Mutex::new(Store {
                path: Some(path),
                records,
                denied: Vec::new(),
                trust_closed: false,
            }))),
            problem,
        )
    }

    /// A list that is never saved (tests, screenshot mode).
    pub fn in_memory() -> Self {
        Self(Arc::new(Mutex::new(Store {
            path: None,
            records: Vec::new(),
            denied: Vec::new(),
            trust_closed: false,
        })))
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every paired client, oldest first.
    pub fn list(&self) -> Vec<ClientInfo> {
        self.store().infos()
    }

    /// [`Clients::list`] if nobody holds the list right now, else `None`
    /// at once. A connection that authenticates holds it while it writes
    /// the file and waits for the disk (`sync_all`); the control window,
    /// which shows the list every frame, uses this and keeps the list it
    /// had, so a phone reconnecting never makes a frame wait on the disk
    /// (D462).
    pub fn try_list(&self) -> Option<Vec<ClientInfo>> {
        let store = match self.0.try_lock() {
            Ok(store) => store,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        Some(store.infos())
    }

    /// How many clients are paired.
    pub fn len(&self) -> usize {
        self.store().records.len()
    }

    /// Whether none is.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pairs a new client labelled `label`, with phone access and nothing
    /// known about the device: its id and its first token. Fails if the
    /// list is full or cannot be saved; nothing is paired then.
    pub fn pair(&self, label: &str) -> std::io::Result<(String, Token)> {
        self.pair_device(label, &Device::default(), false)
    }

    /// Pairs `device`, labelled `label`, with the desktop scope when the
    /// operator approved it (D275).
    pub fn pair_device(
        &self,
        label: &str,
        device: &Device,
        desktop: bool,
    ) -> std::io::Result<(String, Token)> {
        let mut store = self.store();
        if store.records.len() >= MAX_PAIRED {
            return Err(std::io::Error::other(format!(
                "{MAX_PAIRED} remotes are paired already; revoke one first"
            )));
        }
        let id = loop {
            let id = format!("c{}", secret::hex(&secret::random::<4>()?));
            if store.records.iter().all(|r| r.id != id) {
                break id;
            }
        };
        let token = Token::generate()?;
        let now = now_secs();
        store.records.push(Record {
            desktop,
            id: id.clone(),
            label: clean_label(label),
            name: clean_label(&device.name),
            platform: clean(&device.platform, MAX_PLATFORM),
            address: device.address.map(|a| a.to_string()).unwrap_or_default(),
            paired_at: now,
            last_seen: now,
            token_sha256: token.digest(),
            next_sha256: None,
            trusted_as: None,
        });
        if let Err(error) = store.save() {
            store.records.pop();
            return Err(error);
        }
        Ok((id, token))
    }

    /// The client `token` belongs to, with its next token (see the module
    /// docs on rotation), or `None` for a token that is not, or no longer,
    /// valid.
    pub fn authenticate(&self, token: &str) -> Option<Authenticated> {
        self.authenticate_at(token, None)
    }

    /// [`Clients::authenticate`] for a connection from `address`, which
    /// the client's record then shows (D500): a phone the network gave a
    /// new address is listed under it.
    pub fn authenticate_from(&self, token: &str, address: IpAddr) -> Option<Authenticated> {
        self.authenticate_at(token, Some(address))
    }

    fn authenticate_at(&self, token: &str, address: Option<IpAddr>) -> Option<Authenticated> {
        let candidate = secret::digest(token);
        let mut store = self.store();
        let before = store.records.clone();
        let record = store.records.iter_mut().find(|r| {
            secret::same(candidate.as_bytes(), r.token_sha256.as_bytes())
                || r.next_sha256
                    .as_deref()
                    .is_some_and(|next| secret::same(candidate.as_bytes(), next.as_bytes()))
        })?;
        if record.next_sha256.as_deref() == Some(candidate.as_str()) {
            // The new token's first use: the old one dies.
            record.token_sha256 = candidate;
        }
        record.last_seen = now_secs();
        if let Some(address) = address {
            record.address = address.to_string();
        }
        let client_id = record.id.clone();
        let desktop = record.desktop;
        // Without the OS random source there is no next token; the current
        // one stays valid and rotation is tried again at the next connect.
        let rotated = Token::generate()
            .map_err(|error| log::warn!("remote: no new token this time ({error})"))
            .ok();
        record.next_sha256 = rotated.as_ref().map(Token::digest);
        match store.save() {
            Ok(()) => Some(Authenticated {
                client_id,
                rotated,
                desktop,
            }),
            Err(error) => {
                log::warn!("remote: could not save the client list ({error}); token not rotated");
                store.records = before;
                Some(Authenticated {
                    desktop,
                    client_id,
                    rotated: None,
                })
            }
        }
    }

    /// Client `id`'s connection ended: it was last seen now (D500). A
    /// client no longer paired is ignored; a desktop record, never saved,
    /// changes in memory only.
    pub fn seen(&self, id: &str) {
        let mut store = self.store();
        let Some(record) = store.records.iter_mut().find(|r| r.id == id) else {
            return;
        };
        record.last_seen = now_secs();
        if !record.desktop
            && let Err(error) = store.save()
        {
            log::warn!("remote: could not save the client list after a disconnect: {error}");
        }
    }

    /// Revokes client `id`. Returns whether it was paired.
    pub fn revoke(&self, id: &str) -> bool {
        let mut store = self.store();
        let before = store.records.len();
        let denied: Vec<String> = store
            .records
            .iter()
            .filter(|r| r.id == id)
            .filter_map(|r| r.trusted_as.clone())
            .collect();
        store.records.retain(|r| r.id != id);
        // A trusted controller the operator revokes stays out for the run:
        // it would otherwise prove its way straight back in (D288).
        store.denied.extend(denied);
        let removed = store.records.len() != before;
        if removed && let Err(error) = store.save() {
            log::warn!("remote: could not save the client list after a revoke: {error}");
        }
        removed
    }

    /// Revokes every client. Returns how many there were.
    pub fn revoke_all(&self) -> usize {
        let mut store = self.store();
        let count = store.records.len();
        let denied: Vec<String> = store
            .records
            .iter()
            .filter_map(|r| r.trusted_as.clone())
            .collect();
        store.denied.extend(denied);
        // Everyone out means the trust file stops opening the door too,
        // for the rest of the run (D288).
        store.trust_closed = true;
        store.records.clear();
        if let Err(error) = store.save() {
            log::warn!("remote: could not save the client list after revoking all: {error}");
        }
        count
    }

    /// Revokes every client paired for full desktop access (D275), when
    /// the host switches that access off or stops listening. Returns their
    /// ids, so the listener can drop their connections.
    pub fn revoke_desktop(&self) -> Vec<String> {
        let mut store = self.store();
        let mut revoked = Vec::new();
        store.records.retain(|record| {
            if record.desktop {
                revoked.push(record.id.clone());
            }
            !record.desktop
        });
        // Desktop records are never in the file, so there is nothing to save.
        revoked
    }

    /// Pairs a controller that proved the trust file (D288), for full
    /// desktop control, in memory only. The record has no token anyone
    /// holds, so it can only be used through the connection that proved
    /// it, and it is removed when that connection ends
    /// ([`Clients::forget_trusted`]).
    ///
    /// A controller id already paired (its earlier connection not yet
    /// noticed dead, say) is replaced, and the replaced ids come back so
    /// the listener can close those connections and free their lease. The
    /// list is bounded as for phones.
    pub fn pair_trusted(
        &self,
        label: &str,
        device: &Device,
        controller_id: &str,
    ) -> Result<TrustedPairing, TrustedRefusal> {
        let mut store = self.store();
        if store.trust_closed || store.denied.iter().any(|d| d == controller_id) {
            return Err(TrustedRefusal::Revoked);
        }
        let mut superseded = Vec::new();
        store.records.retain(|record| {
            let same = record.trusted_as.as_deref() == Some(controller_id);
            if same {
                superseded.push(record.id.clone());
            }
            !same
        });
        if store.records.len() >= MAX_PAIRED {
            return Err(TrustedRefusal::Full);
        }
        let id = loop {
            let id = format!(
                "c{}",
                secret::hex(&secret::random::<4>().map_err(TrustedRefusal::NoRandom)?)
            );
            if store.records.iter().all(|r| r.id != id) {
                break id;
            }
        };
        // A token that is generated, digested and dropped: the record
        // exists to be listed and revoked, never to be authenticated.
        let token = Token::generate().map_err(TrustedRefusal::NoRandom)?;
        let now = now_secs();
        store.records.push(Record {
            desktop: true,
            id: id.clone(),
            label: clean_label(label),
            name: clean_label(&device.name),
            platform: clean(&device.platform, MAX_PLATFORM),
            address: device.address.map(|a| a.to_string()).unwrap_or_default(),
            paired_at: now,
            last_seen: now,
            token_sha256: token.digest(),
            next_sha256: None,
            trusted_as: Some(controller_id.to_owned()),
        });
        Ok(TrustedPairing {
            client_id: id,
            superseded,
        })
    }

    /// Whether `controller_id` may not pair by trust file now: the operator
    /// revoked it, or revoked everyone, this run.
    pub fn trust_blocked(&self, controller_id: &str) -> bool {
        let store = self.store();
        store.trust_closed || store.denied.iter().any(|d| d == controller_id)
    }

    /// Removes a trusted pairing whose connection has ended. Not a revoke:
    /// the controller may prove its way in again.
    pub fn forget_trusted(&self, id: &str) {
        self.store()
            .records
            .retain(|r| !(r.id == id && r.trusted_as.is_some()));
    }

    /// Whether client `id` is still paired.
    pub fn contains(&self, id: &str) -> bool {
        self.store().records.iter().any(|r| r.id == id)
    }
}

impl Store {
    /// Every record as the desktop lists it.
    fn infos(&self) -> Vec<ClientInfo> {
        self.records
            .iter()
            .map(|r| ClientInfo {
                id: r.id.clone(),
                label: r.label.clone(),
                name: some(&r.name),
                platform: some(&r.platform),
                address: r.address.parse().ok(),
                access: match (r.desktop, r.trusted_as.is_some()) {
                    (_, true) => Access::Trusted,
                    (true, false) => Access::Desktop,
                    (false, false) => Access::Phone,
                },
                paired_at: r.paired_at,
                last_seen: r.last_seen,
            })
            .collect()
    }

    /// Writes the whole list to a temporary file and renames it into place.
    fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = File {
            version: FILE_VERSION,
            // Full desktop grants live for this run only (D275).
            clients: self
                .records
                .iter()
                .filter(|record| !record.desktop)
                .cloned()
                .collect(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        let temporary = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut out = options.open(&temporary)?;
        out.write_all(text.as_bytes())?;
        out.write_all(b"\n")?;
        out.sync_all()?;
        drop(out);
        std::fs::rename(&temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "strobe-remote-clients-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn phone_scope_is_the_legacy_default_and_desktop_scope_survives_rotation() -> Result<()> {
        let clients = Clients::in_memory();
        let (_, phone) = clients.pair("phone")?;
        let (_, desktop) = clients.pair_device("desktop", &Device::default(), true)?;
        let phone_auth = clients
            .authenticate(phone.expose())
            .context("phone authentication")?;
        assert!(!phone_auth.desktop, "phone scope stays limited");
        let desktop_auth = clients
            .authenticate(desktop.expose())
            .context("desktop authentication")?;
        assert!(desktop_auth.desktop, "explicit desktop scope");
        let rotated = desktop_auth.rotated.context("rotated token")?;
        assert!(
            clients
                .authenticate(rotated.expose())
                .is_some_and(|auth| auth.desktop),
            "rotated token keeps desktop scope"
        );
        let legacy = serde_json::to_value(&clients.store().records[0])?;
        assert!(
            legacy.get("desktop").is_none(),
            "ordinary clients retain the legacy file shape"
        );
        let restored: Record = serde_json::from_value(legacy)?;
        assert!(!restored.desktop, "old files default to phone permissions");
        let native = serde_json::to_value(&clients.store().records[1])?;
        let restored: Record = serde_json::from_value(native)?;
        assert!(
            restored.desktop,
            "explicit grant is serialized and restored"
        );
        Ok(())
    }

    #[test]
    fn a_desktop_grant_is_never_saved_and_ends_with_the_switch() -> Result<()> {
        let dir = scratch("desktop-grant");
        let (clients, _) = Clients::load(&dir);
        let (phone_id, phone) = clients.pair("phone")?;
        let (desktop_id, desktop) =
            clients.pair_device("Full desktop: x", &Device::default(), true)?;
        let rotated = clients
            .authenticate(desktop.expose())
            .and_then(|auth| auth.rotated)
            .context("a desktop token authenticates and rotates")?;
        let text = std::fs::read_to_string(dir.join(CLIENTS_FILE))?;
        assert!(
            !text.contains(&desktop_id) && !text.contains("desktop"),
            "a desktop grant never reaches the file: {text}"
        );
        assert!(text.contains(&phone_id), "the phone is saved as before");
        let (restarted, _) = Clients::load(&dir);
        assert!(
            restarted.authenticate(rotated.expose()).is_none(),
            "a restart ends the desktop grant"
        );
        assert_eq!(
            clients.revoke_desktop(),
            vec![desktop_id.clone()],
            "switching desktop access off revokes the desktop client"
        );
        assert!(
            clients.authenticate(rotated.expose()).is_none(),
            "and its token is dead"
        );
        assert!(
            clients.authenticate(phone.expose()).is_some(),
            "phones are untouched"
        );
        // A file written by the first cut of D275, which saved the grant.
        let forged = format!(
            "{{\"version\":1,\"clients\":[{{\"desktop\":true,\"id\":\"c0000abcd\",\"label\":\"x\",\
             \"paired_at\":1,\"last_seen\":1,\"token_sha256\":\"{}\"}}]}}",
            secret::digest(desktop.expose())
        );
        std::fs::write(dir.join(CLIENTS_FILE), forged)?;
        let (old_file, _) = Clients::load(&dir);
        assert!(
            old_file.is_empty() && old_file.authenticate(desktop.expose()).is_none(),
            "a saved desktop grant is not honoured"
        );
        let _ = std::fs::remove_dir_all(&dir); // Best effort: a temp dir.
        Ok(())
    }

    #[test]
    fn a_paired_token_authenticates_and_rotates() -> Result<()> {
        let clients = Clients::in_memory();
        let (id, first) = clients.pair("iPhone at 10.0.0.7")?;
        let auth = clients
            .authenticate(first.expose())
            .context("first token")?;
        assert_eq!(auth.client_id, id, "the right client");
        let second = auth.rotated.context("a new token")?;
        assert_ne!(second, first, "rotated");
        let again = clients
            .authenticate(first.expose())
            .context("the old token works until the new one is used")?;
        let third = again.rotated.context("and is rotated again")?;
        assert!(
            clients.authenticate(second.expose()).is_none(),
            "the second token was replaced before it was used"
        );
        clients
            .authenticate(third.expose())
            .context("the latest token works")?;
        assert!(
            clients.authenticate(first.expose()).is_none(),
            "and once it is used, the first is dead"
        );
        Ok(())
    }

    #[test]
    fn the_list_can_be_read_without_waiting_for_a_save() -> Result<()> {
        let clients = Clients::in_memory();
        let (id, _) = clients.pair("phone")?;
        let listed = clients.try_list().context("free: read at once")?;
        assert_eq!(
            listed.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec![id.as_str()],
            "the same list as list()"
        );
        let held = clients.store();
        assert!(
            clients.try_list().is_none(),
            "held (a save in progress): no wait, no list"
        );
        drop(held);
        assert!(clients.try_list().is_some(), "free again");
        Ok(())
    }

    #[test]
    fn unknown_and_revoked_tokens_are_refused() -> Result<()> {
        let clients = Clients::in_memory();
        let (a, token_a) = clients.pair("a")?;
        let (_, token_b) = clients.pair("b")?;
        assert!(
            clients.authenticate("0".repeat(64).as_str()).is_none(),
            "unknown"
        );
        assert!(clients.authenticate("").is_none(), "empty");
        assert!(clients.revoke(&a), "revoked");
        assert!(!clients.revoke(&a), "only once");
        assert!(clients.authenticate(token_a.expose()).is_none(), "a is out");
        assert!(clients.authenticate(token_b.expose()).is_some(), "b is not");
        assert_eq!(clients.revoke_all(), 1, "revoke all");
        assert!(
            clients.authenticate(token_b.expose()).is_none(),
            "b is out too"
        );
        assert!(clients.is_empty(), "nobody is paired");
        Ok(())
    }

    #[test]
    fn the_file_holds_digests_and_survives_a_restart() -> Result<()> {
        let dir = scratch("restart");
        let (clients, problem) = Clients::load(&dir);
        assert!(problem.is_none(), "no file is not a problem: {problem:?}");
        let (id, token) = clients.pair("Android at 192.168.1.9")?;
        let text = std::fs::read_to_string(dir.join(CLIENTS_FILE))?;
        assert!(!text.contains(token.expose()), "no token on disk");
        assert!(text.contains(&token.digest()), "its digest is");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.join(CLIENTS_FILE))?
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "owner only: {mode:o}");
        }
        drop(clients);

        let (restarted, problem) = Clients::load(&dir);
        assert!(problem.is_none(), "{problem:?}");
        let auth = restarted
            .authenticate(token.expose())
            .context("a paired phone survives a restart")?;
        assert_eq!(auth.client_id, id, "as the same client");
        assert_eq!(restarted.list().len(), 1, "listed");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_damaged_file_is_set_aside_and_nobody_is_paired() -> Result<()> {
        let dir = scratch("damaged");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(CLIENTS_FILE), "{not json")?;
        let (clients, problem) = Clients::load(&dir);
        assert!(problem.is_some(), "the problem is reported");
        assert!(clients.is_empty(), "nobody is paired");
        assert!(dir.join("remote-clients.json.bad").exists(), "set aside");

        let bad_entry = r#"{"version":1,"clients":[{"id":"c00000000","label":"x","paired_at":0,"last_seen":0,"token_sha256":"short"}]}"#;
        std::fs::write(dir.join(CLIENTS_FILE), bad_entry)?;
        let (clients, problem) = Clients::load(&dir);
        assert!(clients.is_empty(), "a malformed digest is dropped");
        assert!(problem.is_some(), "and reported");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_list_is_bounded_and_labels_are_clean() -> Result<()> {
        let clients = Clients::in_memory();
        let (id, token) = clients.pair(&format!("evil\u{1b}[31m{}", "x".repeat(500)))?;
        let label = clients
            .list()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| c.label)
            .context("listed")?;
        assert!(!label.contains('\u{1b}'), "no control characters");
        assert_eq!(label.chars().count(), MAX_LABEL, "cut to the limit");
        for _ in 1..MAX_PAIRED {
            clients.pair("x")?;
        }
        assert!(clients.pair("one too many").is_err(), "the list is full");
        let printed = format!("{clients:?}");
        assert!(
            !printed.contains(&token.digest()) && !printed.contains(token.expose()),
            "neither a token nor its digest prints: {printed}"
        );
        Ok(())
    }

    #[test]
    fn a_trusted_pairing_is_in_memory_only_listed_replaced_and_revocable() -> Result<()> {
        let dir = scratch("trusted");
        let (clients, _) = Clients::load(&dir);
        let first = clients.pair_trusted("Full desktop: trusted a, x", &Device::default(), "a")?;
        assert!(first.superseded.is_empty(), "nothing to replace");
        let again = clients.pair_trusted("Full desktop: trusted a, x", &Device::default(), "a")?;
        assert_eq!(
            again.superseded,
            vec![first.client_id.clone()],
            "the same controller replaces itself"
        );
        assert_eq!(clients.len(), 1, "one record, not a growing list");
        let other = clients.pair_trusted("Full desktop: trusted b, x", &Device::default(), "b")?;
        assert_eq!(clients.len(), 2, "a second controller is a second record");
        let text = std::fs::read_to_string(dir.join(CLIENTS_FILE)).unwrap_or_default();
        assert!(
            !text.contains("trusted"),
            "never written to the file: {text}"
        );
        let (id, token) = clients.pair("phone")?;
        assert!(
            clients.authenticate(token.expose()).is_some(),
            "phones work"
        );
        assert!(clients.revoke(&again.client_id), "revoked");
        assert!(clients.trust_blocked("a"), "and kept out for the run");
        assert!(
            matches!(
                clients.pair_trusted("x", &Device::default(), "a"),
                Err(TrustedRefusal::Revoked)
            ),
            "it cannot pair again"
        );
        assert!(!clients.trust_blocked("b"), "others are not blocked");
        clients.forget_trusted(&other.client_id);
        assert!(
            !clients.contains(&other.client_id),
            "forgotten at disconnect"
        );
        assert!(
            clients.pair_trusted("x", &Device::default(), "b").is_ok(),
            "forgetting is not revoking"
        );
        assert_eq!(
            clients.revoke_desktop().len(),
            1,
            "the kill switch takes them"
        );
        clients.pair_trusted("x", &Device::default(), "c")?;
        clients.revoke_all();
        assert!(
            clients.trust_blocked("c") && clients.trust_blocked("new"),
            "revoke all closes trusted pairing to every controller id"
        );
        assert!(
            clients.contains(&id) || clients.is_empty(),
            "phones revoked"
        );
        Ok(())
    }

    #[test]
    fn trusted_pairings_respect_the_list_bound() -> Result<()> {
        let clients = Clients::in_memory();
        for n in 0..MAX_PAIRED {
            clients.pair_trusted("x", &Device::default(), &format!("c{n}"))?;
        }
        assert!(
            matches!(
                clients.pair_trusted("x", &Device::default(), "one-more"),
                Err(TrustedRefusal::Full)
            ),
            "full"
        );
        clients.pair_trusted("x", &Device::default(), "c0")?;
        Ok(())
    }

    fn ipad() -> Device {
        Device {
            name: "Ana's iPad".into(),
            platform: "iPhone".into(),
            address: Some(IpAddr::from([192, 168, 1, 23])),
        }
    }

    fn only(clients: &Clients) -> Result<ClientInfo> {
        let mut listed = clients.list();
        anyhow::ensure!(listed.len() == 1, "one client: {listed:?}");
        listed.pop().context("listed")
    }

    #[test]
    fn device_details_are_listed_follow_the_address_and_survive_a_restart() -> Result<()> {
        let dir = scratch("details");
        let (clients, _) = Clients::load(&dir);
        let (id, token) =
            clients.pair_device("Ana's iPad, iPhone at 192.168.1.23", &ipad(), false)?;
        let listed = only(&clients)?;
        assert_eq!(
            (
                listed.name.as_deref(),
                listed.platform.as_deref(),
                listed.address,
                listed.access
            ),
            (
                Some("Ana's iPad"),
                Some("iPhone"),
                Some(IpAddr::from([192, 168, 1, 23])),
                Access::Phone
            ),
            "what tells it apart is listed"
        );
        clients
            .authenticate_from(token.expose(), IpAddr::from([192, 168, 1, 77]))
            .context("authenticates")?;
        assert_eq!(
            only(&clients)?.address,
            Some(IpAddr::from([192, 168, 1, 77])),
            "a new address from the network is shown"
        );
        clients.store().records[0].last_seen = 1;
        clients.seen(&id);
        assert!(
            only(&clients)?.last_seen > 1,
            "a connection ending moves last seen"
        );
        clients.seen("c00000000");
        let before = only(&clients)?;
        drop(clients);

        let (restarted, problem) = Clients::load(&dir);
        assert!(problem.is_none(), "{problem:?}");
        assert_eq!(only(&restarted)?, before, "every detail survives a restart");
        let text = std::fs::read_to_string(dir.join(CLIENTS_FILE))?;
        assert!(
            !text.contains(token.expose()) && !format!("{before:?}").contains(token.expose()),
            "no token on disk or in the list"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_file_from_before_the_details_reads_them_from_its_label() -> Result<()> {
        let dir = scratch("legacy-details");
        std::fs::create_dir_all(&dir)?;
        let digest = "0".repeat(64);
        let old = format!(
            "{{\"version\":1,\"clients\":[\
             {{\"id\":\"c00000001\",\"label\":\"Windows at 10.1.10.124\",\"paired_at\":5,\
             \"last_seen\":6,\"token_sha256\":\"{digest}\"}},\
             {{\"id\":\"c00000002\",\"label\":\"Ana's iPad, iPhone at fe80::1\",\"paired_at\":5,\
             \"last_seen\":6,\"token_sha256\":\"{digest}\"}},\
             {{\"id\":\"c00000003\",\"label\":\"phone\",\"paired_at\":5,\
             \"last_seen\":6,\"token_sha256\":\"{digest}\"}}]}}"
        );
        std::fs::write(dir.join(CLIENTS_FILE), old)?;
        let (clients, problem) = Clients::load(&dir);
        assert!(problem.is_none(), "an old file is not damaged: {problem:?}");
        let details: Vec<_> = clients
            .list()
            .into_iter()
            .map(|c| (c.name, c.platform, c.address))
            .collect();
        assert_eq!(
            details,
            vec![
                (
                    None,
                    Some("Windows".to_string()),
                    Some(IpAddr::from([10, 1, 10, 124]))
                ),
                (None, Some("iPhone".to_string()), "fe80::1".parse().ok()),
                (None, None, None),
            ],
            "platform and address from the label; a label without them gives none"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_damaged_detail_costs_the_detail_never_the_pairing() -> Result<()> {
        let dir = scratch("damaged-details");
        std::fs::create_dir_all(&dir)?;
        let token = Token::generate()?;
        let damaged = format!(
            "{{\"version\":1,\"clients\":[{{\"id\":\"c00000001\",\"label\":\"x\",\
             \"name\":5,\"platform\":[\"iPhone\"],\"address\":\"not an address\",\
             \"paired_at\":5,\"last_seen\":6,\"token_sha256\":\"{}\"}}]}}",
            token.digest()
        );
        std::fs::write(dir.join(CLIENTS_FILE), damaged)?;
        let (clients, problem) = Clients::load(&dir);
        assert!(problem.is_none(), "nothing to report: {problem:?}");
        let listed = only(&clients)?;
        assert_eq!(
            (listed.name, listed.platform, listed.address),
            (None, None, None),
            "each damaged detail is dropped"
        );
        assert!(
            clients.authenticate(token.expose()).is_some(),
            "and the device is still paired"
        );
        let long = format!(
            "{{\"version\":1,\"clients\":[{{\"id\":\"c00000001\",\"label\":\"x\",\
             \"name\":\"a\\u001b[31m{}\",\"platform\":\"{}\",\"address\":\"10.0.0.1\",\
             \"paired_at\":5,\"last_seen\":6,\"token_sha256\":\"{}\"}}]}}",
            "n".repeat(500),
            "p".repeat(500),
            token.digest()
        );
        std::fs::write(dir.join(CLIENTS_FILE), long)?;
        let (clients, _) = Clients::load(&dir);
        let listed = only(&clients)?;
        let name = listed.name.context("a name")?;
        assert!(
            !name.contains('\u{1b}') && name.chars().count() == MAX_LABEL,
            "a name is cleaned and cut: {name:?}"
        );
        assert_eq!(
            listed.platform.map(|p| p.chars().count()),
            Some(MAX_PLATFORM),
            "so is a platform"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_kind_of_client_is_listed() -> Result<()> {
        let clients = Clients::in_memory();
        clients.pair_device("phone", &ipad(), false)?;
        clients.pair_device("desktop", &ipad(), true)?;
        clients.pair_trusted("trusted", &ipad(), "venue")?;
        assert_eq!(
            clients.list().iter().map(|c| c.access).collect::<Vec<_>>(),
            vec![Access::Phone, Access::Desktop, Access::Trusted],
            "phone, approved desktop, trusted desktop"
        );
        Ok(())
    }
}
