//! Who may come in.
//!
//! A Workbench on the machine a person sits at is opened by the secret of
//! its run. A Workbench served at an address is opened by whoever was let
//! in here: a device of the owner's, holding a passkey; a program, holding
//! a token that says what it may do; and, when every device is lost, one of
//! the codes the owner was shown once.
//!
//! Nothing of the ceremony is of this product's making: a passkey is
//! `WebAuthn` as browsers carry it, checked by `webauthn-rs`. What is kept
//! here is who was let in, the halves of their keys that check them, and
//! what was done. Every secret a person or a program comes with - a word, a
//! code, a session, a token - is kept as its digest and cannot be read back.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use url::Url;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Uuid, Webauthn, WebauthnBuilder,
};

use crate::now_ms;

/// A word is good for this long when it lets a second device in.
const A_WORD_FOR_A_DEVICE_MS: i64 = 10 * 60 * 1000;
/// A word printed at the first start is good until the next start.
const A_WORD_AT_THE_FIRST_START_MS: i64 = 24 * 60 * 60 * 1000;
/// A ceremony begun and not finished is forgotten after this long.
const A_CEREMONY_MS: i64 = 5 * 60 * 1000;
/// A session nobody came with for this long is over.
const A_SESSION_UNSEEN_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// When a session was last seen is written down no more often than this.
const SEEN_EVERY_MS: i64 = 60 * 1000;
/// How many codes to come back with are made at once.
pub const CODES_MADE: usize = 8;
/// Tries that failed are counted over this long.
const TRIES_OVER_MS: i64 = 15 * 60 * 1000;
/// How many may fail from one place before it is refused for a while.
const TRIES_FROM_ONE_PLACE: usize = 5;
/// How many may fail from everywhere before everybody is refused for a while.
const TRIES_FROM_EVERYWHERE: usize = 40;

/// What was done is kept this far back, so that whoever knocks all day
/// fills nothing up.
const HAPPENED_KEPT: i64 = 5000;

/// Letters a person can read aloud and type without mistaking one for another.
const LETTERS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta(
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS words(
  digest TEXT PRIMARY KEY,
  first INTEGER NOT NULL,
  made_ms INTEGER NOT NULL,
  good_until_ms INTEGER NOT NULL,
  used_ms INTEGER
);
CREATE TABLE IF NOT EXISTS devices(
  device_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  credential TEXT NOT NULL UNIQUE,
  passkey TEXT NOT NULL,
  added_ms INTEGER NOT NULL,
  last_ms INTEGER,
  last_from TEXT,
  removed_ms INTEGER
);
CREATE TABLE IF NOT EXISTS sessions(
  digest TEXT PRIMARY KEY,
  device_id TEXT,
  made_ms INTEGER NOT NULL,
  seen_ms INTEGER NOT NULL,
  from_place TEXT NOT NULL,
  ended_ms INTEGER
);
CREATE TABLE IF NOT EXISTS codes(
  digest TEXT PRIMARY KEY,
  made_ms INTEGER NOT NULL,
  used_ms INTEGER
);
CREATE TABLE IF NOT EXISTS tokens(
  token_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  digest TEXT NOT NULL UNIQUE,
  may TEXT NOT NULL,
  made_ms INTEGER NOT NULL,
  last_ms INTEGER,
  last_from TEXT,
  withdrawn_ms INTEGER
);
CREATE TABLE IF NOT EXISTS happened(
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  at_ms INTEGER NOT NULL,
  what TEXT NOT NULL,
  refused INTEGER NOT NULL,
  who TEXT NOT NULL,
  from_place TEXT NOT NULL
);
";

#[derive(Debug, thiserror::Error)]
pub enum AccessError {
    /// What was asked cannot be, and why may be said to whoever asked.
    #[error("{0}")]
    Invalid(String),
    /// Whoever asked is not let in. The words are for a person who came
    /// with the wrong thing; they say nothing of what would have been right.
    #[error("{0}")]
    Refused(String),
    /// Too many tries failed.
    #[error("too many tries failed; try again in {minutes} minutes")]
    TooManyTries { minutes: i64 },
    #[error("who may come in could not be read at {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("who may come in could not be read: {0}")]
    Book(String),
}

impl From<rusqlite::Error> for AccessError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Book(error.to_string())
    }
}

/// What a program that holds a token may do.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum May {
    /// Whatever the page does.
    Everything,
    /// Say that it is time to look at what is due, and nothing else.
    SayWhatIsDue,
}

