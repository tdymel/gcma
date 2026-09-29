//! `CommitStore` over `cat-file`, `hash-object` and `commit-tree`.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use super::runner::{GitCli, lines_input};
use crate::application::ports::CommitStore;
use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{Commit, NewCommit, build_commit_buffer, format_tz};
use crate::domain::settings::Signing;

impl GitCli {
    /// Writes a commit object from raw bytes (never `commit-tree`: it drops headers).
    fn write_raw(&self, c: &NewCommit) -> Result<String> {
        let buf = build_commit_buffer(c);
        let out = self.run_stdin(&["hash-object", "-t", "commit", "-w", "--stdin"], &buf)?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// Writes a signed commit through `commit-tree -S` (uses the user's signing config).
    /// Unknown headers cannot be carried over by commit-tree.
    fn write_signed(&self, c: &NewCommit) -> Result<String> {
        let mut cmd = self.command();
        cmd.arg("commit-tree").arg("-S").arg(c.tree);
        for p in c.parents {
            cmd.arg("-p").arg(p);
        }
        cmd.env("GIT_AUTHOR_NAME", OsStr::from_bytes(&c.author.name));
        cmd.env("GIT_AUTHOR_EMAIL", OsStr::from_bytes(&c.author.email));
        cmd.env(
            "GIT_AUTHOR_DATE",
            format!("{} {}", c.author.time, format_tz(c.author.tz)),
        );
        cmd.env("GIT_COMMITTER_NAME", OsStr::from_bytes(&c.committer.name));
        cmd.env("GIT_COMMITTER_EMAIL", OsStr::from_bytes(&c.committer.email));
        cmd.env(
            "GIT_COMMITTER_DATE",
            format!("{} {}", c.committer.time, format_tz(c.committer.tz)),
        );
        let o = Self::exec(cmd, Some(c.message))?;
        if !o.ok {
            return Err(Error::Git(format!(
                "commit-tree -S failed (is signing configured?): {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    }
}

impl CommitStore for GitCli {
    fn read_commits(&self, oids: &[String]) -> Result<Vec<Commit>> {
        match &self.objects {
            Some(o) => o.read_commits(oids),
            None => self.cat_commits(oids),
        }
    }

    fn objects_exist(&self, oids: &[String]) -> Result<bool> {
        match &self.objects {
            Some(o) => o.objects_exist(oids),
            None => self.check_objects(oids),
        }
    }

    fn write_commit(&self, commit: &NewCommit, signing: Signing) -> Result<String> {
        match (&self.objects, signing) {
            (Some(o), Signing::Strip) => o.write_commit(commit, signing),
            (None, Signing::Strip) => self.write_raw(commit),
            (_, Signing::Resign) => self.write_signed(commit),
        }
    }
}

impl GitCli {
    fn cat_commits(&self, oids: &[String]) -> Result<Vec<Commit>> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let out = self.run_stdin(&["cat-file", "--batch"], lines_input(oids).as_bytes())?;
        let mut pos = 0usize;
        let mut commits = Vec::with_capacity(oids.len());
        for oid in oids {
            let nl = out[pos..]
                .iter()
                .position(|&b| b == b'\n')
                .ok_or_else(|| Error::Git("truncated cat-file output".into()))?
                + pos;
            let header = String::from_utf8_lossy(&out[pos..nl]).to_string();
            let parts: Vec<&str> = header.split(' ').collect();
            if parts.len() != 3 || parts[1] != "commit" {
                return Err(Error::Git(format!(
                    "object {oid} is not a readable commit ({header})"
                )));
            }
            let size: usize = parts[2]
                .parse()
                .map_err(|_| Error::Git(format!("bad cat-file header: {header}")))?;
            let body = out
                .get(nl + 1..nl + 1 + size)
                .ok_or_else(|| Error::Git("truncated cat-file body".into()))?;
            commits.push(Commit::parse(oid, body)?);
            pos = nl + 1 + size + 1;
        }
        Ok(commits)
    }

    fn check_objects(&self, oids: &[String]) -> Result<bool> {
        if oids.is_empty() {
            return Ok(true);
        }
        let out = self.run_stdin(&["cat-file", "--batch-check"], lines_input(oids).as_bytes())?;
        Ok(!String::from_utf8_lossy(&out).contains("missing"))
    }
}
