#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use chrono::{Datelike, TimeZone, Timelike};
use tempfile::TempDir;

pub struct Repo {
    pub dir: TempDir,
    pub home: TempDir,
    /// Forces the object backend of every `ghma` run (`git` or `gix`); `None` keeps the default.
    pub backend: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub oid: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub an: String,
    pub ae: String,
    pub at: i64,
    pub cn: String,
    pub ce: String,
    pub ct: i64,
    pub subject: String,
}

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_ghma")
}

pub fn base_cmd(prog: &str, dir: &Path, home: &Path) -> Command {
    let mut c = Command::new(prog);
    c.current_dir(dir);
    c.env("HOME", home);
    c.env("GIT_CONFIG_GLOBAL", "/dev/null");
    c.env("GIT_CONFIG_SYSTEM", "/dev/null");
    c.env("GIT_CONFIG_NOSYSTEM", "1");
    c.env("GIT_TERMINAL_PROMPT", "0");
    c.env_remove("GIT_DIR");
    c.env_remove("GIT_WORK_TREE");
    c
}

impl Repo {
    pub fn new() -> Repo {
        let r = Repo {
            dir: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
            backend: None,
        };
        r.git(&["init", "-q", "-b", "main"]);
        r.git(&["config", "user.name", "Old Me"]);
        r.git(&["config", "user.email", "me@home.org"]);
        r.git(&["config", "commit.gpgsign", "false"]);
        r
    }

