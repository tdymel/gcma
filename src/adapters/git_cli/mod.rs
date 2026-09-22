//! The git CLI adapter: every port is implemented by spawning the `git` plumbing commands.

mod history;
mod objects;
mod refs;
mod worktree;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::domain::error::{Error, Result};

/// A work tree reached through the `git` binary.
#[derive(Debug, Clone)]
pub struct GitCli {
    dir: PathBuf,
}

struct Out {
    ok: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl GitCli {
    /// Opens the work tree containing `dir`.
    pub fn open(dir: &Path) -> Result<GitCli> {
        let probe = GitCli {
            dir: dir.to_path_buf(),
        };
        let top = probe
            .text(&["rev-parse", "--show-toplevel"])
            .map_err(|_| Error::Usage("not inside a git work tree".into()))?;
        Ok(GitCli {
            dir: PathBuf::from(top),
        })
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

    fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut c = self.command();
        c.args(args);
        let o = self.exec(c, None)?;
        if !o.ok {
            return Err(Self::fail(args, &o));
        }
        Ok(o.stdout)
    }

    /// Runs and reports only whether git exited successfully.
    fn succeeds(&self, args: &[&str]) -> Result<bool> {
        let mut c = self.command();
        c.args(args);
        Ok(self.exec(c, None)?.ok)
    }

    fn run_stdin(&self, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
        let mut c = self.command();
        c.args(args);
        let o = self.exec(c, Some(input))?;
        if !o.ok {
            return Err(Self::fail(args, &o));
        }
        Ok(o.stdout)
    }

    fn text(&self, args: &[&str]) -> Result<String> {
        let out = self.run(args)?;
        Ok(String::from_utf8_lossy(&out)
            .trim_end_matches('\n')
            .to_string())
    }

    /// Stdout of a command that may legitimately fail (an unresolvable rev, a missing ref).
    fn try_text(&self, args: &[&str]) -> Result<Option<String>> {
        let mut c = self.command();
        c.args(args);
        let o = self.exec(c, None)?;
        Ok(o.ok.then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()))
    }
}

/// Newline-separated stdin for `--stdin` style commands.
fn lines_input(items: &[String]) -> String {
    let mut s = String::new();
    for i in items {
        s.push_str(i);
        s.push('\n');
    }
    s
}