/// By what somebody came.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "by", rename_all = "kebab-case")]
pub enum CameBy {
    /// The secret of this run, on the machine the Workbench runs on.
    ThisRun,
    /// A device of the owner's, signed in.
    Device { device_id: String, name: String },
    /// A code to come back with. Nothing but a device can be added so.
    Code,
    /// A token a program holds.
    Token {
        token_id: String,
        name: String,
        may: Vec<May>,
    },
    /// The word of the product this harness is built into.
    Embedder,
}

/// Who asks. Found before anything is answered.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Principal {
    #[serde(flatten)]
    pub by: CameBy,
}

impl Principal {
    #[must_use]
    pub fn of_this_run() -> Self {
        Self {
            by: CameBy::ThisRun,
        }
    }

    /// Whether whoever asks may do this.
    #[must_use]
    pub fn may(&self, what: May) -> bool {
        match &self.by {
            CameBy::ThisRun | CameBy::Device { .. } | CameBy::Embedder => true,
            // A code lets a person in to register a device; the routes that
            // do that ask for nothing here.
            CameBy::Code => false,
            CameBy::Token { may, .. } => may.contains(&May::Everything) || may.contains(&what),
        }
    }

    /// Who it was, in the words of what was done.
    #[must_use]
    pub fn named(&self) -> String {
        match &self.by {
            CameBy::ThisRun => "this computer".into(),
            CameBy::Device { name, .. } => name.clone(),
            CameBy::Code => "a code to come back with".into(),
            CameBy::Token { name, .. } => format!("token {name}"),
            CameBy::Embedder => "the product".into(),
        }
    }
}

/// A device that may come in.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Device {
    pub device_id: String,
    pub name: String,
    pub added_ms: i64,
    pub last_ms: Option<i64>,
    pub last_from: Option<String>,
}

/// A token, without what opens with it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Token {
    pub token_id: String,
    pub name: String,
    pub may: Vec<May>,
    pub made_ms: i64,
    pub last_ms: Option<i64>,
    pub last_from: Option<String>,
}

/// Something that was done, or refused.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Happened {
    pub at_ms: i64,
    pub what: String,
    pub refused: bool,
    pub who: String,
    pub from: String,
}

/// The codes to come back with, as far as they may be known.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CodesLeft {
    pub left: usize,
    pub made: usize,
    pub made_ms: Option<i64>,
}

/// What a browser is handed to begin a ceremony with.
#[derive(Clone, Debug, Serialize)]
pub struct Begun {
    pub ceremony: String,
    /// What the browser's own `WebAuthn` call is given.
    pub asked: Value,
}

/// Somebody who came in.
#[derive(Clone, Debug)]
pub struct CameIn {
    /// What the browser keeps, in a cookie the page cannot read.
    pub session: String,
    pub principal: Principal,
    /// The codes to come back with, when this made them. Said once.
    pub codes: Option<Vec<String>>,
}

enum Ceremony {
    Registering {
        word: String,
        first: bool,
        name: String,
        state: PasskeyRegistration,
    },
    SigningIn {
        state: PasskeyAuthentication,
    },
}

struct Begin {
    made_ms: i64,
    ceremony: Ceremony,
}

#[derive(Default)]
struct Tries {
    failed: Vec<(i64, String)>,
}

/// The book of who may come in, for a Workbench served at one address.
pub struct Access {
    path: PathBuf,
    address: Url,
    /// The name a passkey shows for whom it lets in.
    owner: String,
    webauthn: Webauthn,
    book: Mutex<Connection>,
    ceremonies: Mutex<BTreeMap<String, Begin>>,
    tries: Mutex<Tries>,
}

impl std::fmt::Debug for Access {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Access")
            .field("path", &self.path)
            .field("address", &self.address.as_str())
            .finish_non_exhaustive()
    }
}

fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

fn random<const N: usize>() -> Result<[u8; N], AccessError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|error| AccessError::Book(format!("no random bytes: {error}")))?;
    Ok(bytes)
}

/// Letters in groups of four, each letter drawn evenly.
fn spelled(groups: usize) -> Result<String, AccessError> {
    let mut word = String::new();
    let mut drawn = 0;
    // A byte is thrown away when it would favour the first letters.
    let even = u8::try_from(256 - 256 % LETTERS.len()).unwrap_or(u8::MAX);
    while drawn < groups * 4 {
        for byte in random::<16>()? {
            if byte >= even || drawn == groups * 4 {
                continue;
            }
            if drawn > 0 && drawn % 4 == 0 {
                word.push('-');
            }
            word.push(char::from(LETTERS[usize::from(byte) % LETTERS.len()]));
            drawn += 1;
        }
    }
    Ok(word)
}

fn secret(prefix: &str) -> Result<String, AccessError> {
    Ok(format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random::<32>()?)
    ))
}

