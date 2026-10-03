//! The outside world of a repository: ssh signing and bare remotes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::repo::Repo;

/// Whether `ssh-keygen` exists. On CI a missing one is an error, so a signing test cannot pass by
/// silently doing nothing; elsewhere the test is skipped (returns false).
pub fn ssh_keygen_or_skip() -> bool {
    let present = Command::new("ssh-keygen").arg("-?").output().is_ok();
    if !present {
        assert!(
            std::env::var_os("CI").is_none(),
            "ssh-keygen is required for the signing tests on CI"
        );
        eprintln!("skipping: ssh-keygen not available");
    }
    present
}

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
    /// Configures SSH signing and returns the allowed-signers file that verifies it.
    pub fn ssh_signing(&self) -> PathBuf {
        let key = self.home.path().join("sign_key");
        let o = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .output()
            .unwrap();
        assert!(o.status.success());
        let public = std::fs::read_to_string(format!("{}.pub", key.display())).unwrap();
        let allowed = self.home.path().join("allowed_signers");
        std::fs::write(&allowed, format!("me@home.org {public}")).unwrap();
        self.git(&["config", "gpg.format", "ssh"]);
        self.git(&[
            "config",
            "user.signingkey",
            &format!("{}.pub", key.display()),
        ]);
        self.git(&[
            "config",
            "gpg.ssh.allowedSignersFile",
            allowed.to_str().unwrap(),
        ]);
        allowed
    }

    /// Adds a bare repository as `origin` and returns its path.
    pub fn bare_remote(&self) -> PathBuf {
        let remote = self.home.path().join("remote.git");
        let o = Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(&remote)
            .stdout(Stdio::null())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(o.status.success());
        self.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        remote
    }

    /// A bare remote (`origin`), the config `cfg` and the pre-push hook; returns the remote.
    pub fn hooked(&self, cfg: &str) -> PathBuf {
        let remote = self.bare_remote();
        self.config(cfg);
        self.gcma_ok(&["hook", "install"]);
        remote
    }
}
