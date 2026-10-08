//! The hosts of one person (ADR-0019): this host's own key, the hosts it
//! was introduced to, and the link between them.
//!
//! Each host makes a key at its first start that never leaves it; the key
//! is the host's identity on the wire (iroh), and its public half, shortened,
//! is the fingerprint a person reads. The person is the root: what makes a
//! host theirs is an act on a page they are signed in to. A host that is
//! already theirs introduces a new one - given the new host's address and a
//! word the new host showed once - and vouches for it: a signed statement of
//! whose it is, what it is called, what it may do and until when. Trust is
//! that vouch, checked at every meeting and renewed by the two meeting.
//! "Forget" withdraws it.
//!
//! What travels between hosts is the Workbench's own HTTP door, carried over
//! an iroh stream: nothing of SWEM's is on the wire that the page does not
//! already speak.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey};
use rusqlite::{Connection as Book, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// What the hosts of one person say to each other over iroh: the host's
/// own HTTP door. A label for the wire, nothing more.
pub const ALPN: &[u8] = b"swem-host-door/1";

/// How long a vouch is good for: weeks, renewed whenever the two meet.
const A_VOUCH_IS_GOOD_FOR: Duration = Duration::from_hours(30 * 24);
/// How long a word shown to be added by is good for.
const A_WORD_IS_GOOD_FOR: Duration = Duration::from_mins(10);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS hosts (
    host_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    addr TEXT NOT NULL,
    vouch TEXT NOT NULL,
    signature TEXT NOT NULL,
    vouched_by TEXT NOT NULL,
    until_ms INTEGER NOT NULL,
    added_ms INTEGER NOT NULL,
    seen_ms INTEGER,
    forgotten_ms INTEGER
);
CREATE TABLE IF NOT EXISTS words (
    word TEXT PRIMARY KEY,
    made_ms INTEGER NOT NULL,
    used_ms INTEGER
);
CREATE TABLE IF NOT EXISTS mine (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

#[derive(Debug, thiserror::Error)]
pub enum PeersError {
    #[error("{path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Refused(String),
    #[error("the hosts book: {0}")]
    Book(String),
    #[error("{0}")]
    Link(String),
}

impl From<rusqlite::Error> for PeersError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Book(error.to_string())
    }
}

/// The signed statement one host makes about another: whose it is, what it
/// is called, what it may do, until when, and who says so.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Vouch {
    /// The host vouched for.
    pub host: String,
    pub name: String,
    /// The person whose it is, as the vouching host names them.
    pub owner: String,
    /// What it may do on the vouching host: everything, for now.
    pub may: String,
    pub until_ms: u64,
    /// The host that signs.
    pub by: String,
}

impl Vouch {
    fn bytes(&self) -> Vec<u8> {
        // One shape, so that what is signed is what is checked.
        json!({
            "host": self.host,
            "name": self.name,
            "owner": self.owner,
            "may": self.may,
            "until_ms": self.until_ms,
            "by": self.by,
        })
        .to_string()
        .into_bytes()
    }
}

/// A vouch as it travels and is kept: the statement and its signature.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Vouched {
    pub vouch: Vouch,
    /// The signature, base64url.
    pub signature: String,
}

impl Vouched {
    /// Whether `signature` is the signer's over the statement.
    ///
    /// # Errors
    ///
    /// The signer is not a key, the signature is not one, or it is not theirs.
    pub fn check(&self) -> Result<(), PeersError> {
        let by: EndpointId = self
            .vouch
            .by
            .parse()
            .map_err(|_| PeersError::Invalid("the voucher is not a host".into()))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|_| PeersError::Invalid("the signature is not one".into()))?;
        let bytes: [u8; 64] = bytes
            .try_into()
            .map_err(|_| PeersError::Invalid("the signature is not one".into()))?;
        let signature = iroh::Signature::from_bytes(&bytes);
        by.verify(&self.vouch.bytes(), &signature)
            .map_err(|_| PeersError::Invalid("the signature is not the voucher's".into()))
    }
}

/// A host of the person's, as the page shows it.
#[derive(Clone, Debug, Serialize)]
pub struct HostShown {
    pub host_id: String,
    pub name: String,
    pub fingerprint: String,
    pub vouched_by: String,
    pub until_ms: u64,
    pub added_ms: u64,
    pub seen_ms: Option<u64>,
}