fn identifier(prefix: &str) -> Result<String, AccessError> {
    let mut id = String::from(prefix);
    for byte in random::<8>()? {
        let _ = write!(id, "{byte:02x}");
    }
    Ok(id)
}

/// What is kept of a secret: its digest.
fn digest(of: &str) -> String {
    let mut text = String::new();
    for byte in Sha256::digest(of.as_bytes()) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// A word or a code as it was typed, with what a person adds taken out.
fn as_typed(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|letter| letter.to_ascii_lowercase())
        .collect()
}

fn closed_to_others(path: &Path, directory: bool) -> Result<(), AccessError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = if directory { 0o700 } else { 0o600 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
            AccessError::Io {
                path: path.to_owned(),
                message: error.to_string(),
            }
        })?;
    }
    #[cfg(not(unix))]
    let _ = (path, directory);
    Ok(())
}

impl Access {
    /// Open the book kept in `root` for a Workbench served at `address`.
    ///
    /// # Errors
    ///
    /// The address is not one a passkey can belong to, or the book cannot
    /// be opened and closed to everybody else.
    pub fn open(root: &Path, address: &Url, owner: &str) -> Result<Self, AccessError> {
        let Some(name) = address.host_str() else {
            return Err(AccessError::Invalid(format!(
                "{address} names no place a passkey can belong to"
            )));
        };
        let webauthn = WebauthnBuilder::new(name, address)
            .map(|builder| builder.rp_name("SWEM Workbench"))
            .and_then(WebauthnBuilder::build)
            .map_err(|error| {
                AccessError::Invalid(format!("a passkey cannot belong to {address}: {error}"))
            })?;
        let io = |path: &Path| {
            let path = path.to_owned();
            move |error: std::io::Error| AccessError::Io {
                path,
                message: error.to_string(),
            }
        };
        fs::create_dir_all(root).map_err(io(root))?;
        closed_to_others(root, true)?;
        let path = root.join("access.db");
        let book = Connection::open(&path)?;
        closed_to_others(&path, false)?;
        book.pragma_update(None, "journal_mode", "WAL")?;
        book.busy_timeout(std::time::Duration::from_secs(5))?;
        book.execute_batch(SCHEMA)?;
        Ok(Self {
            path,
            address: address.clone(),
            owner: owner.to_owned(),
            webauthn,
            book: Mutex::new(book),
            ceremonies: Mutex::new(BTreeMap::new()),
            tries: Mutex::new(Tries::default()),
        })
    }

    /// The address this Workbench is served at.
    #[must_use]
    pub fn address(&self) -> &Url {
        &self.address
    }

    /// Whether somebody made this Workbench theirs.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn claimed(&self) -> Result<bool, AccessError> {
        let devices: i64 = held(&self.book).query_row(
            "SELECT COUNT(*) FROM devices WHERE removed_ms IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(devices > 0)
    }

    /// The word of a first start: made when this Workbench belongs to
    /// nobody, and none when it belongs to somebody. A word made at an
    /// earlier start and never used is good no more.
    ///
    /// # Errors
    ///
    /// The book cannot be written.
    pub fn word_of_a_first_start(&self) -> Result<Option<String>, AccessError> {
        if self.claimed()? {
            return Ok(None);
        }
        held(&self.book).execute("DELETE FROM words WHERE first = 1 AND used_ms IS NULL", [])?;
        self.word(true, A_WORD_AT_THE_FIRST_START_MS).map(Some)
    }

    /// A word that lets one more device be registered. Used once, soon.
    ///
    /// # Errors
    ///
    /// Whoever asks may not, or the book cannot be written.
    pub fn word_for_a_device(&self, who: &Principal, from: &str) -> Result<String, AccessError> {
        if !matches!(
            who.by,
            CameBy::ThisRun | CameBy::Device { .. } | CameBy::Code | CameBy::Embedder
        ) {
            return Err(AccessError::Refused(
                "a token does not let a device in".into(),
            ));
        }
        let word = self.word(false, A_WORD_FOR_A_DEVICE_MS)?;
        self.write_down(
            "A word was made for one more device",
            false,
            &who.named(),
            from,
        )?;
        Ok(word)
    }

    fn word(&self, first: bool, good_for_ms: i64) -> Result<String, AccessError> {
        let word = spelled(4)?;
        let now = now_ms();
        held(&self.book).execute(
            "INSERT INTO words(digest, first, made_ms, good_until_ms) VALUES (?1, ?2, ?3, ?4)",
            params![digest(&as_typed(&word)), first, now, now + good_for_ms],
        )?;
        Ok(word)
    }

