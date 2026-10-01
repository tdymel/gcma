//! Building history in a `Repo`: commits at given times, by given identities, with exact bytes.

use std::io::Write;
use std::process::Stdio;

use super::Repo;

impl Repo {
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
}