/// This host, as the page shows it: its face, and how to add it from a
/// page of another host.
#[derive(Clone, Debug, Serialize)]
pub struct ThisHost {
    pub host_id: String,
    pub name: String,
    pub fingerprint: String,
    /// Where it is reached, for another host to be given.
    pub address: String,
    /// The word to add it by, good for a while; made when asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word: Option<String>,
    pub word_good_for_minutes: u64,
}

/// What a host says of itself when two meet.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Introduced {
    pub host: String,
    pub name: String,
    pub address: String,
}

/// What the host that introduces says to the new one: the word the new
/// host showed, who is calling, and its vouch for the new host.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Meeting {
    pub word: String,
    pub caller: Introduced,
    pub vouched: Vouched,
    /// The other hosts of the person, each vouched for by the caller, so that
    /// the new host knows them all.
    #[serde(default)]
    pub others: Vec<(Introduced, Vouched)>,
}

/// One host told of another by a host it trusts.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Told {
    pub host: Introduced,
    pub vouched: Vouched,
}

fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ms_of(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

fn closed_to_others(path: &Path, directory: bool) -> Result<(), PeersError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = if directory { 0o700 } else { 0o600 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|error| {
            PeersError::Io {
                path: path.to_owned(),
                message: error.to_string(),
            }
        })?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, directory);
    }
    Ok(())
}

/// A fingerprint a person reads: the first letters of the key, in fours.
#[must_use]
pub fn fingerprint(id: &EndpointId) -> String {
    let whole = id.to_string();
    whole
        .chars()
        .take(12)
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Letters in groups of four, each letter drawn evenly.
fn a_word() -> Result<String, PeersError> {
    const LETTERS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = [0_u8; 12];
    getrandom::fill(&mut bytes)
        .map_err(|error| PeersError::Book(format!("no random bytes: {error}")))?;
    let letters: Vec<char> = bytes
        .iter()
        .map(|byte| LETTERS[usize::from(*byte) % LETTERS.len()] as char)
        .collect();
    Ok(letters
        .chunks(4)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-"))
}

/// An address a person pastes: the host's key and where it was last seen,
/// in one string.
///
/// # Errors
///
/// The string is not an address.
pub fn address_of(text: &str) -> Result<EndpointAddr, PeersError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text.trim())
        .map_err(|_| PeersError::Invalid("that is not a host's address".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| PeersError::Invalid("that is not a host's address".into()))
}

fn address_text(addr: &EndpointAddr) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(addr).unwrap_or_default())
}

/// The name this machine goes by.
fn this_machines_name() -> String {
    let said = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .filter(|name| !name.is_empty())
        })
        .or_else(|| {
            std::env::var("COMPUTERNAME")
                .ok()
                .filter(|name| !name.is_empty())
        });
    said.unwrap_or_else(|| "this host".to_owned())
}

/// This host's key, the book of the hosts it knows, and its endpoint.
pub struct Peers {
    secret: SecretKey,
    owner: Mutex<String>,
    book: Mutex<Book>,
    endpoint: OnceLock<Endpoint>,
    relay: RelayMode,
}

impl std::fmt::Debug for Peers {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Peers")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

impl Peers {
    /// Open this host under `root`, making its key and name at the first
    /// start. `relay` is how its peers reach it when nothing direct works.
    ///
    /// # Errors
    ///
    /// The root cannot be made, the key cannot be read or written, or the
    /// book cannot be opened.
    pub fn open(root: &Path, relay: RelayMode) -> Result<Self, PeersError> {
        let io = |path: &Path| {
            let path = path.to_owned();
            move |error: std::io::Error| PeersError::Io {
                path,
                message: error.to_string(),
            }
        };
        std::fs::create_dir_all(root).map_err(io(root))?;
        closed_to_others(root, true)?;
        let key_path = root.join("secret.key");
        let secret = match std::fs::read(&key_path) {
            Ok(bytes) => {
                let bytes: [u8; 32] = bytes.try_into().map_err(|_| PeersError::Io {
                    path: key_path.clone(),
                    message: "not a key".into(),
                })?;
                SecretKey::from_bytes(&bytes)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let secret = SecretKey::generate();
                crate::closed::write(&key_path, &secret.to_bytes()).map_err(io(&key_path))?;
                closed_to_others(&key_path, false)?;
                secret
            }
            Err(error) => return Err(io(&key_path)(error)),
        };
        let path = root.join("hosts.db");
        let book = Book::open(&path)?;
        closed_to_others(&path, false)?;
        book.pragma_update(None, "journal_mode", "WAL")?;
        book.busy_timeout(Duration::from_secs(5))?;
        book.execute_batch(SCHEMA)?;
        let name: Option<String> = book
            .query_row("SELECT value FROM mine WHERE key = 'name'", [], |row| {
                row.get(0)
            })
            .optional()?;
        if name.is_none() {
            book.execute(
                "INSERT INTO mine (key, value) VALUES ('name', ?1)",
                params![this_machines_name()],
            )?;
        }
        Ok(Self {
            secret,
            owner: Mutex::new("you".to_owned()),
            book: Mutex::new(book),
            endpoint: OnceLock::new(),
            relay,
        })
    }