    /// A repository whose `ghma` runs use the backend `seed` selects: the two alternate, so a
    /// series of seeds covers both (only `git` when the `gix` feature is not built).
    pub fn for_seed(seed: u64) -> Repo {
        let mut r = Repo::new();
        r.backend = Some(if cfg!(feature = "gix") && seed % 2 == 1 {
            "gix"
        } else {
            "git"
        });
        r
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn cmd(&self, prog: &str) -> Command {
        base_cmd(prog, self.path(), self.home.path())
    }

    pub fn git_out(&self, args: &[&str]) -> Output {
        self.cmd("git").args(args).output().unwrap()
    }

    pub fn git(&self, args: &[&str]) -> String {
        let o = self.git_out(args);
        assert!(
            o.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout)
            .trim_end_matches('\n')
            .to_string()
    }

    pub fn ghma(&self, args: &[&str]) -> Output {
        let mut c = self.cmd(bin());
        if let Some(b) = self.backend {
            c.env("GHMA_BACKEND", b);
        }
        c.args(args).output().unwrap()
    }

    pub fn ghma_ok(&self, args: &[&str]) -> String {
        let o = self.ghma(args);
        assert!(
            o.status.success(),
            "ghma {:?} failed ({:?}): {}{}",
            args,
            o.status.code(),
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).to_string()
    }

    pub fn code(o: &Output) -> i32 {
        o.status.code().unwrap_or(-1)
    }

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.path().join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    pub fn config(&self, yaml: &str) {
        self.write("ghma.yml", yaml);
    }

    /// Commit a new file at an explicit time as the default (old) identity.
    pub fn commit_at(&self, name: &str, msg: &str, unix: i64) -> String {
        self.commit_as(name, msg, unix, "Old Me", "me@home.org")
    }

    /// Like `commit_at`, written with the given UTC offset (`+0100`) instead of `+0000`.
    pub fn commit_at_offset(&self, name: &str, msg: &str, unix: i64, offset: &str) -> String {
        self.write(name, &format!("content of {name}\n"));
        self.git(&["add", name]);
        let date = format!("{unix} {offset}");
        let o = self
            .cmd("git")
            .args(["commit", "-q", "-m", msg])
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "commit failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        self.git(&["rev-parse", "HEAD"])
    }

    pub fn commit_as(&self, name: &str, msg: &str, unix: i64, who: &str, email: &str) -> String {
        self.write(name, &format!("content of {name}\n"));
        self.git(&["add", name]);
        let date = format!("{unix} +0000");
        let o = self
            .cmd("git")
            .args(["commit", "-q", "-m", msg])
            .env("GIT_AUTHOR_NAME", who)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", who)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "commit failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        self.git(&["rev-parse", "HEAD"])
    }

    /// Like `commit_as`, with a name and email that need not be UTF-8. `git commit` recodes
    /// identities, so the object is written by hand.
    pub fn commit_as_bytes(
        &self,
        name: &str,
        msg: &str,
        unix: i64,
        who: &[u8],
        email: &[u8],
    ) -> String {
        self.commit_raw(name, msg.as_bytes(), unix, who, email)
    }

    /// Writes a commit object by hand, so names and message bytes are stored exactly as given
    /// (`git commit` re-encodes messages that are not UTF-8).
    pub fn commit_raw(
        &self,
        name: &str,
        msg: &[u8],
        unix: i64,
        who: &[u8],
        email: &[u8],
    ) -> String {
        use std::io::Write;
        self.write(name, &format!("content of {name}\n"));
        self.git(&["add", name]);
        let tree = self.git(&["write-tree"]);
        let mut raw = format!("tree {tree}\n").into_bytes();
        if self
            .git_out(&["rev-parse", "-q", "--verify", "HEAD"])
            .status
            .success()
        {
            raw.extend(format!("parent {}\n", self.git(&["rev-parse", "HEAD"])).bytes());
        }
        for role in ["author", "committer"] {
            raw.extend(format!("{role} ").bytes());
            raw.extend_from_slice(who);
            raw.extend(b" <");
            raw.extend_from_slice(email);
            raw.extend(format!("> {unix} +0000\n").bytes());
        }
        raw.extend(b"\n");
        raw.extend_from_slice(msg);
        let mut child = self
            .cmd("git")
            .args([
                "hash-object",
                "-t",
                "commit",
                "-w",
                "--literally",
                "--stdin",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&raw).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        let oid = String::from_utf8(out.stdout).unwrap().trim().to_string();
        self.git(&["update-ref", "HEAD", &oid]);
        oid
    }

    /// Commit with exactly these message bytes, at `unix` in UTC, as the default identity.
    pub fn commit_msg(&self, name: &str, msg: &[u8], unix: i64) -> String {
        self.commit_raw(name, msg, unix, b"Old Me", b"me@home.org")
    }

    /// The message bytes of a commit, exactly as stored.
    pub fn message_bytes(&self, rev: &str) -> Vec<u8> {
        let raw = self.cat(&self.git(&["rev-parse", rev]));
        let at = raw.windows(2).position(|w| w == b"\n\n").unwrap();
        raw[at + 2..].to_vec()
    }

    /// Commit several files (path, content) at once, as the default identity.
    pub fn commit_files(&self, files: &[(&str, &str)], msg: &str, unix: i64) -> String {
        for (name, content) in files {
            self.write(name, content);
            self.git(&["add", "-f", name]);
        }
        let date = format!("{unix} +0000");
        let o = self
            .cmd("git")
            .args(["commit", "-q", "--allow-empty", "-m", msg])
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "commit failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        self.git(&["rev-parse", "HEAD"])
    }

    /// Build a linear history of `n` commits starting at `start` (2 days apart).
    pub fn linear(&self, n: usize, start: i64) -> Vec<String> {
        (0..n)
            .map(|i| {
                self.commit_at(
                    &format!("f{i}.txt"),
                    &format!("commit {i}"),
                    start + i as i64 * 172800,
                )
            })
            .collect()
    }

    pub fn log(&self) -> Vec<Row> {
        self.log_rev("HEAD")
    }

    pub fn log_rev(&self, rev: &str) -> Vec<Row> {
        let out = self.git(&[
            "log",
            "--reverse",
            "--topo-order",
            "--format=%H|%T|%P|%an|%ae|%at|%cn|%ce|%ct|%s",
            rev,
        ]);
        out.lines()
            .map(|l| {
                let f: Vec<&str> = l.splitn(10, '|').collect();
                Row {
                    oid: f[0].into(),
                    tree: f[1].into(),
                    parents: f[2].split_whitespace().map(|s| s.to_string()).collect(),
                    an: f[3].into(),
                    ae: f[4].into(),
                    at: f[5].parse().unwrap(),
                    cn: f[6].into(),
                    ce: f[7].into(),
                    ct: f[8].parse().unwrap(),
                    subject: f[9].into(),
                }
            })
            .collect()
    }

    pub fn fsck(&self) {
        let o = self.git_out(&["fsck", "--strict", "--no-dangling"]);
        assert!(
            o.status.success(),
            "git fsck failed: {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
    }

    /// Raw commit object bytes.
    pub fn cat(&self, oid: &str) -> Vec<u8> {
        self.cmd("git")
            .args(["cat-file", "commit", oid])
            .output()
            .unwrap()
            .stdout
    }

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
}

/// Pairs every old commit with the new commit that replaced it: same tree and subject, and the
/// parents are the replacements of the old parents, in the same order. Panics when a commit has
/// no counterpart or two commits would share one.
pub fn map_commits(old: &[Row], new: &[Row]) -> std::collections::HashMap<String, String> {
    map_commits_by(old, new, true)
}

/// `map_commits`, optionally ignoring the subjects (for runs that rewrite messages).
pub fn map_commits_by(
    old: &[Row],
    new: &[Row],
    same_subject: bool,
) -> std::collections::HashMap<String, String> {
    let mut map: std::collections::HashMap<String, String> = Default::default();
    let mut taken: std::collections::HashSet<&str> = Default::default();
    for o in old {
        let parents: Vec<String> = o.parents.iter().map(|p| map[p].clone()).collect();
        let n = new
            .iter()
            .find(|n| {
                !taken.contains(n.oid.as_str())
                    && n.tree == o.tree
                    && (!same_subject || n.subject == o.subject)
                    && n.parents == parents
            })
            .unwrap_or_else(|| panic!("no replacement for {} ({:?})", o.oid, o.subject));
        taken.insert(&n.oid);
        map.insert(o.oid.clone(), n.oid.clone());
    }
    map
}

/// Asserts that the two histories are the same shape and every tree and subject is untouched.
pub fn assert_same_content(old: &[Row], new: &[Row]) {
    assert_eq!(old.len(), new.len(), "commit count changed");
    map_commits(old, new);
}

/// Same shape and trees, but the messages may differ.
pub fn assert_same_shape(old: &[Row], new: &[Row]) {
    assert_eq!(old.len(), new.len(), "commit count changed");
    map_commits_by(old, new, false);
}

impl Repo {
    /// Full commit messages by commit id, for every commit reachable from `rev`.
    pub fn messages(&self, rev: &str) -> std::collections::HashMap<String, String> {
        self.git(&["log", "--format=%H%x00%B%x01", rev])
            .split('\u{1}')
            .filter(|rec| !rec.trim().is_empty())
            .map(|rec| {
                let (oid, msg) = rec.trim_start_matches('\n').split_once('\0').unwrap();
                (oid.to_string(), msg.to_string())
            })
            .collect()
    }
}

pub fn berlin_cfg(extra: &str) -> String {
    format!(
        "version: 1\nfrom: 2026-01-05\nto: 2026-03-01\ntimezone: Europe/Berlin\nschedule:\n  days: [mon, tue, wed, thu, fri]\n  hours: \"09:30-18:00\"\n  distribution: bursty\n  seed: 7\n{extra}"
    )
}

pub fn assert_scheduled(rows: &[Row]) {
    let tz: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
    let from = tz
        .with_ymd_and_hms(2026, 1, 5, 0, 0, 0)
        .unwrap()
        .timestamp();
    let to = tz
        .with_ymd_and_hms(2026, 3, 2, 0, 0, 0)
        .unwrap()
        .timestamp();
    for r in rows {
        assert_eq!(r.at, r.ct, "author time equals committer time");
        assert!(r.ct >= from && r.ct < to, "inside the range");
        let l = tz.timestamp_opt(r.ct, 0).unwrap();
        assert!(l.weekday().number_from_monday() <= 5, "weekday: {l}");
        let mins = l.hour() * 60 + l.minute();
        assert!((570..1080).contains(&mins), "working hours: {l}");
    }
    let by: std::collections::HashMap<&str, i64> =
        rows.iter().map(|r| (r.oid.as_str(), r.ct)).collect();
    for r in rows {
        for p in &r.parents {
            assert!(by[p.as_str()] <= r.ct, "monotone along parents");
        }
    }
}

/// Whether `ssh-keygen` exists. On CI a missing one is an error, so a signing test cannot pass by
/// silently doing nothing; elsewhere the test is skipped (returns false).
pub fn ssh_keygen_or_skip() -> bool {
    let present = std::process::Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .is_ok();
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
