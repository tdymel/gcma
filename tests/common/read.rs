//! Reading a `Repo` back: the history, raw commit bytes, the refs and the backups.

use std::collections::HashMap;

use super::Repo;

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

impl Repo {
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
    pub fn messages(&self, rev: &str) -> HashMap<String, String> {
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

    /// The id of the first backup, read from the refs instead of from `gcma restore`.
    pub fn backup_id_from_refs(&self) -> String {
        self.git(&["for-each-ref", "--format=%(refname)", "refs/gcma/backup/"])
            .lines()
            .next()
            .unwrap()
            .rsplit('/')
            .nth(1)
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
