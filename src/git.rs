//! Thin plumbing layer over the `git` binary. All repository access goes through here.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::domain::error::{Error, Result};
use crate::domain::history::commit::{Commit, NewCommit, build_commit_buffer, format_tz};
use crate::domain::settings::Backend;

#[derive(Debug, Clone)]
pub struct Git {
    dir: PathBuf,
    backend: Backend,
    #[cfg(feature = "gix")]
    gix: Option<gix::Repository>,
}

struct Out {
    ok: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Git {
    pub fn open(dir: &Path) -> Result<Git> {
        let g = Git {
            dir: dir.to_path_buf(),
            backend: Backend::Git,
            #[cfg(feature = "gix")]
            gix: None,
        };
        let top = g
            .text(&["rev-parse", "--show-toplevel"])
            .map_err(|_| Error::Usage("not inside a git work tree".into()))?;
        Ok(Git {
            dir: PathBuf::from(top),
            backend: Backend::Git,
            #[cfg(feature = "gix")]
            gix: None,
        })
    }

    /// Selects the object backend. `Gix` fails cleanly when it was not compiled in.
    pub fn with_backend(mut self, backend: Backend) -> Result<Git> {
        self.backend = backend;
        #[cfg(feature = "gix")]
        {
            self.gix = match backend {
                Backend::Git => None,
                Backend::Gix => Some(
                    gix::open(&self.dir)
                        .map_err(|e| Error::Git(format!("gix cannot open the repository: {e}")))?,
                ),
            };
        }
        #[cfg(not(feature = "gix"))]
        if backend == Backend::Gix {
            return Err(Error::Usage(
                "this build has no gix backend (rebuild with `--features gix`)".into(),
            ));
        }
        Ok(self)
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    #[cfg(feature = "gix")]
    fn gix_oid(oid: &str) -> Result<gix::ObjectId> {
        gix::ObjectId::from_hex(oid.as_bytes())
            .map_err(|e| Error::Git(format!("bad object id {oid:?}: {e}")))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command(&self) -> Command {
        let mut c = Command::new("git");
        c.arg("-C").arg(&self.dir);
        c.env("LC_ALL", "C");
        c.env_remove("GIT_DIR");
        c.env_remove("GIT_WORK_TREE");
        c.env_remove("GIT_INDEX_FILE");
        c
    }

    fn exec(&self, mut cmd: Command, stdin: Option<&[u8]>) -> Result<Out> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = cmd.spawn()?;
        let writer = stdin.map(|data| {
            let mut si = child.stdin.take().expect("piped stdin");
            let data = data.to_vec();
            std::thread::spawn(move || {
                let _ = si.write_all(&data);
            })
        });
        let out = child.wait_with_output()?;
        if let Some(w) = writer {
            let _ = w.join();
        }
        Ok(Out {
            ok: out.status.success(),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }

    fn fail(args: &[&str], o: &Out) -> Error {
        Error::Git(format!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        ))
    }

    pub fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut c = self.command();
        c.args(args);
        let o = self.exec(c, None)?;
        if !o.ok {
            return Err(Self::fail(args, &o));
        }
        Ok(o.stdout)
    }

    /// Runs and reports only whether git exited successfully.
    pub fn succeeds(&self, args: &[&str]) -> Result<bool> {
        let mut c = self.command();
        c.args(args);
        Ok(self.exec(c, None)?.ok)
    }

    pub fn run_stdin(&self, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
        let mut c = self.command();
        c.args(args);
        let o = self.exec(c, Some(input))?;
        if !o.ok {
            return Err(Self::fail(args, &o));
        }
        Ok(o.stdout)
    }

    pub fn text(&self, args: &[&str]) -> Result<String> {
        let out = self.run(args)?;
        Ok(String::from_utf8_lossy(&out)
            .trim_end_matches('\n')
            .to_string())
    }

    /// `rev-parse --verify`; None when the rev does not resolve.
    pub fn rev_parse(&self, rev: &str) -> Result<Option<String>> {
        let spec = format!("{rev}^{{commit}}");
        let mut c = self.command();
        c.args(["rev-parse", "--verify", "-q", &spec]);
        let o = self.exec(c, None)?;
        if o.ok {
            Ok(Some(String::from_utf8_lossy(&o.stdout).trim().to_string()))
        } else {
            Ok(None)
        }
    }

    pub fn current_branch_ref(&self) -> Result<Option<String>> {
        let mut c = self.command();
        c.args(["symbolic-ref", "-q", "HEAD"]);
        let o = self.exec(c, None)?;
        if o.ok {
            Ok(Some(String::from_utf8_lossy(&o.stdout).trim().to_string()))
        } else {
            Ok(None)
        }
    }

    pub fn ref_value(&self, refname: &str) -> Result<Option<String>> {
        let mut c = self.command();
        c.args(["rev-parse", "--verify", "-q", refname]);
        let o = self.exec(c, None)?;
        if o.ok {
            Ok(Some(String::from_utf8_lossy(&o.stdout).trim().to_string()))
        } else {
            Ok(None)
        }
    }

    pub fn upstream_oid(&self, branch_ref: &str) -> Result<Option<String>> {
        // `@{upstream}` only resolves with a short branch name, not `refs/heads/<name>`.
        let short = branch_ref.strip_prefix("refs/heads/").unwrap_or(branch_ref);
        let spec = format!("{short}@{{upstream}}");
        self.rev_parse(&spec)
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        let mut c = self.command();
        c.args(["merge-base", a, b]);
        let o = self.exec(c, None)?;
        if o.ok {
            Ok(Some(String::from_utf8_lossy(&o.stdout).trim().to_string()))
        } else {
            Ok(None)
        }
    }

    pub fn is_ancestor(&self, a: &str, b: &str) -> Result<bool> {
        self.succeeds(&["merge-base", "--is-ancestor", a, b])
    }

    pub fn git_path(&self, p: &str) -> Result<PathBuf> {
        let t = self.text(&["rev-parse", "--git-path", p])?;
        let pb = PathBuf::from(t);
        Ok(if pb.is_absolute() {
            pb
        } else {
            self.dir.join(pb)
        })
    }

    /// `rev-list --parents <args>`; returns (oid, parents) in output order.
    pub fn rev_list_parents(&self, args: &[&str]) -> Result<Vec<(String, Vec<String>)>> {
        let mut full = vec!["rev-list", "--parents"];
        full.extend_from_slice(args);
        let out = self.text(&full)?;
        Ok(out
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                let mut it = l.split(' ').map(|s| s.to_string());
                let oid = it.next().unwrap();
                (oid, it.collect())
            })
            .collect())
    }

