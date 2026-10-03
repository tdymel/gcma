//! Signing with ssh keys.

use std::path::PathBuf;
use std::process::Command;

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
}
