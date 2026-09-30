//! Installing the git hook shims (`pre-push`, and `post-commit` on request). A shim only calls
//! `gcma hook run`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::fsutil;
use super::git_cli::GitCli;
use crate::domain::error::{Error, Result};

const MARKER: &str = "gcma-managed-hook";

/// A hook gcma can install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hook {
    PrePush,
    PostCommit,
}

impl Hook {
    pub const ALL: [Hook; 2] = [Hook::PrePush, Hook::PostCommit];

    pub fn name(self) -> &'static str {
        match self {
            Hook::PrePush => "pre-push",
            Hook::PostCommit => "post-commit",
        }
    }

    /// The hook git runs under `name`, if gcma has one.
    pub fn from_name(name: &str) -> Option<Hook> {
        Hook::ALL.into_iter().find(|h| h.name() == name)
    }

    /// The shim's last line. `pre-push` hands over to gcma, whose exit code decides the push. A
    /// commit has already happened, so `post-commit` never fails.
    fn run_line(self) -> String {
        match self {
            Hook::PrePush => "exec \"$GCMA\" hook run pre-push \"$@\"".into(),
            Hook::PostCommit => "\"$GCMA\" hook run post-commit \"$@\" || :".into(),
        }
    }

    /// What the shim does when the binary it names has been removed.
    fn gone(self) -> &'static str {
        match self {
            Hook::PrePush => "exit 1",
            Hook::PostCommit => "exit 0",
        }
    }

    fn script(self, exe: &str) -> String {
        format!(
            "#!/bin/sh\n# {MARKER}\nGCMA='{}'\n[ -x \"$GCMA\" ] || {{ echo \"gcma: $GCMA is gone; reinstall the hook with: gcma hook install --force\" >&2; {}; }}\n{}\n",
            exe.replace('\'', "'\\''"),
            self.gone(),
            self.run_line()
        )
    }

    fn path(self, repo: &GitCli) -> Result<PathBuf> {
        repo.git_path(&format!("hooks/{}", self.name()))
    }
}

/// `None` when there is no file, else whether gcma wrote it. Refuses a symbolic link.
fn is_managed(path: &Path) -> Result<Option<bool>> {
    Ok(fsutil::read_regular(path)?.map(|b| String::from_utf8_lossy(&b).contains(MARKER)))
}

/// Writes the shims to the repository's hooks directory. Nothing is written when one of them would
/// overwrite a hook gcma did not write (unless `force`).
pub fn install(repo: &GitCli, hooks: &[Hook], force: bool) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for hook in hooks {
        let path = hook.path(repo)?;
        if !force && is_managed(&path)? == Some(false) {
            return Err(Error::Precondition(format!(
                "{} already exists and is not managed by gcma; use --force to overwrite",
                path.display()
            )));
        }
        paths.push(path);
    }
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "gcma".into());
    for (hook, path) in hooks.iter().zip(&paths) {
        if let Some(dir) = path.parent() {
            fs_err::create_dir_all(dir)?;
        }
        fsutil::write_regular(path, hook.script(&exe).as_bytes())?;
        fs_err::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(paths)
}

/// Removes every shim gcma wrote and returns which. A hook gcma did not write is left alone; it is
/// only an error when that is all there was.
pub fn uninstall(repo: &GitCli) -> Result<Vec<Hook>> {
    let mut removed = Vec::new();
    let mut foreign = None;
    for hook in Hook::ALL {
        let path = hook.path(repo)?;
        match is_managed(&path)? {
            None => {}
            Some(false) => foreign = Some(path),
            Some(true) => {
                fsutil::remove_regular(&path)?;
                removed.push(hook);
            }
        }
    }
    match foreign {
        Some(path) if removed.is_empty() => Err(Error::Precondition(format!(
            "{} is not managed by gcma; not removing it",
            path.display()
        ))),
        _ => Ok(removed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shims_name_the_binary_and_carry_the_marker() {
        for hook in Hook::ALL {
            let s = hook.script("/opt/it's/gcma");
            assert!(s.starts_with("#!/bin/sh\n"));
            assert!(s.contains(MARKER));
            assert!(s.contains("GCMA='/opt/it'\\''s/gcma'"), "{s}");
            assert!(s.contains(&format!("hook run {}", hook.name())), "{s}");
        }
    }

    #[test]
    fn a_hook_is_found_by_its_name() {
        for hook in Hook::ALL {
            assert_eq!(Hook::from_name(hook.name()), Some(hook));
        }
        assert_eq!(Hook::from_name("pre-commit"), None);
    }

    #[test]
    fn only_post_commit_can_never_fail_the_commit() {
        let post = Hook::PostCommit.script("gcma");
        assert!(post.contains("|| :") && post.contains("exit 0;"));
        let pre = Hook::PrePush.script("gcma");
        assert!(pre.contains("exec \"$GCMA\"") && pre.contains("exit 1;"));
    }
}