    /// Begin registering a device for whoever came with a word.
    ///
    /// # Errors
    ///
    /// The word is not one that was given, too many tries failed, or the
    /// name is empty.
    pub fn begin_registering(
        &self,
        word: &str,
        name: &str,
        from: &str,
    ) -> Result<Begun, AccessError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(AccessError::Invalid(
                "a device is given a name of sixty letters at most".into(),
            ));
        }
        self.not_too_many_tries(from)?;
        let word = digest(&as_typed(word));
        let now = now_ms();
        let first: Option<bool> = held(&self.book)
            .query_row(
                "SELECT first FROM words WHERE digest = ?1 AND used_ms IS NULL AND good_until_ms > ?2",
                params![word, now],
                |row| row.get(0),
            )
            .optional()?;
        let Some(first) = first else {
            self.failed(from);
            self.write_down("Refused: a word nobody was given", true, "", from)?;
            return Err(AccessError::Refused(
                "This is not a word that was given, or it was used, or it is too old.".into(),
            ));
        };
        if first && self.claimed()? {
            return Err(AccessError::Refused(
                "This Workbench belongs to somebody already.".into(),
            ));
        }
        let known = self
            .passkeys()?
            .into_iter()
            .map(|(_, _, passkey)| passkey.cred_id().clone())
            .collect::<Vec<_>>();
        let (asked, state) = self
            .webauthn
            .start_passkey_registration(self.owner_id()?, &self.owner, &self.owner, Some(known))
            .map_err(|error| AccessError::Book(format!("the ceremony did not begin: {error}")))?;
        self.begun(
            Ceremony::Registering {
                word,
                first,
                name: name.to_owned(),
                state,
            },
            &asked,
        )
    }

    /// Finish registering the device: what its browser answered is checked,
    /// the word is used, and whoever holds the device is in.
    ///
    /// # Errors
    ///
    /// The ceremony is not one that was begun, the answer does not check,
    /// or the word was used meanwhile.
    pub fn finish_registering(
        &self,
        ceremony: &str,
        answered: &Value,
        from: &str,
    ) -> Result<CameIn, AccessError> {
        let Some(Ceremony::Registering {
            word,
            first,
            name,
            state,
        }) = self.taken(ceremony)
        else {
            return Err(AccessError::Refused(
                "This was not begun here, or it took too long. Begin again.".into(),
            ));
        };
        let answered: RegisterPublicKeyCredential = serde_json::from_value(answered.clone())
            .map_err(|error| {
                AccessError::Invalid(format!("not what a browser answers: {error}"))
            })?;
        let passkey = match self.webauthn.finish_passkey_registration(&answered, &state) {
            Ok(passkey) => passkey,
            Err(error) => {
                self.failed(from);
                self.write_down("Refused: a device that did not check", true, &name, from)?;
                return Err(AccessError::Refused(format!(
                    "The device did not check: {error}"
                )));
            }
        };
        let device_id = identifier("d_")?;
        let credential = serde_json::to_string(passkey.cred_id())
            .map_err(|error| AccessError::Book(error.to_string()))?;
        let kept = serde_json::to_string(&passkey)
            .map_err(|error| AccessError::Book(error.to_string()))?;
        let now = now_ms();
        {
            let mut book = held(&self.book);
            let change = book.transaction()?;
            let used = change.execute(
                "UPDATE words SET used_ms = ?1 WHERE digest = ?2 AND used_ms IS NULL AND good_until_ms > ?1",
                params![now, word],
            )?;
            if used != 1 {
                return Err(AccessError::Refused(
                    "The word was used meanwhile. Ask for another.".into(),
                ));
            }
            change.execute(
                "INSERT INTO devices(device_id, name, credential, passkey, added_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![device_id, name, credential, kept, now],
            )?;
            change.commit()?;
        }
        let principal = Principal {
            by: CameBy::Device {
                device_id: device_id.clone(),
                name: name.clone(),
            },
        };
        let codes = if first {
            self.write_down("This Workbench was made yours", false, &name, from)?;
            Some(self.new_codes(&principal, from)?)
        } else {
            self.write_down(&format!("A device was added: {name}"), false, &name, from)?;
            None
        };
        let session = self.session_for(Some(&device_id), from)?;
        Ok(CameIn {
            session,
            principal,
            codes,
        })
    }

    /// Begin signing in with a device that was registered.
    ///
    /// # Errors
    ///
    /// No device may come in yet.
    pub fn begin_signing_in(&self) -> Result<Begun, AccessError> {
        // A passkey cannot be guessed, so tries with one are not limited:
        // a limit here would only let a stranger keep the owner out.
        let passkeys = self
            .passkeys()?
            .into_iter()
            .map(|(_, _, passkey)| passkey)
            .collect::<Vec<_>>();
        if passkeys.is_empty() {
            return Err(AccessError::Refused(
                "No device may come in here yet.".into(),
            ));
        }
        let (asked, state) = self
            .webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(|error| AccessError::Book(format!("the ceremony did not begin: {error}")))?;
        self.begun(Ceremony::SigningIn { state }, &asked)
    }

    /// Finish signing in.
    ///
    /// # Errors
    ///
    /// The ceremony is not one that was begun, or the answer does not check
    /// against a device that may come in.
    pub fn finish_signing_in(
        &self,
        ceremony: &str,
        answered: &Value,
        from: &str,
    ) -> Result<CameIn, AccessError> {
        let Some(Ceremony::SigningIn { state }) = self.taken(ceremony) else {
            return Err(AccessError::Refused(
                "This was not begun here, or it took too long. Begin again.".into(),
            ));
        };
        let answered: PublicKeyCredential =
            serde_json::from_value(answered.clone()).map_err(|error| {
                AccessError::Invalid(format!("not what a browser answers: {error}"))
            })?;
        let checked = match self
            .webauthn
            .finish_passkey_authentication(&answered, &state)
        {
            Ok(checked) => checked,
            Err(error) => {
                self.write_down("Refused: a passkey that did not check", true, "", from)?;
                return Err(AccessError::Refused(format!(
                    "The passkey did not check: {error}"
                )));
            }
        };
        let Some((device_id, name, mut passkey)) = self
            .passkeys()?
            .into_iter()
            .find(|(_, _, passkey)| passkey.cred_id() == checked.cred_id())
        else {
            self.write_down("Refused: a device that was taken away", true, "", from)?;
            return Err(AccessError::Refused("This device may not come in.".into()));
        };
        // What the device counts is kept, so a copy of it is noticed.
        if passkey.update_credential(&checked) == Some(true) {
            let kept = serde_json::to_string(&passkey)
                .map_err(|error| AccessError::Book(error.to_string()))?;
            held(&self.book).execute(
                "UPDATE devices SET passkey = ?1 WHERE device_id = ?2",
                params![kept, device_id],
            )?;
        }
        held(&self.book).execute(
            "UPDATE devices SET last_ms = ?1, last_from = ?2 WHERE device_id = ?3",
            params![now_ms(), from, device_id],
        )?;
        self.write_down("Came in with a passkey", false, &name, from)?;
        let session = self.session_for(Some(&device_id), from)?;
        Ok(CameIn {
            session,
            principal: Principal {
                by: CameBy::Device { device_id, name },
            },
            codes: None,
        })
    }

    /// Come back with a code, to register a device. The code is used.
    ///
    /// # Errors
    ///
    /// The code is not one that was given, or too many tries failed.
    pub fn come_back(&self, code: &str, from: &str) -> Result<CameIn, AccessError> {
        self.not_too_many_tries(from)?;
        let used = held(&self.book).execute(
            "UPDATE codes SET used_ms = ?1 WHERE digest = ?2 AND used_ms IS NULL",
            params![now_ms(), digest(&as_typed(code))],
        )?;
        if used != 1 {
            self.failed(from);
            self.write_down("Refused: a code nobody was given", true, "", from)?;
            return Err(AccessError::Refused(
                "This is not a code that was given, or it was used.".into(),
            ));
        }
        self.write_down("Came back with a code", false, "", from)?;
        let session = self.session_for(None, from)?;
        Ok(CameIn {
            session,
            principal: Principal { by: CameBy::Code },
            codes: None,
        })
    }

    /// Who came with this session, if it is one that goes on.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn of_session(&self, session: &str) -> Result<Option<Principal>, AccessError> {
        let session = digest(session);
        let now = now_ms();
        let book = held(&self.book);
        let found: Option<(Option<String>, i64)> = book
            .query_row(
                "SELECT device_id, seen_ms FROM sessions
                 WHERE digest = ?1 AND ended_ms IS NULL AND seen_ms > ?2",
                params![session, now - A_SESSION_UNSEEN_MS],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((device_id, seen_ms)) = found else {
            return Ok(None);
        };
        let by = match device_id {
            None => CameBy::Code,
            Some(device_id) => {
                let name: Option<String> = book
                    .query_row(
                        "SELECT name FROM devices WHERE device_id = ?1 AND removed_ms IS NULL",
                        params![device_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(name) = name else {
                    return Ok(None);
                };
                CameBy::Device { device_id, name }
            }
        };
        if now - seen_ms > SEEN_EVERY_MS {
            book.execute(
                "UPDATE sessions SET seen_ms = ?1 WHERE digest = ?2",
                params![now, session],
            )?;
        }
        Ok(Some(Principal { by }))
    }

    /// Who came with this token, if it is one that was made and not
    /// withdrawn. A token that is not is written down as refused.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn of_token(&self, token: &str, from: &str) -> Result<Option<Principal>, AccessError> {
        // What opens with a token is too long to be guessed; tries are
        // written down and not limited, so a program is not kept out by
        // somebody else's guessing.
        let found: Option<(String, String, String, Option<i64>)> = held(&self.book)
            .query_row(
                "SELECT token_id, name, may, withdrawn_ms FROM tokens WHERE digest = ?1",
                params![digest(token)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        match found {
            Some((token_id, name, may, None)) => {
                held(&self.book).execute(
                    "UPDATE tokens SET last_ms = ?1, last_from = ?2 WHERE token_id = ?3",
                    params![now_ms(), from, token_id],
                )?;
                let may = serde_json::from_str(&may)
                    .map_err(|error| AccessError::Book(error.to_string()))?;
                Ok(Some(Principal {
                    by: CameBy::Token {
                        token_id,
                        name,
                        may,
                    },
                }))
            }
            Some((_, name, _, Some(_))) => {
                self.write_down("Refused: a token that was withdrawn", true, &name, from)?;
                Ok(None)
            }
            None => {
                self.write_down("Refused: a token nobody was given", true, "", from)?;
                Ok(None)
            }
        }
    }

    /// End a session: whoever holds it is out.
    ///
    /// # Errors
    ///
    /// The book cannot be written.
    pub fn end_session(&self, session: &str, from: &str) -> Result<(), AccessError> {
        let who = self.of_session(session)?;
        held(&self.book).execute(
            "UPDATE sessions SET ended_ms = ?1 WHERE digest = ?2 AND ended_ms IS NULL",
            params![now_ms(), digest(session)],
        )?;
        if let Some(who) = who {
            self.write_down("Signed out", false, &who.named(), from)?;
        }
        Ok(())
    }

    /// The devices that may come in, the one added first first.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn devices(&self) -> Result<Vec<Device>, AccessError> {
        let book = held(&self.book);
        let mut asked = book.prepare(
            "SELECT device_id, name, added_ms, last_ms, last_from FROM devices
             WHERE removed_ms IS NULL ORDER BY added_ms, device_id",
        )?;
        let rows = asked.query_map([], |row| {
            Ok(Device {
                device_id: row.get(0)?,
                name: row.get(1)?,
                added_ms: row.get(2)?,
                last_ms: row.get(3)?,
                last_from: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Take a device away: it comes in no more, and what it had open is
    /// closed. The last device is not taken away while no code is left,
    /// because nobody could come in afterwards.
    ///
    /// # Errors
    ///
    /// There is no such device, or it is the last way in.
    pub fn take_away(
        &self,
        device_id: &str,
        who: &Principal,
        from: &str,
    ) -> Result<(), AccessError> {
        let devices = self.devices()?;
        let Some(device) = devices.iter().find(|one| one.device_id == device_id) else {
            return Err(AccessError::Invalid("there is no such device".into()));
        };
        if devices.len() == 1 && self.codes_left()?.left == 0 {
            return Err(AccessError::Invalid(
                "This is the only device that may come in and no code is left. Add another \
                 device or make new codes first."
                    .into(),
            ));
        }
        let now = now_ms();
        {
            let mut book = held(&self.book);
            let change = book.transaction()?;
            change.execute(
                "UPDATE devices SET removed_ms = ?1 WHERE device_id = ?2",
                params![now, device_id],
            )?;
            change.execute(
                "UPDATE sessions SET ended_ms = ?1 WHERE device_id = ?2 AND ended_ms IS NULL",
                params![now, device_id],
            )?;
            change.commit()?;
        }
        self.write_down(
            &format!("A device was taken away: {}", device.name),
            false,
            &who.named(),
            from,
        )
    }

    /// Make the codes to come back with anew. The ones made before are
    /// good no more. They are said here and cannot be read back.
    ///
    /// # Errors
    ///
    /// The book cannot be written.
    pub fn new_codes(&self, who: &Principal, from: &str) -> Result<Vec<String>, AccessError> {
        let mut codes = Vec::with_capacity(CODES_MADE);
        for _ in 0..CODES_MADE {
            codes.push(spelled(3)?);
        }
        let now = now_ms();
        {
            let mut book = held(&self.book);
            let change = book.transaction()?;
            change.execute("DELETE FROM codes", [])?;
            for code in &codes {
                change.execute(
                    "INSERT INTO codes(digest, made_ms) VALUES (?1, ?2)",
                    params![digest(&as_typed(code)), now],
                )?;
            }
            change.commit()?;
        }
        self.write_down(
            "Codes to come back with were made",
            false,
            &who.named(),
            from,
        )?;
        Ok(codes)
    }

    /// How many codes are left.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn codes_left(&self) -> Result<CodesLeft, AccessError> {
        let (made, left, made_ms): (i64, i64, Option<i64>) = held(&self.book).query_row(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE used_ms IS NULL), MAX(made_ms) FROM codes",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(CodesLeft {
            left: usize::try_from(left).unwrap_or(0),
            made: usize::try_from(made).unwrap_or(0),
            made_ms,
        })
    }

    /// The tokens that were made and not withdrawn.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn tokens(&self) -> Result<Vec<Token>, AccessError> {
        let book = held(&self.book);
        let mut asked = book.prepare(
            "SELECT token_id, name, may, made_ms, last_ms, last_from FROM tokens
             WHERE withdrawn_ms IS NULL ORDER BY made_ms, token_id",
        )?;
        let rows = asked.query_map([], |row| {
            let may: String = row.get(2)?;
            Ok(Token {
                token_id: row.get(0)?,
                name: row.get(1)?,
                may: serde_json::from_str(&may).unwrap_or_default(),
                made_ms: row.get(3)?,
                last_ms: row.get(4)?,
                last_from: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Make a token for a program. What opens with it is said here and
    /// cannot be read back.
    ///
    /// # Errors
    ///
    /// The name is empty, it may do nothing, or whoever asks holds a token
    /// themselves.
    pub fn make_token(
        &self,
        name: &str,
        may: &[May],
        who: &Principal,
        from: &str,
    ) -> Result<(Token, String), AccessError> {
        if !matches!(
            who.by,
            CameBy::ThisRun | CameBy::Device { .. } | CameBy::Embedder
        ) {
            return Err(AccessError::Refused(
                "a token is made by a person who is signed in".into(),
            ));
        }
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(AccessError::Invalid(
                "a token is given a name of sixty letters at most".into(),
            ));
        }
        let mut may = may.to_vec();
        may.sort_unstable();
        may.dedup();
        if may.is_empty() {
            return Err(AccessError::Invalid(
                "a token is made for something it may do".into(),
            ));
        }
        let opens = secret("swem_")?;
        let token = Token {
            token_id: identifier("t_")?,
            name: name.to_owned(),
            may,
            made_ms: now_ms(),
            last_ms: None,
            last_from: None,
        };
        held(&self.book).execute(
            "INSERT INTO tokens(token_id, name, digest, may, made_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                token.token_id,
                token.name,
                digest(&opens),
                serde_json::to_string(&token.may)
                    .map_err(|error| AccessError::Book(error.to_string()))?,
                token.made_ms
            ],
        )?;
        self.write_down(
            &format!("A token was made: {name}"),
            false,
            &who.named(),
            from,
        )?;
        Ok((token, opens))
    }

    /// Withdraw a token: it opens nothing from now on.
    ///
    /// # Errors
    ///
    /// There is no such token.
    pub fn withdraw(&self, token_id: &str, who: &Principal, from: &str) -> Result<(), AccessError> {
        let name: Option<String> = held(&self.book)
            .query_row(
                "SELECT name FROM tokens WHERE token_id = ?1 AND withdrawn_ms IS NULL",
                params![token_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(name) = name else {
            return Err(AccessError::Invalid("there is no such token".into()));
        };
        held(&self.book).execute(
            "UPDATE tokens SET withdrawn_ms = ?1 WHERE token_id = ?2",
            params![now_ms(), token_id],
        )?;
        self.write_down(
            &format!("A token was withdrawn: {name}"),
            false,
            &who.named(),
            from,
        )
    }

    /// What was done, the latest first.
    ///
    /// # Errors
    ///
    /// The book cannot be read.
    pub fn happened(&self, at_most: usize) -> Result<Vec<Happened>, AccessError> {
        let book = held(&self.book);
        let mut asked = book.prepare(
            "SELECT at_ms, what, refused, who, from_place FROM happened
             ORDER BY seq DESC LIMIT ?1",
        )?;
        let rows = asked.query_map(params![i64::try_from(at_most).unwrap_or(i64::MAX)], |row| {
            Ok(Happened {
                at_ms: row.get(0)?,
                what: row.get(1)?,
                refused: row.get(2)?,
                who: row.get(3)?,
                from: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Write down that somebody was refused at the door itself.
    ///
    /// # Errors
    ///
    /// The book cannot be written.
    pub fn refused(&self, what: &str, from: &str) -> Result<(), AccessError> {
        self.write_down(what, true, "", from)
    }

    fn write_down(
        &self,
        what: &str,
        refused: bool,
        who: &str,
        from: &str,
    ) -> Result<(), AccessError> {
        let book = held(&self.book);
        book.execute(
            "INSERT INTO happened(at_ms, what, refused, who, from_place) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![now_ms(), what, refused, who, from],
        )?;
        book.execute(
            "DELETE FROM happened WHERE seq <= (SELECT MAX(seq) FROM happened) - ?1",
            params![HAPPENED_KEPT],
        )?;
        Ok(())
    }

    fn owner_id(&self) -> Result<Uuid, AccessError> {
        let book = held(&self.book);
        let kept: Option<String> = book
            .query_row("SELECT value FROM meta WHERE key = 'owner'", [], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(kept) = kept
            && let Ok(id) = Uuid::parse_str(&kept)
        {
            return Ok(id);
        }
        let id = Uuid::from_bytes(random::<16>()?);
        book.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('owner', ?1)",
            params![id.to_string()],
        )?;
        Ok(id)
    }

    fn passkeys(&self) -> Result<Vec<(String, String, Passkey)>, AccessError> {
        let book = held(&self.book);
        let mut asked =
            book.prepare("SELECT device_id, name, passkey FROM devices WHERE removed_ms IS NULL")?;
        let rows = asked.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut passkeys = Vec::new();
        for row in rows {
            let (device_id, name, kept) = row?;
            let passkey = serde_json::from_str(&kept).map_err(|error| {
                AccessError::Book(format!("the key of {name} cannot be read: {error}"))
            })?;
            passkeys.push((device_id, name, passkey));
        }
        Ok(passkeys)
    }

    fn session_for(&self, device_id: Option<&str>, from: &str) -> Result<String, AccessError> {
        let session = secret("")?;
        let now = now_ms();
        held(&self.book).execute(
            "INSERT INTO sessions(digest, device_id, made_ms, seen_ms, from_place)
             VALUES (?1, ?2, ?3, ?3, ?4)",
            params![digest(&session), device_id, now, from],
        )?;
        Ok(session)
    }

    fn begun<Asked: Serialize>(
        &self,
        ceremony: Ceremony,
        asked: &Asked,
    ) -> Result<Begun, AccessError> {
        let id = identifier("c_")?;
        let now = now_ms();
        let mut ceremonies = held(&self.ceremonies);
        ceremonies.retain(|_, begun| now - begun.made_ms < A_CEREMONY_MS);
        ceremonies.insert(
            id.clone(),
            Begin {
                made_ms: now,
                ceremony,
            },
        );
        Ok(Begun {
            ceremony: id,
            asked: serde_json::to_value(asked)
                .map_err(|error| AccessError::Book(error.to_string()))?,
        })
    }

    fn taken(&self, ceremony: &str) -> Option<Ceremony> {
        let begun = held(&self.ceremonies).remove(ceremony)?;
        (now_ms() - begun.made_ms < A_CEREMONY_MS).then_some(begun.ceremony)
    }

    fn not_too_many_tries(&self, from: &str) -> Result<(), AccessError> {
        let now = now_ms();
        let mut tries = held(&self.tries);
        tries.failed.retain(|(at, _)| now - at < TRIES_OVER_MS);
        let here = tries
            .failed
            .iter()
            .filter(|(_, place)| place == from)
            .count();
        if here < TRIES_FROM_ONE_PLACE && tries.failed.len() < TRIES_FROM_EVERYWHERE {
            return Ok(());
        }
        let oldest = tries
            .failed
            .iter()
            .filter(|(_, place)| here < TRIES_FROM_ONE_PLACE || place == from)
            .map(|(at, _)| *at)
            .min()
            .unwrap_or(now);
        Err(AccessError::TooManyTries {
            minutes: ((oldest + TRIES_OVER_MS - now) / 60_000).max(1),
        })
    }

    fn failed(&self, from: &str) {
        held(&self.tries).failed.push((now_ms(), from.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::{as_typed, digest, spelled};

    #[test]
    fn a_word_is_spelled_in_groups_a_person_can_read() {
        let word = spelled(4).expect("a word");
        assert_eq!(word.len(), 19, "{word}");
        assert_eq!(word.split('-').count(), 4, "{word}");
        assert!(
            word.chars()
                .all(|letter| letter == '-' || !"il o01".contains(letter)),
            "{word}"
        );
    }

    #[test]
    fn a_word_is_the_same_word_however_it_was_typed() {
        assert_eq!(as_typed(" K7FQ 9x2m-tp4c\n"), "k7fq9x2mtp4c");
        assert_eq!(
            digest(&as_typed("K7FQ-9X2M")),
            digest(&as_typed("k7fq 9x2m"))
        );
    }
}
