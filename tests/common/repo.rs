//! A scratch repository (with its own `HOME`) and the builders and readers the tests share.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

pub struct Repo {
    pub dir: TempDir,
    pub home: TempDir,
    /// Forces the object backend of every `gcma` run (`git` or `gix`); `None` keeps the default.
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
    env!("CARGO_BIN_EXE_gcma")
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

    /// A repository whose `gcma` runs use the backend `seed` selects: the two alternate, so a
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

    // ---------- running commands ----------

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

    pub fn gcma(&self, args: &[&str]) -> Output {
        let mut c = self.cmd(bin());
        if let Some(b) = self.backend {
            c.env("GCMA_BACKEND", b);
        }
        c.args(args).output().unwrap()
    }

    pub fn gcma_ok(&self, args: &[&str]) -> String {
        let o = self.gcma(args);
        assert!(
            o.status.success(),
            "gcma {:?} failed ({:?}): {}{}",
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

    // ---------- files and config ----------

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.path().join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    pub fn config(&self, yaml: &str) {
        self.write("gcma.yml", yaml);
    }

    // ---------- building history ----------

    /// Commits what is staged, with the author and committer date `date` (`<unix> <offset>`) and,
    /// when given, `who` as author and committer. Returns the new commit's id.
    fn commit_staged(
        &self,
        extra_args: &[&str],
        msg: &str,
        date: &str,
        who: Option<(&str, &str)>,
    ) -> String {
        let mut c = self.cmd("git");
        c.args(["commit", "-q"])
            .args(extra_args)
            .args(["-m", msg])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date);
        if let Some((name, email)) = who {
            c.env("GIT_AUTHOR_NAME", name)
                .env("GIT_AUTHOR_EMAIL", email)
                .env("GIT_COMMITTER_NAME", name)
                .env("GIT_COMMITTER_EMAIL", email);
        }
        let o = c.output().unwrap();
        assert!(
            o.status.success(),
            "commit failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        self.git(&["rev-parse", "HEAD"])
    }

    /// Writes and stages the file `name`, then commits it (see `commit_staged`).
    fn commit_new_file(
        &self,
        name: &str,
        msg: &str,
        date: &str,
        who: Option<(&str, &str)>,
    ) -> String {
        self.write(name, &format!("content of {name}\n"));
        self.git(&["add", name]);
        self.commit_staged(&[], msg, date, who)
    }

    /// Commit a new file at an explicit time as the default (old) identity.
    pub fn commit_at(&self, name: &str, msg: &str, unix: i64) -> String {
        self.commit_as(name, msg, unix, "Old Me", "me@home.org")
    }

    /// Like `commit_at`, written with the given UTC offset (`+0100`) instead of `+0000`.
    pub fn commit_at_offset(&self, name: &str, msg: &str, unix: i64, offset: &str) -> String {
        self.commit_new_file(name, msg, &format!("{unix} {offset}"), None)
    }

    pub fn commit_as(&self, name: &str, msg: &str, unix: i64, who: &str, email: &str) -> String {
        self.commit_new_file(name, msg, &format!("{unix} +0000"), Some((who, email)))
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

    /// Commit with exactly these message bytes, at `unix` in UTC, as the default identity.
    pub fn commit_msg(&self, name: &str, msg: &[u8], unix: i64) -> String {
        self.commit_raw(name, msg, unix, b"Old Me", b"me@home.org")
    }

    /// Writes a commit object by hand, so names and message bytes are stored exactly as given
    /// (`git commit` re-encodes messages that are not UTF-8).
    fn commit_raw(&self, name: &str, msg: &[u8], unix: i64, who: &[u8], email: &[u8]) -> String {
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

    /// Commit several files (path, content) at once, as the default identity.
    pub fn commit_files(&self, files: &[(&str, &str)], msg: &str, unix: i64) -> String {
        for (name, content) in files {
            self.write(name, content);
            self.git(&["add", "-f", name]);
        }
        self.commit_staged(&["--allow-empty"], msg, &format!("{unix} +0000"), None)
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

    // ---------- reading history ----------

    pub fn log(&self) -> Vec<Row> {
        self.log_rev("HEAD")
    }

    fn log_rev(&self, rev: &str) -> Vec<Row> {
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

    /// (committer time, committer UTC offset in minutes) of every commit, oldest first.
    pub fn committer_offsets(&self) -> Vec<(i64, i32)> {
        self.git(&[
            "log",
            "--reverse",
            "--topo-order",
            "--format=%cd",
            "--date=raw",
        ])
        .lines()
        .map(|l| {
            let (t, off) = l.split_once(' ').unwrap();
            let sign = if off.starts_with('-') { -1 } else { 1 };
            let (h, m): (i32, i32) = (off[1..3].parse().unwrap(), off[3..5].parse().unwrap());
            (t.parse().unwrap(), sign * (h * 60 + m))
        })
        .collect()
    }

    /// All refs and their targets: the thing that must not change when a run is refused.
    pub fn refs(&self) -> String {
        self.git(&["for-each-ref", "--format=%(refname) %(objectname)"])
    }

    /// `git status --porcelain` without the line of the config file.
    pub fn status_without_config(&self) -> String {
        self.git(&["status", "--porcelain"])
            .lines()
            .filter(|l| !l.contains("gcma.yml"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The id of the first backup `gcma restore` lists.
    pub fn backup_id(&self) -> String {
        self.gcma_ok(&["restore"])
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    /// Raw commit object bytes.
    pub fn cat(&self, oid: &str) -> Vec<u8> {
        self.cmd("git")
            .args(["cat-file", "commit", oid])
            .output()
            .unwrap()
            .stdout
    }

    /// The message bytes of a commit, exactly as stored.
    pub fn message_bytes(&self, rev: &str) -> Vec<u8> {
        let raw = self.cat(&self.git(&["rev-parse", rev]));
        let at = raw.windows(2).position(|w| w == b"\n\n").unwrap();
        raw[at + 2..].to_vec()
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
}
