//! An agent's files, confined to the folder it was given.
//!
//! Every request names a root and a place under it. A place is a list of
//! names, never a path of the machine: what climbs out is refused before
//! it is joined to anything, and what is found is resolved and must still
//! be under the root, so a link planted in the folder serves nothing
//! outside it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RUNNER_FS_SCHEMA: &str = "swem:runner-fs@0.1";

/// What the product asks about an agent's files.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FsRequest {
    /// What a folder holds; the root itself when `dir` is empty.
    List {
        root: PathBuf,
        #[serde(default)]
        dir: String,
    },
    /// A file's bytes, when it is no bigger than `at_most`.
    Read {
        root: PathBuf,
        path: String,
        at_most: u64,
    },
    /// Write a file. `was` is the digest of what was read: the file must
    /// still be that. With neither `was` nor `over` the file is new and
    /// must not be there. `over` writes whatever is there.
    Write {
        root: PathBuf,
        path: String,
        /// The bytes, in standard base64.
        bytes: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        was: Option<String>,
        #[serde(default)]
        over: bool,
    },
    MakeDir {
        root: PathBuf,
        path: String,
    },
    Rename {
        root: PathBuf,
        from: String,
        to: String,
    },
    /// Remove a file, or a folder with what it holds when `with_all` says
    /// so.
    Remove {
        root: PathBuf,
        path: String,
        #[serde(default)]
        with_all: bool,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Folder,
    /// A link that leads out of the folder or nowhere; it is said and not
    /// followed.
    Link,
    Other,
}

/// One thing in a folder.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub byte_length: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_ms: Option<u64>,
}

/// Why something was refused, for the product to act on; `said` beside it
/// is for a person.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// The place is not under the root.
    Outside,
    NotFound,
    NotAFile,
    NotAFolder,
    TooBig,
    /// The file is not what was read any more.
    Changed,
    /// Something is there under that name already.
    Exists,
    /// A folder that holds something was to be removed without it.
    NotEmpty,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum FsAnswer {
    Listed {
        entries: Vec<Entry>,
    },
    Read {
        /// The bytes, in standard base64.
        bytes: String,
        sha256: String,
        byte_length: u64,
    },
    Written {
        sha256: String,
        byte_length: u64,
    },
    Done,
    Refused {
        why: Why,
        said: String,
        /// For a file that changed: what it is now, nothing when it is
        /// gone.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        now: Option<String>,
    },
}

/// The digest a file is known by between reading and writing it.
#[must_use]
pub fn digest_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

struct Refusal(Why, String);

impl From<Refusal> for FsAnswer {
    fn from(refusal: Refusal) -> Self {
        Self::Refused {
            why: refusal.0,
            said: refusal.1,
            now: None,
        }
    }
}

fn failed(path: &Path, error: &std::io::Error) -> Refusal {
    let why = match error.kind() {
        std::io::ErrorKind::NotFound => Why::NotFound,
        std::io::ErrorKind::AlreadyExists => Why::Exists,
        std::io::ErrorKind::DirectoryNotEmpty => Why::NotEmpty,
        _ => Why::Failed,
    };
    Refusal(why, format!("{}: {error}", shown(path)))
}

