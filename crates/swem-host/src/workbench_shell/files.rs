//! An agent's files, as the page reads and changes them.
//!
//! Whatever machine the agent lives in, its files are reached one way: by
//! asking the runner. On this machine the runner's library answers here,
//! in this process. Nothing outside the folder the agent works in is
//! reached, and that is the runner's refusal, not a check of this module.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use swem_runner::{Entry, FsAnswer, FsRequest, Why};

use super::{WorkbenchShellError, WorkbenchShellState};

/// What a page is handed to edit. Bigger than this is work for an editor
/// on the machine, not for a tab.
const MAX_EDITED_BYTES: u64 = 2 * 1024 * 1024;
/// What a page is handed to open or keep.
const MAX_SERVED_BYTES: u64 = 64 * 1024 * 1024;

/// A file as the page opens it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FileOpened {
    pub path: String,
    /// What it says, when it is text a person can edit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// What it is known by until it is written.
    pub sha256: String,
    pub byte_length: u64,
}

/// A file as the page saves it.
#[derive(Clone, Debug, Deserialize)]
pub struct SaveFileBody {
    pub path: String,
    /// What a person wrote.
    #[serde(default)]
    pub text: String,
    /// A file a person brought from their own machine, in standard
    /// base64; it is what is written when it is there.
    #[serde(default)]
    pub bytes: Option<String>,
    /// What was read; nothing for a file that is new.
    #[serde(default)]
    pub was: Option<String>,
    /// The person chose theirs over what is there.
    #[serde(default)]
    pub over: bool,
}

/// What a person does to the tree.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum ChangeTreeBody {
    MakeFolder {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
    Remove {
        path: String,
        #[serde(default)]
        with_all: bool,
    },
}

/// A file that is not what was read any more: the person is told and
/// chooses.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SavedOrNot {
    pub saved: bool,
    /// What the file is known by now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Why it was not saved, in words.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub said: String,
}

fn refusal(why: Why, said: String) -> WorkbenchShellError {
    match why {
        Why::NotFound => WorkbenchShellError::NotFound(said),
        Why::Outside | Why::NotAFile | Why::NotAFolder => WorkbenchShellError::Invalid(said),
        Why::TooBig | Why::Changed | Why::Exists | Why::NotEmpty => {
            WorkbenchShellError::Conflict(said)
        }
        Why::Failed => WorkbenchShellError::Failed(said),
    }
}

fn unexpected() -> WorkbenchShellError {
    WorkbenchShellError::Failed("the runner answered something else than was asked".into())
}

impl WorkbenchShellState {
    /// Ask the runner of the machine an agent lives in. Today every agent's
    /// folder is on this machine, a container's as well, so the library
    /// answers here.
    async fn ask_about_files(
        &self,
        profile_id: &str,
        asking: impl FnOnce(std::path::PathBuf) -> FsRequest + Send + 'static,
    ) -> Result<FsAnswer, WorkbenchShellError> {
        let profile = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let request = asking(profile.workspace);
        tokio::task::spawn_blocking(move || swem_runner::answer(&request))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// What a folder of the agent's holds; the folder it works in when
    /// `dir` is empty.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown agent, a place out
    /// of its folder, and a folder that is not there.
    pub async fn agent_tree(
        &self,
        profile_id: &str,
        dir: &str,
    ) -> Result<Vec<Entry>, WorkbenchShellError> {
        let dir = dir.to_owned();
        match self
            .ask_about_files(profile_id, move |root| FsRequest::List { root, dir })
            .await?
        {
            FsAnswer::Listed { entries } => Ok(entries),
            FsAnswer::Refused { why, said, .. } => Err(refusal(why, said)),
            _ => Err(unexpected()),
        }
    }

    async fn agent_bytes(
        &self,
        profile_id: &str,
        path: &str,
        at_most: u64,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        let path = path.to_owned();
        match self
            .ask_about_files(profile_id, move |root| FsRequest::Read {
                root,
                path,
                at_most,
            })
            .await?
        {
            FsAnswer::Read { bytes, sha256, .. } => STANDARD
                .decode(bytes)
                .map(|bytes| (bytes, sha256))
                .map_err(|error| WorkbenchShellError::Failed(error.to_string())),
            FsAnswer::Refused { why, said, .. } => Err(refusal(why, said)),
            _ => Err(unexpected()),
        }
    }

    /// A file of the agent's as the page opens it: its text when it is
    /// text, and what it is known by until it is written.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown agent, a place out
    /// of its folder, what is not a file, and a file too big for a page.
    pub async fn agent_file(
        &self,
        profile_id: &str,
        path: &str,
    ) -> Result<FileOpened, WorkbenchShellError> {
        let (bytes, sha256) = self.agent_bytes(profile_id, path, MAX_EDITED_BYTES).await?;
        let byte_length = bytes.len() as u64;
        // Text is what reads as text and holds nothing a text does not.
        let text = String::from_utf8(bytes)
            .ok()
            .filter(|text| !text.contains('\0'));
        Ok(FileOpened {
            path: path.to_owned(),
            text,
            sha256,
            byte_length,
        })
    }

    /// A file's bytes and what to serve them as, for a person to open or
    /// keep.
    ///
    /// # Errors
    ///
    /// As [`Self::agent_file`].
    pub async fn agent_file_as_it_is(
        &self,
        profile_id: &str,
        path: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        let (bytes, _) = self.agent_bytes(profile_id, path, MAX_SERVED_BYTES).await?;
        Ok((bytes, crate::workbench_files::media_type_of(path)))
    }

    /// Save what a person wrote. A file that changed since it was read is
    /// not written: that is an answer, not a failure, because the person
    /// has something to choose.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown agent, a place out
    /// of its folder, and a file that cannot be written.
    pub async fn save_agent_file(
        &self,
        profile_id: &str,
        body: SaveFileBody,
    ) -> Result<SavedOrNot, WorkbenchShellError> {
        match self
            .ask_about_files(profile_id, move |root| FsRequest::Write {
                root,
                path: body.path,
                bytes: body
                    .bytes
                    .unwrap_or_else(|| STANDARD.encode(body.text.as_bytes())),
                was: body.was,
                over: body.over,
            })
            .await?
        {
            FsAnswer::Written { sha256, .. } => Ok(SavedOrNot {
                saved: true,
                sha256: Some(sha256),
                said: String::new(),
            }),
            FsAnswer::Refused {
                why: Why::Changed,
                said,
                now,
            } => Ok(SavedOrNot {
                saved: false,
                sha256: now,
                said,
            }),
            FsAnswer::Refused { why, said, .. } => Err(refusal(why, said)),
            _ => Err(unexpected()),
        }
    }

    /// Make a folder, rename, remove.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown agent, a place out
    /// of its folder, a name that is taken, and a folder that holds
    /// something and was to be removed without it.
    pub async fn change_agent_tree(
        &self,
        profile_id: &str,
        body: ChangeTreeBody,
    ) -> Result<(), WorkbenchShellError> {
        match self
            .ask_about_files(profile_id, move |root| match body {
                ChangeTreeBody::MakeFolder { path } => FsRequest::MakeDir { root, path },
                ChangeTreeBody::Rename { from, to } => FsRequest::Rename { root, from, to },
                ChangeTreeBody::Remove { path, with_all } => FsRequest::Remove {
                    root,
                    path,
                    with_all,
                },
            })
            .await?
        {
            FsAnswer::Done => Ok(()),
            FsAnswer::Refused { why, said, .. } => Err(refusal(why, said)),
            _ => Err(unexpected()),
        }
    }
}