    pub fn rev_count(&self, rev: &str) -> Result<usize> {
        Ok(self
            .text(&["rev-list", "--count", rev])?
            .trim()
            .parse()
            .unwrap_or(0))
    }

    pub fn read_commits(&self, oids: &[String]) -> Result<Vec<Commit>> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        #[cfg(feature = "gix")]
        if let Some(repo) = &self.gix {
            return oids
                .iter()
                .map(|oid| {
                    let obj = repo
                        .find_object(Self::gix_oid(oid)?)
                        .map_err(|e| Error::Git(format!("cannot read {oid}: {e}")))?;
                    if obj.kind != gix::objs::Kind::Commit {
                        return Err(Error::Git(format!("object {oid} is not a commit")));
                    }
                    Commit::parse(oid, &obj.data)
                })
                .collect();
        }
        let mut input = Vec::new();
        for o in oids {
            input.extend_from_slice(o.as_bytes());
            input.push(b'\n');
        }
        let out = self.run_stdin(&["cat-file", "--batch"], &input)?;
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

    /// Which of `oids` exist as objects.
    pub fn objects_exist(&self, oids: &[String]) -> Result<bool> {
        if oids.is_empty() {
            return Ok(true);
        }
        #[cfg(feature = "gix")]
        if let Some(repo) = &self.gix {
            for o in oids {
                if !repo.has_object(Self::gix_oid(o)?) {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        let mut input = Vec::new();
        for o in oids {
            input.extend_from_slice(o.as_bytes());
            input.push(b'\n');
        }
        let out = self.run_stdin(&["cat-file", "--batch-check"], &input)?;
        Ok(!String::from_utf8_lossy(&out).contains("missing"))
    }

    /// Writes a commit object from raw bytes (never `commit-tree`: it drops headers).
    pub fn write_commit_raw(&self, c: &NewCommit) -> Result<String> {
        let buf = build_commit_buffer(c);
        #[cfg(feature = "gix")]
        if let Some(repo) = &self.gix {
            let id = gix::objs::Write::write_buf(&repo.objects, gix::objs::Kind::Commit, &buf)
                .map_err(|e| Error::Git(format!("gix cannot write a commit: {e}")))?;
            return Ok(id.to_string());
        }
        let out = self.run_stdin(&["hash-object", "-t", "commit", "-w", "--stdin"], &buf)?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// Writes a signed commit through `commit-tree -S` (uses the user's signing config).
    /// Unknown headers cannot be carried over by commit-tree.
    pub fn write_commit_signed(&self, c: &NewCommit) -> Result<String> {
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
        let o = self.exec(cmd, Some(c.message))?;
        if !o.ok {
            return Err(Error::Git(format!(
                "commit-tree -S failed (is signing configured?): {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    }

    /// Runs an `update-ref --stdin` transaction. `commands` are lines like `create <ref> <new>`.
    pub fn update_refs(&self, message: &str, commands: &[String]) -> Result<()> {
        let mut input = String::from("start\n");
        for c in commands {
            input.push_str(c);
            input.push('\n');
        }
        input.push_str("prepare\ncommit\n");
        let mut c = self.command();
        c.args(["update-ref", "-m", message, "--stdin"]);
        let o = self.exec(c, Some(input.as_bytes()))?;
        if !o.ok {
            return Err(Error::TipMoved(format!(
                "ref transaction failed (did the branch move?): {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )));
        }
        Ok(())
    }

    pub fn for_each_ref(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let out = self.text(&["for-each-ref", "--format=%(refname) %(objectname)", prefix])?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect())
    }

    /// Of the given commits, those NOT reachable from `upstream`.
    pub fn unpushed_among(&self, oids: &[String], upstream: &str) -> Result<HashSet<String>> {
        if oids.is_empty() {
            return Ok(HashSet::new());
        }
        let mut input = String::new();
        for o in oids {
            input.push_str(o);
            input.push('\n');
        }
        let not = format!("^{upstream}");
        let out = self.run_stdin(
            &["rev-list", "--no-walk=unsorted", "--stdin", &not],
            input.as_bytes(),
        )?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .map(|s| s.to_string())
            .collect())
    }

    pub fn is_shallow(&self) -> Result<bool> {
        Ok(self.text(&["rev-parse", "--is-shallow-repository"])? == "true")
    }

    pub fn has_replace_refs(&self) -> Result<bool> {
        Ok(!self
            .text(&["for-each-ref", "--count=1", "refs/replace"])?
            .is_empty())
    }

    pub fn has_grafts(&self) -> Result<bool> {
        Ok(self.git_path("info/grafts")?.exists())
    }

    pub fn index_dirty(&self) -> Result<bool> {
        Ok(!self.succeeds(&["diff", "--cached", "--quiet"])?)
    }

    pub fn operation_in_progress(&self) -> Result<Option<&'static str>> {
        for (p, name) in [
            ("rebase-merge", "rebase"),
            ("rebase-apply", "rebase/am"),
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry-pick"),
            ("REVERT_HEAD", "revert"),
        ] {
            if self.git_path(p)?.exists() {
                return Ok(Some(name));
            }
        }
        Ok(None)
    }

    pub fn diff_quiet(&self, a: &str, b: &str) -> Result<bool> {
        self.succeeds(&["diff", "--quiet", a, b])
    }

    /// (additions, deletions, files) against the first parent, for many commits in one process.
    /// The result is in the order of `oids`.
    pub fn numstats(&self, oids: &[String]) -> Result<Vec<(u64, u64, u64)>> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let mut input = String::new();
        for o in oids {
            input.push_str(o);
            input.push('\n');
        }
        let out = self.run_stdin(
            &[
                "log",
                "--no-walk=unsorted",
                "--stdin",
                "--numstat",
                "--format=@@%H",
                "--diff-merges=first-parent",
            ],
            input.as_bytes(),
        )?;
        let text = String::from_utf8_lossy(&out);
        let mut stats = Vec::with_capacity(oids.len());
        let mut cur: Option<(u64, u64, u64)> = None;
        for l in text.lines() {
            if l.starts_with("@@") {
                stats.extend(cur.take());
                cur = Some((0, 0, 0));
            } else if let (Some(c), false) = (cur.as_mut(), l.is_empty()) {
                let mut it = l.split('\t');
                let (x, y) = (it.next().unwrap_or("-"), it.next().unwrap_or("-"));
                c.0 += x.parse::<u64>().unwrap_or(0);
                c.1 += y.parse::<u64>().unwrap_or(0);
                c.2 += 1;
            }
        }
        stats.extend(cur.take());
        if stats.len() != oids.len() {
            return Err(Error::Git(format!(
                "numstat returned {} entries for {} commits",
                stats.len(),
                oids.len()
            )));
        }
        Ok(stats)
    }

    /// Tags (`refs/tags/*`, lightweight or annotated) and notes that point at any of `oids`.
    pub fn labels_pointing_at(&self, oids: &HashSet<String>) -> Result<Vec<String>> {
        let mut found = Vec::new();
        let out = self.text(&[
            "for-each-ref",
            "--format=%(refname) %(objectname) %(*objectname)",
            "refs/tags",
        ])?;
        for l in out.lines() {
            let parts: Vec<&str> = l.split(' ').collect();
            if parts.len() >= 2
                && (oids.contains(parts[1]) || parts.get(2).is_some_and(|t| oids.contains(*t)))
            {
                found.push(parts[0].to_string());
            }
        }
        if let Ok(notes) = self.text(&["notes", "list"]) {
            for l in notes.lines() {
                if let Some((_, target)) = l.split_once(' ')
                    && oids.contains(target)
                {
                    found.push(format!("note on {}", &target[..target.len().min(8)]));
                }
            }
        }
        Ok(found)
    }
}
