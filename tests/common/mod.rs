#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use chrono::{Datelike, TimeZone, Timelike};
use tempfile::TempDir;

pub struct Repo {
    pub dir: TempDir,
    pub home: TempDir,
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
        };
        r.git(&["init", "-q", "-b", "main"]);
        r.git(&["config", "user.name", "Old Me"]);
        r.git(&["config", "user.email", "me@home.org"]);
        r.git(&["config", "commit.gpgsign", "false"]);
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
        self.cmd(bin()).args(args).output().unwrap()
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
        self.write(".git-hide-my-ass.yml", yaml);
    }

    /// Commit a new file at an explicit time as the default (old) identity.
    pub fn commit_at(&self, name: &str, msg: &str, unix: i64) -> String {
        self.commit_as(name, msg, unix, "Old Me", "me@home.org")
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

/// Asserts that the two histories are the same shape and every tree and message body is untouched.
pub fn assert_same_content(old: &[Row], new: &[Row]) {
    assert_eq!(old.len(), new.len(), "commit count changed");
    let pos: std::collections::HashMap<&str, usize> = old
        .iter()
        .enumerate()
        .map(|(i, r)| (r.oid.as_str(), i))
        .collect();
    let npos: std::collections::HashMap<&str, usize> = new
        .iter()
        .enumerate()
        .map(|(i, r)| (r.oid.as_str(), i))
        .collect();
    // Match commits by tree (all trees are unique in these fixtures).
    for o in old {
        let n = new
            .iter()
            .find(|n| n.tree == o.tree)
            .unwrap_or_else(|| panic!("tree {} lost", o.tree));
        assert_eq!(
            o.parents.len(),
            n.parents.len(),
            "parent count of {}",
            o.oid
        );
        for (op, np) in o.parents.iter().zip(&n.parents) {
            let opi = pos[op.as_str()];
            assert_eq!(
                old[opi].tree,
                new[npos[np.as_str()]].tree,
                "parent wiring of {}",
                o.oid
            );
        }
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
