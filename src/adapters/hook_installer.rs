//! Installing the git `pre-push` hook shim. The shim only calls `gcma hook run`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::fsutil;
use super::git_cli::GitCli;
use crate::domain::error::{Error, Result};

const MARKER: &str = "gcma-managed-hook";
/// Where git looks for the hook, relative to the git directory.
const PRE_PUSH: &str = "hooks/pre-push";

/// `None` when there is no file, else whether gcma wrote it. Refuses a symbolic link.
fn is_managed(path: &Path) -> Result<Option<bool>> {
    Ok(fsutil::read_regular(path)?.map(|b| String::from_utf8_lossy(&b).contains(MARKER)))
}

/// Writes the shim to the repository's `hooks/pre-push`.
pub fn install(repo: &GitCli, force: bool) -> Result<PathBuf> {
    let path = repo.git_path(PRE_PUSH)?;
    if !force && is_managed(&path)? == Some(false) {
        return Err(Error::Precondition(format!(
            "{} already exists and is not managed by gcma; use --force to overwrite",
            path.display()
        )));
    }
    if let Some(dir) = path.parent() {
        fs_err::create_dir_all(dir)?;
    }
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "gcma".into());
    let script = format!(
        "#!/bin/sh\n# {MARKER}\nGCMA='{}'\n[ -x \"$GCMA\" ] || {{ echo \"gcma: $GCMA is gone; reinstall the hook with: gcma hook install --force\" >&2; exit 1; }}\nexec \"$GCMA\" hook run pre-push \"$@\"\n",
        exe.replace('\'', "'\\''")
    );
    fsutil::write_regular(&path, script.as_bytes())?;
    fs_err::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

/// Removes the shim; `false` when there was none. Refuses a hook gcma did not write.
pub fn uninstall(repo: &GitCli) -> Result<bool> {
    let path = repo.git_path(PRE_PUSH)?;
    match is_managed(&path)? {
        None => Ok(false),
        Some(false) => Err(Error::Precondition(format!(
            "{} is not managed by gcma; not removing it",
            path.display()
        ))),
        Some(true) => {
            fsutil::remove_regular(&path)?;
            Ok(true)
        }
    }
}