    /// Whom this host belongs to, as the vouches it signs name them.
    pub fn owned_by(&self, owner: &str) {
        owner.clone_into(&mut held(&self.owner));
    }

    #[must_use]
    pub fn id(&self) -> EndpointId {
        self.secret.public()
    }

    #[must_use]
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.id())
    }

    /// What this host is called.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn name(&self) -> Result<String, PeersError> {
        Ok(held(&self.book)
            .query_row("SELECT value FROM mine WHERE key = 'name'", [], |row| {
                row.get(0)
            })
            .optional()?
            .unwrap_or_else(this_machines_name))
    }

    /// Call this host something.
    ///
    /// # Errors
    ///
    /// The name is empty, or the book cannot be written.
    pub fn call_it(&self, name: &str) -> Result<(), PeersError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(PeersError::Invalid("a host needs a name".into()));
        }
        held(&self.book).execute(
            "INSERT INTO mine (key, value) VALUES ('name', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![name],
        )?;
        Ok(())
    }

    /// Bind this host's endpoint, so that its peers can reach it and it
    /// them. Bound once; the endpoint lives as long as the host.
    ///
    /// # Errors
    ///
    /// The endpoint cannot be bound.
    pub async fn bind(&self) -> Result<Endpoint, PeersError> {
        if let Some(endpoint) = self.endpoint.get() {
            return Ok(endpoint.clone());
        }
        let builder = match self.relay {
            // Nothing of n0's, nothing found over DNS: for two hosts on one
            // machine, and for tests, which never reach a public relay.
            RelayMode::Disabled => Endpoint::builder(iroh::endpoint::presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .clear_address_lookup(),
            ref relay => Endpoint::builder(iroh::endpoint::presets::N0).relay_mode(relay.clone()),
        };
        let endpoint = builder
            .secret_key(self.secret.clone())
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|error| PeersError::Link(format!("this host cannot be reached: {error}")))?;
        let _ = self.endpoint.set(endpoint.clone());
        Ok(endpoint)
    }

    /// The endpoint, once bound.
    #[must_use]
    pub fn endpoint(&self) -> Option<&Endpoint> {
        self.endpoint.get()
    }

    /// Where this host is reached: its address as a string another host's
    /// page takes.
    ///
    /// # Errors
    ///
    /// The endpoint is not bound.
    pub fn address(&self) -> Result<String, PeersError> {
        let endpoint = self
            .endpoint
            .get()
            .ok_or_else(|| PeersError::Link("this host is not reachable yet".into()))?;
        Ok(address_text(&endpoint.addr()))
    }

    /// This host as the page shows it, with a fresh word when one is asked for.
    ///
    /// # Errors
    ///
    /// The book cannot be read or written.
    pub fn this_host(&self, with_a_word: bool) -> Result<ThisHost, PeersError> {
        let word = if with_a_word {
            Some(self.a_word_to_be_added_by()?)
        } else {
            None
        };
        Ok(ThisHost {
            host_id: self.id().to_string(),
            name: self.name()?,
            fingerprint: self.fingerprint(),
            address: self.address().unwrap_or_default(),
            word,
            word_good_for_minutes: A_WORD_IS_GOOD_FOR.as_secs() / 60,
        })
    }

    /// A word another host's page adds this host by. Good for a while, used
    /// once; the one shown last is the one that counts.
    ///
    /// # Errors
    ///
    /// The book cannot be written.
    pub fn a_word_to_be_added_by(&self) -> Result<String, PeersError> {
        let book = held(&self.book);
        let now = now_ms();
        let good_since = now.saturating_sub(ms_of(A_WORD_IS_GOOD_FOR));
        if let Some(word) = book
            .query_row(
                "SELECT word FROM words WHERE used_ms IS NULL AND made_ms > ?1 \
                 ORDER BY made_ms DESC LIMIT 1",
                params![good_since],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(word);
        }
        book.execute("DELETE FROM words WHERE used_ms IS NULL", [])?;
        let word = a_word()?;
        book.execute(
            "INSERT INTO words (word, made_ms) VALUES (?1, ?2)",
            params![word, now],
        )?;
        Ok(word)
    }

    /// Use the word this host showed: once, while fresh.
    ///
    /// # Errors
    ///
    /// It is not the word, or it was used, or it is old.
    pub fn use_the_word(&self, word: &str) -> Result<(), PeersError> {
        let book = held(&self.book);
        let now = now_ms();
        let good_since = now.saturating_sub(ms_of(A_WORD_IS_GOOD_FOR));
        let changed = book.execute(
            "UPDATE words SET used_ms = ?1 WHERE word = ?2 AND used_ms IS NULL AND made_ms > ?3",
            params![now, word.trim(), good_since],
        )?;
        if changed == 0 {
            return Err(PeersError::Refused(
                "that is not the word this host showed, or it is used or old".into(),
            ));
        }
        Ok(())
    }

    /// Vouch for `host`: whose it is, its name, what it may, until weeks from now.
    #[must_use]
    pub fn vouch_for(&self, host: &EndpointId, name: &str) -> Vouched {
        let vouch = Vouch {
            host: host.to_string(),
            name: name.to_owned(),
            owner: held(&self.owner).clone(),
            may: "everything".into(),
            until_ms: now_ms() + ms_of(A_VOUCH_IS_GOOD_FOR),
            by: self.id().to_string(),
        };
        let signature = self.secret.sign(&vouch.bytes());
        Vouched {
            vouch,
            signature: base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(signature.to_bytes()),
        }
    }

    /// Whether `by` is a host this one trusts now: itself, or one in the
    /// book, not forgotten, within its time.
    fn trusts(&self, by: &str) -> Result<bool, PeersError> {
        if by == self.id().to_string() {
            return Ok(true);
        }
        let now = now_ms();
        let count: i64 = held(&self.book).query_row(
            "SELECT COUNT(*) FROM hosts WHERE host_id = ?1 AND forgotten_ms IS NULL AND until_ms > ?2",
            params![by, now],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Keep `host` as one of the person's, by `vouched`, checked: the
    /// signature is the voucher's, the voucher is trusted, the time is not
    /// up, the vouch is for this host.
    ///
    /// # Errors
    ///
    /// The vouch does not hold.
    pub fn keep(&self, host: &Introduced, vouched: &Vouched) -> Result<(), PeersError> {
        vouched.check()?;
        if vouched.vouch.host != host.host {
            return Err(PeersError::Invalid("the vouch is for another host".into()));
        }
        if vouched.vouch.until_ms <= now_ms() {
            return Err(PeersError::Invalid("the vouch is past its time".into()));
        }
        if !self.trusts(&vouched.vouch.by)? {
            return Err(PeersError::Refused(format!(
                "{} is vouched for by a host this one does not trust",
                host.name
            )));
        }
        if host.host == self.id().to_string() {
            return Err(PeersError::Invalid("a host does not keep itself".into()));
        }
        let now = now_ms();
        held(&self.book).execute(
            "INSERT INTO hosts (host_id, name, addr, vouch, signature, vouched_by, until_ms, added_ms, seen_ms, forgotten_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, NULL) \
             ON CONFLICT(host_id) DO UPDATE SET name = excluded.name, addr = excluded.addr, \
             vouch = excluded.vouch, signature = excluded.signature, vouched_by = excluded.vouched_by, \
             until_ms = excluded.until_ms, seen_ms = excluded.seen_ms, forgotten_ms = NULL",
            params![
                host.host,
                host.name,
                host.address,
                serde_json::to_string(&vouched.vouch).unwrap_or_default(),
                vouched.signature,
                vouched.vouch.by,
                vouched.vouch.until_ms,
                now,
            ],
        )?;
        Ok(())
    }

    /// Seen now: the two met, and this host renews its own vouch for the
    /// other, so that a host that keeps meeting its peers never falls out.
    fn seen(
        &self,
        host: &EndpointId,
        name: &str,
        addr: Option<&EndpointAddr>,
    ) -> Result<(), PeersError> {
        let vouched = self.vouch_for(host, name);
        let now = now_ms();
        let book = held(&self.book);
        match addr {
            Some(addr) => book.execute(
                "UPDATE hosts SET name = ?2, addr = ?3, vouch = ?4, signature = ?5, vouched_by = ?6, until_ms = ?7, seen_ms = ?8 \
                 WHERE host_id = ?1 AND forgotten_ms IS NULL",
                params![
                    host.to_string(),
                    name,
                    address_text(addr),
                    serde_json::to_string(&vouched.vouch).unwrap_or_default(),
                    vouched.signature,
                    vouched.vouch.by,
                    vouched.vouch.until_ms,
                    now
                ],
            )?,
            None => book.execute(
                "UPDATE hosts SET vouch = ?2, signature = ?3, vouched_by = ?4, until_ms = ?5, seen_ms = ?6 \
                 WHERE host_id = ?1 AND forgotten_ms IS NULL",
                params![
                    host.to_string(),
                    serde_json::to_string(&vouched.vouch).unwrap_or_default(),
                    vouched.signature,
                    vouched.vouch.by,
                    vouched.vouch.until_ms,
                    now
                ],
            )?,
        };
        Ok(())
    }

    /// The hosts of the person, as the page shows them: not forgotten.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn hosts(&self) -> Result<Vec<HostShown>, PeersError> {
        let book = held(&self.book);
        let mut rows = book.prepare(
            "SELECT host_id, name, vouched_by, until_ms, added_ms, seen_ms FROM hosts \
             WHERE forgotten_ms IS NULL ORDER BY added_ms",
        )?;
        let shown = rows
            .query_map([], |row| {
                let host_id: String = row.get(0)?;
                Ok(HostShown {
                    fingerprint: host_id
                        .parse::<EndpointId>()
                        .map(|id| fingerprint(&id))
                        .unwrap_or_default(),
                    host_id,
                    name: row.get(1)?,
                    vouched_by: row.get(2)?,
                    until_ms: row.get(3)?,
                    added_ms: row.get(4)?,
                    seen_ms: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(shown)
    }

    /// The hosts this one knows, each with its vouch, for telling a new
    /// host of them.
    fn known(&self) -> Result<Vec<(Introduced, Vouched)>, PeersError> {
        let book = held(&self.book);
        let mut rows = book.prepare(
            "SELECT host_id, name, addr, vouch, signature FROM hosts \
             WHERE forgotten_ms IS NULL AND until_ms > ?1",
        )?;
        let known = rows
            .query_map(params![now_ms()], |row| {
                let vouch: String = row.get(3)?;
                Ok((
                    Introduced {
                        host: row.get(0)?,
                        name: row.get(1)?,
                        address: row.get(2)?,
                    },
                    serde_json::from_str::<Vouch>(&vouch).map(|vouch| Vouched {
                        vouch,
                        signature: row.get(4).unwrap_or_default(),
                    }),
                ))
            })?
            .filter_map(Result::ok)
            .filter_map(|(host, vouched)| vouched.ok().map(|vouched| (host, vouched)))
            .collect();
        Ok(known)
    }

    /// Whether `host` is one of the person's now, and what it is called.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn trusted(&self, host: &EndpointId) -> Result<Option<String>, PeersError> {
        Ok(held(&self.book)
            .query_row(
                "SELECT name FROM hosts WHERE host_id = ?1 AND forgotten_ms IS NULL AND until_ms > ?2",
                params![host.to_string(), now_ms()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Where `host` was last seen.
    fn addr_of(&self, host: &EndpointId) -> Result<EndpointAddr, PeersError> {
        let text: Option<String> = held(&self.book)
            .query_row(
                "SELECT addr FROM hosts WHERE host_id = ?1 AND forgotten_ms IS NULL AND until_ms > ?2",
                params![host.to_string(), now_ms()],
                |row| row.get(0),
            )
            .optional()?;
        match text {
            Some(text) => address_of(&text),
            None => Err(PeersError::Refused(format!(
                "{} is not one of your hosts",
                fingerprint(host)
            ))),
        }
    }

    /// Forget `host`: it comes in no more, and is reached no more.
    ///
    /// # Errors
    ///
    /// It is not one of the person's hosts.
    pub fn forget(&self, host: &EndpointId) -> Result<String, PeersError> {
        let book = held(&self.book);
        let name: Option<String> = book
            .query_row(
                "SELECT name FROM hosts WHERE host_id = ?1 AND forgotten_ms IS NULL",
                params![host.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(name) = name else {
            return Err(PeersError::Refused("that is not one of your hosts".into()));
        };
        book.execute(
            "UPDATE hosts SET forgotten_ms = ?2 WHERE host_id = ?1",
            params![host.to_string(), now_ms()],
        )?;
        Ok(name)
    }

    // --- The link.

    /// Reach `host` and ask it, over its own door. One request per stream.
    ///
    /// # Errors
    ///
    /// The host is not one of the person's, cannot be reached, or does not
    /// answer in HTTP.
    pub async fn ask(
        &self,
        host: &EndpointId,
        request: hyper::Request<http_body_util::Full<hyper::body::Bytes>>,
    ) -> Result<hyper::Response<hyper::body::Incoming>, PeersError> {
        let addr = self.addr_of(host)?;
        let endpoint = self.bind().await?;
        let connection = endpoint.connect(addr, ALPN).await.map_err(|error| {
            PeersError::Link(format!("{} is out of reach: {error}", fingerprint(host)))
        })?;
        let response = ask_over(&connection, request).await?;
        let _ = self.seen(host, &self.trusted(host)?.unwrap_or_default(), None);
        Ok(response)
    }

    /// Introduce a new host: reach it at `address`, give it `word`, vouch for
    /// it, tell it of the others, and keep it. The new host's name is what
    /// it says it is.
    ///
    /// # Errors
    ///
    /// The address is not one, the host is out of reach, or it refuses the word.
    pub async fn introduce(&self, address: &str, word: &str) -> Result<HostShown, PeersError> {
        let addr = address_of(address)?;
        let endpoint = self.bind().await?;
        let connection = endpoint
            .connect(addr.clone(), ALPN)
            .await
            .map_err(|error| PeersError::Link(format!("the host is out of reach: {error}")))?;
        let new = connection.remote_id();
        let meeting = Meeting {
            word: word.trim().to_owned(),
            caller: Introduced {
                host: self.id().to_string(),
                name: self.name()?,
                address: self.address()?,
            },
            vouched: self.vouch_for(&new, "a new host"),
            others: self.known()?,
        };
        let request = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri("/api/hosts/meet")
            .header("content-type", "application/json")
            .body(http_body_util::Full::new(hyper::body::Bytes::from(
                serde_json::to_vec(&meeting).unwrap_or_default(),
            )))
            .map_err(|error| PeersError::Link(error.to_string()))?;
        let response = ask_over(&connection, request).await?;
        let status = response.status();
        let body = read_body(response).await?;
        if !status.is_success() {
            let said = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the host refused")
                .to_owned();
            return Err(PeersError::Refused(said));
        }
        let introduced: Introduced = serde_json::from_value(body).map_err(|_| {
            PeersError::Link("the host answered in words this one does not know".into())
        })?;
        if introduced.host != new.to_string() {
            return Err(PeersError::Refused("the host answered as another".into()));
        }
        let vouched = self.vouch_for(&new, &introduced.name);
        self.keep(&introduced, &vouched)?;
        // The others are told of the new one, best effort: a host that is
        // asleep hears of it when the two meet.
        for (other, _) in self.known()? {
            if other.host == introduced.host {
                continue;
            }
            let Ok(other_id) = other.host.parse::<EndpointId>() else {
                continue;
            };
            let told = Told {
                host: introduced.clone(),
                vouched: vouched.clone(),
            };
            let _ = self.tell(&other_id, &told).await;
        }
        self.hosts()?
            .into_iter()
            .find(|shown| shown.host_id == introduced.host)
            .ok_or_else(|| PeersError::Book("the host was kept and is not there".into()))
    }

    async fn tell(&self, host: &EndpointId, told: &Told) -> Result<(), PeersError> {
        let request = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri("/api/hosts/told")
            .header("content-type", "application/json")
            .body(http_body_util::Full::new(hyper::body::Bytes::from(
                serde_json::to_vec(told).unwrap_or_default(),
            )))
            .map_err(|error| PeersError::Link(error.to_string()))?;
        let _ = self.ask(host, request).await?;
        Ok(())
    }

    /// Met by a host that introduces this one: `meeting` came over a stream
    /// from `caller`, who is not trusted yet, and whoever answers checked
    /// the word first - this host's own, or the one its first start printed.
    ///
    /// # Errors
    ///
    /// The vouch does not hold.
    pub fn met(&self, caller: &EndpointId, meeting: &Meeting) -> Result<Introduced, PeersError> {
        if meeting.caller.host != caller.to_string() {
            return Err(PeersError::Refused("the caller is not who it says".into()));
        }
        // The caller's vouch for this host: checked, and kept for showing.
        meeting.vouched.check()?;
        if meeting.vouched.vouch.host != self.id().to_string()
            || meeting.vouched.vouch.by != meeting.caller.host
        {
            return Err(PeersError::Refused("the vouch is not for this host".into()));
        }
        held(&self.book).execute(
            "INSERT INTO mine (key, value) VALUES ('introduced', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![serde_json::to_string(&meeting.vouched).unwrap_or_default()],
        )?;
        // The caller is the person's: the word said so. This host vouches
        // for it itself, and trusts the others on the caller's word.
        let vouched = self.vouch_for(caller, &meeting.caller.name);
        self.keep(&meeting.caller, &vouched)?;
        for (other, vouched) in &meeting.others {
            let _ = self.keep(other, vouched);
        }
        Ok(Introduced {
            host: self.id().to_string(),
            name: self.name()?,
            address: self.address()?,
        })
    }

    /// Told of another host by a trusted one.
    ///
    /// # Errors
    ///
    /// The vouch does not hold.
    pub fn told(&self, by: &EndpointId, told: &Told) -> Result<(), PeersError> {
        if told.vouched.vouch.by != by.to_string() {
            return Err(PeersError::Refused("the vouch is not the teller's".into()));
        }
        self.keep(&told.host, &told.vouched)
    }

    /// A host this one knows met it: seen now, this host's vouch renewed.
    pub fn met_again(&self, host: &EndpointId, name: &str) {
        let _ = self.seen(host, name, None);
    }
}

/// Ask over one new stream of `connection`, and read the answer.
async fn ask_over(
    connection: &Connection,
    request: hyper::Request<http_body_util::Full<hyper::body::Bytes>>,
) -> Result<hyper::Response<hyper::body::Incoming>, PeersError> {
    let (send, recv) = connection
        .open_bi()
        .await
        .map_err(|error| PeersError::Link(format!("no stream to the host: {error}")))?;
    let stream = tokio::io::join(recv, send);
    let (mut sender, link) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .map_err(|error| {
                PeersError::Link(format!("the host did not answer in HTTP: {error}"))
            })?;
    tokio::spawn(async move {
        let _ = link.await;
    });
    sender
        .send_request(request)
        .await
        .map_err(|error| PeersError::Link(format!("the host did not answer: {error}")))
}

/// The body of an answer, as JSON.
///
/// # Errors
///
/// The body cannot be read, or is not JSON.
pub async fn read_body(
    response: hyper::Response<hyper::body::Incoming>,
) -> Result<Value, PeersError> {
    use http_body_util::BodyExt as _;
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|error| PeersError::Link(error.to_string()))?
        .to_bytes();
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| PeersError::Link("the host did not answer in JSON".into()))
}

/// Who came over a stream: a host of the person's, by name, or one not yet.
#[derive(Clone, Debug)]
pub enum Came {
    Trusted { host: EndpointId, name: String },
    Unknown { host: EndpointId },
}

/// Answer every stream of `connection` with `answer`, one request a stream,
/// until it closes.
pub async fn serve_connection<Answer, Fut>(connection: Connection, came: Came, answer: Answer)
where
    Answer: Fn(Came, hyper::Request<hyper::body::Incoming>) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = hyper::Response<crate::workbench_shell::ShellBody>>
        + Send
        + 'static,
{
    loop {
        let Ok((send, recv)) = connection.accept_bi().await else {
            break;
        };
        let came = came.clone();
        let answer = answer.clone();
        tokio::spawn(async move {
            let stream = tokio::io::join(recv, send);
            let service = hyper::service::service_fn(move |request| {
                let came = came.clone();
                let answer = answer.clone();
                async move { Ok::<_, std::convert::Infallible>(answer(came, request).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
        });
    }
}

/// How peers reach this host when nothing direct works, from what a person
/// said: `none` for no relay at all, a URL for a relay of their own, or
/// nothing said for the relays of n0.
///
/// # Errors
///
/// The text is neither `none` nor a URL.
pub fn relay_mode_of(said: Option<&str>) -> Result<RelayMode, PeersError> {
    match said.map(str::trim) {
        None | Some("") => Ok(RelayMode::Default),
        Some("none") => Ok(RelayMode::Disabled),
        Some(url) => {
            let url: iroh::RelayUrl = url
                .parse()
                .map_err(|_| PeersError::Invalid(format!("{url} is not a relay's address")))?;
            Ok(RelayMode::Custom(iroh::RelayMap::from(url)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "swem-peers-{label}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).expect("a root");
        root
    }

    #[test]
    fn a_key_is_made_once_and_kept() {
        let root = root("key");
        let first = Peers::open(&root, RelayMode::Disabled).expect("open").id();
        let again = Peers::open(&root, RelayMode::Disabled)
            .expect("open again")
            .id();
        assert_eq!(first, again);
        let bytes = std::fs::read(root.join("secret.key")).expect("the key file");
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn a_vouch_holds_only_as_signed() {
        let a = Peers::open(&root("a"), RelayMode::Disabled).expect("a");
        let b = Peers::open(&root("b"), RelayMode::Disabled).expect("b");
        let vouched = a.vouch_for(&b.id(), "b");
        vouched.check().expect("a's own signature holds");
        let mut bent = vouched.clone();
        bent.vouch.name = "c".into();
        assert!(bent.check().is_err(), "a changed statement does not hold");
        let mut forged = vouched;
        forged.vouch.by = b.id().to_string();
        assert!(
            forged.check().is_err(),
            "another's name on it does not hold"
        );
    }

    #[test]
    fn a_host_is_kept_only_on_a_trusted_word_and_forgotten_on_request() {
        let a = Peers::open(&root("a2"), RelayMode::Disabled).expect("a");
        let b = Peers::open(&root("b2"), RelayMode::Disabled).expect("b");
        let c = Peers::open(&root("c2"), RelayMode::Disabled).expect("c");
        let b_said = Introduced {
            host: b.id().to_string(),
            name: "b".into(),
            address: String::new(),
        };
        // c vouches for b; a does not trust c: refused.
        assert!(a.keep(&b_said, &c.vouch_for(&b.id(), "b")).is_err());
        // a vouches for b itself: kept.
        a.keep(&b_said, &a.vouch_for(&b.id(), "b")).expect("kept");
        assert_eq!(a.trusted(&b.id()).expect("read"), Some("b".to_owned()));
        // Now b vouches for c: a trusts b, so c is kept.
        let c_said = Introduced {
            host: c.id().to_string(),
            name: "c".into(),
            address: String::new(),
        };
        a.keep(&c_said, &b.vouch_for(&c.id(), "c"))
            .expect("kept on b's word");
        assert_eq!(a.hosts().expect("hosts").len(), 2);
        a.forget(&b.id()).expect("forgotten");
        assert_eq!(a.trusted(&b.id()).expect("read"), None);
        assert_eq!(a.hosts().expect("hosts").len(), 1);
    }

    #[test]
    fn a_word_is_used_once_and_only_while_fresh() {
        let a = Peers::open(&root("w"), RelayMode::Disabled).expect("a");
        let word = a.a_word_to_be_added_by().expect("a word");
        assert_eq!(a.a_word_to_be_added_by().expect("the same word"), word);
        a.use_the_word(&word).expect("used");
        assert!(a.use_the_word(&word).is_err(), "not twice");
        assert!(a.use_the_word("nope-nope-nope").is_err());
    }
}
