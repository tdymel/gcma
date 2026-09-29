//! The process runner: spawning `git` and collecting its output.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::application::ports::CommitStore;
use crate::domain::error::{Error, Result};

/// Set on every git process we start: a gcma that a hook of such a process starts is nested, and the
/// post-commit hook refuses to run again from there.
pub const NESTED_ENV: &str = "GCMA_IN_HOOK";

/// A work tree reached through the `git` binary. Commit reads and unsigned writes can be handed to
/// another object backend (see `with_objects`); everything else always goes through git.
pub struct GitCli {
    dir: PathBuf,
    pub(super) objects: Option<Box<dyn CommitStore>>,
}

pub(super) struct Out {
    pub(super) ok: bool,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

impl GitCli {
    /// Opens the work tree containing `dir`.
    pub fn open(dir: &Path) -> Result<GitCli> {
        let probe = GitCli {
            dir: dir.to_path_buf(),
            objects: None,
        };
        let top = probe
            .text(&["rev-parse", "--show-toplevel"])
            .map_err(|_| Error::Usage("not inside a git work tree".into()))?;
        Ok(GitCli {
            dir: PathBuf::from(top),
            objects: None,
        })
    }

    /// Serves commit reads and unsigned commit writes from `objects` instead of spawning git.
    #[cfg(feature = "gix")]
    pub fn with_objects(mut self, objects: Box<dyn CommitStore>) -> GitCli {
        self.objects = Some(objects);
        self
    }

    /// The work tree root.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A path inside the git directory (e.g. `hooks/pre-push`), as git resolves it.
    pub fn git_path(&self, p: &str) -> Result<PathBuf> {
        let t = self.text(&["rev-parse", "--git-path", p])?;
        let pb = PathBuf::from(t);
        Ok(if pb.is_absolute() {
            pb
        } else {
            self.dir.join(pb)
        })
    }

    pub(super) fn command(&self) -> Command {
        let mut c = Command::new("git");
        c.arg("-C").arg(&self.dir);
        c.env("LC_ALL", "C");
        c.env(NESTED_ENV, "1");
        c.env_remove("GIT_DIR");
        c.env_remove("GIT_WORK_TREE");
        c.env_remove("GIT_INDEX_FILE");
        c
    }

    pub(super) fn exec(mut cmd: Command, stdin: Option<&[u8]>) -> Result<Out> {
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

    pub(super) fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        self.run_with(args, None)
    }

    pub(super) fn run_stdin(&self, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
        self.run_with(args, Some(input))
    }

    /// Spawns `git args` (with `stdin` piped in, if any) and collects the result, whatever the
    /// exit status.
    pub(super) fn output(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Out> {
        let mut c = self.command();
        c.args(args);
        Self::exec(c, stdin)
    }

    fn run_with(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
        let o = self.output(args, input)?;
        if !o.ok {
            return Err(Self::fail(args, &o));
        }
        Ok(o.stdout)
    }

    /// Runs and reports only whether git exited successfully.
    pub(super) fn succeeds(&self, args: &[&str]) -> Result<bool> {
        Ok(self.output(args, None)?.ok)
    }

    pub(super) fn text(&self, args: &[&str]) -> Result<String> {
        let out = self.run(args)?;
        Ok(String::from_utf8_lossy(&out)
            .trim_end_matches('\n')
            .to_string())
    }

    /// Stdout of a command that may legitimately fail (an unresolvable rev, a missing ref).
    pub(super) fn try_text(&self, args: &[&str]) -> Result<Option<String>> {
        let o = self.output(args, None)?;
        Ok(o.ok.then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()))
    }
}

/// Newline-separated stdin for `--stdin` style commands.
pub(super) fn lines_input(items: &[String]) -> String {
    let mut s = String::new();
    for i in items {
        s.push_str(i);
        s.push('\n');
    }
    s
}
