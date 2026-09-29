//! Kept closed: what a person gave this product is read by them alone.
//!
//! A value a person gave - a key, a header of a server of theirs, what was
//! said in a chat - is in a file only its owner reads, in a directory only
//! its owner enters. That is what there is on a computer of one's own, and
//! on a server it keeps whoever else has an account there out.

use std::io::Write as _;
use std::path::Path;

/// Close a directory to everybody but its owner. Done every time it is
/// opened, so that one somebody opened up is closed again.
pub(crate) fn directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Close a file that is there to everybody but its owner.
pub(crate) fn file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Write a file whole, closed from the moment it is made: written beside
/// where it goes and moved there, so nobody reads half of it or reads it
/// before it is closed.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let beside = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut made = options.open(&beside)?;
        made.write_all(bytes)?;
        made.sync_all()?;
        file(&beside)?;
        std::fs::rename(&beside, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&beside);
    }
    written
}
