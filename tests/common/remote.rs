//! Bare remotes: a repository with `origin` and the hooks installed, more remotes next to it, and
//! reading back what a push left on a remote.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::repo::Repo;

/// The tip of `branch` in a bare repository.
pub fn remote_tip(remote: &Path, branch: &str) -> String {
    let o = Command::new("git")
        .arg("--git-dir")
        .arg(remote)
        .args(["rev-parse", branch])
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// The files of `main` in a bare repository, one path per line.
pub fn remote_files(remote: &Path) -> String {
    let o = Command::new("git")
        .arg("--git-dir")
        .arg(remote)
        .args(["ls-tree", "-r", "--name-only", "main"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).to_string()
}

impl Repo {
    /// A new repository with a bare `origin` (see `remote`), the config `cfg` and the pre-push
    /// hook.
    pub fn hooked(cfg: &str) -> Repo {
        Repo::with_hooks(cfg, &["hook", "install"])
    }

    /// `hooked` with the post-commit hook too.
    pub fn hooked_post_commit(cfg: &str) -> Repo {
        Repo::with_hooks(cfg, &["hook", "install", "--post-commit"])
    }

    fn with_hooks(cfg: &str, install: &[&str]) -> Repo {
        let r = Repo::new();
        r.bare_remote();
        r.config(cfg);
        r.gcma_ok(install);
        r
    }

    /// The path of the bare `origin` that `bare_remote` (or `hooked`) adds.
    pub fn remote(&self) -> PathBuf {
        self.remote_path("origin")
    }

    /// Adds a bare repository as `origin` and returns its path.
    pub fn bare_remote(&self) -> PathBuf {
        self.add_remote("origin")
    }

    /// Adds an empty bare repository as the remote `name` (it has no remote-tracking refs yet) and
    /// returns its path.
    pub fn add_remote(&self, name: &str) -> PathBuf {
        let remote = self.remote_path(name);
        let o = Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(&remote)
            .stdout(Stdio::null())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(o.status.success());
        self.git(&["remote", "add", name, remote.to_str().unwrap()]);
        remote
    }

    fn remote_path(&self, name: &str) -> PathBuf {
        self.home.path().join(format!("{name}.git"))
    }
}
