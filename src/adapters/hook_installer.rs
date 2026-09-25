//! Installing the git `pre-push` hook shim. The shim only calls `gcma hook run`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::fsutil;
use crate::domain::error::{Error, Result};

const MARKER: &str = "gcma-managed-hook";

/// Writes the shim to `path` (the repository's `hooks/pre-push`).
pub fn install(path: &Path, force: bool) -> Result<PathBuf> {
    if path.exists() && !force {
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        if !existing.contains(MARKER) {
            return Err(Error::Precondition(format!(
                "{} already exists and is not managed by gcma; use --force to overwrite",
                path.display()
            )));
        }
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
    fsutil::write_regular(path, script.as_bytes())?;
    fs_err::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path.to_path_buf())
}

/// Removes the shim; `false` when there was none. Refuses a hook gcma did not write.
pub fn uninstall(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if !existing.contains(MARKER) {
        return Err(Error::Precondition(format!(
            "{} is not managed by gcma; not removing it",
            path.display()
        )));
    }
    fs_err::remove_file(path)?;
    Ok(true)
}