/// A path as a person reads it: its last name.
fn shown(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The names of a place, none of which climbs or is a path of the machine.
fn names(place: &str) -> Result<Vec<&str>, Refusal> {
    let outside = || {
        Refusal(
            Why::Outside,
            format!("{place:?} is not a place in its folder"),
        )
    };
    if place.is_empty() {
        return Ok(Vec::new());
    }
    place
        .split('/')
        .map(|name| {
            if name.is_empty()
                || name == "."
                || name == ".."
                || name.len() > 255
                || name
                    .chars()
                    .any(|character| matches!(character, '\\' | '\0') || character.is_control())
            {
                Err(outside())
            } else {
                Ok(name)
            }
        })
        .collect()
}

fn the_root(root: &Path) -> Result<PathBuf, Refusal> {
    let root = fs::canonicalize(root)
        .map_err(|error| Refusal(Why::NotFound, format!("its folder is not there: {error}")))?;
    if root.is_dir() {
        Ok(root)
    } else {
        Err(Refusal(
            Why::NotAFolder,
            "its folder is not a folder".into(),
        ))
    }
}

/// Something that is there, resolved, and still under the root.
fn there(root: &Path, place: &str) -> Result<PathBuf, Refusal> {
    let root = the_root(root)?;
    let mut path = root.clone();
    path.extend(names(place)?);
    let found = fs::canonicalize(&path).map_err(|error| failed(&path, &error))?;
    if found.starts_with(&root) {
        Ok(found)
    } else {
        Err(Refusal(
            Why::Outside,
            format!("{} leads out of its folder", shown(&path)),
        ))
    }
}

/// A place where something is or is to be: its folder is there and under
/// the root, and what is there under its name is not a link that leads
/// out. The root itself is never such a place.
fn place_for(root: &Path, place: &str) -> Result<PathBuf, Refusal> {
    let names = names(place)?;
    let Some((name, folder)) = names.split_last() else {
        return Err(Refusal(
            Why::Outside,
            "its folder itself is not a place in it".into(),
        ));
    };
    let folder = there(root, &folder.join("/"))?;
    if !folder.is_dir() {
        return Err(Refusal(
            Why::NotAFolder,
            format!("{} is not a folder", shown(&folder)),
        ));
    }
    let path = folder.join(name);
    if fs::symlink_metadata(&path).is_ok_and(|found| found.file_type().is_symlink()) {
        // What it leads to is what would be written or removed.
        let root = the_root(root)?;
        if !fs::canonicalize(&path).is_ok_and(|found| found.starts_with(&root)) {
            return Err(Refusal(
                Why::Outside,
                format!("{name} leads out of its folder"),
            ));
        }
    }
    Ok(path)
}

fn modified_ms(found: &fs::Metadata) -> Option<u64> {
    found
        .modified()
        .ok()
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| u64::try_from(since.as_millis()).ok())
}

fn list(root: &Path, dir: &str) -> Result<FsAnswer, Refusal> {
    let folder = there(root, dir)?;
    if !folder.is_dir() {
        return Err(Refusal(
            Why::NotAFolder,
            format!("{} is not a folder", shown(&folder)),
        ));
    }
    let root = the_root(root)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&folder).map_err(|error| failed(&folder, &error))? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let Ok(as_it_is) = fs::symlink_metadata(&path) else {
            continue;
        };
        // A link is what it leads to when that is under the root.
        let found = if as_it_is.file_type().is_symlink() {
            match fs::canonicalize(&path) {
                Ok(led_to) if led_to.starts_with(&root) => fs::metadata(&led_to).ok(),
                _ => None,
            }
        } else {
            Some(as_it_is)
        };
        entries.push(match found {
            Some(found) => Entry {
                name,
                kind: if found.is_dir() {
                    EntryKind::Folder
                } else if found.is_file() {
                    EntryKind::File
                } else {
                    EntryKind::Other
                },
                byte_length: if found.is_file() { found.len() } else { 0 },
                modified_ms: modified_ms(&found),
            },
            None => Entry {
                name,
                kind: EntryKind::Link,
                byte_length: 0,
                modified_ms: None,
            },
        });
    }
    // Folders first, then by name as a person reads names.
    entries.sort_by(|left, right| {
        (
            left.kind != EntryKind::Folder,
            left.name.to_lowercase(),
            &left.name,
        )
            .cmp(&(
                right.kind != EntryKind::Folder,
                right.name.to_lowercase(),
                &right.name,
            ))
    });
    Ok(FsAnswer::Listed { entries })
}

fn read(root: &Path, place: &str, at_most: u64) -> Result<FsAnswer, Refusal> {
    let path = there(root, place)?;
    let found = fs::metadata(&path).map_err(|error| failed(&path, &error))?;
    if !found.is_file() {
        return Err(Refusal(
            Why::NotAFile,
            format!("{} is not a file", shown(&path)),
        ));
    }
    if found.len() > at_most {
        return Err(Refusal(
            Why::TooBig,
            format!("{} is {} bytes", shown(&path), found.len()),
        ));
    }
    let bytes = fs::read(&path).map_err(|error| failed(&path, &error))?;
    Ok(FsAnswer::Read {
        sha256: digest_of(&bytes),
        byte_length: bytes.len() as u64,
        bytes: STANDARD.encode(&bytes),
    })
}

