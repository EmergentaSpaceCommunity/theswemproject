//! Inbox and outbox: what a person hands the agent, and what it hands back.
//!
//! A directory is the only interface every agent already has. An agent that
//! speaks no protocol for files still opens a path, and the two directories
//! here sit inside the agent's own workspace - which is exactly the boundary
//! an agent working alone may read and write inside. So handing a file over
//! needs no capability negotiation, and getting one back needs no resource
//! link: the agent writes a file where it works, and the person finds it.
//!
//! Only the two names below are served, only their immediate children, and
//! only by a name that is a name rather than a path. Nothing here follows a
//! symlink out, because nothing here resolves anything but `workspace/<area>/
//! <name>` and then checks where it landed.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::WorkbenchShellError;

/// What a person hands the agent.
pub const INBOX: &str = "inbox";
/// What the agent hands back.
pub const OUTBOX: &str = "outbox";

const AREAS: [&str; 2] = [INBOX, OUTBOX];
/// A file the page offers to open is read whole. Bigger than this is work for
/// the workspace itself, not for a panel in a browser tab.
const MAX_SERVED_BYTES: u64 = 64 * 1024 * 1024;

/// One file in one of the two directories, as the page lists it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HandedFile {
    pub area: String,
    pub name: String,
    pub byte_length: u64,
}

/// A name that names a file rather than a place. Anything with a separator,
/// a drive letter or a dot-dot in it is refused before it is joined to
/// anything, so the check below is a second lock rather than the only one.
///
/// # Errors
///
/// Returns [`WorkbenchShellError::Invalid`] for an empty name, a name that is
/// `.` or `..`, or one carrying a path separator or a NUL.
pub fn safe_name(requested: &str) -> Result<String, WorkbenchShellError> {
    let refuse = || WorkbenchShellError::Invalid(format!("{requested:?} is not a file name"));
    if requested.is_empty()
        || requested.len() > 255
        || requested == "."
        || requested == ".."
        || requested
            .chars()
            .any(|character| matches!(character, '/' | '\\' | '\0') || character.is_control())
    {
        return Err(refuse());
    }
    Ok(requested.to_owned())
}

fn area_path(workspace: &Path, area: &str) -> Result<PathBuf, WorkbenchShellError> {
    if !AREAS.contains(&area) {
        return Err(WorkbenchShellError::Invalid(format!(
            "{area:?} is neither {INBOX} nor {OUTBOX}"
        )));
    }
    Ok(workspace.join(area))
}

/// Where `name` lives, having checked that it lives there: the joined path is
/// resolved and must still be an immediate child of the area's own directory.
/// A symlink planted in the workspace therefore serves nothing outside it.
async fn resolved_child(
    workspace: &Path,
    area: &str,
    name: &str,
) -> Result<PathBuf, WorkbenchShellError> {
    let directory = area_path(workspace, area)?;
    let name = safe_name(name)?;
    let canonical_directory = tokio::fs::canonicalize(&directory)
        .await
        .map_err(|_| WorkbenchShellError::NotFound(format!("no {area} yet")))?;
    let path = tokio::fs::canonicalize(canonical_directory.join(&name))
        .await
        .map_err(|_| WorkbenchShellError::NotFound(format!("no {name} in {area}")))?;
    if path.parent() != Some(canonical_directory.as_path()) {
        return Err(WorkbenchShellError::Invalid(format!(
            "{name} does not stay in {area}"
        )));
    }
    Ok(path)
}

/// Put `bytes` in the agent's inbox under a name derived from `name`, and say
/// where it landed. A name already taken by different bytes gets a numbered
/// neighbour rather than overwriting what is there: a person handing over two
/// files called `notes.txt` means two files.
///
/// # Errors
///
/// Returns [`WorkbenchShellError`] for an unusable name or a workspace this
/// process cannot write in.
pub async fn hand_over(
    workspace: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<PathBuf, WorkbenchShellError> {
    let name = safe_name(name)?;
    let directory = area_path(workspace, INBOX)?;
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_owned(), format!(".{extension}")),
        _ => (name.clone(), String::new()),
    };
    for attempt in 0..1_000_u32 {
        let candidate = if attempt == 0 {
            directory.join(&name)
        } else {
            directory.join(format!("{stem}-{attempt}{extension}"))
        };
        match tokio::fs::try_exists(&candidate).await {
            Ok(true) => {
                // The same bytes under the same name is the same file, not a
                // second one: a person who sends a file twice gets one.
                if tokio::fs::read(&candidate)
                    .await
                    .is_ok_and(|already| already == bytes)
                {
                    return Ok(candidate);
                }
            }
            Ok(false) => {
                tokio::fs::write(&candidate, bytes)
                    .await
                    .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
                return Ok(candidate);
            }
            Err(error) => return Err(WorkbenchShellError::Failed(error.to_string())),
        }
    }
    Err(WorkbenchShellError::Conflict(format!(
        "the inbox already holds a thousand files called {name}"
    )))
}

/// Everything in both directories, inbox first and each sorted by name. A
/// directory that does not exist is empty, not an error: an agent that has
/// handed nothing back has no outbox, and saying so as a failure would make
/// the panel read as broken.
///
/// # Errors
///
/// Returns [`WorkbenchShellError::Failed`] when a directory exists and cannot
/// be read.
pub async fn list(workspace: &Path) -> Result<Vec<HandedFile>, WorkbenchShellError> {
    let mut found = Vec::new();
    for area in AREAS {
        let directory = area_path(workspace, area)?;
        let mut entries = match tokio::fs::read_dir(&directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(WorkbenchShellError::Failed(error.to_string())),
        };
        let mut here = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if safe_name(&name).is_err() {
                continue;
            }
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            here.push(HandedFile {
                area: area.to_owned(),
                name,
                byte_length: metadata.len(),
            });
        }
        here.sort_by(|left, right| left.name.cmp(&right.name));
        found.extend(here);
    }
    Ok(found)
}

/// One file's bytes, with the media type the page should serve them as.
///
/// # Errors
///
/// Returns [`WorkbenchShellError`] for an unknown area, an unusable name, a
/// file that is not there, or one too big to hand to a browser tab.
pub async fn read(
    workspace: &Path,
    area: &str,
    name: &str,
) -> Result<(Vec<u8>, String), WorkbenchShellError> {
    let path = resolved_child(workspace, area, name).await?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    if !metadata.is_file() {
        return Err(WorkbenchShellError::Invalid(format!(
            "{name} is not a file"
        )));
    }
    if metadata.len() > MAX_SERVED_BYTES {
        return Err(WorkbenchShellError::Conflict(format!(
            "{name} is {} bytes, more than a page should be handed",
            metadata.len()
        )));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    Ok((bytes, media_type_of(name)))
}

/// What to serve a file as, by its extension. Deliberately a short list: the
/// point is that a person can read a note and look at a picture the agent left
/// them without downloading it first. Everything else is bytes.
///
/// The list holds nothing a domain owns. A harness that knows what a `.wav` is
/// has started to know about music, and `genericity::the_harness_names_no_domain_word`
/// refuses that - so an agent's audio, model or project file downloads rather
/// than opening in the page, and the domain that owns it is what gives it a
/// player.
fn media_type_of(name: &str) -> String {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "txt" | "log" | "md" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "csv" => "text/csv; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
    .to_owned()
}
