//! Where what a person gave is kept: the keeper of secrets.
//!
//! A key a provider was given, what a profile is launched with, the values
//! a declared server carries - each is a document that must not be read by
//! anybody but its owner. On a computer of one's own they are files closed
//! to others under the data root. A product that builds the harness in has
//! a place of its own for such things, and keeps them there: it supplies
//! the keeper, and the harness asks it and nothing else.
//!
//! A keeper is asked for documents by name. The names are the harness's,
//! stable, and say what the document is: `keys/<provider>.json`,
//! `profiles/<id>/secrets.json`, `mcp-servers/<name>.json`.

use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Whoever keeps what a person gave.
pub trait SecretKeeper: Debug + Send + Sync {
    /// What keeps them, in words a page can say.
    fn kept_by(&self) -> String;

    /// The document called `name`, if it is kept.
    ///
    /// # Errors
    ///
    /// The document is there and cannot be read.
    fn read(&self, name: &str) -> std::io::Result<Option<Vec<u8>>>;

    /// Keep the document called `name`, in place of what was kept under it.
    ///
    /// # Errors
    ///
    /// It cannot be kept.
    fn write(&self, name: &str, bytes: &[u8]) -> std::io::Result<()>;

    /// Forget the document called `name`. Forgetting what is not kept is done.
    ///
    /// # Errors
    ///
    /// It is there and cannot be forgotten.
    fn remove(&self, name: &str) -> std::io::Result<()>;

    /// The names of the documents kept directly under `under`, a name that
    /// ends in a stroke.
    ///
    /// # Errors
    ///
    /// What is under it cannot be listed.
    fn list(&self, under: &str) -> std::io::Result<Vec<String>>;
}

/// A keeper shared by whoever asks it.
pub type Keeper = Arc<dyn SecretKeeper>;

/// A keeper that keeps nothing: what a store over nowhere has until it is
/// given one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nowhere;

impl SecretKeeper for Nowhere {
    fn kept_by(&self) -> String {
        "nowhere".into()
    }

    fn read(&self, _name: &str) -> std::io::Result<Option<Vec<u8>>> {
        Ok(None)
    }

    fn write(&self, name: &str, _bytes: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(format!(
            "{name} cannot be kept nowhere"
        )))
    }

    fn remove(&self, _name: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn list(&self, _under: &str) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// The keeper a computer of one's own has: files closed to everybody but
/// their owner, under a root, named as the documents are.
#[derive(Clone, Debug)]
pub struct InFiles {
    root: PathBuf,
}

impl InFiles {
    /// Keep under `root`, which is made and closed to others.
    ///
    /// # Errors
    ///
    /// The root cannot be made or closed.
    pub fn at(root: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(root)?;
        crate::closed::directory(root)?;
        Ok(Self {
            root: std::fs::canonicalize(root)?,
        })
    }

    /// Where a document is kept; a name that climbs is refused.
    fn place(&self, name: &str) -> std::io::Result<PathBuf> {
        if name.is_empty()
            || name.starts_with('/')
            || name
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{name:?} is not the name of a document"),
            ));
        }
        Ok(self.root.join(name))
    }
}

impl SecretKeeper for InFiles {
    fn kept_by(&self) -> String {
        "a file on this computer that only you can read".into()
    }

    fn read(&self, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        match std::fs::read(self.place(name)?) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        let place = self.place(name)?;
        if let Some(directory) = place.parent() {
            std::fs::create_dir_all(directory)?;
            crate::closed::directory(directory)?;
        }
        crate::closed::write(&place, bytes)
    }

    fn remove(&self, name: &str) -> std::io::Result<()> {
        match std::fs::remove_file(self.place(name)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn list(&self, under: &str) -> std::io::Result<Vec<String>> {
        let directory = if under.is_empty() {
            self.root.clone()
        } else {
            self.place(under.trim_end_matches('/'))?
        };
        let mut names = Vec::new();
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && let Some(name) = entry.file_name().to_str()
                && !name.starts_with('.')
            {
                names.push(format!("{under}{name}"));
            }
        }
        names.sort();
        Ok(names)
    }
}