/// Put bytes where a file is or is to be: beside it under another name,
/// then in its place, so nobody reads half of it. What the file allowed
/// stays what it allows.
fn put(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let folder = path.parent().unwrap_or(Path::new("."));
    let beside = folder.join(format!(
        ".{}.{}.swem-writing",
        shown(path),
        std::process::id()
    ));
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&beside)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Ok(was) = fs::metadata(path) {
            fs::set_permissions(&beside, was.permissions())?;
        }
        fs::rename(&beside, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&beside);
    }
    written
}

fn write(
    root: &Path,
    place: &str,
    bytes: &str,
    was: Option<&str>,
    over: bool,
) -> Result<FsAnswer, Refusal> {
    let path = place_for(root, place)?;
    let bytes = STANDARD.decode(bytes).map_err(|error| {
        Refusal(
            Why::Failed,
            format!("what was sent cannot be read: {error}"),
        )
    })?;
    let is_there = fs::symlink_metadata(&path).ok();
    if is_there.as_ref().is_some_and(fs::Metadata::is_dir) {
        return Err(Refusal(
            Why::NotAFile,
            format!("{} is a folder", shown(&path)),
        ));
    }
    if !over {
        let now = is_there
            .as_ref()
            .and_then(|_| fs::read(&path).ok())
            .map(|now| digest_of(&now));
        match (was, &now) {
            (Some(was), Some(now)) if was == now => {}
            (Some(_), _) => {
                return Ok(FsAnswer::Refused {
                    why: Why::Changed,
                    said: if now.is_some() {
                        format!("{} changed since it was read", shown(&path))
                    } else {
                        format!("{} is gone since it was read", shown(&path))
                    },
                    now,
                });
            }
            (None, Some(_)) => {
                return Err(Refusal(
                    Why::Exists,
                    format!("{} is there already", shown(&path)),
                ));
            }
            (None, None) => {}
        }
    }
    put(&path, &bytes).map_err(|error| failed(&path, &error))?;
    Ok(FsAnswer::Written {
        sha256: digest_of(&bytes),
        byte_length: bytes.len() as u64,
    })
}

fn make_dir(root: &Path, place: &str) -> Result<FsAnswer, Refusal> {
    let path = place_for(root, place)?;
    fs::create_dir(&path).map_err(|error| failed(&path, &error))?;
    Ok(FsAnswer::Done)
}

fn rename(root: &Path, from: &str, to: &str) -> Result<FsAnswer, Refusal> {
    let from = place_for(root, from)?;
    let to = place_for(root, to)?;
    if fs::symlink_metadata(&from).is_err() {
        return Err(Refusal(
            Why::NotFound,
            format!("{} is not there", shown(&from)),
        ));
    }
    if fs::symlink_metadata(&to).is_ok() {
        return Err(Refusal(
            Why::Exists,
            format!("{} is there already", shown(&to)),
        ));
    }
    fs::rename(&from, &to).map_err(|error| failed(&from, &error))?;
    Ok(FsAnswer::Done)
}

fn remove(root: &Path, place: &str, with_all: bool) -> Result<FsAnswer, Refusal> {
    let path = place_for(root, place)?;
    let found = fs::symlink_metadata(&path).map_err(|error| failed(&path, &error))?;
    // A link is removed, never what it leads to.
    if found.is_dir() {
        if with_all {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_dir(&path)
        }
    } else {
        fs::remove_file(&path)
    }
    .map_err(|error| failed(&path, &error))?;
    Ok(FsAnswer::Done)
}

/// Answer one request.
#[must_use]
pub fn answer(request: &FsRequest) -> FsAnswer {
    match request {
        FsRequest::List { root, dir } => list(root, dir),
        FsRequest::Read {
            root,
            path,
            at_most,
        } => read(root, path, *at_most),
        FsRequest::Write {
            root,
            path,
            bytes,
            was,
            over,
        } => write(root, path, bytes, was.as_deref(), *over),
        FsRequest::MakeDir { root, path } => make_dir(root, path),
        FsRequest::Rename { root, from, to } => rename(root, from, to),
        FsRequest::Remove {
            root,
            path,
            with_all,
        } => remove(root, path, *with_all),
    }
    .unwrap_or_else(FsAnswer::from)
}
